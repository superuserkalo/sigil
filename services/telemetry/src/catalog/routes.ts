/**
 * Public read routes over the catalog enrichment. No player data is involved: no consent gate, no
 * secret needed, and they never fail because enrichment is behind (an empty index is an answer).
 *
 *   GET /v1/catalog/tags              the whole index: string table, per-mod ids
 *   GET /v1/catalog/tags?since=<ms>   mods enriched after <ms>, with their own string table
 *   GET /v1/catalog/mod/<id>          per-mod detail: the file pre-selection hint (flagged)
 */

import { getState, loadIndex } from "./enrich";
import { modDetail, PACK, PUBLISH_VERSION, type CatalogIndex, type JudgmentRows } from "./publish";
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

const EMPTY: CatalogIndex = { v: PUBLISH_VERSION, built: 0, strings: [], mods: [] };

export async function handleTags(request: Request, env: Env, url: URL, respond: Respond): Promise<Response> {
  const sinceText = url.searchParams.get("since");
  if (sinceText !== null && !/^\d{1,15}$/.test(sinceText)) return respond(400, { error: "bad_since" });
  const meta = await getState(env.DB, "index_meta");
  const baseTag = meta === null ? "empty" : (JSON.parse(meta.value) as { etag: string }).etag;
  const etag = sinceText === null ? baseTag : `${baseTag}-${sinceText}`;
  const cached = notModified(request, etag, TAGS_MAX_AGE);
  if (cached !== null) return cached;
  const index = (await loadIndex(env.DB)) ?? EMPTY;
  const body = sinceText === null ? index : delta(index, Number(sinceText));
  return respond(200, body, cacheHeaders(etag, TAGS_MAX_AGE));
}

/** Mods enriched after `since`, renumbered against their own string table (ids are per build). */
export function delta(index: CatalogIndex, since: number): CatalogIndex {
  const strings: string[] = [];
  const ids = new Map<number, number>();
  const remap = (old: number): number => {
    let id = ids.get(old);
    if (id === undefined) {
      id = strings.length;
      strings.push(index.strings[old] ?? "");
      ids.set(old, id);
    }
    return id;
  };
  const mods = index.mods
    .filter((mod) => mod.u > since)
    .map((mod) => ({
      ...mod,
      x: mod.x.map((packed) => {
        return remap(Math.floor(packed / PACK)) * PACK + (packed % PACK);
      }),
    }));
  return { v: index.v, built: index.built, strings, mods };
}

export async function handleModDetail(request: Request, env: Env, id: number, respond: Respond): Promise<Response> {
  const { results } = await env.DB.prepare("SELECT judgment, data, enriched_at FROM mod_tags WHERE mod_id = ?1")
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
