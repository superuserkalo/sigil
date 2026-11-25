/**
 * The community crosshair library (ADR 0019, owner approved 2026-11-25). No accounts.
 *
 *   POST /v1/crosshairs                    publish `{ name, config }`, returns `{ id, delete_token }`
 *   GET  /v1/crosshairs?sort=&cursor=      60 visible crosshairs per page, cacheable
 *   POST /v1/crosshairs/<id>/delete        `{ delete_token }`, removes the row
 *   POST /v1/crosshairs/<id>/use           counts an apply, once per network per crosshair per day,
 *                                          at most 30 per network per day
 *   POST /v1/crosshairs/<id>/report        once per network per crosshair; 5 networks hide a
 *                                          community row; at most 5 reports per network per day
 *   GET  /v1/crosshairs/admin/flagged      owner only: reported or hidden rows
 *   POST /v1/crosshairs/admin/<id>/remove  owner only
 *   POST /v1/crosshairs/admin/<id>/restore owner only: shows the row again, reports back to 0
 *   POST /v1/crosshairs/admin/official     owner only: upsert `{ id, name, config }` as by Sigil
 *
 * Privacy: a crosshair row holds its name, canonical config, the config hash, counts, flags, the
 * created time and a SHA-256 of its delete token. No install id, player id or address.
 *
 * Two network markers, both HMACs under `PRODUCT_BUCKET_KEY` of the caller's /24 or /48:
 *
 * - The daily bucket (with the UTC day) sits in `crosshair_uses` and `crosshair_daily`, which the
 *   daily job empties after 2 days. `crosshair_daily` does not say which crosshair.
 * - The report marker (with the crosshair id, no day) sits in `crosshair_reports` for 90 days. A
 *   network counts once per crosshair however many days it reports, and a marker for one crosshair
 *   cannot be matched to the same network's marker for another or to any other table.
 *
 * Hiding is decided from the number of distinct report rows, never from a counter, and Sigil's
 * own rows are never hidden by reports.
 */

import { readJsonBody } from "../body";
import { networkPrefix, utcDay } from "../aggregates";
import { sha256Hex, toHex } from "../hash";
import { adminAllowed, secretOk } from "../product";
import { canonicalText, configHash, parseConfig } from "./config";
import { parseName } from "./name";

export const PAGE_SIZE = 60;
export const LIST_MAX_AGE = 300;
export const PUBLISHES_PER_BUCKET_PER_DAY = 3;
export const PUBLISHES_PER_DAY = 300;
export const MAX_ROWS = 20_000;
export const REPORTS_TO_HIDE = 5;
export const REPORTS_PER_BUCKET_PER_DAY = 5;
export const USES_PER_BUCKET_PER_DAY = 30;
/** How long the daily-bucket rows live (`retention.ts`). */
export const CROSSHAIR_DEDUPE_MS = 2 * 24 * 60 * 60 * 1000;
/** How long a report marker lives (`retention.ts`). */
export const CROSSHAIR_REPORT_MS = 90 * 24 * 60 * 60 * 1000;

const PUBLISH_BODY_LIMIT = 2 * 1024;
const DELETE_BODY_LIMIT = 256;

const COMMUNITY_ID = /^[0-9a-f]{16}$/;
const OFFICIAL_ID = /^sigil-[a-z0-9-]{1,32}$/;
const ID = "(?:[0-9a-f]{16}|sigil-[a-z0-9-]{1,32})";
const ACTION_PATH = new RegExp(`^/v1/crosshairs/(${ID})/(delete|use|report)$`);
const ADMIN_ACTION_PATH = new RegExp(`^/v1/crosshairs/admin/(${ID})/(remove|restore)$`);
const TOKEN = /^[A-Za-z0-9_-]{43}$/;

type Respond = (status: number, body: unknown, extra?: Record<string, string>) => Response;
type Handler = (request: Request, env: Env, url: URL) => Promise<Response>;
export type CrosshairRoute = { readonly method: "GET" | "POST"; readonly run: Handler };

type Secrets = { readonly PRODUCT_BUCKET_KEY?: string; readonly PRODUCT_ADMIN_KEY?: string };

type Row = {
  id: string;
  name: string;
  config: string;
  uses: number;
  reports: number;
  official: number;
  hidden: number;
  created_at: number;
  created_day: string;
};

/**
 * The route for a crosshair path, or undefined when the path is not one. `/v1/crosshairs` takes GET
 * and POST, so the route returned for it follows the request's method.
 */
export function crosshairRoute(path: string, method: string, respond: Respond): CrosshairRoute | undefined {
  if (path === "/v1/crosshairs") {
    return method === "POST"
      ? { method: "POST", run: (request, env) => publish(request, env, respond) }
      : { method: "GET", run: (_request, env, url) => list(env, url, respond) };
  }
  if (path === "/v1/crosshairs/admin/flagged") return { method: "GET", run: (request, env) => adminFlagged(request, env, respond) };
  if (path === "/v1/crosshairs/admin/official") return { method: "POST", run: (request, env) => adminOfficial(request, env, respond) };
  const admin = ADMIN_ACTION_PATH.exec(path);
  if (admin !== null) {
    const [, id = "", action] = admin;
    return { method: "POST", run: (request, env) => adminAction(request, env, id, action === "remove" ? "remove" : "restore", respond) };
  }
  const match = ACTION_PATH.exec(path);
  if (match === null) return undefined;
  const [, id = "", action] = match;
  if (action === "delete") return { method: "POST", run: (request, env) => remove(request, env, id, respond) };
  if (action === "use") return { method: "POST", run: (request, env) => use(request, env, id, respond) };
  return { method: "POST", run: (request, env) => report(request, env, id, respond) };
}

function bucketKey(env: Env): string | undefined {
  return (env as Env & Secrets).PRODUCT_BUCKET_KEY;
}

function adminKey(env: Env): string | undefined {
  return (env as Env & Secrets).PRODUCT_ADMIN_KEY;
}

const encoder = new TextEncoder();

/** HMAC-SHA256 under `PRODUCT_BUCKET_KEY` of the UTC day and the caller's network prefix, 16 hex. */
export async function crosshairBucket(address: string | null, secret: string, day: string): Promise<string | null> {
  const prefix = networkPrefix(address);
  if (prefix === null) return null;
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = await crypto.subtle.sign("HMAC", key, encoder.encode(`sigil-crosshair-bucket\n${day}\n${prefix}`));
  return toHex(new Uint8Array(mac)).slice(0, 16);
}

/**
 * HMAC-SHA256 under `PRODUCT_BUCKET_KEY` of the crosshair id and the caller's network prefix, with
 * no day: the same network gets the same marker for one crosshair on every day, and unrelated
 * markers for different crosshairs.
 */
export async function reportMarker(address: string | null, secret: string, crosshairId: string): Promise<string | null> {
  const prefix = networkPrefix(address);
  if (prefix === null) return null;
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const mac = await crypto.subtle.sign("HMAC", key, encoder.encode(`sigil-crosshair-report\n${crosshairId}\n${prefix}`));
  return toHex(new Uint8Array(mac)).slice(0, 32);
}

/** The bucket for this request, or the response that refuses it. */
async function requestBucket(request: Request, env: Env, now: number, respond: Respond): Promise<{ bucket: string; secret: string } | { response: Response }> {
  const secret = bucketKey(env);
  if (!secretOk(secret)) {
    await request.body?.cancel();
    return { response: respond(503, { error: "not_configured" }) };
  }
  const bucket = await crosshairBucket(request.headers.get("cf-connecting-ip"), secret, utcDay(now));
  if (bucket === null) {
    await request.body?.cancel();
    return { response: respond(400, { error: "no_network" }) };
  }
  return { bucket, secret };
}

function bodyError(reason: "too_large" | "invalid_utf8" | "invalid_json", respond: Respond): Response {
  if (reason === "too_large") return respond(413, { error: "body_too_large" });
  return respond(400, { error: reason });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function base64url(bytes: Uint8Array): string {
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function randomHex(bytes: number): string {
  const buffer = new Uint8Array(bytes);
  crypto.getRandomValues(buffer);
  return toHex(buffer);
}

/** Equal-length hex strings compared without an early exit. */
function sameHex(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

function tokenHash(token: string): Promise<string> {
  return sha256Hex(`sigil-crosshair-delete\n${token}`);
}

type Validated = { readonly name: string; readonly config: string; readonly hash: string };

async function validateEntry(raw: unknown, respond: Respond): Promise<Validated | Response> {
  if (!isRecord(raw)) return respond(400, { error: "bad_body" });
  const name = parseName(raw.name);
  if (!name.ok) return respond(400, { error: name.error });
  const config = parseConfig(raw.config);
  if (!config.ok) return respond(400, { error: "bad_config", field: config.field });
  return { name: name.name, config: canonicalText(config.config), hash: await configHash(config.config) };
}

async function existingId(db: D1Database, hash: string): Promise<string | null> {
  const row = await db.prepare("SELECT id FROM crosshairs WHERE config_hash = ?1").bind(hash).first<{ id: string }>();
  return row?.id ?? null;
}

async function publish(request: Request, env: Env, respond: Respond): Promise<Response> {
  const now = Date.now();
  const day = utcDay(now);
  const gate = await requestBucket(request, env, now, respond);
  if ("response" in gate) return gate.response;
  const body = await readJsonBody(request, PUBLISH_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason, respond);
  if (isRecord(body.value)) {
    for (const key of Object.keys(body.value)) if (key !== "name" && key !== "config") return respond(400, { error: "bad_body" });
  }
  const entry = await validateEntry(body.value, respond);
  if (entry instanceof Response) return entry;

  const duplicate = await existingId(env.DB, entry.hash);
  if (duplicate !== null) return respond(409, { error: "duplicate", id: duplicate });

  const [mine, today, total] = await env.DB.batch<{ n: number }>([
    env.DB.prepare("SELECT COUNT(*) AS n FROM crosshair_daily WHERE bucket = ?1 AND kind = 'publish'").bind(gate.bucket),
    env.DB.prepare("SELECT COUNT(*) AS n FROM crosshair_daily WHERE kind = 'publish' AND day = ?1").bind(day),
    env.DB.prepare("SELECT COUNT(*) AS n FROM crosshairs"),
  ]);
  if ((mine?.results[0]?.n ?? 0) >= PUBLISHES_PER_BUCKET_PER_DAY) return respond(429, { error: "publish_limit" });
  if ((today?.results[0]?.n ?? 0) >= PUBLISHES_PER_DAY) return respond(429, { error: "daily_limit" });
  if ((total?.results[0]?.n ?? 0) >= MAX_ROWS) return respond(507, { error: "library_full" });

  const id = randomHex(8);
  const tokenBytes = new Uint8Array(32);
  crypto.getRandomValues(tokenBytes);
  const token = base64url(tokenBytes);

  // One transaction: the row goes in only while every limit still holds (two publishes racing past
  // the reads above cannot both pass), and the limit row goes in only when the crosshair did.
  const [inserted] = await env.DB.batch([
    env.DB.prepare(
      `INSERT OR IGNORE INTO crosshairs (id, name, config, config_hash, uses, reports, official, hidden, created_at, created_day, delete_token_hash)
       SELECT ?1, ?2, ?3, ?4, 0, 0, 0, 0, ?5, ?6, ?7
       WHERE (SELECT COUNT(*) FROM crosshair_daily WHERE bucket = ?8 AND kind = 'publish') < ?9
         AND (SELECT COUNT(*) FROM crosshair_daily WHERE kind = 'publish' AND day = ?6) < ?10
         AND (SELECT COUNT(*) FROM crosshairs) < ?11`,
    ).bind(id, entry.name, entry.config, entry.hash, now, day, await tokenHash(token), gate.bucket, PUBLISHES_PER_BUCKET_PER_DAY, PUBLISHES_PER_DAY, MAX_ROWS),
    env.DB.prepare(
      `INSERT INTO crosshair_daily (bucket, kind, day, created_at)
       SELECT ?1, 'publish', ?2, ?3 WHERE EXISTS (SELECT 1 FROM crosshairs WHERE id = ?4 AND created_at = ?3)`,
    ).bind(gate.bucket, day, now, id),
  ]);
  if ((inserted?.meta.changes ?? 0) === 0) {
    const raced = await existingId(env.DB, entry.hash);
    if (raced !== null) return respond(409, { error: "duplicate", id: raced });
    return respond(429, { error: "publish_limit" });
  }
  return respond(201, { id, delete_token: token });
}

type Cursor = { readonly s: "used"; readonly u: number; readonly i: string } | { readonly s: "new"; readonly t: number; readonly i: string };

function encodeCursor(cursor: Cursor): string {
  return base64url(encoder.encode(JSON.stringify(cursor)));
}

function decodeCursor(text: string, sort: "used" | "new"): Cursor | null {
  if (!/^[A-Za-z0-9_-]{1,200}$/.test(text)) return null;
  let value: unknown;
  try {
    const padded = text.replace(/-/g, "+").replace(/_/g, "/");
    value = JSON.parse(atob(padded + "=".repeat((4 - (padded.length % 4)) % 4)));
  } catch {
    return null;
  }
  if (!isRecord(value) || value.s !== sort || typeof value.i !== "string") return null;
  if (!COMMUNITY_ID.test(value.i) && !OFFICIAL_ID.test(value.i)) return null;
  if (sort === "used") {
    const u = value.u;
    if (typeof u !== "number" || !Number.isSafeInteger(u) || u < 0) return null;
    return { s: "used", u, i: value.i };
  }
  const t = value.t;
  if (typeof t !== "number" || !Number.isSafeInteger(t) || t < 0) return null;
  return { s: "new", t, i: value.i };
}

function publicRow(row: Row): Record<string, unknown> {
  return {
    id: row.id,
    name: row.name,
    config: JSON.parse(row.config) as unknown,
    uses: row.uses,
    official: row.official === 1,
    created_day: row.created_day,
  };
}

/**
 * Keyset pages over visible rows only. `hidden = 0` is part of every query, so a cursor (which is
 * only a position: a count or a time, and an id) can never reach a hidden row, whatever it holds.
 */
async function list(env: Env, url: URL, respond: Respond): Promise<Response> {
  const sortText = url.searchParams.get("sort") ?? "used";
  if (sortText !== "used" && sortText !== "new") return respond(400, { error: "bad_sort" });
  const sort: "used" | "new" = sortText;
  const cursorText = url.searchParams.get("cursor");
  const cursor = cursorText === null || cursorText === "" ? null : decodeCursor(cursorText, sort);
  if (cursorText !== null && cursorText !== "" && cursor === null) return respond(400, { error: "bad_cursor" });

  const columns = "id, name, config, uses, reports, official, hidden, created_at, created_day";
  let statement: D1PreparedStatement;
  if (sort === "used") {
    statement =
      cursor === null || cursor.s !== "used"
        ? env.DB.prepare(`SELECT ${columns} FROM crosshairs WHERE hidden = 0 ORDER BY uses DESC, id ASC LIMIT ?1`).bind(PAGE_SIZE + 1)
        : env.DB.prepare(
            `SELECT ${columns} FROM crosshairs WHERE hidden = 0 AND (uses < ?2 OR (uses = ?2 AND id > ?3))
             ORDER BY uses DESC, id ASC LIMIT ?1`,
          ).bind(PAGE_SIZE + 1, cursor.u, cursor.i);
  } else {
    statement =
      cursor === null || cursor.s !== "new"
        ? env.DB.prepare(`SELECT ${columns} FROM crosshairs WHERE hidden = 0 ORDER BY created_at DESC, id DESC LIMIT ?1`).bind(PAGE_SIZE + 1)
        : env.DB.prepare(
            `SELECT ${columns} FROM crosshairs WHERE hidden = 0 AND (created_at < ?2 OR (created_at = ?2 AND id < ?3))
             ORDER BY created_at DESC, id DESC LIMIT ?1`,
          ).bind(PAGE_SIZE + 1, cursor.t, cursor.i);
  }
  const { results } = await statement.all<Row>();
  const page = results.slice(0, PAGE_SIZE);
  const last = page[page.length - 1];
  const next =
    results.length > PAGE_SIZE && last !== undefined
      ? encodeCursor(sort === "used" ? { s: "used", u: last.uses, i: last.id } : { s: "new", t: last.created_at, i: last.id })
      : null;
  return respond(200, { crosshairs: page.map(publicRow), next_cursor: next }, { "cache-control": `public, max-age=${LIST_MAX_AGE}` });
}

async function remove(request: Request, env: Env, id: string, respond: Respond): Promise<Response> {
  const body = await readJsonBody(request, DELETE_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason, respond);
  const token = isRecord(body.value) ? body.value.delete_token : undefined;
  if (typeof token !== "string" || !TOKEN.test(token)) return respond(400, { error: "bad_token" });
  const row = await env.DB.prepare("SELECT delete_token_hash FROM crosshairs WHERE id = ?1")
    .bind(id)
    .first<{ delete_token_hash: string | null }>();
  if (row === null) return respond(404, { error: "not_found" });
  // Official rows have no token, so no token deletes them.
  const given = await tokenHash(token);
  const stored = row.delete_token_hash;
  if (stored === null || !sameHex(given, stored)) return respond(403, { error: "wrong_token" });
  await deleteRow(env.DB, id);
  return respond(200, { deleted: true });
}

async function deleteRow(db: D1Database, id: string): Promise<number> {
  const results = await db.batch([
    db.prepare("DELETE FROM crosshairs WHERE id = ?1").bind(id),
    db.prepare("DELETE FROM crosshair_uses WHERE crosshair_id = ?1").bind(id),
    db.prepare("DELETE FROM crosshair_reports WHERE crosshair_id = ?1").bind(id),
  ]);
  return results[0]?.meta.changes ?? 0;
}

async function visible(db: D1Database, id: string): Promise<{ official: number } | null> {
  return db.prepare("SELECT official FROM crosshairs WHERE id = ?1 AND hidden = 0").bind(id).first<{ official: number }>();
}

async function use(request: Request, env: Env, id: string, respond: Respond): Promise<Response> {
  const now = Date.now();
  const gate = await requestBucket(request, env, now, respond);
  if ("response" in gate) return gate.response;
  await request.body?.cancel();
  if ((await visible(env.DB, id)) === null) return respond(404, { error: "not_found" });
  // One transaction: the dedupe row goes in only under the network's daily cap, the cap row and
  // the count only when it did (`changes()` is the previous statement's).
  const [added] = await env.DB.batch([
    env.DB.prepare(
      `INSERT OR IGNORE INTO crosshair_uses (bucket, crosshair_id, created_at)
       SELECT ?1, ?2, ?3 WHERE (SELECT COUNT(*) FROM crosshair_daily WHERE bucket = ?1 AND kind = 'use') < ?4`,
    ).bind(gate.bucket, id, now, USES_PER_BUCKET_PER_DAY),
    env.DB.prepare("INSERT INTO crosshair_daily (bucket, kind, day, created_at) SELECT ?1, 'use', ?2, ?3 WHERE changes() > 0").bind(
      gate.bucket,
      utcDay(now),
      now,
    ),
    env.DB.prepare("UPDATE crosshairs SET uses = uses + 1 WHERE id = ?1 AND changes() > 0").bind(id),
  ]);
  return respond(200, { counted: (added?.meta.changes ?? 0) > 0 });
}

async function report(request: Request, env: Env, id: string, respond: Respond): Promise<Response> {
  const now = Date.now();
  const gate = await requestBucket(request, env, now, respond);
  if ("response" in gate) return gate.response;
  await request.body?.cancel();
  const row = await visible(env.DB, id);
  if (row === null) return respond(404, { error: "not_found" });
  // Sigil's own crosshairs are never hidden by reports, so a report on one stores nothing.
  if (row.official === 1) return respond(200, { counted: false });
  const marker = await reportMarker(request.headers.get("cf-connecting-ip"), gate.secret, id);
  if (marker === null) return respond(400, { error: "no_network" });
  const known = await env.DB.prepare("SELECT 1 AS n FROM crosshair_reports WHERE marker = ?1").bind(marker).first();
  if (known !== null) return respond(200, { counted: false });
  const today = await env.DB.prepare("SELECT COUNT(*) AS n FROM crosshair_daily WHERE bucket = ?1 AND kind = 'report'")
    .bind(gate.bucket)
    .first<number>("n");
  if ((today ?? 0) >= REPORTS_PER_BUCKET_PER_DAY) return respond(429, { error: "report_limit" });
  const [added] = await env.DB.batch([
    env.DB.prepare(
      `INSERT OR IGNORE INTO crosshair_reports (marker, crosshair_id, created_at)
       SELECT ?1, ?2, ?3 WHERE (SELECT COUNT(*) FROM crosshair_daily WHERE bucket = ?4 AND kind = 'report') < ?5`,
    ).bind(marker, id, now, gate.bucket, REPORTS_PER_BUCKET_PER_DAY),
    env.DB.prepare("INSERT INTO crosshair_daily (bucket, kind, day, created_at) SELECT ?1, 'report', ?2, ?3 WHERE changes() > 0").bind(
      gate.bucket,
      utcDay(now),
      now,
    ),
    // The count is the distinct report rows, so nothing but distinct networks can raise it.
    env.DB.prepare(
      `UPDATE crosshairs SET
         reports = (SELECT COUNT(*) FROM crosshair_reports WHERE crosshair_id = ?1),
         hidden = CASE WHEN official = 0 AND (SELECT COUNT(*) FROM crosshair_reports WHERE crosshair_id = ?1) >= ?2 THEN 1 ELSE hidden END
       WHERE id = ?1`,
    ).bind(id, REPORTS_TO_HIDE),
  ]);
  return respond(200, { counted: (added?.meta.changes ?? 0) > 0 });
}

/** 503 without a usable admin key, 401 for a wrong one, null when the caller is the owner. */
async function adminRefusal(request: Request, env: Env, respond: Respond): Promise<Response | null> {
  const admin = adminKey(env);
  if (!secretOk(admin)) {
    await request.body?.cancel();
    return respond(503, { error: "not_configured" });
  }
  if (!(await adminAllowed(request.headers.get("authorization"), admin))) {
    await request.body?.cancel();
    return respond(401, { error: "unauthorized" });
  }
  return null;
}

async function adminFlagged(request: Request, env: Env, respond: Respond): Promise<Response> {
  const refused = await adminRefusal(request, env, respond);
  if (refused !== null) return refused;
  const { results } = await env.DB.prepare(
    `SELECT id, name, config, uses, reports, official, hidden, created_at, created_day FROM crosshairs
     WHERE hidden = 1 OR reports > 0 ORDER BY hidden DESC, reports DESC, id ASC LIMIT 500`,
  ).all<Row>();
  return respond(200, {
    crosshairs: results.map((row) => ({ ...publicRow(row), reports: row.reports, hidden: row.hidden === 1 })),
  });
}

async function adminAction(request: Request, env: Env, id: string, action: "remove" | "restore", respond: Respond): Promise<Response> {
  const refused = await adminRefusal(request, env, respond);
  if (refused !== null) return refused;
  await request.body?.cancel();
  if (action === "remove") {
    return (await deleteRow(env.DB, id)) > 0 ? respond(200, { removed: true }) : respond(404, { error: "not_found" });
  }
  const result = await env.DB.prepare("UPDATE crosshairs SET hidden = 0, reports = 0 WHERE id = ?1").bind(id).run();
  if (result.meta.changes === 0) return respond(404, { error: "not_found" });
  await env.DB.prepare("DELETE FROM crosshair_reports WHERE crosshair_id = ?1").bind(id).run();
  return respond(200, { restored: true });
}

async function adminOfficial(request: Request, env: Env, respond: Respond): Promise<Response> {
  const refused = await adminRefusal(request, env, respond);
  if (refused !== null) return refused;
  const body = await readJsonBody(request, PUBLISH_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason, respond);
  const id = isRecord(body.value) ? body.value.id : undefined;
  if (typeof id !== "string" || !OFFICIAL_ID.test(id)) return respond(400, { error: "bad_id" });
  const entry = await validateEntry(body.value, respond);
  if (entry instanceof Response) return entry;
  const other = await existingId(env.DB, entry.hash);
  if (other !== null && other !== id) return respond(409, { error: "duplicate", id: other });
  const now = Date.now();
  await env.DB.prepare(
    `INSERT INTO crosshairs (id, name, config, config_hash, uses, reports, official, hidden, created_at, created_day, delete_token_hash)
     VALUES (?1, ?2, ?3, ?4, 0, 0, 1, 0, ?5, ?6, NULL)
     ON CONFLICT(id) DO UPDATE SET name = excluded.name, config = excluded.config, config_hash = excluded.config_hash,
       official = 1, hidden = 0, reports = 0, delete_token_hash = NULL`,
  )
    .bind(id, entry.name, entry.config, entry.hash, now, utcDay(now))
    .run();
  return respond(200, { id });
}
