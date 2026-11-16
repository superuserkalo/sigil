import { mkdir, rm, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { crc32 } from "../src/vpk.ts";

export type FixtureEntry = {
  /** Exact game path, for example `models/heroes_wip/billy/billy.vmdl_c`. */
  path: string;
  data: Buffer;
  preload?: Buffer;
  /** Chunk index for external storage. Omit for the directory file. */
  chunk?: number;
};

export type VpkFixture = {
  directory: Buffer;
  chunks: Map<number, Buffer>;
};

export function buildVpk(entries: readonly FixtureEntry[], version: 1 | 2 = 2): VpkFixture {
  const types = new Map<string, Map<string, Array<{ name: string; entry: FixtureEntry }>>>();
  const chunks = new Map<number, Buffer[]>();
  const directoryData: Buffer[] = [];
  let directoryOffset = 0;

  for (const entry of entries) {
    const parsed = splitPath(entry.path);
    let directories = types.get(parsed.extension);
    if (directories === undefined) {
      directories = new Map();
      types.set(parsed.extension, directories);
    }

    let files = directories.get(parsed.directory);
    if (files === undefined) {
      files = [];
      directories.set(parsed.directory, files);
    }

    files.push({ name: parsed.name, entry });
  }

  const tree: Buffer[] = [];

  for (const [extension, directories] of types) {
    tree.push(cstring(extension));

    for (const [directory, files] of directories) {
      tree.push(cstring(directory));

      for (const file of files) {
        const preload = file.entry.preload ?? Buffer.alloc(0);
        const archiveIndex =
          file.entry.chunk === undefined ? 0x7fff : file.entry.chunk;
        let offset: number;

        if (archiveIndex === 0x7fff) {
          offset = directoryOffset;
          directoryData.push(file.entry.data);
          directoryOffset += file.entry.data.length;
        } else {
          const list = chunks.get(archiveIndex) ?? [];
          offset = list.reduce((total, part) => total + part.length, 0);
          list.push(file.entry.data);
          chunks.set(archiveIndex, list);
        }

        const metadata = Buffer.alloc(18);
        metadata.writeUInt32LE(crc32(Buffer.concat([preload, file.entry.data])), 0);
        metadata.writeUInt16LE(preload.length, 4);
        metadata.writeUInt16LE(archiveIndex, 6);
        metadata.writeUInt32LE(offset, 8);
        metadata.writeUInt32LE(file.entry.data.length, 12);
        metadata.writeUInt16LE(0xffff, 16);

        tree.push(cstring(file.name), metadata, preload);
      }

      tree.push(Buffer.from([0]));
    }

    tree.push(Buffer.from([0]));
  }

  tree.push(Buffer.from([0]));

  const treeBuffer = Buffer.concat(tree);
  const dataBuffer = Buffer.concat(directoryData);
  const header =
    version === 2
      ? (() => {
          const buffer = Buffer.alloc(28);
          buffer.writeUInt32LE(0x55aa1234, 0);
          buffer.writeUInt32LE(2, 4);
          buffer.writeUInt32LE(treeBuffer.length, 8);
          buffer.writeUInt32LE(dataBuffer.length, 12);
          buffer.writeUInt32LE(0, 16);
          buffer.writeUInt32LE(0, 20);
          buffer.writeUInt32LE(0, 24);
          return buffer;
        })()
      : (() => {
          const buffer = Buffer.alloc(12);
          buffer.writeUInt32LE(0x55aa1234, 0);
          buffer.writeUInt32LE(1, 4);
          buffer.writeUInt32LE(treeBuffer.length, 8);
          return buffer;
        })();

  return {
    directory: Buffer.concat([header, treeBuffer, dataBuffer]),
    chunks: new Map([...chunks].map(([index, parts]) => [index, Buffer.concat(parts)])),
  };
}

export async function writeFixture(root: string, fixture: VpkFixture): Promise<string> {
  await mkdir(root, { recursive: true });
  const directoryPath = join(root, "pak01_dir.vpk");
  await writeFile(directoryPath, fixture.directory);

  for (const [index, data] of fixture.chunks) {
    await writeFile(join(root, `pak01_${index.toString().padStart(3, "0")}.vpk`), data);
  }

  return directoryPath;
}

export async function withTempFixture(
  fixture: VpkFixture,
  run: (root: string, directoryPath: string) => Promise<void>,
): Promise<void> {
  const root = join(process.cwd(), ".scratch", "tmp", `vpk-${randomUUID()}`);
  await mkdir(root, { recursive: true });

  try {
    const directoryPath = await writeFixture(root, fixture);
    await run(root, directoryPath);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

function splitPath(path: string): { extension: string; directory: string; name: string } {
  const slash = path.lastIndexOf("/");
  const directory = slash === -1 ? " " : path.slice(0, slash);
  const file = slash === -1 ? path : path.slice(slash + 1);
  const dot = file.lastIndexOf(".");

  if (dot <= 0) return { extension: " ", directory, name: file };

  return {
    extension: file.slice(dot + 1),
    directory,
    name: file.slice(0, dot),
  };
}

function cstring(value: string): Buffer {
  return Buffer.from(`${value}\0`, "utf8");
}
