/**
 * The scheduled enrichment run: bounded work, resumable, stops on the first remote error.
 *
 * Each run walks GameBanana's Deadlock mod list (newest updates and newest uploads first, then a
 * cursor page that backfills the rest of the catalog), picks the mods whose page changed or whose
 * judgments are out of date, reads each one's public page, asks Workers AI for candidate search
 * terms, and sends Jev one request per mod with every stale judgment's questions over one shared
 * state. Then it may classify one official patch note, and rebuilds the published tag index.
 *
 * Fail closed: without `TYPESAFE_API_KEY` nothing is fetched or sent; without the `AI` binding the
 * search-term judgment is skipped and the rest runs.
 */

import { aiBinding, expand, type Expansion } from "./expand";
import { GameBanana, type ListedMod, type ModComment, type ModUpdate, type Profile } from "./gamebanana";
import { loadShot } from "./images";
import {
  COMMENT_WINDOW_DAYS,
  JUDGMENTS,
  MAX_COMMENTS,
  MAX_FILES,
  MAX_OTHERS,
  MAX_SHOTS,
  MAX_UPDATES,
  SEP,
  type Judgment,
  type ModInput,
  type Other,
  type Part,
  type Stored,
} from "./judgments";
import { Budget, BudgetSpent, liveNet, RemoteError, type Net } from "./net";
import { fetchPatches, PATCH_CHUNK, PATCH_VERSION, patchRequest, readPatch, type PatchLine } from "./patches";
import { buildIndex, PUBLISH_VERSION, type JudgmentRows, type ModMeta } from "./publish";
import { askJev, JEV_MODEL, RejectedRequest, typesafeKey, USD_PER_TOKEN, type Asset, type Question, type SystemOneBody } from "./typesafe";
import { normalize } from "./aliases";
import { embeddingsOn, embedText, EMBED_MODEL } from "./embed";
import { LEARN_INTERVAL_MS, learnFromGaps } from "./learn";
import type { CatalogIndex } from "./publish";
import { sha256Hex } from "../hash";

/** Subrequests per run: under the Workers free-plan limit of 50, with room to spare. */
export const RUN_SUBREQUESTS = 45;
/** A run that would spend more than this stops and logs an error, so a bug cannot run away. */
export const MAX_RUN_USD = 25;
/** Image tokens per screenshot, for the estimate before a call (the real count comes back). */
export const IMAGE_TOKEN_GUESS = 1600;
export const HEALTH_MIN_AGE_MS = 3 * 24 * 60 * 60 * 1000;
export const HEROES_MAX_AGE_MS = 24 * 60 * 60 * 1000;
export const INDEX_MIN_AGE_MS = 60 * 60 * 1000;
export const LEASE_MS = 10 * 60 * 1000;
/** Defaults for the two caps; override with the Worker vars ENRICH_RUN_MODS and ENRICH_DAILY_USD. */
export const DEFAULT_RUN_MODS = 6;
export const DEFAULT_DAILY_USD = 25;
/** A mod not seen in the listing for this long (withdrawn or deleted) leaves the index. */
export const UNSEEN_MS = 14 * 24 * 60 * 60 * 1000;

function config(env: Env): { runMods: number; dailyUsd: number } {
  const vars = env as Env & { readonly ENRICH_RUN_MODS?: string; readonly ENRICH_DAILY_USD?: string };
  const runMods = Number(vars.ENRICH_RUN_MODS ?? DEFAULT_RUN_MODS);
  const dailyUsd = Number(vars.ENRICH_DAILY_USD ?? DEFAULT_DAILY_USD);
  return {
    runMods: Number.isFinite(runMods) && runMods > 0 ? Math.min(50, Math.floor(runMods)) : DEFAULT_RUN_MODS,
    dailyUsd: Number.isFinite(dailyUsd) && dailyUsd > 0 ? dailyUsd : DEFAULT_DAILY_USD,
  };
}

/** Dollars spent so far today (UTC), from the running tally. */
async function spentToday(db: D1Database, now: number): Promise<number> {
  const day = new Date(now).toISOString().slice(0, 10);
  const row = await getState(db, "spend_day");
  const parsed = row === null ? null : (JSON.parse(row.value) as { day: string; usd: number });
  return parsed !== null && parsed.day === day ? parsed.usd : 0;
}
const DESCRIPTION_CHARS = 2000;

export type RunReport = {
  skipped?: string;
  /** Mods sent to Jev. */
  mods: number;
  /** Mods whose page bump changed nothing in the content hash: no model call. */
  unchanged: number;
  failed: number;
  withdrawn: number;
  judgments: number;
  tokens: number;
  usd: number;
  patches: number;
  stopped?: string;
  indexBuilt: boolean;
};

type TagRow = { judgment: string; version: number; source: string; model: string; data: string; enriched_at: number; hash: string };

// ---------------------------------------------------------------------------------------------
// Small state helpers
// ---------------------------------------------------------------------------------------------

export async function getState(db: D1Database, key: string): Promise<{ value: string; updatedAt: number } | null> {
  const row = await db.prepare("SELECT value, updated_at FROM enrich_state WHERE key = ?1").bind(key).first<{ value: string; updated_at: number }>();
  return row === null ? null : { value: row.value, updatedAt: row.updated_at };
}

export async function setState(db: D1Database, key: string, value: string, now: number): Promise<void> {
  await db
    .prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
    .bind(key, value, now)
    .run();
}

function sourceFor(judgment: Judgment, mod: ListedMod): string {
  return judgment.name === "health" ? `p:${mod.postCount}` : `u:${mod.updated}`;
}

/** Which judgments need (re)running for this mod, given its stored rows. */
export function staleJudgments(mod: ListedMod, rows: readonly TagRow[], now: number, aiReady: boolean): Judgment[] {
  return JUDGMENTS.filter((judgment) => {
    if (judgment.name === "terms" && !aiReady) return false;
    const row = rows.find((candidate) => candidate.judgment === judgment.name);
    if (row === undefined) return true;
    if (row.version !== judgment.version || row.model !== JEV_MODEL) return true;
    if (row.source === sourceFor(judgment, mod)) return false;
    // Comments move often: refresh health at most every few days.
    if (judgment.name === "health") return now - row.enriched_at >= HEALTH_MIN_AGE_MS;
    return true;
  });
}

// ---------------------------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------------------------

export async function runEnrichment(env: Env, now: number, net: Net = liveNet, subrequests = RUN_SUBREQUESTS): Promise<RunReport> {
  const report: RunReport = { mods: 0, unchanged: 0, failed: 0, withdrawn: 0, judgments: 0, tokens: 0, usd: 0, patches: 0, indexBuilt: false };
  const key = typesafeKey(env);
  if (key === null) {
    log({ event: "enrich_skipped", reason: "no_typesafe_key" });
    return { ...report, skipped: "no_typesafe_key" };
  }
  const lease = await getState(env.DB, "lease");
  if (lease !== null && Number(lease.value) > now) return { ...report, skipped: "lease_held" };
  await setState(env.DB, "lease", String(now + LEASE_MS), now);

  const caps = config(env);
  const dayStart = await spentToday(env.DB, now);
  if (dayStart >= caps.dailyUsd) {
    log({ event: "error", what: "enrich_daily_cap", usd: dayStart });
    await setState(env.DB, "lease", "0", now);
    return { ...report, skipped: "daily_cap" };
  }
  const budget = new Budget(subrequests);
  const gb = new GameBanana(net, budget);
  const ai = aiBinding(env);
  let changed = false;
  try {
    const heroes = await heroNames(env.DB, gb, now);
    const listed = await candidates(env.DB, gb);
    for (const mod of listed) {
      await rememberMod(env.DB, mod, now);
    }
    for (const mod of listed) {
      const rows = await tagRows(env.DB, mod.id);
      if (await gaveUp(env.DB, mod)) continue;
      const stale = staleJudgments(mod, rows, now, ai !== null);
      if (stale.length === 0) continue;
      if (budget.remaining < 4 + MAX_SHOTS) {
        report.stopped = "budget";
        break;
      }
      if (report.mods + report.unchanged + report.failed >= caps.runMods) {
        report.stopped = "run_mods";
        break;
      }
      if (dayStart + report.usd >= caps.dailyUsd) {
        report.stopped = "daily_cap";
        log({ event: "error", what: "enrich_daily_cap", usd: dayStart + report.usd });
        break;
      }
      let done: Outcome;
      try {
        done = await enrichMod(env, net, budget, gb, key, ai, mod, stale, heroes, now, report);
      } catch (error) {
        // A remote refusing or the budget ending stops the run; anything else is this mod's
        // problem (a hostile or broken page): record it and go on to the next mod.
        if (error instanceof RemoteError || error instanceof BudgetSpent) throw error;
        log({ event: "enrich_mod_error", mod: mod.id, message: error instanceof Error ? `${error.name}: ${error.message}`.slice(0, 200) : "unknown" });
        await noteFailure(env.DB, mod, now);
        report.failed += 1;
        continue;
      }
      if (done === "stop") break;
      if (done === "stored") changed = true;
    }
    await advanceCursor(env.DB, listed, now, ai !== null);
    if (report.stopped === undefined && budget.remaining >= 3) {
      if (await enrichPatches(env, net, budget, key, heroes, now, report)) changed = true;
    }
    if (report.stopped === undefined && budget.remaining >= 2) {
      const last = await getState(env.DB, "learn_at");
      if (last === null || now - Number(last.value) >= LEARN_INTERVAL_MS) {
        await setState(env.DB, "learn_at", String(now), now);
        const bucket = (env as Env & { readonly PRODUCT_BUCKET_KEY?: string }).PRODUCT_BUCKET_KEY;
        const learned = await learnFromGaps(env.DB, net, budget, key, bucket, await loadIndex(env.DB), now);
        report.tokens += learned.tokens;
        report.usd += learned.tokens * USD_PER_TOKEN;
        if (learned.added > 0) changed = true;
      }
    }
  } catch (error) {
    report.stopped = error instanceof BudgetSpent ? "budget" : error instanceof RemoteError ? `${error.host}_${error.status}` : "error";
    log({ event: "enrich_error", what: report.stopped, message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
  } finally {
    await addSpend(env.DB, report, now);
    await setState(env.DB, "lease", "0", now);
  }
  try {
    report.indexBuilt = await maybeBuildIndex(env.DB, now, changed, false, indexOptions(env));
  } catch (error) {
    log({ event: "enrich_index_error", message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
  }
  log({ event: "enrich_run", mods: report.mods, unchanged: report.unchanged, failed: report.failed, withdrawn: report.withdrawn, judgments: report.judgments, tokens: report.tokens, usd: Math.round(report.usd * 10000) / 10000, patches: report.patches, stopped: report.stopped ?? "" });
  return report;
}

async function heroNames(db: D1Database, gb: GameBanana, now: number): Promise<string[]> {
  const cached = await getState(db, "heroes");
  if (cached !== null && now - cached.updatedAt < HEROES_MAX_AGE_MS) return JSON.parse(cached.value) as string[];
  const heroes = await gb.heroes();
  if (heroes.length > 0) await setState(db, "heroes", JSON.stringify(heroes), now);
  return heroes.length > 0 ? heroes : cached === null ? [] : (JSON.parse(cached.value) as string[]);
}

/** Newest updates, newest uploads, then the backfill cursor page. Deduplicated, in that order. */
async function candidates(db: D1Database, gb: GameBanana): Promise<ListedMod[]> {
  const cursor = Number((await getState(db, "cursor"))?.value ?? "2");
  const lists = [await gb.listMods("Generic_LatestUpdated", 1), await gb.listMods("Generic_Newest", 1), await gb.listMods("Generic_LatestUpdated", Math.max(2, cursor))];
  const seen = new Set<number>();
  const out: ListedMod[] = [];
  for (const list of lists) {
    for (const mod of list) {
      if (seen.has(mod.id)) continue;
      seen.add(mod.id);
      out.push(mod);
    }
  }
  await setState(db, "cursor_page_size", String(lists[2]?.length ?? 0), 0);
  return out;
}

/** Move to the next backfill page once nothing on the current one is stale; wrap at the end. */
async function advanceCursor(db: D1Database, listed: readonly ListedMod[], now: number, aiReady: boolean): Promise<void> {
  const cursor = Number((await getState(db, "cursor"))?.value ?? "2");
  const size = Number((await getState(db, "cursor_page_size"))?.value ?? "0");
  const page = listed.slice(-size);
  for (const mod of page) {
    if (staleJudgments(mod, await tagRows(db, mod.id), now, aiReady).length > 0 && !(await gaveUp(db, mod))) return;
  }
  await setState(db, "cursor", String(size < 20 ? 2 : Math.max(2, cursor) + 1), now);
}

async function tagRows(db: D1Database, modId: number): Promise<TagRow[]> {
  const { results } = await db.prepare("SELECT judgment, version, source, model, data, enriched_at, hash FROM mod_tags WHERE mod_id = ?1").bind(modId).all<TagRow>();
  return results;
}

async function rememberMod(db: D1Database, mod: ListedMod, now: number): Promise<void> {
  await db
    .prepare(
      `INSERT INTO catalog_mods (mod_id, mod_updated, added_at, title, title_norm, category, subcategory, submitter_id, gb_rated, post_count, seen_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
       ON CONFLICT(mod_id) DO UPDATE SET mod_updated = excluded.mod_updated, title = excluded.title, title_norm = excluded.title_norm,
         category = excluded.category, subcategory = excluded.subcategory, gb_rated = excluded.gb_rated,
         post_count = excluded.post_count, seen_at = excluded.seen_at`,
    )
    .bind(mod.id, mod.updated, mod.added, mod.title, normalize(mod.title), mod.category, mod.subcategory, mod.submitterId, mod.rated ? 1 : 0, mod.postCount, now)
    .run();
}

/** Two content rejections for the same page version: skip it until the page changes. */
async function gaveUp(db: D1Database, mod: ListedMod): Promise<boolean> {
  const row = await db.prepare("SELECT source, data FROM mod_tags WHERE mod_id = ?1 AND judgment = '_failed'").bind(mod.id).first<{ source: string; data: string }>();
  return row !== null && row.source === `u:${mod.updated}` && (JSON.parse(row.data) as { attempts: number }).attempts >= 2;
}

async function noteFailure(db: D1Database, mod: ListedMod, now: number): Promise<void> {
  const row = await db.prepare("SELECT source, data FROM mod_tags WHERE mod_id = ?1 AND judgment = '_failed'").bind(mod.id).first<{ source: string; data: string }>();
  const attempts = row !== null && row.source === `u:${mod.updated}` ? (JSON.parse(row.data) as { attempts: number }).attempts + 1 : 1;
  await storeRow(db, mod.id, "_failed", 0, `u:${mod.updated}`, "", { attempts }, now);
}

function rowStatement(db: D1Database, modId: number, judgment: string, version: number, source: string, model: string, data: Stored, now: number, hash = ""): D1PreparedStatement {
  return db
    .prepare(
      `INSERT INTO mod_tags (mod_id, judgment, version, source, model, data, enriched_at, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
       ON CONFLICT(mod_id, judgment) DO UPDATE SET version = excluded.version, source = excluded.source, model = excluded.model,
         data = excluded.data, enriched_at = excluded.enriched_at, hash = excluded.hash`,
    )
    .bind(modId, judgment, version, source, model, JSON.stringify(data), now, hash);
}

async function storeRow(db: D1Database, modId: number, judgment: string, version: number, source: string, model: string, data: Stored, now: number, hash = ""): Promise<void> {
  await rowStatement(db, modId, judgment, version, source, model, data, now, hash).run();
}

/** Duplicate candidates found in code: same normalised title, or the same long description by another author. */
async function duplicateCandidates(db: D1Database, mod: ListedMod, descSig: string): Promise<Other[]> {
  const { results } = await db
    .prepare(
      `SELECT mod_id, title, category, desc_excerpt FROM catalog_mods
       WHERE mod_id != ?1 AND desc_excerpt != '' AND (title_norm = ?2 OR (?3 != '' AND desc_sig = ?3 AND submitter_id != ?4))
       ORDER BY added_at ASC LIMIT ?5`,
    )
    .bind(mod.id, normalize(mod.title), descSig, mod.submitterId, MAX_OTHERS)
    .all<{ mod_id: number; title: string; category: string; desc_excerpt: string }>();
  return results.map((row) => ({ id: row.mod_id, title: row.title, category: row.category, description: row.desc_excerpt }));
}

/** The public content a judgment reads: title, categories, text, files and screenshot URLs. */
export async function contentHash(mod: ListedMod, profile: Profile): Promise<string> {
  const parts = [
    mod.title,
    mod.category,
    mod.subcategory,
    profile.summary,
    profile.text,
    profile.files.map((file) => `${file.id}:${file.name}:${file.size}:${file.note}`).join("|"),
    profile.shots.map((shot) => shot.full).join("|"),
    String(profile.updatesCount),
  ];
  return (await sha256Hex(parts.join("\n"))).slice(0, 32);
}

async function descriptionSignature(profile: Profile): Promise<string> {
  const text = normalize(profile.text);
  return text.length >= 80 ? (await sha256Hex(text)).slice(0, 24) : "";
}

type Outcome = "stored" | "skipped" | "stop";

async function enrichMod(
  env: Env,
  net: Net,
  budget: Budget,
  gb: GameBanana,
  key: string,
  ai: ReturnType<typeof aiBinding>,
  mod: ListedMod,
  staleIn: readonly Judgment[],
  heroes: readonly string[],
  now: number,
  report: RunReport,
): Promise<Outcome> {
  let stale = [...staleIn];
  const needs = new Set<Part>(stale.flatMap((judgment) => judgment.needs));
  const profile = await gb.profile(mod.id);
  if (profile === null) {
    await env.DB.prepare("UPDATE catalog_mods SET withdrawn = 1 WHERE mod_id = ?1").bind(mod.id).run();
    report.withdrawn += 1;
    return "stored";
  }
  const hash = await contentHash(mod, profile);
  await env.DB.prepare("UPDATE catalog_mods SET content_hash = ?2, withdrawn = 0 WHERE mod_id = ?1").bind(mod.id, hash).run();
  // Same content under a new timestamp (a like, a file scan, a re-save): a judgment that was made on
  // exactly this content (its row carries the hash, written in the same batch as the judgment)
  // only moves to the new source, and nothing is spent. A row made on other content never moves.
  {
    const rows = await tagRows(env.DB, mod.id);
    const onlyMoved = stale.filter((judgment) => {
      const row = rows.find((candidate) => candidate.judgment === judgment.name);
      return row !== undefined && row.hash === hash && row.version === judgment.version && row.model === JEV_MODEL && judgment.name !== "health";
    });
    for (const judgment of onlyMoved) {
      await env.DB.prepare("UPDATE mod_tags SET source = ?3 WHERE mod_id = ?1 AND judgment = ?2").bind(mod.id, judgment.name, sourceFor(judgment, mod)).run();
    }
    stale = stale.filter((judgment) => !onlyMoved.includes(judgment));
    if (stale.length === 0) {
      report.unchanged += 1;
      return "skipped";
    }
  }
  // Every judgment row of this run, written together at the end in one batch with the mod's
  // judged hash, so a run that fails midway leaves nothing half-current.
  const writes: D1PreparedStatement[] = [];
  const descSig = await descriptionSignature(profile);
  await env.DB.prepare("UPDATE catalog_mods SET desc_excerpt = ?2, desc_sig = ?3 WHERE mod_id = ?1")
    .bind(mod.id, [profile.summary, profile.text].join("\n").trim().slice(0, 600), descSig)
    .run();
  const updates: ModUpdate[] = needs.has("updates") && profile.updatesCount > 0 ? (await gb.updates(mod.id, MAX_UPDATES)).slice(0, MAX_UPDATES) : [];
  const cutoff = now / 1000 - COMMENT_WINDOW_DAYS * 86400;
  const comments: ModComment[] =
    needs.has("comments") && mod.postCount > 0 ? (await gb.comments(mod.id, MAX_COMMENTS)).filter((comment) => comment.added >= cutoff).slice(0, MAX_COMMENTS) : [];
  const shots: { index: number; picture: Awaited<ReturnType<typeof loadShot>> & object }[] = [];
  if (needs.has("screenshots") || stale.some((judgment) => judgment.name === "terms")) {
    for (const [index, shot] of profile.shots.slice(0, MAX_SHOTS).entries()) {
      if (budget.remaining < 3) break;
      const picture = await loadShot(gb, shot);
      if (picture !== null) shots.push({ index, picture });
    }
  }
  const others = needs.has("others") ? await duplicateCandidates(env.DB, mod, descSig) : [];

  let expansion: Expansion | null = null;
  // One generator call for the text and one for the cover picture: two subrequests.
  if (ai !== null && stale.some((judgment) => judgment.name === "terms") && budget.remaining >= 3 && budget.take() && budget.take()) {
    try {
      expansion = await expand(ai, mod, profile, shots[0]?.picture ?? null);
      if (expansion !== null) report.usd += expansion.usd;
    } catch (error) {
      log({ event: "expand_error", message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
    }
  }

  const input: ModInput = { mod, profile, updates, comments, shots, others, heroes, expansion, now };
  const applicable = stale.filter((judgment) => judgment.applies(input));
  // A judgment with nothing to judge is recorded as such, so it is not retried until the page changes.
  for (const judgment of stale) {
    if (applicable.includes(judgment) || (judgment.name === "terms" && expansion === null)) continue;
    writes.push(rowStatement(env.DB, mod.id, judgment.name, judgment.version, sourceFor(judgment, mod), JEV_MODEL, { na: 1 }, now, hash));
  }
  const commit = async (): Promise<void> => {
    writes.push(env.DB.prepare("UPDATE catalog_mods SET judged_hash = ?2 WHERE mod_id = ?1").bind(mod.id, hash));
    await env.DB.batch(writes);
  };
  if (applicable.length === 0) {
    await commit();
    return "stored";
  }

  let usedInput: ModInput = input;
  let body = modRequest(input, applicable, true);
  let asked = applicable;
  const estimate = (JSON.stringify(body).length - shots.reduce((sum, shot) => sum + shot.picture.base64.length, 0)) / 4 + shots.length * IMAGE_TOKEN_GUESS;
  if (report.usd + estimate * USD_PER_TOKEN > MAX_RUN_USD) {
    report.stopped = "cost_cap";
    log({ event: "error", what: "enrich_cost_cap", usd: report.usd });
    return "stop";
  }
  let result;
  try {
    result = await askJev(net, budget, key, body);
  } catch (error) {
    if (!(error instanceof RejectedRequest)) throw error;
    if (shots.length === 0) {
      await noteFailure(env.DB, mod, now);
      return "skipped";
    }
    // Retry once on text alone, without the judgments that only read pictures.
    asked = applicable.filter((judgment) => judgment.name !== "cover");
    usedInput = { ...input, shots: [] };
    body = modRequest(usedInput, asked, false);
    try {
      result = await askJev(net, budget, key, body);
    } catch (retryError) {
      if (!(retryError instanceof RejectedRequest)) throw retryError;
      await noteFailure(env.DB, mod, now);
      return "skipped";
    }
  }
  report.tokens += result.inputTokens;
  report.usd += result.inputTokens * USD_PER_TOKEN;
  report.mods += 1;
  for (const judgment of asked) {
    const read = (id: string) => result.answers[`${judgment.name}${SEP}${id}`];
    const data = judgment.read(read, usedInput);
    if (data === null) continue;
    writes.push(rowStatement(env.DB, mod.id, judgment.name, judgment.version, sourceFor(judgment, mod), JEV_MODEL, data, now, hash));
    report.judgments += 1;
    if (judgment.name === "terms" && ai !== null && embeddingsOn(env) && budget.take()) {
      const kept = (data.terms as { term: string; p: number }[]).filter((term) => term.p >= 0.7).map((term) => term.term);
      try {
        const vector = await embedText(ai, [mod.title, ...kept].join(", "));
        if (vector !== null) writes.push(rowStatement(env.DB, mod.id, "embed", 1, sourceFor(judgment, mod), EMBED_MODEL, { v: vector }, now, hash));
      } catch (error) {
        log({ event: "embed_error", message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
      }
    }
  }
  await commit();
  return "stored";
}

/**
 * One request for one mod: the shared state holds only the parts the asked judgments need. Every
 * field here is public mod page content (see the rule in `typesafe.ts`).
 */
export function modRequest(input: ModInput, judgments: readonly Judgment[], withImages: boolean): SystemOneBody {
  const needs = new Set<Part>(judgments.flatMap((judgment) => judgment.needs));
  const state: Record<string, unknown> = {
    title: input.mod.title,
    category: input.mod.category,
    subcategory: input.mod.subcategory,
  };
  if (needs.has("text") || needs.has("files")) {
    state.description = [input.profile.summary, input.profile.text].filter((part) => part.length > 0).join("\n").slice(0, DESCRIPTION_CHARS);
    if (input.profile.links.length > 0) state.links = input.profile.links.map((link) => ({ label: link.label }));
  }
  if (needs.has("files") && input.profile.files.length > 0) {
    state.files = input.profile.files.slice(0, MAX_FILES).map((file) => ({ name: file.name, size_mb: Math.round(file.size / 104857.6) / 10, note: file.note }));
  }
  const assets: Record<string, Asset> = {};
  if (withImages && input.shots.length > 0 && (needs.has("screenshots") || needs.has("text"))) {
    state.screenshots = input.shots.map((shot, at) => {
      const ref = `img-${crypto.randomUUID()}-${at}`;
      assets[ref] = { type: "image", format: shot.picture.format, data_base64: shot.picture.base64 };
      return ref;
    });
  }
  if (needs.has("updates") && input.updates.length > 0) {
    state.updates = input.updates.map((update) => ({
      title: update.title,
      changes: update.changes.slice(0, 12),
      text: update.text.slice(0, 400),
      age_days: Math.max(0, Math.round((input.now / 1000 - update.added) / 86400)),
    }));
  }
  if (needs.has("comments") && input.comments.length > 0) {
    // Commenters become c1, c2... in order of appearance: no name or member id is sent.
    const alias = new Map<number, string>();
    state.comments = input.comments.map((comment) => {
      if (!alias.has(comment.poster)) alias.set(comment.poster, `c${alias.size + 1}`);
      return { commenter: alias.get(comment.poster), age_days: Math.max(0, Math.round((input.now / 1000 - comment.added) / 86400)), text: comment.text.slice(0, 300) };
    });
  }
  if (needs.has("others") && input.others.length > 0) {
    state.others = input.others.map((other) => ({ title: other.title, category: other.category, description: other.description.slice(0, 600) }));
  }
  if (input.expansion !== null && judgments.some((judgment) => judgment.name === "terms")) {
    state.terms = input.expansion.terms.map((term) => term.term);
  }
  const questions: Record<string, Question> = {};
  for (const judgment of judgments) {
    for (const [id, question] of Object.entries(judgment.ask(withImages ? input : { ...input, shots: [] }))) questions[`${judgment.name}${SEP}${id}`] = question;
  }
  return Object.keys(assets).length > 0 ? { model: JEV_MODEL, state, questions, assets } : { model: JEV_MODEL, state, questions };
}

// ---------------------------------------------------------------------------------------------
// Patch notes
// ---------------------------------------------------------------------------------------------

type PatchProgress = { lines: PatchLine[]; next: number; total: number };

async function enrichPatches(env: Env, net: Net, budget: Budget, key: string, heroes: readonly string[], now: number, report: RunReport): Promise<boolean> {
  const patches = await fetchPatches(net, budget);
  for (const item of patches) {
    const row = await env.DB.prepare("SELECT version, model, data FROM patch_tags WHERE patch_id = ?1").bind(item.id).first<{ version: number; model: string; data: string }>();
    let progress: PatchProgress = { lines: [], next: 0, total: item.lines.length };
    if (row !== null && row.version === PATCH_VERSION && row.model === JEV_MODEL) {
      progress = JSON.parse(row.data) as PatchProgress;
      if (progress.next >= progress.total) continue;
    }
    let stored = false;
    while (progress.next < item.lines.length && budget.remaining >= 1) {
      const body = patchRequest(item, progress.next);
      const result = await askJev(net, budget, key, body);
      report.tokens += result.inputTokens;
      report.usd += result.inputTokens * USD_PER_TOKEN;
      progress = { lines: [...progress.lines, ...readPatch((id) => result.answers[id], item, progress.next, heroes)], next: progress.next + PATCH_CHUNK, total: item.lines.length };
      await env.DB.prepare(
        `INSERT INTO patch_tags (patch_id, version, model, title, posted_at, data, enriched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(patch_id) DO UPDATE SET version = excluded.version, model = excluded.model, title = excluded.title, data = excluded.data, enriched_at = excluded.enriched_at`,
      )
        .bind(item.id, PATCH_VERSION, JEV_MODEL, item.title, item.date, JSON.stringify(progress), now)
        .run();
      stored = true;
    }
    if (stored) report.patches += 1;
    // One patch per run.
    return stored;
  }
  return false;
}

// ---------------------------------------------------------------------------------------------
// Spend and the published index
// ---------------------------------------------------------------------------------------------

async function addSpend(db: D1Database, report: RunReport, now: number): Promise<void> {
  if (report.tokens === 0 && report.usd === 0) return;
  const current = JSON.parse((await getState(db, "spend"))?.value ?? '{"tokens":0,"usd":0}') as { tokens: number; usd: number };
  const next = { tokens: current.tokens + report.tokens, usd: Math.round((current.usd + report.usd) * 1e6) / 1e6 };
  await setState(db, "spend", JSON.stringify(next), now);
  const day = new Date(now).toISOString().slice(0, 10);
  await setState(db, "spend_day", JSON.stringify({ day, usd: Math.round(((await spentToday(db, now)) + report.usd) * 1e6) / 1e6 }), now);
  log({ event: "enrich_spend", tokens_total: next.tokens, usd_total: next.usd });
}

export type IndexOptions = { readonly embeddings?: boolean; readonly learned?: boolean };

/** `INDEX_EMBEDDINGS = "1"` and `LEARNED_TERMS_PUBLISH = "1"` (Worker vars), both off by default. */
export function indexOptions(env: Env): IndexOptions {
  const vars = env as Env & { readonly LEARNED_TERMS_PUBLISH?: string };
  return { embeddings: embeddingsOn(env), learned: vars.LEARNED_TERMS_PUBLISH === "1" };
}

export async function maybeBuildIndex(db: D1Database, now: number, changed: boolean, force = false, options: IndexOptions = {}): Promise<boolean> {
  const meta = await getState(db, "index_meta");
  const parsed = meta === null ? null : (JSON.parse(meta.value) as { v: number; built: number });
  const outdated = parsed === null || parsed.v !== PUBLISH_VERSION;
  if (!force && !outdated && !(changed && now - parsed.built >= INDEX_MIN_AGE_MS)) return false;
  await buildAndStoreIndex(db, now, options);
  return true;
}

/** D1 rows top out at 2 MB: the gzipped index is stored in chunks well under that. */
export const INDEX_CHUNK = 900_000;

export async function buildAndStoreIndex(db: D1Database, now: number, options: IndexOptions = {}): Promise<void> {
  const mods = await db
    .prepare(
      `SELECT c.mod_id, c.title, c.subcategory, c.content_hash != c.judged_hash AS stale, MAX(t.enriched_at) AS enriched_at
       FROM catalog_mods c JOIN mod_tags t ON t.mod_id = c.mod_id AND t.judgment != '_failed'
       WHERE c.withdrawn = 0 AND c.seen_at >= ?1
       GROUP BY c.mod_id ORDER BY c.mod_id`,
    )
    .bind(now - UNSEEN_MS)
    .all<{ mod_id: number; title: string; subcategory: string; stale: number; enriched_at: number }>();
  const rows = new Map<number, Record<string, Stored>>();
  const tags = await db.prepare("SELECT mod_id, judgment, data FROM mod_tags WHERE judgment != '_failed'").all<{ mod_id: number; judgment: string; data: string }>();
  for (const tag of tags.results) {
    const data = JSON.parse(tag.data) as Stored;
    if (data.na === 1) continue;
    const entry = rows.get(tag.mod_id) ?? {};
    entry[tag.judgment] = data;
    rows.set(tag.mod_id, entry);
  }
  const heroes = JSON.parse((await getState(db, "heroes"))?.value ?? "[]") as string[];
  const metas: ModMeta[] = mods.results.map((row) => ({ id: row.mod_id, title: row.title, subcategory: row.subcategory, enrichedAt: row.enriched_at, stale: row.stale === 1 }));
  const index = buildIndex(metas, rows as ReadonlyMap<number, JudgmentRows>, heroes, now, options.embeddings === true, options.learned === true);
  const json = JSON.stringify(index);
  const etag = (await sha256Hex(json)).slice(0, 32);
  const packed = await gzipBase64(json);
  const chunks = Math.max(1, Math.ceil(packed.length / INDEX_CHUNK));
  for (let at = 0; at < chunks; at += 1) await setState(db, `index:${at}`, packed.slice(at * INDEX_CHUNK, (at + 1) * INDEX_CHUNK), now);
  await setState(db, "index_meta", JSON.stringify({ v: PUBLISH_VERSION, built: now, etag, chunks, mods: index.mods.length, bytes: json.length, gzip: Math.round((packed.length * 3) / 4) }), now);
}

/** The stored index, or null before the first build. */
export async function loadIndex(db: D1Database): Promise<CatalogIndex | null> {
  const meta = await getState(db, "index_meta");
  if (meta === null) return null;
  const { chunks } = JSON.parse(meta.value) as { chunks: number };
  let packed = "";
  for (let at = 0; at < chunks; at += 1) packed += (await getState(db, `index:${at}`))?.value ?? "";
  return JSON.parse(await gunzipBase64(packed)) as CatalogIndex;
}

export async function gzipBase64(text: string): Promise<string> {
  const stream = new Blob([text]).stream().pipeThrough(new CompressionStream("gzip"));
  const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  let binary = "";
  for (let at = 0; at < bytes.length; at += 0x8000) binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  return btoa(binary);
}

export async function gunzipBase64(encoded: string): Promise<string> {
  const bytes = Uint8Array.from(atob(encoded), (char) => char.charCodeAt(0));
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
  return new Response(stream).text();
}

function log(fields: Record<string, string | number>): void {
  console.log(JSON.stringify(fields));
}
