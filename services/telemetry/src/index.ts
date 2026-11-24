/**
 * Sigil's opt-in telemetry Worker.
 *
 * Eight endpoints, one cron job, no UI:
 *
 *   POST /v1/events       usage events, one Analytics Engine point per event
 *   POST /v1/crash        scrubbed crash report (panic, unclean exit or native), deduplicated
 *                         in D1 by fingerprint, with the session's scrubbed log tail
 *   POST /v1/diagnostics  scrubbed bundle the player chose to send, returns a support code
 *   GET  /v1/identify     hash lookup for an installed VPK, stores nothing
 *   POST /v1/hashes       hash contributed by an app that just installed that file
 *   POST /v1/product      opt-in product data records, "Help build Sigil" (ADR 0017)
 *   POST /v1/product/delete  every raw product row for one install, on the player's press
 *   GET  /v1/product/related co-installed mods that pass the distinct-contributor floor
 *
 * The app is the only intended client and nothing here is fetched by the desktop app on its own:
 * every request is behind a Settings > Privacy switch (docs/decisions/0014-opt-in-telemetry.md,
 * docs/privacy.md).
 *
 * Rules that hold for every request:
 *
 * - `x-sigil-client: 1` and a semver `x-sigil-version` are required.
 * - The body is read with a byte cap, never buffered unbounded (`body.ts`).
 * - No address, user agent, install id or key is stored, and log lines never include a body.
 */

import { readJsonBody } from "./body";
import { diagnosticsCode } from "./codes";
import { isEventName } from "./events";
import { crashFingerprint } from "./fingerprint";
import { sha256Hex } from "./hash";
import { allowRequest } from "./limiter";
import { contribute, related } from "./aggregates";
import { scheduledJob } from "./cron";
import {
  installHash,
  secretOk,
  parseInstallId,
  parseProduct,
  parseRelated,
} from "./product";
import { scrub } from "./scrub";
import {
  isSemver,
  parseCrash,
  parseDiagnostics,
  parseEvents,
  parseHashes,
  parseIdentify,
  type CrashBody,
  type DiagnosticsBody,
  type EventsBody,
  type HashesBody,
} from "./validate";

const EVENTS_BODY_LIMIT = 32 * 1024;
// Message 8 KiB, backtrace 48 KiB and the log tail 16 KiB, with room for the envelope.
const CRASH_BODY_LIMIT = 96 * 1024;
const DIAGNOSTICS_BODY_LIMIT = 1024 * 1024;
const HASHES_BODY_LIMIT = 4 * 1024;
const PRODUCT_BODY_LIMIT = 32 * 1024;
const PRODUCT_DELETE_BODY_LIMIT = 1024;

const MAX_IDENTIFY_MATCHES = 5;
/// Distinct installs that must report a mapping before identify returns it.
const MIN_IDENTIFY_REPORTS = 2;

type Handler = (request: Request, env: Env, url: URL) => Promise<Response>;

type Route = {
  readonly method: "GET" | "POST";
  readonly run: Handler;
};

const ROUTES: Readonly<Record<string, Route>> = {
  "/v1/events": { method: "POST", run: handleEvents },
  "/v1/crash": { method: "POST", run: handleCrash },
  "/v1/diagnostics": { method: "POST", run: handleDiagnostics },
  "/v1/identify": { method: "GET", run: handleIdentify },
  "/v1/hashes": { method: "POST", run: handleHashes },
  "/v1/product": { method: "POST", run: handleProduct },
  "/v1/product/delete": { method: "POST", run: handleProductDelete },
  "/v1/product/related": { method: "GET", run: handleRelated },
};

type ProductSecrets = { readonly PRODUCT_SALT?: string; readonly PRODUCT_BUCKET_KEY?: string };

/** The product salt secret, when the owner set one (`wrangler secret put PRODUCT_SALT`). */
function productSalt(env: Env): string | undefined {
  return (env as Env & ProductSecrets).PRODUCT_SALT;
}

/** The network bucket key (`wrangler secret put PRODUCT_BUCKET_KEY`). Never in the repo. */
function bucketKey(env: Env): string | undefined {
  return (env as Env & ProductSecrets).PRODUCT_BUCKET_KEY;
}

/**
 * The product routes run only with both secrets set and long enough. Otherwise they answer 503
 * `not_configured` and store nothing: no unsalted hash, no aggregate. The app treats it as "try
 * later" and keeps its queue.
 */
function productSecrets(env: Env): { readonly salt: string; readonly bucket: string } | null {
  const salt = productSalt(env);
  const bucket = bucketKey(env);
  return secretOk(salt) && secretOk(bucket) ? { salt, bucket } : null;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const started = Date.now();
    const path = new URL(request.url).pathname;
    let response: Response;

    try {
      response = await dispatch(request, env);
    } catch (error) {
      logLine({
        event: "request_error",
        path,
        message: describeError(error),
      });
      response = errorResponse(500, "internal");
    }

    logLine({
      event: "request",
      method: request.method,
      path,
      status: response.status,
      durationMs: Date.now() - started,
    });
    return response;
  },

  async scheduled(controller: ScheduledController, env: Env): Promise<void> {
    await scheduledJob(env, Date.now(), controller.cron);
  },
} satisfies ExportedHandler<Env>;

async function dispatch(request: Request, env: Env): Promise<Response> {
  const url = new URL(request.url);
  const route = ROUTES[url.pathname];
  if (route === undefined) return errorResponse(404, "not_found");
  if (request.method !== route.method) {
    return jsonResponse(405, { error: "method_not_allowed" }, { allow: route.method });
  }

  if (!clientAccepted(request)) return errorResponse(400, "bad_client");

  const allowed = await allowRequest(request, env.LIMITER, Date.now());
  if (!allowed) return errorResponse(429, "rate_limited");

  return route.run(request, env, url);
}

function clientAccepted(request: Request): boolean {
  if (request.headers.get("x-sigil-client") !== "1") return false;
  const version = request.headers.get("x-sigil-version") ?? "";
  return isSemver(version);
}

async function handleEvents(request: Request, env: Env): Promise<Response> {
  const body = await readJsonBody(request, EVENTS_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);

  const parsed = parseEvents(body.value, Date.now());
  if (!parsed.ok) return errorResponse(400, parsed.error);

  const events: EventsBody = parsed.value;
  // Only a hash of the app's random install id reaches Analytics Engine, as the sampling index.
  const index = await sha256Hex(events.installId);

  let accepted = 0;
  let dropped = 0;
  for (const event of events.events) {
    if (!isEventName(event.name)) {
      dropped += 1;
      continue;
    }
    env.SIGIL_EVENTS.writeDataPoint({
      blobs: [event.name, event.page, events.version, events.os],
      doubles: [1],
      indexes: [index],
    });
    accepted += 1;
  }

  return jsonResponse(202, { accepted, dropped });
}

async function handleCrash(request: Request, env: Env): Promise<Response> {
  const body = await readJsonBody(request, CRASH_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);

  const parsed = parseCrash(body.value);
  if (!parsed.ok) return errorResponse(400, parsed.error);

  const crash: CrashBody = parsed.value;
  // The install id is validated so the client is held to the shape, then dropped: it is never
  // stored, so a crash row cannot be tied to an install.
  const message = scrub(crash.message);
  const backtrace = scrub(crash.backtrace);
  const context = scrub(crash.context);
  const fingerprint = await crashFingerprint(message, backtrace);
  const now = Date.now();

  // A repeat keeps the newest log tail when it brings one: the latest session says the most.
  await env.DB.prepare(
    `INSERT INTO crashes (created_at, version, os, message, backtrace, fingerprint, repeats, last_seen_at, kind, context)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?1, ?7, ?8)
     ON CONFLICT(fingerprint) DO UPDATE SET
       repeats = crashes.repeats + 1,
       last_seen_at = excluded.last_seen_at,
       context = CASE WHEN excluded.context = '' THEN crashes.context ELSE excluded.context END`,
  )
    .bind(now, crash.version, crash.os, message, backtrace, fingerprint, crash.kind, context)
    .run();

  const stored = await env.DB.prepare("SELECT repeats FROM crashes WHERE fingerprint = ?1")
    .bind(fingerprint)
    .first<{ repeats: number }>();

  return jsonResponse(202, { repeats: stored?.repeats ?? 1 });
}

async function handleDiagnostics(request: Request, env: Env): Promise<Response> {
  const body = await readJsonBody(request, DIAGNOSTICS_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);

  const parsed = parseDiagnostics(body.value);
  if (!parsed.ok) return errorResponse(400, parsed.error);

  const diagnostics: DiagnosticsBody = parsed.value;
  // The install id is validated and dropped, as with crashes.
  const summary = scrub(diagnostics.summary);
  const log = scrub(diagnostics.log);
  const now = Date.now();

  // A code collision is astronomically unlikely, but a lost bundle is worse than a retry.
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const code = diagnosticsCode();
    const result = await env.DB.prepare(
      `INSERT OR IGNORE INTO diagnostics (created_at, code, version, os, summary, log)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6)`,
    )
      .bind(now, code, diagnostics.version, diagnostics.os, summary, log)
      .run();

    if (result.meta.changes > 0) return jsonResponse(202, { code });
  }

  return errorResponse(500, "code_unavailable");
}

async function handleIdentify(_request: Request, env: Env, url: URL): Promise<Response> {
  const parsed = parseIdentify(url.searchParams);
  if (!parsed.ok) return errorResponse(400, parsed.error);

  const { results } = await env.DB.prepare(
    `SELECT gamebanana_mod_id, file_id, reports
     FROM mod_hashes
     WHERE sha256 = ?1 AND size = ?2 AND reports >= ?4
     ORDER BY reports DESC, updated_at DESC
     LIMIT ?3`,
  )
    .bind(parsed.value.sha256, parsed.value.size, MAX_IDENTIFY_MATCHES, MIN_IDENTIFY_REPORTS)
    .all<{ gamebanana_mod_id: number; file_id: number; reports: number }>();

  return jsonResponse(200, {
    matches: results.map((match) => ({
      gamebananaModId: match.gamebanana_mod_id,
      fileId: match.file_id,
      reports: match.reports,
    })),
  });
}

async function handleHashes(request: Request, env: Env): Promise<Response> {
  const body = await readJsonBody(request, HASHES_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);

  const parsed = parseHashes(body.value);
  if (!parsed.ok) return errorResponse(400, parsed.error);

  // Several submissions can ship the same file, so one hash maps to several mods and the report
  // count decides which one identify returns first. A contribution counts once per install, and
  // identify answers only once MIN_IDENTIFY_REPORTS installs agree, so one client cannot poison it.
  const contribution: HashesBody = parsed.value;
  const now = Date.now();
  // Count each install once: a second report from the same install changes nothing.
  const contributor = await sha256Hex(`sigil-hash-contributor:${contribution.installId}`);
  const added = await env.DB.prepare(
    `INSERT OR IGNORE INTO hash_contributors (sha256, size, gamebanana_mod_id, file_id, contributor, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)`,
  )
    .bind(contribution.sha256, contribution.size, contribution.gamebananaModId, contribution.fileId, contributor, now)
    .run();
  if (added.meta.changes === 0) {
    const known = await env.DB.prepare(
      `SELECT reports FROM mod_hashes
       WHERE sha256 = ?1 AND size = ?2 AND gamebanana_mod_id = ?3 AND file_id = ?4`,
    )
      .bind(contribution.sha256, contribution.size, contribution.gamebananaModId, contribution.fileId)
      .first<{ reports: number }>();
    return jsonResponse(202, { reports: known?.reports ?? 1 });
  }
  await env.DB.prepare(
    `INSERT INTO mod_hashes (sha256, size, gamebanana_mod_id, file_id, reports, created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
     ON CONFLICT(sha256, size, gamebanana_mod_id, file_id) DO UPDATE SET
       reports = mod_hashes.reports + 1,
       updated_at = excluded.updated_at`,
  )
    .bind(contribution.sha256, contribution.size, contribution.gamebananaModId, contribution.fileId, now)
    .run();

  const stored = await env.DB.prepare(
    `SELECT reports FROM mod_hashes
     WHERE sha256 = ?1 AND size = ?2 AND gamebanana_mod_id = ?3 AND file_id = ?4`,
  )
    .bind(contribution.sha256, contribution.size, contribution.gamebananaModId, contribution.fileId)
    .first<{ reports: number }>();

  return jsonResponse(202, { reports: stored?.reports ?? 1 });
}

async function handleProduct(request: Request, env: Env): Promise<Response> {
  const secrets = productSecrets(env);
  if (secrets === null) {
    await request.body?.cancel();
    return errorResponse(503, "not_configured");
  }
  const body = await readJsonBody(request, PRODUCT_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);

  const parsed = parseProduct(body.value);
  if (!parsed.ok) return errorResponse(400, parsed.error);

  const product = parsed.value;
  const now = Date.now();
  const hash = await installHash(product.installId, secrets.salt);

  // Raw rows carry the install hash only: no bucket, no address, no user agent.
  if (product.records.length > 0) {
    await env.DB.batch(
      product.records.map((record) =>
        env.DB.prepare(
          `INSERT INTO product_events (created_at, install_hash, type, version, os, os_build, game_build, payload)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)`,
        ).bind(now, hash, record.type, product.version, product.os, product.osBuild, product.gameBuild, JSON.stringify(record.fields)),
      ),
    );
  }
  // Open-epoch contributions: none without the bucket secret or past the bucket's cap.
  await contribute(env.DB, secrets.bucket, request.headers.get("cf-connecting-ip"), hash, product.records, now);

  return jsonResponse(202, { accepted: product.records.length, dropped: product.dropped });
}

/** Knowing the raw install id is the proof: only the app that made it can send it. */
async function handleProductDelete(request: Request, env: Env): Promise<Response> {
  const secrets = productSecrets(env);
  if (secrets === null) {
    await request.body?.cancel();
    return errorResponse(503, "not_configured");
  }
  const body = await readJsonBody(request, PRODUCT_DELETE_BODY_LIMIT);
  if (!body.ok) return bodyError(body.reason);
  const raw = body.value;
  const installId = typeof raw === "object" && raw !== null ? parseInstallId((raw as Record<string, unknown>).installId) : null;
  if (installId === null) return errorResponse(400, "bad_install_id");

  const hash = await installHash(installId, secrets.salt);
  const results = await env.DB.batch(
    // Raw events and open-epoch contributions (the only rows with a bucket). Closed-epoch totals
    // are counts with no install hash, so there is nothing of this install in them to delete.
    ["product_events", "product_contrib"].map((table) =>
      env.DB.prepare(`DELETE FROM ${table} WHERE install_hash = ?1`).bind(hash),
    ),
  );
  const deleted = results.reduce((sum, result) => sum + (result.meta.changes ?? 0), 0);
  return jsonResponse(200, { deleted });
}

/** Co-installed mods that pass the distinct-contributor floor (`aggregates.ts`). */
async function handleRelated(_request: Request, env: Env, url: URL): Promise<Response> {
  const secrets = productSecrets(env);
  if (secrets === null) return errorResponse(503, "not_configured");
  const mod = parseRelated(url.searchParams);
  if (mod === null) return errorResponse(400, "bad_mod");
  return jsonResponse(200, { mods: await related(env.DB, secrets.bucket, mod) });
}

function bodyError(reason: "too_large" | "invalid_utf8" | "invalid_json"): Response {
  if (reason === "too_large") return errorResponse(413, "body_too_large");
  if (reason === "invalid_utf8") return errorResponse(400, "invalid_utf8");
  return errorResponse(400, "invalid_json");
}

function errorResponse(status: number, error: string): Response {
  return jsonResponse(status, { error });
}

function jsonResponse(status: number, body: unknown, extra?: Record<string, string>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
      "referrer-policy": "no-referrer",
      ...extra,
    },
  });
}

function describeError(error: unknown): string {
  if (error instanceof Error) return scrub(`${error.name}: ${error.message}`).slice(0, 300);
  return "unknown error";
}

/** Structured JSON, one line per record. Never the body, never a header value. */
function logLine(fields: Record<string, string | number>): void {
  console.log(JSON.stringify(fields));
}
