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
 *    stored. Without the secret no bucket is made: raw records are kept and no aggregate is
 *    written or published (fail closed).
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
import type { ProductRecord } from "./product";

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

export type Contributor = { readonly hash: string; readonly bucket: string; readonly epoch: number; readonly day: string };

export function epochOf(now: number): number {
  return Math.floor(now / EPOCH_MS);
}

export function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}

/** Without the bucket secret nothing is aggregated or published (fail closed). */
export function hasBucketKey(secret: string | undefined): secret is string {
  return secret !== undefined && secret.length >= 16;
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

/** The aggregate keys one record contributes to. */
export function keysOf(record: ProductRecord): { kind: string; key: string }[] {
  const f = record.fields;
  switch (record.type) {
    case "enabled_set": {
      const mods = (f.mods as { mod_id: number }[]).slice(0, 24).map((m) => m.mod_id);
      const keys = mods.map((mod) => ({ kind: "mod", key: String(mod) }));
      for (let i = 0; i < mods.length; i += 1) {
        for (let j = i + 1; j < mods.length; j += 1) {
          const a = Math.min(mods[i] as number, mods[j] as number);
          const b = Math.max(mods[i] as number, mods[j] as number);
          keys.push({ kind: "pair", key: `${a}|${b}` });
        }
      }
      return keys;
    }
    case "compat_finding":
      return [{ kind: "health", key: `${f.mod_id}|${f.finding}|${f.game_build ?? 0}` }];
    case "repair_result":
      return [{ kind: "health", key: `${f.mod_id}|repair_${f.repair}|${f.ok ? 1 : 0}` }];
    case "install_result":
      return f.ok ? [] : [{ kind: "health", key: `${f.mod_id}|install_${f.reason ?? "unknown"}` }];
    case "benchmark":
      return [{ kind: "bench", key: `${f.scene}|${f.width}x${f.height}|${f.preset ?? ""}` }];
    case "hardware":
      return f.gpu === null ? [] : [{ kind: "hardware", key: `${f.gpu}|${f.width}x${f.height}` }];
    case "browse_query":
      return typeof f.search === "string" ? [{ kind: "search", key: f.search }] : [];
    default:
      return [];
  }
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
): Promise<boolean> {
  const epoch = epochOf(now);
  const bucket = await bucketOf(address, secret, epoch);
  if (bucket === null) return false;
  const who: Contributor = { hash, bucket, epoch, day: utcDay(now) };

  const room = await db
    .prepare(
      `SELECT COUNT(DISTINCT install_hash) AS installs, COALESCE(SUM(install_hash = ?3), 0) AS mine
       FROM product_contrib WHERE epoch = ?1 AND bucket = ?2`,
    )
    .bind(epoch, bucket, hash)
    .first<{ installs: number; mine: number }>();
  if ((room?.mine ?? 0) === 0 && (room?.installs ?? 0) >= INSTALLS_PER_BUCKET) return false;

  const rows = [{ kind: "seen", key: who.day }, ...records.flatMap(keysOf)].slice(0, MAX_CONTRIB_ROWS);
  await db.batch(
    rows.map(({ kind, key }) =>
      db
        .prepare(
          `INSERT OR IGNORE INTO product_contrib (epoch, kind, key, install_hash, bucket, day, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)`,
        )
        .bind(epoch, kind, key.slice(0, 200), who.hash, who.bucket, who.day, now),
    ),
  );
  return true;
}

/**
 * Close every epoch that ended more than `GRACE_MS` ago: counts to `product_totals`, then its
 * contribution rows go. Returns the epochs closed.
 */
export async function closeEpochs(db: D1Database, now: number): Promise<number[]> {
  const last = epochOf(now - GRACE_MS) - 1;
  const { results } = await db
    .prepare("SELECT DISTINCT epoch FROM product_contrib WHERE epoch <= ?1 ORDER BY epoch")
    .bind(last)
    .all<{ epoch: number }>();
  const closed: number[] = [];
  for (const { epoch } of results) {
    await db.batch([
      db
        .prepare(
          `WITH seen AS (
             SELECT install_hash, bucket, MIN(created_at) AS first_at FROM product_contrib
             WHERE epoch = ?1 AND kind = 'seen' GROUP BY install_hash, bucket
           ),
           ranked AS (
             SELECT install_hash, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY first_at, install_hash) AS place FROM seen
           ),
           days AS (
             SELECT install_hash FROM product_contrib WHERE epoch = ?1 AND kind = 'seen'
             GROUP BY install_hash HAVING COUNT(DISTINCT key) >= ?2
           ),
           eligible AS (
             SELECT DISTINCT r.install_hash FROM ranked r JOIN days d ON d.install_hash = r.install_hash WHERE r.place <= ?3
           )
           INSERT INTO product_totals (epoch, kind, key, mod_a, mod_b, installs, buckets, days)
           SELECT ?1, c.kind, c.key,
             CASE c.kind WHEN 'pair' THEN CAST(substr(c.key, 1, instr(c.key, '|') - 1) AS INTEGER)
                         WHEN 'mod' THEN CAST(c.key AS INTEGER) END,
             CASE c.kind WHEN 'pair' THEN CAST(substr(c.key, instr(c.key, '|') + 1) AS INTEGER) END,
             COUNT(DISTINCT c.install_hash), COUNT(DISTINCT c.bucket), COUNT(DISTINCT c.day)
           FROM product_contrib c JOIN eligible e ON e.install_hash = c.install_hash
           WHERE c.epoch = ?1 AND c.kind <> 'seen'
           GROUP BY c.kind, c.key
           ON CONFLICT(epoch, kind, key) DO UPDATE SET
             installs = excluded.installs, buckets = excluded.buckets, days = excluded.days`,
        )
        .bind(epoch, MIN_DAYS_SEEN, INSTALLS_PER_BUCKET),
      db.prepare("DELETE FROM product_contrib WHERE epoch = ?1").bind(epoch),
    ]);
    closed.push(epoch);
  }
  return closed;
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
