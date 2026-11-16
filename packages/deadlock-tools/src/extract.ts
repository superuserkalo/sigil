import { mkdir, rename, stat, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import {
  crc32,
  isSafeVpkPath,
  readVpkDirectory,
  readVpkEntryData,
} from "./vpk.ts";

export type ExtractOptions = {
  dirVpkPath: string;
  entryPaths: readonly string[];
  destination: string;
  /** Overwrite files that already exist. Default false. */
  overwrite?: boolean;
  /** Verify the VPK CRC32 before writing. Default true. */
  verifyCrc?: boolean;
  /** Hard cap for one call. Default 500. */
  maxFiles?: number;
};

export type ExtractedFile = {
  path: string;
  target: string;
  bytes: number;
};

export type ExtractIssue = {
  path: string;
  message: string;
};

export type ExtractResult = {
  destination: string;
  files: ExtractedFile[];
  skipped: ExtractIssue[];
  errors: ExtractIssue[];
  totalBytes: number;
};

export async function extractFromVpk(options: ExtractOptions): Promise<ExtractResult> {
  const maxFiles = options.maxFiles ?? 500;
  const overwrite = options.overwrite === true;
  const verifyCrc = options.verifyCrc !== false;
  const destination = resolve(options.destination);
  const result: ExtractResult = {
    destination,
    files: [],
    skipped: [],
    errors: [],
    totalBytes: 0,
  };

  const requested = [...new Set(options.entryPaths.map((path) => path.trim()).filter(Boolean))];

  if (requested.length === 0) {
    result.errors.push({ path: "", message: "No entry paths were requested." });
    return result;
  }

  if (requested.length > maxFiles) {
    result.errors.push({
      path: "",
      message: `Requested ${requested.length} files but one call is limited to ${maxFiles}.`,
    });
    return result;
  }

  const directory = await readVpkDirectory(options.dirVpkPath);

  for (const requestedPath of requested) {
    if (!isSafeVpkPath(requestedPath)) {
      result.errors.push({
        path: requestedPath,
        message: "Entry path is absolute, uses backslashes, or contains a parent segment.",
      });
      continue;
    }

    const entry = directory.byPath.get(requestedPath.toLowerCase());

    if (entry === undefined) {
      result.errors.push({ path: requestedPath, message: "Path is not in the game index." });
      continue;
    }

    const target = safeTarget(destination, entry.path);

    if (target === null) {
      result.errors.push({ path: requestedPath, message: "Path resolves outside the destination." });
      continue;
    }

    if (!overwrite && (await exists(target))) {
      result.skipped.push({ path: requestedPath, message: "File already exists. Pass overwrite to replace it." });
      continue;
    }

    try {
      const data = await readVpkEntryData(directory, entry);
      const actualCrc = crc32(data);

      if (verifyCrc && actualCrc !== entry.crc32) {
        result.errors.push({
          path: requestedPath,
          message: `CRC32 mismatch: expected 0x${entry.crc32.toString(16)}, got 0x${actualCrc.toString(16)}.`,
        });
        continue;
      }

      await writeAtomic(target, data);
      result.files.push({ path: entry.path, target, bytes: data.length });
      result.totalBytes += data.length;
    } catch (error) {
      result.errors.push({
        path: requestedPath,
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  return result;
}

export function formatExtractResult(result: ExtractResult): string {
  const lines: string[] = [];

  if (result.files.length === 0 && result.errors.length === 0) {
    lines.push(`Nothing extracted. ${result.skipped.length} file(s) already existed.`);
    return lines.join("\n");
  }

  if (result.files.length > 0) {
    lines.push(
      `Extracted ${result.files.length} file(s), ${result.totalBytes} bytes, into ${result.destination}`,
    );
    for (const file of result.files) lines.push(`  ${file.path} -> ${file.target}`);
  }

  for (const skipped of result.skipped) lines.push(`SKIPPED ${skipped.path}: ${skipped.message}`);

  for (const issue of result.errors) {
    lines.push(`ERROR ${issue.path.length === 0 ? "" : `${issue.path}: `}${issue.message}`);
  }

  return lines.join("\n");
}

function safeTarget(destination: string, entryPath: string): string | null {
  const target = resolve(destination, entryPath);
  const rel = relative(destination, target);

  if (rel.length === 0 || rel.startsWith("..") || isAbsolute(rel)) return null;
  return target;
}

async function writeAtomic(target: string, data: Buffer): Promise<void> {
  await mkdir(dirname(target), { recursive: true });
  const temporary = `${target}.${process.pid}.tmp`;
  await writeFile(temporary, data);
  await rename(temporary, target);
}

async function exists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}
