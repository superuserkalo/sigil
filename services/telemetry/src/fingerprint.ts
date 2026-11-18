/**
 * Crash fingerprints.
 *
 * A crash repeated fifty times should be one row with a count, not fifty rows. The fingerprint
 * covers the first message line and the first few backtrace frames, with addresses, line numbers
 * and columns removed so a rebuild that moves code does not look like a new crash. Everything here
 * runs on text that was already scrubbed, so no path carries a user name.
 */

import { sha256Hex } from "./hash";

const MAX_MESSAGE_CHARS = 200;
const MAX_FRAMES = 5;

/** The stable part of a backtrace: trimmed lines, no pointers, line numbers or columns. */
export function fingerprintFrames(backtrace: string): readonly string[] {
  const lines = backtrace
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .slice(0, MAX_FRAMES);

  return lines.map((line) =>
    line
      .replace(/0x[0-9a-fA-F]+/g, "0x")
      .replace(/:\d+:\d+/g, "")
      .replace(/\s+/g, " "),
  );
}

export async function crashFingerprint(message: string, backtrace: string): Promise<string> {
  const firstLine = (message.split(/\r?\n/)[0] ?? "").trim().slice(0, MAX_MESSAGE_CHARS);
  return sha256Hex([firstLine, ...fingerprintFrames(backtrace)].join("\n"));
}
