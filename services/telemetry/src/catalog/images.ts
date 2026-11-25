/**
 * Screenshots for Jev. The Worker cannot resize, so it picks a GameBanana variant that already
 * fits Jev's limits (280 to 1568 pixels a side, PNG or baseline JPEG) and reads the real size from
 * the file header before sending it.
 */

import type { GameBanana, Shot } from "./gamebanana";

export const MIN_SIDE = 280;
export const MAX_SIDE = 1568;
export const MAX_IMAGE_BYTES = 4 * 1024 * 1024;

export type Picture = {
  readonly format: "png" | "jpg";
  readonly width: number;
  readonly height: number;
  readonly base64: string;
};

/** Width and height from a PNG IHDR or a JPEG start-of-frame. `progressive` is set for SOF2. */
export function imageSize(
  bytes: Uint8Array,
): { format: "png" | "jpg"; width: number; height: number; progressive: boolean } | null {
  if (bytes.length > 24 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    return { format: "png", width: view.getUint32(16), height: view.getUint32(20), progressive: false };
  }
  if (bytes.length > 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    let at = 2;
    while (at + 9 < bytes.length) {
      if (bytes[at] !== 0xff) {
        at += 1;
        continue;
      }
      const marker = bytes[at + 1] ?? 0;
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
        at += 2;
        continue;
      }
      const length = ((bytes[at + 2] ?? 0) << 8) | (bytes[at + 3] ?? 0);
      const isFrame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
      if (isFrame) {
        const height = ((bytes[at + 5] ?? 0) << 8) | (bytes[at + 6] ?? 0);
        const width = ((bytes[at + 7] ?? 0) << 8) | (bytes[at + 8] ?? 0);
        return { format: "jpg", width, height, progressive: marker === 0xc2 };
      }
      at += 2 + length;
    }
  }
  return null;
}

export function fitsJev(width: number, height: number): boolean {
  return width >= MIN_SIDE && height >= MIN_SIDE && width <= MAX_SIDE && height <= MAX_SIDE;
}

/**
 * The 530-wide variant when it is tall enough, else the full file when its header says it fits.
 * Null when neither fits (the mod is then judged on text only for that picture).
 */
export async function loadShot(gb: GameBanana, shot: Shot): Promise<Picture | null> {
  const candidates: string[] = [];
  if (shot.small !== null && fitsJev(shot.small.width, shot.small.height)) candidates.push(shot.small.url);
  candidates.push(shot.full);
  for (const url of candidates) {
    const bytes = await gb.image(url, MAX_IMAGE_BYTES);
    if (bytes === null) continue;
    const size = imageSize(bytes);
    if (size === null || size.progressive || !fitsJev(size.width, size.height)) continue;
    return { format: size.format, width: size.width, height: size.height, base64: toBase64(bytes) };
  }
  return null;
}

export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  const step = 0x8000;
  for (let at = 0; at < bytes.length; at += step) {
    binary += String.fromCharCode(...bytes.subarray(at, at + step));
  }
  return btoa(binary);
}
