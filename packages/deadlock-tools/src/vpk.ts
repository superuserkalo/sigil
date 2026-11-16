import { open } from "node:fs/promises";

/**
 * VPK container reader.
 *
 * Facts come from the VPK v1/v2 format and the two reference readers we checked
 * on 2026-11-16: SteamDatabase/ValvePak and craftablescience/SourcePP (vpkpp).
 * A VPK directory file holds a tree of entry metadata. Entry data lives either
 * in the directory file itself or in numbered chunk files next to it.
 */

export const VPK_MAGIC = 0x55aa1234;
export const VPK_DIRECTORY_ARCHIVE = 0x7fff;

const HEADER_V1_SIZE = 12;
const HEADER_V2_SIZE = 28;
const ENTRY_METADATA_SIZE = 18;
const ENTRY_TERMINATOR = 0xffff;
const MAX_TREE_BYTES = 512 * 1024 * 1024;
const EMPTY_BUFFER = Buffer.alloc(0);

export type VpkHeader = {
  version: 1 | 2;
  headerSize: number;
  treeSize: number;
  fileDataSectionSize: number;
  archiveMd5SectionSize: number;
  otherMd5SectionSize: number;
  signatureSectionSize: number;
};

export type VpkEntry = {
  /** Full game path with forward slashes, for example `models/heroes_wip/billy/billy.vmdl_c`. */
  path: string;
  crc32: number;
  /** Bytes stored in the directory tree ahead of the file data. */
  preload: Buffer;
  preloadBytes: number;
  /** `VPK_DIRECTORY_ARCHIVE` when the data is in the directory file. */
  archiveIndex: number;
  /** Data offset. Relative to the file data section for the directory file, else the chunk file start. */
  offset: number;
  /** Bytes in the archive, excluding preload bytes. */
  length: number;
  totalLength: number;
};

export type VpkDirectory = {
  path: string;
  header: VpkHeader;
  entries: VpkEntry[];
  byPath: Map<string, VpkEntry>;
  skipped: number;
  duplicates: number;
};

export type ReadVpkOptions = {
  /** Keep preload bytes in memory. Indexing passes false; extraction needs them. */
  keepPreload?: boolean;
};

export async function readVpkDirectory(
  path: string,
  options: ReadVpkOptions = {},
): Promise<VpkDirectory> {
  const keepPreload = options.keepPreload !== false;
  const handle = await open(path, "r");

  try {
    const stats = await handle.stat();

    if (stats.size < HEADER_V1_SIZE) {
      throw new Error(`${path} is too small to be a VPK file.`);
    }

    const header = await readHeader(handle, stats.size, path);

    if (header.treeSize > MAX_TREE_BYTES) {
      throw new Error(`${path} has a ${header.treeSize} byte tree, above the ${MAX_TREE_BYTES} byte limit.`);
    }

    const treeStart = header.headerSize;
    const treeEnd = treeStart + header.treeSize;

    if (treeEnd > stats.size) {
      throw new Error(
        `${path} claims a ${header.treeSize} byte tree but only ${stats.size - treeStart} bytes remain.`,
      );
    }

    const tree = Buffer.alloc(header.treeSize);
    await readExactly(handle, tree, treeStart, path);

    return {
      path,
      header,
      ...parseTree(tree, keepPreload),
    };
  } finally {
    await handle.close();
  }
}

export async function readVpkEntryData(
  directory: VpkDirectory,
  entry: VpkEntry,
): Promise<Buffer> {
  const output = Buffer.alloc(entry.totalLength);

  if (entry.preload.length > 0) entry.preload.copy(output, 0);

  if (entry.length > 0) {
    const offset =
      entry.archiveIndex === VPK_DIRECTORY_ARCHIVE
        ? directory.header.headerSize + directory.header.treeSize + entry.offset
        : entry.offset;
    const source =
      entry.archiveIndex === VPK_DIRECTORY_ARCHIVE
        ? directory.path
        : chunkPathFor(directory.path, entry.archiveIndex);

    const handle = await open(source, "r");

    try {
      const stats = await handle.stat();
      if (offset + entry.length > stats.size) {
        throw new Error(
          `${entry.path}: data range ${offset}..${offset + entry.length} is past the end of ${source}.`,
        );
      }

      await readExactly(handle, output, offset, source, entry.preload.length, entry.length);
    } finally {
      await handle.close();
    }
  }

  return output;
}

export function chunkPathFor(directoryPath: string, archiveIndex: number): string {
  if (!Number.isInteger(archiveIndex) || archiveIndex < 0 || archiveIndex >= VPK_DIRECTORY_ARCHIVE) {
    throw new RangeError(`Invalid VPK archive index ${archiveIndex}`);
  }

  const directoryMatch = /^(.*)_dir\.vpk$/i.exec(directoryPath);
  const stem = directoryMatch?.[1] ?? directoryPath.replace(/\.vpk$/i, "");

  return `${stem}_${archiveIndex.toString().padStart(3, "0")}.vpk`;
}

export function crc32(data: Uint8Array): number {
  let crc = 0xffffffff;

  for (const byte of data) {
    crc = (crc >>> 8) ^ CRC_TABLE[(crc ^ byte) & 0xff]!;
  }

  return (crc ^ 0xffffffff) >>> 0;
}

const CRC_TABLE = buildCrcTable();

function buildCrcTable(): Uint32Array {
  const table = new Uint32Array(256);

  for (let index = 0; index < 256; index += 1) {
    let value = index;

    for (let bit = 0; bit < 8; bit += 1) {
      value = (value & 1) === 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }

    table[index] = value >>> 0;
  }

  return table;
}

async function readHeader(
  handle: Awaited<ReturnType<typeof open>>,
  size: number,
  path: string,
): Promise<VpkHeader> {
  const buffer = Buffer.alloc(Math.min(size, HEADER_V2_SIZE));
  await readExactly(handle, buffer, 0, path);

  if (buffer.length < HEADER_V1_SIZE) throw new Error(`${path} has a truncated VPK header.`);

  const magic = buffer.readUInt32LE(0);
  if (magic !== VPK_MAGIC) {
    throw new Error(`${path} is not a VPK file (magic 0x${magic.toString(16)}).`);
  }

  const version = buffer.readUInt32LE(4);
  const treeSize = buffer.readUInt32LE(8);

  if (version === 1) {
    return {
      version: 1,
      headerSize: HEADER_V1_SIZE,
      treeSize,
      fileDataSectionSize: 0,
      archiveMd5SectionSize: 0,
      otherMd5SectionSize: 0,
      signatureSectionSize: 0,
    };
  }

  if (version !== 2) {
    throw new Error(`${path} uses unsupported VPK version ${version}.`);
  }

  if (buffer.length < HEADER_V2_SIZE) throw new Error(`${path} has a truncated VPK version 2 header.`);

  return {
    version: 2,
    headerSize: HEADER_V2_SIZE,
    treeSize,
    fileDataSectionSize: buffer.readUInt32LE(12),
    archiveMd5SectionSize: buffer.readUInt32LE(16),
    otherMd5SectionSize: buffer.readUInt32LE(20),
    signatureSectionSize: buffer.readUInt32LE(24),
  };
}

function parseTree(
  tree: Buffer,
  keepPreload: boolean,
): Pick<VpkDirectory, "entries" | "byPath" | "skipped" | "duplicates"> {
  const entries: VpkEntry[] = [];
  const byPath = new Map<string, VpkEntry>();
  const state = { position: 0 };
  let skipped = 0;
  let duplicates = 0;

  while (true) {
    const typeName = readCString(tree, state);
    if (typeName.length === 0) break;

    while (true) {
      const directoryName = readCString(tree, state);
      if (directoryName.length === 0) break;

      const directory = directoryName === " " ? "" : directoryName;

      while (true) {
        const fileName = readCString(tree, state);
        if (fileName.length === 0) break;

        if (state.position + ENTRY_METADATA_SIZE > tree.length) {
          throw new Error("VPK directory tree ends in the middle of an entry.");
        }

        const crc32 = tree.readUInt32LE(state.position);
        const preloadBytes = tree.readUInt16LE(state.position + 4);
        const archiveIndex = tree.readUInt16LE(state.position + 6);
        const offset = tree.readUInt32LE(state.position + 8);
        const length = tree.readUInt32LE(state.position + 12);
        const terminator = tree.readUInt16LE(state.position + 16);
        state.position += ENTRY_METADATA_SIZE;

        if (terminator !== ENTRY_TERMINATOR) {
          throw new Error(
            `VPK entry ${directory}/${fileName} has terminator 0x${terminator.toString(16)}.`,
          );
        }

        if (state.position + preloadBytes > tree.length) {
          throw new Error("VPK directory tree ends in the middle of preload data.");
        }

        const preload = keepPreload
          ? Buffer.from(tree.subarray(state.position, state.position + preloadBytes))
          : EMPTY_BUFFER;
        state.position += preloadBytes;

        const path = joinVpkPath(directory, fileName, typeName);
        if (path === null) {
          skipped += 1;
          continue;
        }

        const entry: VpkEntry = {
          path,
          crc32,
          preload,
          preloadBytes,
          archiveIndex,
          offset,
          length,
          totalLength: preloadBytes + length,
        };
        const key = path.toLowerCase();

        if (byPath.has(key)) duplicates += 1;

        entries.push(entry);
        byPath.set(key, entry);
      }
    }
  }

  return { entries, byPath, skipped, duplicates };
}

function joinVpkPath(directory: string, fileName: string, typeName: string): string | null {
  if (fileName === " " || fileName.length === 0) return null;

  const file = typeName === " " ? fileName : `${fileName}.${typeName}`;
  const path = directory.length === 0 ? file : `${directory}/${file}`;

  return isSafeVpkPath(path) ? path : null;
}

export function isSafeVpkPath(path: string): boolean {
  if (path.length === 0 || path.includes("\0") || path.includes("\\")) return false;
  if (path.startsWith("/") || /^[A-Za-z]:/.test(path)) return false;

  return path.split("/").every((segment) => segment !== "" && segment !== "." && segment !== "..");
}

function readCString(tree: Buffer, state: { position: number }): string {
  const start = state.position;

  while (state.position < tree.length && tree[state.position] !== 0) {
    state.position += 1;
  }

  if (state.position >= tree.length) {
    throw new Error("VPK directory tree ends before a string terminator.");
  }

  const value = tree.toString("utf8", start, state.position);
  state.position += 1;
  return value;
}

async function readExactly(
  handle: Awaited<ReturnType<typeof open>>,
  buffer: Buffer,
  position: number,
  source: string,
  bufferOffset = 0,
  length = buffer.length,
): Promise<void> {
  let read = 0;

  while (read < length) {
    const result = await handle.read(buffer, bufferOffset + read, length - read, position + read);

    if (result.bytesRead === 0) {
      throw new Error(`${source} ended before the expected ${length} bytes could be read.`);
    }

    read += result.bytesRead;
  }
}
