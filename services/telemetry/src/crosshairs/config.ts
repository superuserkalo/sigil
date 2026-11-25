/**
 * The crosshair settings a community crosshair carries (ADR 0019), mirrored from
 * `rust/crates/deadlock-core/src/crosshair.rs` (`Crosshair`, `Crosshair::validate`).
 *
 * Keys are the camelCase names the app's `Crosshair` serialises to. All 19 are required, each with
 * its own kind and range; any other key fails the whole config. `disableHeroSpecificCrosshairs` is
 * the one exception: it is a player preference, not part of how a crosshair looks (share codes
 * leave it out too), so it is accepted as a boolean and dropped, never stored.
 *
 * The stored form is canonical: keys in the order below, opacities rounded to 0.01 the way the
 * game writes them (`citadel_crosshair_pip_opacity 0.70`), whole numbers as whole numbers. Two
 * configs that look the same in game have the same canonical text and the same SHA-256.
 */

import { sha256Hex } from "../hash";

type Spec = { readonly k: "int"; readonly min: number; readonly max: number } | { readonly k: "opacity" } | { readonly k: "bool" };

const SIZE: Spec = { k: "int", min: 0, max: 64 };
const OUTLINE: Spec = { k: "int", min: 0, max: 16 };
const OPACITY: Spec = { k: "opacity" };
const CHANNEL: Spec = { k: "int", min: 0, max: 255 };
const BOOL: Spec = { k: "bool" };

/** Every key, in canonical order (the field order of the Rust struct). */
export const CONFIG_SCHEMA = {
  pipGap: SIZE,
  pipGapStatic: BOOL,
  pipHeight: SIZE,
  pipWidth: SIZE,
  pipOpacity: OPACITY,
  pipOutlineBorder: OUTLINE,
  pipOutlineGap: OUTLINE,
  pipOutlineOpacity: OPACITY,
  dotOpacity: OPACITY,
  dotSize: SIZE,
  dotOutlineBorder: OUTLINE,
  dotOutlineGap: OUTLINE,
  dotOutlineOpacity: OPACITY,
  colorR: CHANNEL,
  colorG: CHANNEL,
  colorB: CHANNEL,
  outlineColorR: CHANNEL,
  outlineColorG: CHANNEL,
  outlineColorB: CHANNEL,
} as const satisfies Record<string, Spec>;

export type ConfigKey = keyof typeof CONFIG_SCHEMA;
export type CrosshairConfig = { readonly [K in ConfigKey]: (typeof CONFIG_SCHEMA)[K] extends { k: "bool" } ? boolean : number };

const KEYS = Object.keys(CONFIG_SCHEMA) as ConfigKey[];
/** Accepted and dropped: a preference, not part of the look. */
const IGNORED_KEYS: readonly string[] = ["disableHeroSpecificCrosshairs"];

export type ConfigResult = { readonly ok: true; readonly config: CrosshairConfig } | { readonly ok: false; readonly field: string };

function value(spec: Spec, raw: unknown): number | boolean | undefined {
  switch (spec.k) {
    case "bool":
      return typeof raw === "boolean" ? raw : undefined;
    case "int":
      return typeof raw === "number" && Number.isSafeInteger(raw) && raw >= spec.min && raw <= spec.max ? raw : undefined;
    case "opacity": {
      if (typeof raw !== "number" || !Number.isFinite(raw) || raw < 0 || raw > 1) return undefined;
      // As the app stores it (`(x * 100).round() / 100`); `+ 0` turns -0 into 0.
      return Math.round(raw * 100) / 100 + 0;
    }
  }
}

/**
 * The canonical config, or the first key that fails: an unknown key, a missing key, a value of the
 * wrong kind or out of range. `field` is `config` when the value is not an object at all.
 */
export function parseConfig(raw: unknown): ConfigResult {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return { ok: false, field: "config" };
  const input = raw as Record<string, unknown>;
  for (const key of Object.keys(input)) {
    if (!(key in CONFIG_SCHEMA) && !IGNORED_KEYS.includes(key)) return { ok: false, field: key };
  }
  for (const key of IGNORED_KEYS) {
    if (key in input && typeof input[key] !== "boolean") return { ok: false, field: key };
  }
  const out: Record<string, number | boolean> = {};
  for (const key of KEYS) {
    if (!Object.hasOwn(input, key)) return { ok: false, field: key };
    const parsed = value(CONFIG_SCHEMA[key], input[key]);
    if (parsed === undefined) return { ok: false, field: key };
    out[key] = parsed;
  }
  return { ok: true, config: out as CrosshairConfig };
}

/** The stored text: keys in canonical order, no whitespace. */
export function canonicalText(config: CrosshairConfig): string {
  const ordered: Record<string, number | boolean> = {};
  for (const key of KEYS) ordered[key] = config[key];
  return JSON.stringify(ordered);
}

export async function configHash(config: CrosshairConfig): Promise<string> {
  return sha256Hex(canonicalText(config));
}
