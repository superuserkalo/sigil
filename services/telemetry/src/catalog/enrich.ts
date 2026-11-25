/**
 * Catalog upkeep for the search index: bounded work per run, resumable, stops on the first remote
 * error.
 *
 * Each run reads GameBanana's Deadlock mod list (newest updates and newest uploads first, then a
 * cursor page that walks the rest of the catalog), reads the public text of the mods whose page
 * timestamp moved (50 per request through `Mod/Multi`), keeps a content hash per mod, marks
 * withdrawn pages, and rebuilds the index at most hourly. No model is needed for any of that.
 *
 * Two optional model steps, each behind a flag and off by default, run only for mods whose
 * content hash changed: term expansion (Workers AI proposes, Jev verifies) and file labels (Jev).
 * Without `TYPESAFE_API_KEY` they never run; without the `AI` binding expansion never runs.
 */

import { aiBinding, expand, withoutHeroNames, type Expansion } from "./expand";
import { GameBanana, MULTI_IDS, type ListedMod, type ModFile, type ModText } from "./gamebanana";
import { JUDGMENTS, MAX_FILES, SEP, type Judgment, type ModInput, type Stored } from "./judgments";
import { Budget, BudgetSpent, liveNet, RemoteError, type Net } from "./net";
import { buildIndex, PUBLISH_VERSION, type CatalogIndex, type JudgmentRows, type ModMeta } from "./publish";
import { askJev, JEV_MODEL, RejectedRequest, typesafeKey, USD_PER_TOKEN, type Question, type SystemOneBody } from "./typesafe";
import { sha256Hex } from "../hash";

/** Outside calls per run: under the Workers free-plan limit of 50, with room to spare. */
export const RUN_SUBREQUESTS = 45;
export const INDEX_MIN_AGE_MS = 60 * 60 * 1000;
export const LEASE_MS = 10 * 60 * 1000;
/** Defaults for the caps; override with the Worker vars ENRICH_RUN_MODS and ENRICH_DAILY_USD. */
export const DEFAULT_RUN_MODS = 20;
export const DEFAULT_DAILY_USD = 1;
/** A mod not seen in the listing for this long leaves the index. */
export const UNSEEN_MS = 14 * 24 * 60 * 60 * 1000;
const DESCRIPTION_CHARS = 1500;

export type Flags = { readonly expansion: boolean; readonly fileLabels: boolean };

/** `CATALOG_EXPANSION = "1"` and `CATALOG_FILE_LABELS = "1"` (Worker vars). Both off by default. */
export function flags(env: Env): Flags {
  const vars = env as Env & { readonly CATALOG_EXPANSION?: string; readonly CATALOG_FILE_LABELS?: string };
  return { expansion: vars.CATALOG_EXPANSION === "1", fileLabels: vars.CATALOG_FILE_LABELS === "1" };
}

function config(env: Env): { runMods: number; dailyUsd: number } {
  const vars = env as Env & { readonly ENRICH_RUN_MODS?: string; readonly ENRICH_DAILY_USD?: string };
  const runMods = Number(vars.ENRICH_RUN_MODS ?? DEFAULT_RUN_MODS);
  const dailyUsd = Number(vars.ENRICH_DAILY_USD ?? DEFAULT_DAILY_USD);
  return {
    runMods: Number.isFinite(runMods) && runMods > 0 ? Math.min(50, Math.floor(runMods)) : DEFAULT_RUN_MODS,
    dailyUsd: Number.isFinite(dailyUsd) && dailyUsd > 0 ? dailyUsd : DEFAULT_DAILY_USD,
  };
}

export type RunReport = {
  skipped?: string;
  /** Mods whose public text was read this run. */
  refreshed: number;
  /** Of those, mods whose content hash did not change. */
  unchanged: number;
  withdrawn: number;
  /** Mods sent to Jev. */
  judged: number;
  failed: number;
  tokens: number;
  usd: number;
  stopped?: string;
  indexBuilt: boolean;
};

type TagRow = { judgment: string; version: number; model: string; hash: string };

type ModRow = { mod_id: number; title: string; category: string; subcategory: string; author: string; description: string; files: string; content_hash: string; judged_hash: string };

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

async function spentToday(db: D1Database, now: number): Promise<number> {
  const day = new Date(now).toISOString().slice(0, 10);
  const row = await getState(db, "spend_day");
  const parsed = row === null ? null : (JSON.parse(row.value) as { day: string; usd: number });
  return parsed !== null && parsed.day === day ? parsed.usd : 0;
}

/** The enabled judgments a mod needs (re)run: missing, older version or model, or made on other content. */
export function staleJudgments(enabled: readonly Judgment[], rows: readonly TagRow[], contentHash: string): Judgment[] {
  return enabled.filter((judgment) => {
    const row = rows.find((candidate) => candidate.judgment === judgment.name);
    return row === undefined || row.version !== judgment.version || row.model !== JEV_MODEL || row.hash !== contentHash;
  });
}

// ---------------------------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------------------------

/** The Worker var `CATALOG_INDEX = "1"` (set in wrangler.jsonc) turns the upkeep on; anything else is the off switch. */
export function indexOn(env: Env): boolean {
  return (env as Env & { readonly CATALOG_INDEX?: string }).CATALOG_INDEX === "1";
}

export async function runEnrichment(env: Env, now: number, net: Net = liveNet, subrequests = RUN_SUBREQUESTS): Promise<RunReport> {
  const report: RunReport = { refreshed: 0, unchanged: 0, withdrawn: 0, judged: 0, failed: 0, tokens: 0, usd: 0, indexBuilt: false };
  if (!indexOn(env)) return { ...report, skipped: "off" };
  const lease = await getState(env.DB, "lease");
  if (lease !== null && Number(lease.value) > now) return { ...report, skipped: "lease_held" };
  await setState(env.DB, "lease", String(now + LEASE_MS), now);

  const budget = new Budget(subrequests);
  const gb = new GameBanana(net, budget);
  let changed = false;
  try {
    const listed = await candidates(env.DB, gb);
    for (const mod of listed) await rememberMod(env.DB, mod, now);
    changed = (await refreshTexts(env.DB, gb, budget, listed, report)) || changed;
    await advanceCursor(env.DB, listed, now);
    changed = (await judge(env, net, budget, listed, now, report)) || changed;
  } catch (error) {
    report.stopped = error instanceof BudgetSpent ? "budget" : error instanceof RemoteError ? `${error.host}_${error.status}` : "error";
    log({ event: "enrich_error", what: report.stopped, message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
  } finally {
    await addSpend(env.DB, report, now);
    await setState(env.DB, "lease", "0", now);
  }
  try {
    report.indexBuilt = await maybeBuildIndex(env.DB, now, changed, false, flags(env));
  } catch (error) {
    log({ event: "enrich_index_error", message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
  }
  log({
    event: "enrich_run",
    refreshed: report.refreshed,
    unchanged: report.unchanged,
    withdrawn: report.withdrawn,
    judged: report.judged,
    failed: report.failed,
    tokens: report.tokens,
    usd: Math.round(report.usd * 10000) / 10000,
    stopped: report.stopped ?? "",
  });
  return report;
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
  await setState(db, "cursor_page", JSON.stringify((lists[2] ?? []).map((mod) => mod.id)), 0);
  return out;
}

/** Next backfill page once every mod on the current one has its text for its current timestamp. */
async function advanceCursor(db: D1Database, listed: readonly ListedMod[], now: number): Promise<void> {
  const cursor = Number((await getState(db, "cursor"))?.value ?? "2");
  const page = JSON.parse((await getState(db, "cursor_page"))?.value ?? "[]") as number[];
  for (const id of page) {
    const mod = listed.find((candidate) => candidate.id === id);
    const row = await db.prepare("SELECT text_updated, withdrawn FROM catalog_mods WHERE mod_id = ?1").bind(id).first<{ text_updated: number; withdrawn: number }>();
    if (mod !== undefined && row !== null && row.withdrawn === 0 && row.text_updated !== mod.updated) return;
  }
  await setState(db, "cursor", String(page.length < 50 ? 2 : Math.max(2, cursor) + 1), now);
}

async function rememberMod(db: D1Database, mod: ListedMod, now: number): Promise<void> {
  await db
    .prepare(
      `INSERT INTO catalog_mods (mod_id, mod_updated, added_at, title, category, subcategory, author, seen_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
       ON CONFLICT(mod_id) DO UPDATE SET mod_updated = excluded.mod_updated, title = excluded.title, category = excluded.category,
         subcategory = excluded.subcategory, author = excluded.author, seen_at = excluded.seen_at`,
    )
    .bind(mod.id, mod.updated, mod.added, mod.title, mod.category, mod.subcategory, mod.author, now)
    .run();
}

/** The public content the index and the judgments read: title, categories, author, text and files. */
export async function contentHash(mod: ListedMod, text: ModText): Promise<string> {
  const parts = [mod.title, mod.category, mod.subcategory, mod.author, text.summary, text.text, text.files.map((file) => `${file.id}:${file.name}:${file.size}:${file.note}`).join("|")];
  return (await sha256Hex(parts.join("\n"))).slice(0, 32);
}

/** Reads the public text of mods whose page moved, 50 per request. True when the index changed. */
async function refreshTexts(db: D1Database, gb: GameBanana, budget: Budget, listed: readonly ListedMod[], report: RunReport): Promise<boolean> {
  const due: ListedMod[] = [];
  for (const mod of listed) {
    const row = await db.prepare("SELECT text_updated FROM catalog_mods WHERE mod_id = ?1").bind(mod.id).first<{ text_updated: number }>();
    if (row === null || row.text_updated !== mod.updated) due.push(mod);
  }
  let changed = false;
  for (let at = 0; at < due.length; at += MULTI_IDS) {
    const chunk = due.slice(at, at + MULTI_IDS);
    if (chunk.length === 0 || budget.remaining < 1) break;
    const { texts, gone } = await gb.texts(chunk.map((mod) => mod.id));
    for (const id of gone) {
      await db.prepare("UPDATE catalog_mods SET withdrawn = 1, text_updated = mod_updated WHERE mod_id = ?1").bind(id).run();
      report.withdrawn += 1;
      changed = true;
    }
    for (const text of texts) {
      const mod = chunk.find((candidate) => candidate.id === text.id);
      if (mod === undefined) continue;
      const hash = await contentHash(mod, text);
      const known = await db.prepare("SELECT content_hash FROM catalog_mods WHERE mod_id = ?1").bind(mod.id).first<{ content_hash: string }>();
      report.refreshed += 1;
      if (known?.content_hash === hash) report.unchanged += 1;
      else changed = true;
      await db
        .prepare("UPDATE catalog_mods SET description = ?2, files = ?3, content_hash = ?4, text_updated = ?5, withdrawn = 0 WHERE mod_id = ?1")
        .bind(mod.id, [text.summary, text.text].filter((part) => part.length > 0).join("\n").slice(0, DESCRIPTION_CHARS), JSON.stringify(text.files.slice(0, MAX_FILES)), hash, mod.updated)
        .run();
    }
  }
  return changed;
}

// ---------------------------------------------------------------------------------------------
// The flagged model steps
// ---------------------------------------------------------------------------------------------

async function judge(env: Env, net: Net, budgetLeft: Budget, listed: readonly ListedMod[], now: number, report: RunReport): Promise<boolean> {
  const enabled = JUDGMENTS.filter((judgment) => (judgment.name === "terms" ? flags(env).expansion : flags(env).fileLabels));
  if (enabled.length === 0) return false;
  const key = typesafeKey(env);
  if (key === null) {
    log({ event: "enrich_models_skipped", reason: "no_typesafe_key" });
    return false;
  }
  const caps = config(env);
  const dayStart = await spentToday(env.DB, now);
  const ai = aiBinding(env);
  let changed = false;
  for (const listedMod of listed) {
    if (report.judged + report.failed >= caps.runMods) {
      report.stopped = "run_mods";
      break;
    }
    if (dayStart + report.usd >= caps.dailyUsd) {
      report.stopped = "daily_cap";
      log({ event: "error", what: "enrich_daily_cap", usd: dayStart + report.usd });
      break;
    }
    if (budgetLeft.remaining < 3) {
      report.stopped = "budget";
      break;
    }
    const row = await env.DB.prepare(
      "SELECT mod_id, title, category, subcategory, author, description, files, content_hash, judged_hash FROM catalog_mods WHERE mod_id = ?1 AND withdrawn = 0 AND content_hash != ''",
    )
      .bind(listedMod.id)
      .first<ModRow>();
    if (row === null) continue;
    const { results: rows } = await env.DB.prepare("SELECT judgment, version, model, hash FROM mod_tags WHERE mod_id = ?1").bind(row.mod_id).all<TagRow>();
    const stale = staleJudgments(enabled, rows, row.content_hash).filter((judgment) => judgment.name !== "terms" || ai !== null);
    if (stale.length === 0) continue;
    try {
      if (await judgeMod(env, net, budgetLeft, key, ai, listedMod, row, stale, now, report)) changed = true;
    } catch (error) {
      // A remote refusing or the budget ending stops the run; anything else is this mod's problem.
      if (error instanceof RemoteError || error instanceof BudgetSpent) throw error;
      log({ event: "enrich_mod_error", mod: row.mod_id, message: error instanceof Error ? `${error.name}: ${error.message}`.slice(0, 200) : "unknown" });
      report.failed += 1;
    }
  }
  return changed;
}

async function judgeMod(
  env: Env,
  net: Net,
  budget: Budget,
  key: string,
  ai: ReturnType<typeof aiBinding>,
  mod: ListedMod,
  row: ModRow,
  stale: readonly Judgment[],
  now: number,
  report: RunReport,
): Promise<boolean> {
  const text: ModText = { id: row.mod_id, summary: "", text: row.description, files: JSON.parse(row.files || "[]") as ModFile[] };
  let expansion: Expansion | null = null;
  if (ai !== null && stale.some((judgment) => judgment.name === "terms") && budget.take()) {
    try {
      expansion = await expand(ai, mod, text);
      if (expansion !== null) {
        report.usd += expansion.usd;
        // The hero a skin replaces is not an outside reference.
        expansion = { ...expansion, terms: withoutHeroNames(expansion.terms, mod.category.toLowerCase() === "skins" ? [mod.subcategory] : []) };
      }
    } catch (error) {
      log({ event: "expand_error", message: error instanceof Error ? error.message.slice(0, 200) : "unknown" });
    }
  }
  const input: ModInput = { mod, text, expansion };
  const applicable = stale.filter((judgment) => judgment.applies(input));
  // Every row of this run is written in one batch at the end, with the hash it was made on and the
  // mod's judged hash, so a run that fails midway leaves nothing half-current.
  const writes: D1PreparedStatement[] = [];
  for (const judgment of stale) {
    if (applicable.includes(judgment) || (judgment.name === "terms" && expansion === null)) continue;
    writes.push(rowStatement(env.DB, row.mod_id, judgment, { na: 1 }, now, row.content_hash));
  }
  if (applicable.length > 0) {
    let result;
    try {
      result = await askJev(net, budget, key, modRequest(input, applicable));
    } catch (error) {
      if (!(error instanceof RejectedRequest)) throw error;
      report.failed += 1;
      return false;
    }
    report.tokens += result.inputTokens;
    report.usd += result.inputTokens * USD_PER_TOKEN;
    report.judged += 1;
    for (const judgment of applicable) {
      const data = judgment.read((id) => result.answers[`${judgment.name}${SEP}${id}`], input);
      if (data !== null) writes.push(rowStatement(env.DB, row.mod_id, judgment, data, now, row.content_hash));
    }
  }
  if (writes.length === 0) return false;
  writes.push(env.DB.prepare("UPDATE catalog_mods SET judged_hash = ?2 WHERE mod_id = ?1").bind(row.mod_id, row.content_hash));
  await env.DB.batch(writes);
  return true;
}

function rowStatement(db: D1Database, modId: number, judgment: Judgment, data: Stored, now: number, hash: string): D1PreparedStatement {
  return db
    .prepare(
      `INSERT INTO mod_tags (mod_id, judgment, version, model, data, hash, enriched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
       ON CONFLICT(mod_id, judgment) DO UPDATE SET version = excluded.version, model = excluded.model, data = excluded.data,
         hash = excluded.hash, enriched_at = excluded.enriched_at`,
    )
    .bind(modId, judgment.name, judgment.version, JEV_MODEL, JSON.stringify(data), hash, now);
}

/** One request for one mod. Every state field is public mod page content (see `typesafe.ts`). */
export function modRequest(input: ModInput, judgments: readonly Judgment[]): SystemOneBody {
  const state: Record<string, unknown> = {
    title: input.mod.title,
    category: input.mod.category,
    subcategory: input.mod.subcategory,
    description: [input.text.summary, input.text.text].filter((part) => part.length > 0).join("\n").slice(0, DESCRIPTION_CHARS),
  };
  if (judgments.some((judgment) => judgment.name === "files")) {
    state.files = input.text.files.slice(0, MAX_FILES).map((file) => ({ name: file.name, size_mb: Math.round(file.size / 104857.6) / 10, note: file.note }));
  }
  if (input.expansion !== null && judgments.some((judgment) => judgment.name === "terms")) {
    state.terms = input.expansion.terms.map((term) => term.term);
  }
  const questions: Record<string, Question> = {};
  for (const judgment of judgments) {
    for (const [id, question] of Object.entries(judgment.ask(input))) questions[`${judgment.name}${SEP}${id}`] = question;
  }
  return { model: JEV_MODEL, state, questions };
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

export async function maybeBuildIndex(db: D1Database, now: number, changed: boolean, force = false, options: Partial<Flags> = {}): Promise<boolean> {
  const meta = await getState(db, "index_meta");
  const parsed = meta === null ? null : (JSON.parse(meta.value) as { v: number; built: number });
  const outdated = parsed === null || parsed.v !== PUBLISH_VERSION;
  if (!force && !outdated && !(changed && now - parsed.built >= INDEX_MIN_AGE_MS)) return false;
  await buildAndStoreIndex(db, now, options);
  return true;
}

/** D1 rows top out at 2 MB: the gzipped index is stored in chunks well under that. */
export const INDEX_CHUNK = 900_000;

export async function buildAndStoreIndex(db: D1Database, now: number, options: Partial<Flags> = {}): Promise<void> {
  const mods = await db
    .prepare(
      `SELECT mod_id, title, category, subcategory, author, description, content_hash != judged_hash AS stale, text_updated
       FROM catalog_mods WHERE withdrawn = 0 AND content_hash != '' AND seen_at >= ?1 ORDER BY mod_id`,
    )
    .bind(now - UNSEEN_MS)
    .all<{ mod_id: number; title: string; category: string; subcategory: string; author: string; description: string; stale: number; text_updated: number }>();
  const rows = new Map<number, Record<string, Stored>>();
  if (options.expansion === true) {
    const tags = await db.prepare("SELECT mod_id, judgment, data FROM mod_tags WHERE judgment = 'terms'").all<{ mod_id: number; judgment: string; data: string }>();
    for (const tag of tags.results) {
      const data = JSON.parse(tag.data) as Stored;
      if (data.na !== 1) rows.set(tag.mod_id, { terms: data });
    }
  }
  const metas: ModMeta[] = mods.results.map((row) => ({
    id: row.mod_id,
    title: row.title,
    category: row.category,
    subcategory: row.subcategory,
    author: row.author,
    description: row.description,
    enrichedAt: row.text_updated * 1000,
    stale: options.expansion === true && rows.has(row.mod_id) && row.stale === 1,
  }));
  const index = buildIndex(metas, rows as ReadonlyMap<number, JudgmentRows>, now, options.expansion === true);
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
