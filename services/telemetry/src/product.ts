/**
 * Opt-in product data, "Help build Sigil" (docs/decisions/0017-opt-in-product-data.md).
 *
 * The app sends typed records only after the player said Share. This file is the Worker's copy
 * of the allowlist in `rust/crates/deadlock-core/src/product_data.rs`:
 *
 * - `PRODUCT_TYPES` lists every record type. A record of any other type is dropped.
 * - `SCHEMA` lists every field of every type. A field not listed is dropped, a required field that
 *   fails its rule drops the record, an optional one that fails becomes null.
 * - Strings are only codes (`^[a-z][a-z0-9_]{0,31}$`, no four digits in a row), search text
 *   (scrubbed again here, `scrubSearch`) and hardware model names (reduced again, `modelName`).
 * - The install id is stored only as a salted SHA-256 (`installHash`). No address or user agent
 *   is read here at all.
 */

import { sha256Hex } from "./hash";

export const PRODUCT_TYPES = [
  "mod_action",
  "mod_kept",
  "enabled_set",
  "variant_chosen",
  "pack_install",
  "install_result",
  "mod_pair",
  "compat_finding",
  "repair_result",
  "off_after_crash",
  "crash_suspect",
  "rec_shown",
  "browse_query",
  "top_heroes",
  "rank_tier",
  "perf_config",
  "benchmark",
  "hardware",
  "feature_use",
  "settings",
  "launch",
  "session",
] as const;

export type ProductType = (typeof PRODUCT_TYPES)[number];

export const MAX_PRODUCT_RECORDS = 50;
export const MAX_SET = 64;
export const MAX_FILTERS = 8;
export const MAX_HEROES = 10;
export const MAX_SEARCH_CHARS = 64;
export const MAX_MODEL_CHARS = 48;

const MAX_GAMEBANANA_ID = 1_000_000_000;

type Spec =
  | { readonly k: "id" }
  | { readonly k: "code"; readonly optional?: boolean }
  | { readonly k: "codes"; readonly max: number }
  | { readonly k: "bool" }
  | { readonly k: "int"; readonly min: number; readonly max: number; readonly optional?: boolean }
  | { readonly k: "enum"; readonly values: readonly string[] }
  | { readonly k: "search" }
  | { readonly k: "model" }
  | { readonly k: "slots" }
  | { readonly k: "ids"; readonly max: number }
  | { readonly k: "heroes" };

const ID: Spec = { k: "id" };
const CODE: Spec = { k: "code" };
const CODE_OPT: Spec = { k: "code", optional: true };
const BOOL: Spec = { k: "bool" };
const BUILD: Spec = { k: "int", min: 1, max: 100_000_000, optional: true };
const RES_W: Spec = { k: "int", min: 320, max: 16384 };
const RES_H: Spec = { k: "int", min: 200, max: 16384 };

const PLAYTIME = ["under_1h", "h1_10", "h10_50", "h50_200", "over_200h"] as const;

export const SCHEMA: Readonly<Record<ProductType, Readonly<Record<string, Spec>>>> = {
  mod_action: {
    mod_id: ID,
    action: {
      k: "enum",
      values: ["installed", "removed", "enabled", "disabled", "updated", "opened", "saved", "unsaved", "not_interested"],
    },
  },
  mod_kept: { mod_id: ID, kept: { k: "enum", values: ["under_1d", "d1_7", "d7_30", "d30_90", "over_90d"] } },
  enabled_set: { mods: { k: "slots" } },
  variant_chosen: { mod_id: ID, file_id: ID },
  pack_install: { mods: { k: "ids", max: MAX_SET } },
  install_result: { mod_id: ID, ok: BOOL, reason: CODE_OPT },
  mod_pair: { a: ID, b: ID, kind: { k: "enum", values: ["overlap", "conflict"] } },
  compat_finding: { mod_id: ID, finding: CODE, game_build: BUILD },
  repair_result: { mod_id: ID, repair: CODE, ok: BOOL, game_build: BUILD },
  off_after_crash: { mod_id: ID },
  crash_suspect: { mod_id: ID },
  rec_shown: {
    row: CODE,
    position: { k: "int", min: 0, max: 100 },
    mod_id: ID,
    outcome: { k: "enum", values: ["shown", "opened", "installed"] },
  },
  browse_query: {
    sort: CODE_OPT,
    filters: { k: "codes", max: MAX_FILTERS },
    search: { k: "search" },
    results: { k: "int", min: 0, max: 1_000_000 },
  },
  top_heroes: { heroes: { k: "heroes" } },
  rank_tier: { tier: { k: "int", min: 0, max: 11 } },
  perf_config: { config: CODE, version: { k: "int", min: 0, max: 1_000_000 } },
  benchmark: {
    avg_fps: { k: "int", min: 1, max: 2000 },
    low_fps: { k: "int", min: 0, max: 2000 },
    scene: CODE,
    width: RES_W,
    height: RES_H,
    preset: CODE_OPT,
  },
  hardware: {
    gpu: { k: "model" },
    cpu: { k: "model" },
    ram_gb: { k: "int", min: 4, max: 1024 },
    width: RES_W,
    height: RES_H,
    refresh_hz: { k: "int", min: 0, max: 1000 },
  },
  feature_use: { feature: CODE, count: { k: "int", min: 1, max: 10_000 } },
  settings: { theme: CODE, density: CODE, language: CODE, mature: BOOL },
  launch: { modded: BOOL },
  session: { length: { k: "enum", values: ["under_5m", "m5_30", "m30_2h", "over_2h"] } },
};

export type ProductRecord = { readonly type: ProductType; readonly fields: Record<string, unknown> };

export type ProductBody = {
  readonly installId: string;
  readonly version: string;
  readonly os: string;
  readonly osBuild: string;
  readonly gameBuild: number | null;
  readonly records: readonly ProductRecord[];
  readonly dropped: number;
};

export type Parsed<T> = { readonly ok: true; readonly value: T } | { readonly ok: false; readonly error: string };

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SEMVER_PATTERN = /^\d{1,5}\.\d{1,5}\.\d{1,5}(?:-[0-9A-Za-z.-]{1,32})?(?:\+[0-9A-Za-z.-]{1,32})?$/;
const OS_BUILD_PATTERN = /^\d{1,6}(?:\.\d{1,6}){1,3}$/;
const OS_NAMES = ["windows", "macos", "linux", "other"];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isInt(value: unknown, min: number, max: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max;
}

function isId(value: unknown): value is number {
  return isInt(value, 1, MAX_GAMEBANANA_ID);
}

export function isCode(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[a-z][a-z0-9_]{0,31}$/.test(value) &&
    !/\d{4}/.test(value) &&
    (value.match(/\d/g)?.length ?? 0) <= 6
  );
}

const PRIVATE_RULES: readonly RegExp[] = [
  /@/,
  /[/\\]/,
  /(?:^|\s)[a-z]:/,
  /(?:^|\s)~/,
  /:\/\//,
  /\bwww\./,
  /[a-z0-9-]\.[a-z]{2,}\b/,
  /\bu:1:/,
  /\bsteam_[0-5]:/,
  /\d{5,}/,
];

/**
 * Search text, scrubbed the way the app scrubs it: no control characters, lower case, collapsed
 * whitespace, dropped whole when it looks like an email, a path, a link or a long number, then
 * cut to 64 characters. Null when it is dropped.
 */
export function scrubSearch(value: unknown): string | null {
  if (typeof value !== "string" || value.length > 4 * MAX_SEARCH_CHARS) return null;
  // eslint-disable-next-line no-control-regex
  const collapsed = value.replace(/[\u0000-\u001f\u007f-\u009f]/g, "").toLowerCase().split(/\s+/).filter(Boolean).join(" ");
  if (collapsed.length === 0) return null;
  const digits = collapsed.match(/\d/g)?.length ?? 0;
  if (digits > 8 || PRIVATE_RULES.some((rule) => rule.test(collapsed))) return null;
  const cut = Array.from(collapsed).slice(0, MAX_SEARCH_CHARS).join("").trimEnd();
  return cut.length > 0 ? cut : null;
}

/** A GPU or CPU model name, reduced the way the app reduces it. Null when it is not one. */
export function modelName(value: unknown): string | null {
  if (typeof value !== "string" || value.length > 256) return null;
  const first = value.split("/")[0] ?? "";
  if (first.includes("\\") || first.includes(":")) return null;
  let out = first
    .replace(/\((?:r|tm|c)\)|[®™©]/gi, " ")
    .replace(/@.*$/, " ")
    .replace(/\b\d+\.?\d*\s*[gm]hz\b/gi, " ")
    .replace(/\b\d+-core\b/gi, " ")
    .replace(/\b(?:cpu|processor)\b/gi, " ")
    .replace(/[^A-Za-z0-9+-]/g, " ")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
  if (out.length > MAX_MODEL_CHARS) out = out.slice(0, MAX_MODEL_CHARS).trimEnd();
  if (/\d{6}/.test(out) || !/[A-Za-z]/.test(out)) return null;
  return out;
}

/** The value for one field, or `undefined` when it fails a required rule. */
function field(spec: Spec, value: unknown): unknown {
  switch (spec.k) {
    case "id":
      return isId(value) ? value : undefined;
    case "code":
      if (isCode(value)) return value;
      return spec.optional ? null : undefined;
    case "codes": {
      if (!Array.isArray(value)) return [];
      const codes = [...new Set(value.filter(isCode))].sort();
      return codes.slice(0, spec.max);
    }
    case "bool":
      return typeof value === "boolean" ? value : undefined;
    case "int":
      if (isInt(value, spec.min, spec.max)) return value;
      return spec.optional ? null : undefined;
    case "enum":
      return typeof value === "string" && spec.values.includes(value) ? value : undefined;
    case "search":
      return scrubSearch(value);
    case "model":
      return modelName(value);
    case "ids": {
      if (!Array.isArray(value)) return undefined;
      const ids: number[] = [];
      for (const entry of value) {
        if (isId(entry) && !ids.includes(entry)) ids.push(entry);
        if (ids.length >= spec.max) break;
      }
      return ids.length > 0 ? ids : undefined;
    }
    case "slots": {
      if (!Array.isArray(value)) return undefined;
      const seen = new Set<number>();
      const slots: { mod_id: number; slot: number }[] = [];
      for (const entry of value) {
        if (!isRecord(entry) || !isId(entry.mod_id) || !isInt(entry.slot, 0, 999)) continue;
        if (seen.has(entry.mod_id)) continue;
        seen.add(entry.mod_id);
        slots.push({ mod_id: entry.mod_id, slot: entry.slot });
      }
      slots.sort((x, y) => x.slot - y.slot);
      const kept = slots.slice(0, MAX_SET);
      return kept.length > 0 ? kept : undefined;
    }
    case "heroes": {
      if (!Array.isArray(value)) return undefined;
      const heroes: { hero_id: number; playtime: string }[] = [];
      for (const entry of value) {
        if (!isRecord(entry) || !isInt(entry.hero_id, 1, 1000)) continue;
        if (typeof entry.playtime !== "string" || !(PLAYTIME as readonly string[]).includes(entry.playtime)) continue;
        heroes.push({ hero_id: entry.hero_id, playtime: entry.playtime });
      }
      const kept = heroes.slice(0, MAX_HEROES);
      return kept.length > 0 ? kept : undefined;
    }
  }
}

function isProductType(value: unknown): value is ProductType {
  return typeof value === "string" && (PRODUCT_TYPES as readonly string[]).includes(value);
}

/** One record with only its schema's fields, or null when it must be dropped. */
export function parseRecord(raw: unknown): ProductRecord | null {
  if (!isRecord(raw) || !isProductType(raw.type)) return null;
  const schema = SCHEMA[raw.type];
  const fields: Record<string, unknown> = {};
  for (const [name, spec] of Object.entries(schema)) {
    const value = field(spec, raw[name]);
    if (value === undefined) return null;
    fields[name] = value;
  }
  if (raw.type === "mod_pair") {
    const a = fields.a as number;
    const b = fields.b as number;
    if (a === b) return null;
    fields.a = Math.min(a, b);
    fields.b = Math.max(a, b);
  }
  if (raw.type === "benchmark" && (fields.low_fps as number) > (fields.avg_fps as number)) return null;
  return { type: raw.type, fields };
}

export function parseInstallId(value: unknown): string | null {
  if (typeof value !== "string" || !UUID_PATTERN.test(value)) return null;
  return value.toLowerCase();
}

export function parseProduct(raw: unknown): Parsed<ProductBody> {
  if (!isRecord(raw)) return { ok: false, error: "bad_body" };
  const installId = parseInstallId(raw.installId);
  if (installId === null) return { ok: false, error: "bad_install_id" };
  if (typeof raw.version !== "string" || !SEMVER_PATTERN.test(raw.version)) return { ok: false, error: "bad_version" };
  if (typeof raw.os !== "string" || !OS_NAMES.includes(raw.os)) return { ok: false, error: "bad_os" };
  const osBuild = typeof raw.osBuild === "string" && OS_BUILD_PATTERN.test(raw.osBuild) ? raw.osBuild : "";
  const gameBuild = isInt(raw.gameBuild, 1, 100_000_000) ? raw.gameBuild : null;
  if (!Array.isArray(raw.records)) return { ok: false, error: "bad_records" };
  if (raw.records.length === 0 || raw.records.length > MAX_PRODUCT_RECORDS) return { ok: false, error: "bad_record_count" };

  const records: ProductRecord[] = [];
  let dropped = 0;
  for (const entry of raw.records) {
    const record = parseRecord(entry);
    if (record === null) dropped += 1;
    else records.push(record);
  }
  return { ok: true, value: { installId, version: raw.version, os: raw.os, osBuild, gameBuild, records, dropped } };
}

/** Both product secrets must be at least this long, or the product routes answer 503. */
export const MIN_SECRET_CHARS = 32;

/** True when a product secret is set and long enough. */
export function secretOk(secret: string | undefined): secret is string {
  return secret !== undefined && secret.length >= MIN_SECRET_CHARS;
}

/**
 * The only form of the install id this Worker keeps: SHA-256 over the `PRODUCT_SALT` secret and
 * the id. There is no fallback: the routes refuse (503 `not_configured`) before calling this
 * without a salt. Set the salt before the first deploy and never change it, or Delete my data
 * misses older rows.
 */
export async function installHash(installId: string, salt: string): Promise<string> {
  if (!secretOk(salt)) throw new Error("PRODUCT_SALT is not configured");
  return sha256Hex(`${salt}\nsigil-product:${installId}`);
}

export function parseRelated(search: URLSearchParams): number | null {
  const text = search.get("mod") ?? "";
  if (!/^\d{1,10}$/.test(text)) return null;
  const id = Number(text);
  return isId(id) ? id : null;
}
