/**
 * Optional document embeddings for the index (flag `INDEX_EMBEDDINGS = "1"`): Workers AI
 * `bge-small-en-v1.5` (384 dimensions) over the mod's title and kept terms, reduced to 256
 * dimensions by a fixed random projection (Johnson-Lindenstrauss, seeded, so every build and the
 * app use the same matrix), normalised, int8, base64. Public catalog text only. The app does not
 * use them yet: querying them needs the same model on the PC (ADR 0018, possible later addition).
 */

import type { AiRunner } from "./expand";

export const EMBED_MODEL = "@cf/baai/bge-small-en-v1.5";
export const EMBED_IN = 384;
export const EMBED_DIMS = 256;
export const EMBED_SEED = 0x51617;

export function embeddingsOn(env: Env): boolean {
  return (env as Env & { readonly INDEX_EMBEDDINGS?: string }).INDEX_EMBEDDINGS === "1";
}

let matrix: Int8Array | null = null;

/** A fixed +1/-1 matrix from a seeded xorshift generator. */
export function projection(): Int8Array {
  if (matrix !== null) return matrix;
  const out = new Int8Array(EMBED_IN * EMBED_DIMS);
  let state = EMBED_SEED;
  for (let at = 0; at < out.length; at += 1) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    out[at] = (state & 1) === 0 ? 1 : -1;
  }
  matrix = out;
  return out;
}

export function reduce(vector: readonly number[]): Int8Array {
  const weights = projection();
  const reduced = new Float64Array(EMBED_DIMS);
  for (let row = 0; row < EMBED_DIMS; row += 1) {
    let sum = 0;
    for (let col = 0; col < EMBED_IN; col += 1) sum += (weights[row * EMBED_IN + col] ?? 0) * (vector[col] ?? 0);
    reduced[row] = sum;
  }
  const norm = Math.sqrt(reduced.reduce((total, value) => total + value * value, 0)) || 1;
  return Int8Array.from(reduced, (value) => Math.max(-127, Math.min(127, Math.round((value / norm) * 127 * 4))));
}

export async function embedText(ai: AiRunner, text: string): Promise<string | null> {
  const raw = (await ai.run(EMBED_MODEL, { text: [text.slice(0, 1000)] })) as { data?: number[][] } | null;
  const vector = raw?.data?.[0];
  if (!Array.isArray(vector) || vector.length !== EMBED_IN) return null;
  const bytes = new Uint8Array(reduce(vector).buffer);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
