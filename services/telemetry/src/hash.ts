/**
 * Small crypto helpers shared by the collectors.
 *
 * `crypto.subtle` and `crypto.getRandomValues` come from the Workers runtime, so there is no
 * dependency and no polyfill.
 */

const encoder = new TextEncoder();

export function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const byte of bytes) out += byte.toString(16).padStart(2, "0");
  return out;
}

/** Lowercase hex SHA-256 of a UTF-8 string. */
export async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(value));
  return toHex(new Uint8Array(digest));
}
