/**
 * Aggregates from product data, and the rule that decides when one may be shown to players.
 *
 * An install id is a UUID the app makes, so one person could send many made-up ids. Counting
 * install ids alone would let that person cross the 10-install floor of ADR 0017 and push a mod
 * into "players who use this also use". The rule here counts distinct contributors instead:
 *
 * 1. Network bucket. Each request gets a bucket id: a SHA-256 of the limiter's key, which is
 *    itself a SHA-256 of the caller address and a salt that lives only in this isolate's memory
 *    and is replaced every UTC day (`limiter.ts`). The address is never stored. The bucket id is
 *    stored only on aggregate contributions (never on raw events) and goes with them after 13
 *    months or on Delete my data. Because the salt rotates, a bucket cannot be linked across days.
 * 2. New installs per bucket. One bucket may introduce at most `NEW_INSTALLS_PER_BUCKET_DAY`
 *    new install hashes a day. Extra installs are still stored as raw data but never count
 *    toward an aggregate (`eligible = 0`).
 * 3. Seen on two days. An install counts only once it has sent data on at least two UTC days.
 * 4. The floor. An aggregate key (a mod pair, a mod health finding, a hardware class, a search
 *    term) is shown only with at least `FLOOR.installs` eligible installs from `FLOOR.buckets`
 *    distinct buckets on `FLOOR.days` distinct days.
 * 5. Damped ranking. Related mods are ranked by a cosine score over effective support (installs
 *    capped at three per bucket) times a support damper, not by raw counts, and the list is cut.
 */

import { sha256Hex } from "./hash";
import { limiterKey } from "./limiter";
import type { ProductRecord } from "./product";

export const FLOOR = { installs: 10, buckets: 6, days: 3 } as const;
export const NEW_INSTALLS_PER_BUCKET_DAY = 3;
export const MIN_DAYS_SEEN = 2;
/** Damper constant: a pair with support k scores half of what infinite support would. */
const DAMPING = 10;
export const MAX_RELATED = 20;

export type Contributor = { readonly hash: string; readonly bucket: string; readonly day: string };

export function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
}

/** The request's network bucket: 16 hex characters derived from the daily-salted limiter key. */
export async function bucketOf(request: Request, now: number): Promise<string> {
  return (await sha256Hex(`sigil-product-bucket\n${await limiterKey(request, now)}`)).slice(0, 16);
}

/**
 * Note one request from an install: a new install is eligible only while its bucket is under
 * the daily cap; a known install gains a day when today is new for it.
 */
export async function registerInstall(db: D1Database, who: Contributor, now: number): Promise<void> {
  const known = await db
    .prepare("SELECT last_day FROM product_installs WHERE install_hash = ?1")
    .bind(who.hash)
    .first<{ last_day: string }>();
  if (known === null) {
    const counted = await db
      .prepare(
        `INSERT INTO product_buckets (bucket, day, new_installs, created_at) VALUES (?1, ?2, 1, ?3)
         ON CONFLICT(bucket, day) DO UPDATE SET new_installs = product_buckets.new_installs + 1
         RETURNING new_installs`,
      )
      .bind(who.bucket, who.day, now)
      .first<{ new_installs: number }>();
    const eligible = (counted?.new_installs ?? 1) <= NEW_INSTALLS_PER_BUCKET_DAY ? 1 : 0;
    await db
      .prepare(
        `INSERT OR IGNORE INTO product_installs (install_hash, first_day, last_day, days_seen, eligible, created_at, updated_at)
         VALUES (?1, ?2, ?2, 1, ?3, ?4, ?4)`,
      )
      .bind(who.hash, who.day, eligible, now)
      .run();
    return;
  }
  if (known.last_day !== who.day) {
    await db
      .prepare(
        `UPDATE product_installs SET days_seen = days_seen + 1, last_day = ?2, updated_at = ?3 WHERE install_hash = ?1`,
      )
      .bind(who.hash, who.day, now)
      .run();
  } else {
    await db.prepare("UPDATE product_installs SET updated_at = ?2 WHERE install_hash = ?1").bind(who.hash, now).run();
  }
}

/** The aggregate keys one record contributes to, besides mod pairs. */
export function keysOf(record: ProductRecord): { kind: string; key: string }[] {
  const f = record.fields;
  switch (record.type) {
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

export function keyStatement(db: D1Database, kind: string, key: string, who: Contributor, now: number): D1PreparedStatement {
  return db
    .prepare(
      `INSERT INTO product_keys (kind, key, install_hash, bucket, day, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
       ON CONFLICT(kind, key, install_hash) DO UPDATE SET updated_at = excluded.updated_at`,
    )
    .bind(kind, key.slice(0, 200), who.hash, who.bucket, who.day, now);
}

export function pairStatement(db: D1Database, a: number, b: number, who: Contributor, now: number): D1PreparedStatement {
  return db
    .prepare(
      `INSERT INTO product_pairs (mod_a, mod_b, install_hash, bucket, day, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
       ON CONFLICT(mod_a, mod_b, install_hash) DO UPDATE SET updated_at = excluded.updated_at`,
    )
    .bind(a, b, who.hash, who.bucket, who.day, now);
}

const ELIGIBLE = `JOIN product_installs i ON i.install_hash = c.install_hash AND i.eligible = 1 AND i.days_seen >= ${MIN_DAYS_SEEN}`;
const MEETS_FLOOR = `HAVING COUNT(DISTINCT c.install_hash) >= ${FLOOR.installs}
  AND COUNT(DISTINCT c.bucket) >= ${FLOOR.buckets}
  AND COUNT(DISTINCT c.day) >= ${FLOOR.days}`;

/**
 * Keys of one kind that pass the floor, with their eligible install counts. Any aggregate shown
 * to players (mod health, frame rates per hardware class, search terms) must come through here.
 */
export async function publishedKeys(
  db: D1Database,
  kind: string,
  limit: number,
): Promise<{ key: string; installs: number }[]> {
  const { results } = await db
    .prepare(
      `SELECT c.key AS key, COUNT(DISTINCT c.install_hash) AS installs
       FROM product_keys c ${ELIGIBLE}
       WHERE c.kind = ?1
       GROUP BY c.key ${MEETS_FLOOR}
       ORDER BY installs DESC, key ASC
       LIMIT ?2`,
    )
    .bind(kind, limit)
    .all<{ key: string; installs: number }>();
  return results;
}

type Candidate = { other: number; installs: number; buckets: number };

/** Mods used with `mod` that pass the floor, ranked by a damped cosine score. */
export async function related(db: D1Database, mod: number): Promise<{ modId: number; installs: number }[]> {
  const { results } = await db
    .prepare(
      `SELECT c.other AS other, COUNT(DISTINCT c.install_hash) AS installs, COUNT(DISTINCT c.bucket) AS buckets
       FROM (
         SELECT mod_b AS other, install_hash, bucket, day FROM product_pairs WHERE mod_a = ?1
         UNION ALL
         SELECT mod_a AS other, install_hash, bucket, day FROM product_pairs WHERE mod_b = ?1
       ) c ${ELIGIBLE}
       GROUP BY c.other ${MEETS_FLOOR}
       LIMIT 90`,
    )
    .bind(mod)
    .all<Candidate>();
  // D1 allows 100 bound values; 90 candidates and the mod itself stay under it.
  if (results.length === 0) return [];

  const ids = [mod, ...results.map((row) => row.other)];
  const marks = ids.map((_, n) => `?${n + 1}`).join(",");
  const totals = await db
    .prepare(
      `SELECT m AS mod, COUNT(DISTINCT c.install_hash) AS installs FROM (
         SELECT mod_a AS m, install_hash FROM product_pairs WHERE mod_a IN (${marks})
         UNION ALL
         SELECT mod_b AS m, install_hash FROM product_pairs WHERE mod_b IN (${marks})
       ) c ${ELIGIBLE}
       GROUP BY m`,
    )
    .bind(...ids)
    .all<{ mod: number; installs: number }>();
  const total = new Map(totals.results.map((row) => [row.mod, row.installs]));
  const base = Math.max(1, total.get(mod) ?? 1);

  return results
    .map((row) => {
      // A burst from few buckets cannot add more than three installs per bucket.
      const support = Math.min(row.installs, row.buckets * NEW_INSTALLS_PER_BUCKET_DAY);
      const other = Math.max(1, total.get(row.other) ?? 1);
      const score = (support / Math.sqrt(base * other)) * (support / (support + DAMPING));
      return { modId: row.other, installs: row.installs, score };
    })
    .sort((x, y) => y.score - x.score || x.modId - y.modId)
    .slice(0, MAX_RELATED)
    .map(({ modId, installs }) => ({ modId, installs }));
}
