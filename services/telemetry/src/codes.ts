/**
 * The support code returned with a diagnostics bundle.
 *
 * The alphabet has 32 characters and leaves out 0, O, 1 and I, so a player can read a code over
 * the phone or copy it out of a screenshot without guessing. 32 divides 256 evenly, so reducing a
 * random byte modulo 32 stays uniform.
 */

const CODE_ALPHABET = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

export const CODE_PATTERN = /^SGL-[2-9A-HJ-NP-Z]{4}-[2-9A-HJ-NP-Z]{4}$/;

export function diagnosticsCode(): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  let body = "";
  for (const byte of bytes) body += CODE_ALPHABET.charAt(byte % 32);
  return `SGL-${body.slice(0, 4)}-${body.slice(4)}`;
}
