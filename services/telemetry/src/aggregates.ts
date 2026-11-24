/**
 * Aggregates from product data, and the rule that decides when one may be shown to players.
 *
 * An install id is a UUID the app makes, so one person could send many made-up ids. Counting
 * install ids alone would let that person cross the 10-install floor of ADR 0017 and push a mod
 * into "players who use this also use". The rule here counts distinct contributors instead:
 *
 * 1. Network bucket. Each request gets a bucket id: HMAC-SHA256 of the caller's network prefix
 *    (IPv4 /24, IPv6 /48) under the Worker secret `PRODUCT_BUCKET_KEY`, cut to 16 hex
 *    characters. It is the same across isolates and days, the address and the prefix are never
 *    stored, and the bucket id is stored only on aggregate rows (never on raw events) and deleted
 *    with them after 13 months or on Delete my data. Without the secret no bucket is made: raw
 *    records are still stored, and no aggregate row is written or published (fail closed).
 * 2. Installs per bucket. At most `INSTALLS_PER_BUCKET` install hashes per bucket, over the whole
 *    retention window, count toward any aggregate (`eligible = 1`). Later ones from that bucket
 *    are stored as raw data only.
 * 3. Seen on two days. An install counts only once it has sent data on at least two UTC days.
 * 4. The floor. An aggregate key (a mod pair, a mod health finding, a hardware class, a search
 *    term) is shown only with at least `FLOOR.installs` eligible installs from `FLOOR.buckets`
 *    distinct buckets on `FLOOR.days` distinct days.
 * 5. Damped ranking. Related mods are ranked by a cosine score over effective support (installs
 *    capped per bucket) times a support damper, not by raw counts, and the list is cut.
 */

import { toHex } from "./hash";
import type { ProductRecord } from "./product";

export const FLOOR = { installs: 10, buckets: 6, days: 3 } as const;
export const INSTALLS_PER_BUCKET = 3;
export const MIN_DAYS_SEEN = 2;
/** Damper constant: a pair with support k scores half of what infinite support would. */
const DAMPING = 10;
export const MAX_RELATED = 20;

export type Contributor = { readonly hash: string; readonly bucket: string; readonly day: string };

export function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10);
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

/**
 * The request's network bucket: HMAC-SHA256 of its network prefix under `secret`, 16 hex
 * characters. Null without the secret or an address, and then no aggregate row is written.
 */
export async function bucketOf(request: Request, secret: string | undefined): Promise<string | null> {
  if (secret === undefined || secret.length < 16) return null;
  const prefix = networkPrefix(request.headers.get("cf-connecting-ip"));
  if (prefix === null) return null;
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = await crypto.subtle.sign("HMAC", key, encoder.encode(`sigil-product-bucket\n${prefix}`));
  return toHex(new Uint8Array(mac)).slice(0, 16);
}

/**
 * Note one request from an install. A new install is eligible only while its bucket has fewer
 * than `INSTALLS_PER_BUCKET` installs; a known install gains a day when today is new for it.
 */
export async function registerInstall(db: D1Database, who: Contributor, now: number): Promise<void> {
  const known = await db
    .prepare("SELECT last_day FROM product_installs WHERE install_hash = ?1")
    .bind(who.hash)
    .first<{ last_day: string }>();
  if (known === null) {
    const counted = await db
      .prepare(
        `INSERT INTO product_buckets (bucket, installs, created_at, updated_at) VALUES (?1, 1, ?2, ?2)
         ON CONFLICT(bucket) DO UPDATE SET installs = product_buckets.installs + 1, updated_at = excluded.updated_at
         RETURNING installs`,
      )
      .bind(who.bucket, now)
      .first<{ installs: number }>();
    const eligible = (counted?.installs ?? 1) <= INSTALLS_PER_BUCKET ? 1 : 0;
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
  secret: string | undefined,
  kind: string,
  limit: number,
): Promise<{ key: string; installs: number }[]> {
  if (!hasBucketKey(secret)) return [];
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

/** Without the bucket secret nothing is published (fail closed). */
export function hasBucketKey(secret: string | undefined): boolean {
  return secret !== undefined && secret.length >= 16;
}

type Candidate = { other: number; installs: number; buckets: number };

/** Mods used with `mod` that pass the floor, ranked by a damped cosine score. */
export async function related(
  db: D1Database,
  secret: string | undefined,
  mod: number,
): Promise<{ modId: number; installs: number }[]> {
  if (!hasBucketKey(secret)) return [];
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
      const support = Math.min(row.installs, row.buckets * INSTALLS_PER_BUCKET);
      const other = Math.max(1, total.get(row.other) ?? 1);
      const score = (support / Math.sqrt(base * other)) * (support / (support + DAMPING));
      return { modId: row.other, installs: row.installs, score };
    })
    .sort((x, y) => y.score - x.score || x.modId - y.modId)
    .slice(0, MAX_RELATED)
    .map(({ modId, installs }) => ({ modId, installs }));
}
