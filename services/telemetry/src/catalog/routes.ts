/**
 * Public read routes over the catalog enrichment. No player data is involved: no consent gate, no
 * secret needed, and they never fail because enrichment is behind (an empty index is an answer).
 *
 *   GET /v1/catalog/tags              the whole index: string table, alias table, per-mod ids
 *   GET /v1/catalog/tags?since=<ms>   mods enriched after <ms>, with their own string table
 *   GET /v1/catalog/mod/<id>          per-mod detail: files, update notes, health counts, facts
 *   GET /v1/catalog/patch/<id>        one official patch's line classes; `latest` for the newest
 */

import { getState, gunzipBase64 } from "./enrich";
import { modDetail, PUBLISH_VERSION, type CatalogIndex, type JudgmentRows } from "./publish";
import { publishPatch, type PatchLine } from "./patches";
import type { Stored } from "./judgments";

/** A few hours: the index changes at most hourly. */
export const TAGS_MAX_AGE = 3 * 60 * 60;
const DETAIL_MAX_AGE = 60 * 60;

type Respond = (status: number, body: unknown, extra?: Record<string, string>) => Response;

function cacheHeaders(etag: string, maxAge: number): Record<string, string> {
  return { etag: `"${etag}"`, "cache-control": `public, max-age=${maxAge}` };
}

function notModified(request: Request, etag: string, maxAge: number): Response | null {
  const sent = request.headers.get("if-none-match");
  if (sent === null) return null;
  const tags = sent.split(",").map((tag) => tag.trim().replace(/^W\//, ""));
  return tags.includes(`"${etag}"`) || tags.includes("*") ? new Response(null, { status: 304, headers: cacheHeaders(etag, maxAge) }) : null;
}

const EMPTY: CatalogIndex = { v: PUBLISH_VERSION, built: 0, strings: [], aliases: {}, mods: [] };

export async function handleTags(request: Request, env: Env, url: URL, respond: Respond): Promise<Response> {
  const sinceText = url.searchParams.get("since");
  if (sinceText !== null && !/^\d{1,15}$/.test(sinceText)) return respond(400, { error: "bad_since" });
  const meta = await getState(env.DB, "index_meta");
  const stored = meta === null ? null : await getState(env.DB, "index");
  const baseTag = meta === null ? "empty" : (JSON.parse(meta.value) as { etag: string }).etag;
  const etag = sinceText === null ? baseTag : `${baseTag}-${sinceText}`;
  const cached = notModified(request, etag, TAGS_MAX_AGE);
  if (cached !== null) return cached;
  const index = stored === null ? EMPTY : (JSON.parse(await gunzipBase64(stored.value)) as CatalogIndex);
  const body = sinceText === null ? index : delta(index, Number(sinceText));
  return respond(200, body, cacheHeaders(etag, TAGS_MAX_AGE));
}

/** Mods enriched after `since`, renumbered against their own string table (ids are per build). */
export function delta(index: CatalogIndex, since: number): CatalogIndex {
  const strings: string[] = [];
  const ids = new Map<number, number>();
  const mods = index.mods
    .filter((mod) => mod.u > since)
    .map((mod) => ({
      ...mod,
      x: mod.x.map((packed) => {
        const old = Math.floor(packed / 4);
        let id = ids.get(old);
        if (id === undefined) {
          id = strings.length;
          strings.push(index.strings[old] ?? "");
          ids.set(old, id);
        }
        return id * 4 + (packed % 4);
      }),
    }));
  return { v: index.v, built: index.built, strings, aliases: index.aliases, mods };
}

export async function handleModDetail(request: Request, env: Env, id: number, respond: Respond): Promise<Response> {
  const { results } = await env.DB.prepare("SELECT judgment, data, enriched_at FROM mod_tags WHERE mod_id = ?1 AND judgment != '_failed'")
    .bind(id)
    .all<{ judgment: string; data: string; enriched_at: number }>();
  const judged: Record<string, Stored> = {};
  let newest = 0;
  for (const row of results) {
    const data = JSON.parse(row.data) as Stored;
    newest = Math.max(newest, row.enriched_at);
    if (data.na !== 1) judged[row.judgment] = data;
  }
  const etag = `m${id}-${newest}-${PUBLISH_VERSION}`;
  const cached = notModified(request, etag, DETAIL_MAX_AGE);
  if (cached !== null) return cached;
  return respond(200, modDetail(id, judged as JudgmentRows), cacheHeaders(etag, DETAIL_MAX_AGE));
}

export async function handlePatch(request: Request, env: Env, id: string, respond: Respond): Promise<Response> {
  const row =
    id === "latest"
      ? await env.DB.prepare("SELECT patch_id, title, posted_at, data, enriched_at FROM patch_tags ORDER BY posted_at DESC LIMIT 1").first<PatchRow>()
      : await env.DB.prepare("SELECT patch_id, title, posted_at, data, enriched_at FROM patch_tags WHERE patch_id = ?1").bind(id).first<PatchRow>();
  if (row === null) return respond(404, { error: "unknown_patch" });
  const etag = `p${row.patch_id}-${row.enriched_at}`;
  const cached = notModified(request, etag, DETAIL_MAX_AGE);
  if (cached !== null) return cached;
  const progress = JSON.parse(row.data) as { lines: PatchLine[]; next: number; total: number };
  return respond(
    200,
    { ...publishPatch(row.patch_id, row.title, row.posted_at, progress.lines), complete: progress.next >= progress.total },
    cacheHeaders(etag, DETAIL_MAX_AGE),
  );
}

type PatchRow = { patch_id: string; title: string; posted_at: number; data: string; enriched_at: number };
