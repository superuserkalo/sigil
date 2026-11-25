/**
 * Aggregates from product data, and the rule that decides when one may be shown to players
 * (ADR 0017, "Counting distinct contributors").
 *
 * An install id is a UUID the app makes, so one person could send many made-up ids. The rule:
 *
 * 1. Epochs. Time is cut into 30-day epochs (`EPOCH_MS`). Each request gets a network bucket:
 *    HMAC-SHA256 under the Worker secret `PRODUCT_BUCKET_KEY` of the epoch number and the caller's
 *    network prefix (IPv4 /24, IPv6 /48), cut to 16 hex characters. A network's bucket changes
 *    every epoch, so it cannot be linked across epochs. The address and the prefix are never
 *    stored. Without the secret no bucket is made and nothing is published (fail closed); the
 *    routes in `index.ts` answer 503 before getting here.
 * 2. Open-epoch contributions. `product_contrib` is the only table where a bucket sits next to an
 *    install hash. One row per (epoch, kind, key, install): kind `seen` (key = the UTC day, one
 *    row per day the install sent anything), `mod`, `pair`, `health`, `hardware`, `bench`,
 *    `search`. A bucket admits at most `INSTALLS_PER_BUCKET` distinct installs per epoch, counted
 *    from the table itself, so deleting an install frees its slot and nothing grows forever.
 * 3. Closing an epoch (daily cron, `GRACE_MS` after its end). For each (kind, key): eligible
 *    installs (seen on 2 or more days in the epoch, among the first 3 of their bucket), distinct
 *    buckets and distinct days go to `product_totals` as counts only, with no install hash and no
 *    bucket. Then the epoch's contribution rows are deleted.
 * 4. Publishing. A key is shown only when at least one closed epoch met the floor on its own:
 *    `FLOOR.installs` eligible installs from `FLOOR.buckets` buckets on `FLOOR.days` days. Its
 *    strength is the sum over epochs that met the floor; other epochs add nothing. The open epoch
 *    is never published. Related mods rank by a damped cosine score.
 */

import { toHex } from "./hash";
import { secretOk, type ProductRecord } from "./product";

const DAY_MS = 24 * 60 * 60 * 1000;
export const EPOCH_MS = 30 * DAY_MS;
/** Late requests for an epoch still land in it for this long before it closes. */
export const GRACE_MS = 2 * DAY_MS;
export const FLOOR = { installs: 10, buckets: 6, days: 3 } as const;
export const INSTALLS_PER_BUCKET = 3;
export const MIN_DAYS_SEEN = 2;
/** Damper: a pair with support k scores half of what infinite support would. */
const DAMPING = 10;
export const MAX_RELATED = 20;
/** One request writes at most this many contribution rows. */
const MAX_CONTRIB_ROWS = 600;
/** Rows one install may hold per epoch and kind, so one client cannot bloat an epoch. */
export const PER_INSTALL_CAP: Readonly<Record<string, number>> = {
  seen: 40,
  mod: 64,
  pair: 500,
  health: 100,
  bench: 20,
  hardware: 5,
  search: 50,
  attempt: 64,
  use: 64,
  conflict: 200,
  fps: 20,
  search_zero: 50,
  search_few: 50,
  install_day: 64,
};
/** Rising: a mod needs this many installs in the last two weeks of the epoch to rank at all. */
export const RISING_MIN_LATE = 5;
export const MAX_RISING = 50;
/** Health and conflicts read only the closed epochs this recent (about 90 days), so a mod that
 * was fixed stops carrying old reports. */
export const RECENT_EPOCHS = 3;
/** A Browse search with at most this many results counts as a search gap. */
export const FEW_RESULTS = 5;
/** Install failure reasons that point at the mod, not at the player's PC or network. */
export const MOD_REASONS: ReadonlySet<string> = new Set(["nothing_installable", "several_vpks", "other", "unknown"]);
/** Health codes that mean "this mod did not work on that game build". */
export const BROKEN_CODES: ReadonlySet<string> = new Set(["moved", "gone", "skeleton", "off_after_crash", "crash_suspect"]);
/** The most mods one health request may ask about. Two IN lists of this size stay under D1's
 * 100 bound values. */
export const MAX_HEALTH_MODS = 40;
/** Contribution rows deleted per statement and statements per closing run, so the cron job
 * stays short; an epoch that does not finish closing continues the next day. */
const DELETE_CHUNK = 5000;
const DELETE_BUDGET = 40;
/** Kinds whose keys are written by players (search text, hardware names). */
export const FREE_TEXT_KINDS = ["search", "hardware", "fps", "search_zero", "search_few"] as const;

export type Contributor = { readonly hash: string; readonly bucket: string; readonly epoch: number; readonly day: string };

export function epochOf(now: number): number {
  return Math.floor(now / EPOCH_MS);
}

export function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}

/** Without the bucket secret nothing is aggregated or published (fail closed). */
export function hasBucketKey(secret: string | undefined): secret is string {
  return secretOk(secret);
}

/** The caller's network prefix: IPv4 /24 or IPv6 /48, or null when there is no usable address. */
export function networkPrefix(address: string | null): string | null {
  if (address === null) return null;
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(address);
  if (v4 !== null) {
    const parts = v4.slice(1, 4).map(Number);
    if (parts.some((part) => part > 255)) return null;
    return `v4:${parts.join(".")}`;
  }
  if (!address.includes(":") || !/^[0-9a-fA-F:.]+$/.test(address)) return null;
  const [head = "", tail] = address.toLowerCase().split("::");
  const front = head === "" ? [] : head.split(":");
  const back = tail === undefined || tail === "" ? [] : tail.split(":");
  const fill = tail === undefined ? [] : Array(Math.max(0, 8 - front.length - back.length)).fill("0");
  const groups = [...front, ...fill, ...back];
  if (groups.length < 3 || groups.slice(0, 3).some((g) => !/^[0-9a-f]{1,4}$/.test(g))) return null;
  return `v6:${groups
    .slice(0, 3)
    .map((g) => g.padStart(4, "0"))
    .join(":")}`;
}

const encoder = new TextEncoder();

/** HMAC-SHA256(secret, epoch + prefix), 16 hex characters. Null without the secret or a prefix. */
export async function bucketOf(address: string | null, secret: string | undefined, epoch: number): Promise<string | null> {
  if (!hasBucketKey(secret)) return null;
  const prefix = networkPrefix(address);
  if (prefix === null) return null;
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = await crypto.subtle.sign("HMAC", key, encoder.encode(`sigil-product-bucket\n${epoch}\n${prefix}`));
  return toHex(new Uint8Array(mac)).slice(0, 16);
}

/** One aggregate key. `value` and `value2` only for kind `fps`: average and 1% low frame rate. */
export type Contribution = { readonly kind: string; readonly key: string; readonly value?: number; readonly value2?: number };

/** What a record's keys need besides the record: the request's game build and, for a benchmark,
 * the install's hardware class and the versions of the configs it applied. */
export type KeyContext = {
  readonly gameBuild: number | null;
  readonly fps?: FpsClass | null;
};

/** A benchmark's hardware class (GPU model, CPU tier) and the versions of the configs applied. */
export type FpsClass = { readonly gpu: string; readonly tier: string; readonly versions: ReadonlyMap<string, number> };

const NO_CONTEXT: KeyContext = { gameBuild: null };

/**
 * A coarse CPU tier from a reduced model name: `high`, `mid`, `low` or `other`. The hardware class
 * is the GPU model and this tier, so frame rates group PCs that play alike without a CPU table.
 */
export function cpuTier(name: string | null | undefined): string {
  if (typeof name !== "string") return "other";
  const n = name.toLowerCase();
  if (/x3d\b/.test(n)) return "high";
  const ryzen = /ryzen\s+(?:ai\s+)?(\d)\s+(?:pro\s+)?(\d)(\d)\d{2}/.exec(n);
  if (ryzen !== null) {
    const [family, gen] = [Number(ryzen[1]), Number(ryzen[2])];
    if (family >= 7 && gen >= 5) return "high";
    if (family >= 5 && gen >= 3) return "mid";
    return "low";
  }
  const ultra = /core\s+ultra\s+(\d)/.exec(n);
  if (ultra !== null) return Number(ultra[1]) >= 7 ? "high" : "mid";
  const core = /\bi(\d)[\s-]+(\d{4,5})/.exec(n);
  if (core !== null) {
    const family = Number(core[1]);
    const digits = core[2] as string;
    const gen = digits.length === 5 ? Number(digits.slice(0, 2)) : Number(digits.slice(0, 1));
    if (family >= 7 && gen >= 12) return "high";
    if (family >= 5 && gen >= 10) return "mid";
    return "low";
  }
  if (/\bapple\s+m\d/.test(n)) return "mid";
  if (/\b(?:celeron|pentium|athlon|atom)\b/.test(n)) return "low";
  return "other";
}

/** A resolution bucket from a height in pixels. */
export function resolutionBucket(height: number): string {
  if (height < 1000) return "720p";
  if (height < 1300) return "1080p";
  if (height < 1800) return "1440p";
  return "2160p";
}

/** The aggregate keys one record contributes to. */
export function keysOf(record: ProductRecord, ctx: KeyContext = NO_CONTEXT): Contribution[] {
  const f = record.fields;
  const build = ctx.gameBuild ?? 0;
  switch (record.type) {
    case "enabled_set": {
      const mods = (f.mods as { mod_id: number }[]).slice(0, 24).map((m) => m.mod_id);
      const keys: Contribution[] = mods.map((mod) => ({ kind: "mod", key: String(mod) }));
      for (let i = 0; i < mods.length; i += 1) {
        for (let j = i + 1; j < mods.length; j += 1) {
          const a = Math.min(mods[i] as number, mods[j] as number);
          const b = Math.max(mods[i] as number, mods[j] as number);
          keys.push({ kind: "pair", key: `${a}|${b}` });
        }
      }
      // Installs that use the mod on this game build: the base of "reported broken".
      if (build > 0) for (const mod of mods) keys.push({ kind: "use", key: `${mod}|${build}` });
      return keys;
    }
    case "compat_finding":
      return [{ kind: "health", key: `${f.mod_id}|${f.finding}|${f.game_build ?? build}` }];
    case "repair_result":
      return [{ kind: "health", key: `${f.mod_id}|repair_${f.repair}|${f.ok ? 1 : 0}` }];
    case "install_result": {
      const reason = typeof f.reason === "string" ? f.reason : "unknown";
      // Cancelled, offline, game running, disk full: not the mod's fault, not an attempt.
      const counts = f.ok === true || MOD_REASONS.has(reason);
      const keys: Contribution[] = counts ? [{ kind: "attempt", key: String(f.mod_id) }] : [];
      if (f.ok !== true) {
        keys.push({ kind: "health", key: `${f.mod_id}|install_${reason}` });
        if (MOD_REASONS.has(reason)) keys.push({ kind: "health", key: `${f.mod_id}|install_failed` });
      }
      return keys;
    }
    case "mod_action":
      // The contribution row keeps the day of its first send in the epoch: the install day.
      return f.action === "installed" ? [{ kind: "install_day", key: String(f.mod_id) }] : [];
    case "off_after_crash":
      return [{ kind: "health", key: `${f.mod_id}|off_after_crash|${build}` }];
    case "crash_suspect":
      return [{ kind: "health", key: `${f.mod_id}|crash_suspect|${build}` }];
    case "mod_pair":
      return [{ kind: "conflict", key: `${f.a}|${f.b}` }];
    case "benchmark": {
      const keys: Contribution[] = [{ kind: "bench", key: `${f.scene}|${f.width}x${f.height}|${f.preset ?? ""}` }];
      if (ctx.fps) {
        const config = typeof f.preset === "string" ? f.preset : "none";
        const version = ctx.fps.versions.get(config) ?? 0;
        keys.push({
          kind: "fps",
          key: `${ctx.fps.gpu}|${ctx.fps.tier}|${config}|${version}|${resolutionBucket(f.height as number)}`,
          value: f.avg_fps as number,
          value2: f.low_fps as number,
        });
      }
      return keys;
    }
    case "hardware":
      return f.gpu === null ? [] : [{ kind: "hardware", key: `${f.gpu}|${f.width}x${f.height}` }];
    case "browse_query": {
      if (typeof f.search !== "string") return [];
      const keys: Contribution[] = [{ kind: "search", key: f.search }];
      const results = f.results as number;
      if (results === 0) keys.push({ kind: "search_zero", key: f.search });
      else if (results <= FEW_RESULTS) keys.push({ kind: "search_few", key: f.search });
      return keys;
    }
    default:
      return [];
  }
}

/**
 * The hardware class and config versions a benchmark is filed under, from the install's own raw
 * records (the latest `hardware` and `perf_config`). Null without a GPU model.
 */
export async function fpsContext(db: D1Database, hash: string): Promise<FpsClass | null> {
  const { results } = await db
    .prepare(
      `SELECT type, payload FROM product_events
       WHERE install_hash = ?1 AND type IN ('hardware', 'perf_config')
       ORDER BY id DESC LIMIT 40`,
    )
    .bind(hash)
    .all<{ type: string; payload: string }>();
  let gpu: string | null = null;
  let tier = "other";
  let seenHardware = false;
  const versions = new Map<string, number>();
  for (const row of results) {
    let fields: Record<string, unknown>;
    try {
      fields = JSON.parse(row.payload) as Record<string, unknown>;
    } catch {
      continue;
    }
    if (row.type === "hardware" && !seenHardware) {
      seenHardware = true;
      gpu = typeof fields.gpu === "string" ? fields.gpu : null;
      tier = cpuTier(typeof fields.cpu === "string" ? fields.cpu : null);
    } else if (row.type === "perf_config" && typeof fields.config === "string" && !versions.has(fields.config)) {
      versions.set(fields.config, typeof fields.version === "number" ? fields.version : 0);
    }
  }
  return gpu === null ? null : { gpu, tier, versions };
}

/**
 * Add one request's contributions to the open epoch. Nothing without the secret or an address,
 * and nothing when the bucket already holds `INSTALLS_PER_BUCKET` other installs this epoch.
 * Returns whether the install counted.
 */
export async function contribute(
  db: D1Database,
  secret: string | undefined,
  address: string | null,
  hash: string,
  records: readonly ProductRecord[],
  now: number,
  ctx: KeyContext = NO_CONTEXT,
): Promise<boolean> {
  const epoch = epochOf(now);
  const bucket = await bucketOf(address, secret, epoch);
  if (bucket === null) return false;
  const who: Contributor = { hash, bucket, epoch, day: utcDay(now) };

  const slot = await db
    .prepare(
      `SELECT COUNT(DISTINCT install_hash) AS installs, COALESCE(SUM(install_hash = ?3), 0) AS mine
       FROM product_contrib WHERE epoch = ?1 AND bucket = ?2`,
    )
    .bind(epoch, bucket, hash)
    .first<{ installs: number; mine: number }>();
  if ((slot?.mine ?? 0) === 0 && (slot?.installs ?? 0) >= INSTALLS_PER_BUCKET) return false;

  const { results: held } = await db
    .prepare("SELECT kind, COUNT(*) AS n FROM product_contrib WHERE epoch = ?1 AND install_hash = ?2 GROUP BY kind")
    .bind(epoch, hash)
    .all<{ kind: string; n: number }>();
  const room = new Map(Object.entries(PER_INSTALL_CAP));
  for (const row of held) room.set(row.kind, (room.get(row.kind) ?? 0) - row.n);
  const rows: Contribution[] = [{ kind: "seen", key: who.day }, ...records.flatMap((record) => keysOf(record, ctx))]
    .filter(({ kind }) => {
      const left = room.get(kind) ?? 0;
      room.set(kind, left - 1);
      return left > 0;
    })
    .slice(0, MAX_CONTRIB_ROWS);
  if (rows.length === 0) return true;
  await db.batch(
    rows.map(({ kind, key, value, value2 }) =>
      db
        .prepare(
          // A later benchmark for the same key replaces the install's frame rates; nothing else
          // about the row changes, so its bucket and first day stay.
          `INSERT INTO product_contrib (epoch, kind, key, install_hash, bucket, day, created_at, value, value2)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
           ON CONFLICT(epoch, kind, key, install_hash) DO UPDATE SET
             value = COALESCE(excluded.value, product_contrib.value),
             value2 = COALESCE(excluded.value2, product_contrib.value2)`,
        )
        .bind(epoch, kind, key.slice(0, 200), who.hash, who.bucket, who.day, now, value ?? null, value2 ?? null),
    ),
  );
  return true;
}

/**
 * Close every epoch that ended more than `GRACE_MS` ago. Kind by kind: the totals of keys that
 * met the floor in that epoch go to `product_totals` (counts only), then the kind's contribution
 * rows are deleted in chunks. Keys below the floor leave nothing behind. `seen` rows go last,
 * since every kind's eligibility reads them. Work is bounded per run; an unfinished epoch carries
 * on the next day, and a kind's totals, once written, are never recomputed from fewer rows.
 * Returns the epochs fully closed.
 */
export async function closeEpochs(db: D1Database, now: number): Promise<number[]> {
  const last = epochOf(now - GRACE_MS) - 1;
  const { results } = await db
    .prepare("SELECT DISTINCT epoch FROM product_contrib WHERE epoch <= ?1 ORDER BY epoch")
    .bind(last)
    .all<{ epoch: number }>();
  let budget = DELETE_BUDGET;
  const closed: number[] = [];
  for (const { epoch } of results) {
    const { results: kinds } = await db
      .prepare("SELECT DISTINCT kind FROM product_contrib WHERE epoch = ?1 AND kind <> 'seen'")
      .bind(epoch)
      .all<{ kind: string }>();
    for (const { kind } of kinds) {
      await db
        .prepare(
          `${ELIGIBLE}
           INSERT INTO product_totals (epoch, kind, key, mod_a, mod_b, installs, buckets, days, created_at)
           SELECT ?1, c.kind, c.key,
             CASE WHEN c.kind IN ('pair', 'conflict', 'health', 'use') THEN CAST(substr(c.key, 1, instr(c.key, '|') - 1) AS INTEGER)
                  WHEN c.kind IN ('mod', 'attempt', 'install_day') THEN CAST(c.key AS INTEGER) END,
             CASE WHEN c.kind IN ('pair', 'conflict') THEN CAST(substr(c.key, instr(c.key, '|') + 1) AS INTEGER) END,
             COUNT(DISTINCT c.install_hash), COUNT(DISTINCT c.bucket), COUNT(DISTINCT c.day), ?5
           FROM product_contrib c JOIN eligible e ON e.install_hash = c.install_hash
           WHERE c.epoch = ?1 AND c.kind = ?2
           GROUP BY c.key
           HAVING COUNT(DISTINCT c.install_hash) >= ?6 AND COUNT(DISTINCT c.bucket) >= ?7 AND COUNT(DISTINCT c.day) >= ?8
           ON CONFLICT(epoch, kind, key) DO NOTHING`,
        )
        .bind(epoch, kind, MIN_DAYS_SEEN, INSTALLS_PER_BUCKET, now, FLOOR.installs, FLOOR.buckets, FLOOR.days)
        .run();
      if (kind === "fps") await fillMedians(db, epoch);
      if (kind === "install_day") await fillWeeks(db, epoch);
      budget = await deleteChunks(db, epoch, kind, budget);
      if (budget <= 0) return closed;
    }
    budget = await deleteChunks(db, epoch, "seen", budget);
    if (budget <= 0) return closed;
    closed.push(epoch);
  }
  return closed;
}

/** The installs of epoch ?1 that count: seen on ?3 or more days, among the first ?4 of their
 * bucket. Shared by closing and by the frame rate medians. */
const ELIGIBLE = `WITH seen AS (
     SELECT install_hash, bucket, MIN(created_at) AS first_at FROM product_contrib
     WHERE epoch = ?1 AND kind = 'seen' GROUP BY install_hash, bucket
   ),
   ranked AS (
     SELECT install_hash, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY first_at, install_hash) AS place FROM seen
   ),
   days AS (
     SELECT install_hash FROM product_contrib WHERE epoch = ?1 AND kind = 'seen'
     GROUP BY install_hash HAVING COUNT(DISTINCT key) >= ?3
   ),
   eligible AS (
     SELECT DISTINCT r.install_hash FROM ranked r JOIN days d ON d.install_hash = r.install_hash WHERE r.place <= ?4
   )`;

/** The median of a list of whole numbers, rounded. */
export function median(values: readonly number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  if (sorted.length === 0) return 0;
  return sorted.length % 2 === 1 ? (sorted[mid] as number) : Math.round(((sorted[mid - 1] as number) + (sorted[mid] as number)) / 2);
}

/**
 * Frame rate medians for the `fps` keys of an epoch that met the floor: the median average and
 * the median 1% low over the eligible installs. Only the two medians are written; no single
 * install's value leaves the contribution table, which is deleted right after.
 */
async function fillMedians(db: D1Database, epoch: number): Promise<void> {
  const { results } = await db
    .prepare(
      `${ELIGIBLE}
       SELECT c.key AS key, c.value AS value, c.value2 AS value2
       FROM product_contrib c JOIN eligible e ON e.install_hash = c.install_hash
       WHERE c.epoch = ?1 AND c.kind = 'fps' AND c.value IS NOT NULL
         AND c.key IN (SELECT key FROM product_totals WHERE epoch = ?1 AND kind = 'fps' AND median IS NULL)`,
    )
    .bind(epoch, null, MIN_DAYS_SEEN, INSTALLS_PER_BUCKET)
    .all<{ key: string; value: number; value2: number | null }>();
  const byKey = new Map<string, { avg: number[]; low: number[] }>();
  for (const row of results) {
    const entry = byKey.get(row.key) ?? { avg: [], low: [] };
    entry.avg.push(row.value);
    entry.low.push(row.value2 ?? 0);
    byKey.set(row.key, entry);
  }
  if (byKey.size === 0) return;
  await db.batch(
    [...byKey].map(([key, entry]) =>
      db
        .prepare("UPDATE product_totals SET median = ?3, low = ?4 WHERE epoch = ?1 AND kind = 'fps' AND key = ?2")
        .bind(epoch, key, median(entry.avg), median(entry.low)),
    ),
  );
}

/** The bucket of a day of an epoch: days 0-6, 7-13, 14-15, 16-22, 23-29. The first two and the
 * last two are whole weeks, so Rising compares like with like. */
export function weekOf(dayIndex: number): number {
  if (dayIndex < 7) return 0;
  if (dayIndex < 14) return 1;
  if (dayIndex < 16) return 2;
  if (dayIndex < 23) return 3;
  return 4;
}

/**
 * Install counts by week for the `install_day` keys of an epoch that met the floor: eligible
 * installs only, five whole numbers per mod. No install hash, bucket or day leaves the
 * contribution table, which is deleted right after.
 */
async function fillWeeks(db: D1Database, epoch: number): Promise<void> {
  const { results } = await db
    .prepare(
      `${ELIGIBLE}
       SELECT c.key AS key, c.day AS day
       FROM product_contrib c JOIN eligible e ON e.install_hash = c.install_hash
       WHERE c.epoch = ?1 AND c.kind = 'install_day'
         AND c.key IN (SELECT key FROM product_totals WHERE epoch = ?1 AND kind = 'install_day' AND weeks IS NULL)`,
    )
    .bind(epoch, null, MIN_DAYS_SEEN, INSTALLS_PER_BUCKET)
    .all<{ key: string; day: string }>();
  const start = epoch * EPOCH_MS;
  const byKey = new Map<string, number[]>();
  for (const row of results) {
    const at = Date.parse(`${row.day}T00:00:00Z`);
    if (!Number.isFinite(at)) continue;
    const index = Math.max(0, Math.min(29, Math.floor((at - start) / DAY_MS)));
    const weeks = byKey.get(row.key) ?? [0, 0, 0, 0, 0];
    weeks[weekOf(index)] = (weeks[weekOf(index)] ?? 0) + 1;
    byKey.set(row.key, weeks);
  }
  if (byKey.size === 0) return;
  await db.batch(
    [...byKey].map(([key, weeks]) =>
      db.prepare("UPDATE product_totals SET weeks = ?3 WHERE epoch = ?1 AND kind = 'install_day' AND key = ?2").bind(epoch, key, weeks.join(",")),
    ),
  );
}

/** Delete one kind of an epoch's contributions, `DELETE_CHUNK` rows at a time. */
async function deleteChunks(db: D1Database, epoch: number, kind: string, budget: number): Promise<number> {
  let left = budget;
  while (left > 0) {
    const result = await db
      .prepare(
        `DELETE FROM product_contrib WHERE rowid IN (
           SELECT rowid FROM product_contrib WHERE epoch = ?1 AND kind = ?2 LIMIT ?3
         )`,
      )
      .bind(epoch, kind, DELETE_CHUNK)
      .run();
    left -= 1;
    if ((result.meta.changes ?? 0) < DELETE_CHUNK) return left;
  }
  return left;
}

const MEETS_FLOOR = `installs >= ${FLOOR.installs} AND buckets >= ${FLOOR.buckets} AND days >= ${FLOOR.days}`;

/**
 * Keys of one kind that met the floor in at least one closed epoch, with their strength (the sum
 * of installs over the epochs that met it). Every aggregate shown to players (mod health, frame
 * rates per hardware class, search terms) must come through here or `related`.
 */
export async function publishedKeys(
  db: D1Database,
  secret: string | undefined,
  kind: string,
  limit: number,
): Promise<{ key: string; installs: number }[]> {
  if (!hasBucketKey(secret)) return [];
  const { results } = await db
    .prepare(
      `SELECT key, SUM(installs) AS installs FROM product_totals
       WHERE kind = ?1 AND ${MEETS_FLOOR}
       GROUP BY key ORDER BY installs DESC, key ASC LIMIT ?2`,
    )
    .bind(kind, limit)
    .all<{ key: string; installs: number }>();
  return results;
}

/** Mods used with `mod`, from closed epochs that met the floor, ranked by a damped cosine score. */
export async function related(
  db: D1Database,
  secret: string | undefined,
  mod: number,
): Promise<{ modId: number; installs: number }[]> {
  if (!hasBucketKey(secret)) return [];
  const { results } = await db
    .prepare(
      `SELECT CASE WHEN mod_a = ?1 THEN mod_b ELSE mod_a END AS other, SUM(installs) AS installs
       FROM product_totals
       WHERE kind = 'pair' AND (mod_a = ?1 OR mod_b = ?1) AND ${MEETS_FLOOR}
       GROUP BY other
       LIMIT 90`,
    )
    .bind(mod)
    .all<{ other: number; installs: number }>();
  if (results.length === 0) return [];

  // Per-mod totals over every closed epoch, for the cosine denominator. D1 allows 100 bound
  // values; 90 candidates and the mod itself stay under it.
  const ids = [mod, ...results.map((row) => row.other)];
  const marks = ids.map((_, n) => `?${n + 1}`).join(",");
  const totals = await db
    .prepare(`SELECT mod_a AS mod, SUM(installs) AS installs FROM product_totals WHERE kind = 'mod' AND mod_a IN (${marks}) GROUP BY mod_a`)
    .bind(...ids)
    .all<{ mod: number; installs: number }>();
  const total = new Map(totals.results.map((row) => [row.mod, row.installs]));
  const base = Math.max(1, total.get(mod) ?? 1);

  return results
    .map((row) => {
      const support = row.installs;
      const other = Math.max(1, total.get(row.other) ?? 1);
      const score = (support / Math.sqrt(Math.max(base, support) * Math.max(other, support))) * (support / (support + DAMPING));
      return { modId: row.other, installs: row.installs, score };
    })
    .sort((x, y) => y.score - x.score || x.modId - y.modId)
    .slice(0, MAX_RELATED)
    .map(({ modId, installs }) => ({ modId, installs }));
}

// --- what players get back: mod health, frame rates, and the owner's search gaps --------------

/** `until`: Unix seconds when the newest epoch behind the numbers ended, so the app can drop
 * reports made before the creator's last update. */
export type InstallHealth = {
  readonly failed: number;
  readonly attempts: number;
  readonly reasons: { reason: string; installs: number }[];
  readonly until: number;
};
export type BuildHealth = { readonly build: number; readonly reports: number; readonly users: number; readonly until: number };
export type ConflictHealth = { readonly modId: number; readonly reports: number; readonly together: number; readonly until: number };

function epochEnd(epochs: Iterable<number>): number {
  return ((Math.max(...epochs) + 1) * EPOCH_MS) / 1000;
}
export type ModHealth = {
  readonly modId: number;
  readonly install: InstallHealth | null;
  readonly broken: BuildHealth[];
  readonly conflicts: ConflictHealth[];
};

/** The first closed epoch health still reads. */
function recentFrom(now: number): number {
  return epochOf(now) - RECENT_EPOCHS;
}

/**
 * Mod health for up to `MAX_HEALTH_MODS` mods, from the recent closed epochs that met the floor
 * (no open epoch, nothing below the floor). Every number is a count of installs; a rate is two
 * such counts from the same epochs. Mods with nothing published are left out.
 */
export async function modHealth(db: D1Database, secret: string | undefined, mods: readonly number[], now: number): Promise<ModHealth[]> {
  if (!hasBucketKey(secret) || mods.length === 0) return [];
  const ids = [...new Set(mods)].slice(0, MAX_HEALTH_MODS);
  const from = recentFrom(now);
  const marks = ids.map((_, n) => `?${n + 2}`).join(",");
  const { results: own } = await db
    .prepare(
      `SELECT kind, key, mod_a, epoch, installs FROM product_totals
       WHERE kind IN ('health', 'attempt', 'use') AND mod_a IN (${marks}) AND epoch >= ?1 AND ${MEETS_FLOOR}`,
    )
    .bind(from, ...ids)
    .all<{ kind: string; key: string; mod_a: number; epoch: number; installs: number }>();
  const marksB = ids.map((_, n) => `?${n + 2 + ids.length}`).join(",");
  const { results: conflicts } = await db
    .prepare(
      `SELECT key, mod_a, mod_b, SUM(installs) AS installs, MAX(epoch) AS last FROM product_totals
       WHERE kind = 'conflict' AND (mod_a IN (${marks}) OR mod_b IN (${marksB})) AND epoch >= ?1 AND ${MEETS_FLOOR}
       GROUP BY key LIMIT 90`,
    )
    .bind(from, ...ids, ...ids)
    .all<{ key: string; mod_a: number; mod_b: number; installs: number; last: number }>();
  const together = new Map<string, number>();
  if (conflicts.length > 0) {
    const keys = conflicts.map((row) => row.key);
    const { results: pairs } = await db
      .prepare(
        `SELECT key, SUM(installs) AS installs FROM product_totals
         WHERE kind = 'pair' AND key IN (${keys.map((_, n) => `?${n + 2}`).join(",")}) AND epoch >= ?1 AND ${MEETS_FLOOR}
         GROUP BY key`,
      )
      .bind(from, ...keys)
      .all<{ key: string; installs: number }>();
    for (const row of pairs) together.set(row.key, row.installs);
  }

  const out: ModHealth[] = [];
  for (const mod of ids) {
    const rows = own.filter((row) => row.mod_a === mod);
    // Install failures, with attempts from the same epochs.
    const failedRows = rows.filter((row) => row.kind === "health" && row.key === `${mod}|install_failed`);
    let install: InstallHealth | null = null;
    if (failedRows.length > 0) {
      const epochs = new Set(failedRows.map((row) => row.epoch));
      const failed = failedRows.reduce((sum, row) => sum + row.installs, 0);
      const attempts = rows.filter((row) => row.kind === "attempt" && epochs.has(row.epoch)).reduce((sum, row) => sum + row.installs, 0);
      const reasons = new Map<string, number>();
      for (const row of rows) {
        const code = row.kind === "health" ? row.key.split("|")[1] ?? "" : "";
        if (code.startsWith("install_") && code !== "install_failed") reasons.set(code.slice(8), (reasons.get(code.slice(8)) ?? 0) + row.installs);
      }
      install = {
        failed,
        attempts: Math.max(attempts, failed),
        reasons: [...reasons].map(([reason, installs]) => ({ reason, installs })).sort((a, b) => b.installs - a.installs),
        until: epochEnd(epochs),
      };
    }
    // Broken on a game build: the strongest single signal per build (one install can send
    // several, so they are not added), and the installs that used the mod on that build.
    const perBuild = new Map<number, Map<string, number>>();
    const buildEpochs = new Map<number, number[]>();
    for (const row of rows) {
      if (row.kind !== "health") continue;
      const [, code = "", buildText = ""] = row.key.split("|");
      const build = Number(buildText);
      if (!BROKEN_CODES.has(code) || !Number.isSafeInteger(build) || build <= 0) continue;
      const codes = perBuild.get(build) ?? new Map<string, number>();
      codes.set(code, (codes.get(code) ?? 0) + row.installs);
      perBuild.set(build, codes);
      buildEpochs.set(build, [...(buildEpochs.get(build) ?? []), row.epoch]);
    }
    const broken: BuildHealth[] = [...perBuild]
      .map(([build, codes]) => ({
        build,
        reports: Math.max(...codes.values()),
        users: rows.filter((row) => row.kind === "use" && row.key === `${mod}|${build}`).reduce((sum, row) => sum + row.installs, 0),
        until: epochEnd(buildEpochs.get(build) ?? []),
      }))
      .sort((a, b) => b.build - a.build);
    const mine: ConflictHealth[] = conflicts
      .filter((row) => row.mod_a === mod || row.mod_b === mod)
      .map((row) => ({
        modId: row.mod_a === mod ? row.mod_b : row.mod_a,
        reports: row.installs,
        together: Math.max(together.get(row.key) ?? 0, row.installs),
        until: epochEnd([row.last]),
      }))
      .sort((a, b) => b.reports - a.reports || a.modId - b.modId)
      .slice(0, 10);
    if (install !== null || broken.length > 0 || mine.length > 0) out.push({ modId: mod, install, broken, conflicts: mine });
  }
  return out;
}

export type FpsExpectation = { readonly config: string; readonly version: number; readonly fps: number; readonly low: number; readonly installs: number };

/**
 * Frame rates other players measured on the same hardware class (GPU model and CPU tier) at the
 * same resolution bucket, per performance config and version. Medians from closed epochs that met
 * the floor, weighted by their installs. Empty when there is none.
 */
export async function fpsFor(db: D1Database, secret: string | undefined, gpu: string, tier: string, bucket: string): Promise<FpsExpectation[]> {
  if (!hasBucketKey(secret)) return [];
  const prefix = `${gpu}|${tier}|`;
  const { results } = await db
    .prepare(
      `SELECT key, SUM(installs) AS installs, SUM(median * installs) AS fps, SUM(low * installs) AS low FROM product_totals
       WHERE kind = 'fps' AND substr(key, 1, length(?1)) = ?1 AND median IS NOT NULL AND ${MEETS_FLOOR}
       GROUP BY key LIMIT 200`,
    )
    .bind(prefix)
    .all<{ key: string; installs: number; fps: number; low: number }>();
  return results
    .map((row) => {
      const [, , config = "", version = "0", res = ""] = row.key.split("|");
      return { res, config, version: Number(version), fps: Math.round(row.fps / row.installs), low: Math.round(row.low / row.installs), installs: row.installs };
    })
    .filter((row) => row.res === bucket && row.config !== "")
    .map(({ config, version, fps, low, installs }) => ({ config, version, fps, low, installs }))
    .sort((a, b) => b.installs - a.installs || a.config.localeCompare(b.config));
}

/** The owner's search gaps: published Browse searches that found nothing or few mods. */
export async function searchGaps(
  db: D1Database,
  secret: string | undefined,
  limit: number,
): Promise<{ zero: { term: string; installs: number }[]; few: { term: string; installs: number }[] }> {
  const [zero, few] = await Promise.all([publishedKeys(db, secret, "search_zero", limit), publishedKeys(db, secret, "search_few", limit)]);
  const term = ({ key, installs }: { key: string; installs: number }) => ({ term: key, installs });
  return { zero: zero.map(term), few: few.map(term) };
}

/**
 * Rising: mods whose installs grew within the newest closed epoch that has any, ranked by the
 * installs of its last two weeks against its first two, damped so a handful does not lead:
 * ((late + 1) / (early + 1)) * late / (late + 10). Only mods that met the floor over the epoch
 * and have at least `RISING_MIN_LATE` late installs. The open epoch is never read.
 */
export async function rising(db: D1Database, secret: string | undefined): Promise<{ modId: number; recent: number }[]> {
  if (!hasBucketKey(secret)) return [];
  const newest = await db
    .prepare("SELECT MAX(epoch) AS epoch FROM product_totals WHERE kind = 'install_day' AND weeks IS NOT NULL")
    .first<{ epoch: number | null }>();
  if (newest?.epoch === null || newest?.epoch === undefined) return [];
  const { results } = await db
    .prepare(`SELECT mod_a, weeks FROM product_totals WHERE kind = 'install_day' AND epoch = ?1 AND weeks IS NOT NULL AND ${MEETS_FLOOR}`)
    .bind(newest.epoch)
    .all<{ mod_a: number; weeks: string }>();
  return results
    .map((row) => {
      const w = row.weeks.split(",").map(Number);
      const early = (w[0] ?? 0) + (w[1] ?? 0);
      const late = (w[3] ?? 0) + (w[4] ?? 0);
      return { modId: row.mod_a, recent: late, score: ((late + 1) / (early + 1)) * (late / (late + 10)) };
    })
    .filter((row) => row.recent >= RISING_MIN_LATE && Number.isSafeInteger(row.modId))
    .sort((a, b) => b.score - a.score || b.recent - a.recent || a.modId - b.modId)
    .slice(0, MAX_RISING)
    .map(({ modId, recent }) => ({ modId, recent }));
}
