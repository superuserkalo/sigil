/**
 * The learning loop: searches players made that found nothing or got no click, taken ONLY from
 * the search-gap aggregate that already passed the product floor (ADR 0017: at least 10 installs,
 * 6 networks, 3 days in one closed week, scrubbed on the PC and on the Worker). No raw product row
 * is read. For each gap term, candidates come from the local matcher over the tag index; Jev scores
 * each candidate's relevance; the term is added as a "learned" search term to the mods that pass.
 */

import { searchGaps } from "../aggregates";
import { normalize } from "./aliases";
import { search } from "./match";
import { Budget, type Net } from "./net";
import type { CatalogIndex } from "./publish";
import { askJev, JEV_MODEL, LEARN_STATE_KEYS, type Question, type SystemOneBody } from "./typesafe";
import { round, scored, type Stored } from "./judgments";

export const LEARN_VERSION = 1;
export const LEARN_INTERVAL_MS = 24 * 60 * 60 * 1000;
export const LEARN_QUERIES_PER_RUN = 3;
export const LEARN_CANDIDATES = 8;
/** A gap term is judged again at most this often. */
export const LEARN_REPEAT_MS = 30 * 24 * 60 * 60 * 1000;

const LEVELS = [
  "Unrelated to what the search asks for.",
  "Shares a word or a loose theme, but is not what the search is for.",
  "A reasonable match: related to what the search names.",
  "Exactly what the search names: the same franchise, character, item or style.",
];

/**
 * A gap term may only ever become an association between words the public catalog already uses.
 * It is dropped (never sent, never stored) when it looks like personal data or has any word that
 * no public title, verified facet or verified alias contains.
 */
export function piiShaped(term: string): boolean {
  return /\d{5,}/.test(term) || term.includes("@") || /:\/\/|www\.|\.[a-z]{2,}\b/.test(term) || term.length > 40 || term.split(/\s+/).length > 4;
}

export function publicVocabulary(index: CatalogIndex): Set<string> {
  const words = new Set<string>();
  const addWords = (text: string) => {
    for (const word of normalize(text).split(" ")) if (word.length > 0) words.add(word);
  };
  for (const mod of index.mods) addWords(mod.ti);
  for (const text of index.strings) addWords(text);
  return words;
}

export function allowedLearnedTerm(term: string, vocabulary: ReadonlySet<string>): boolean {
  if (piiShaped(term)) return false;
  const words = normalize(term).split(" ").filter((word) => word.length > 0);
  return words.length > 0 && words.every((word) => vocabulary.has(word));
}

export type Candidate = { readonly id: number; readonly title: string; readonly category: string; readonly description: string };

export function learnRequest(query: string, candidates: readonly Candidate[]): SystemOneBody {
  const questions: Record<string, Question> = {};
  candidates.forEach((_, at) => {
    questions[`fit${at}`] = {
      type: "score",
      instructions: `How well does \`candidates[${at}]\` match what a player who searched for \`query\` is looking for?`,
      criteria: LEVELS,
    };
  });
  // `query` is a floor-passed aggregate search term; the candidates are public mod pages.
  return {
    model: JEV_MODEL,
    state: { query, candidates: candidates.map((candidate) => ({ title: candidate.title, category: candidate.category, description: candidate.description.slice(0, 400) })) },
    questions,
  };
}

/** Gap terms, candidates and judgments. Returns the number of terms judged and tokens used. */
export async function learnFromGaps(
  db: D1Database,
  net: Net,
  budget: Budget,
  key: string,
  bucketKey: string | undefined,
  index: CatalogIndex | null,
  now: number,
): Promise<{ queries: number; tokens: number; added: number }> {
  const out = { queries: 0, tokens: 0, added: 0 };
  if (index === null || index.mods.length === 0) return out;
  const gaps = await searchGaps(db, bucketKey, 50);
  const seenRow = await db.prepare("SELECT value FROM enrich_state WHERE key = 'learned_queries'").first<{ value: string }>();
  const seen = JSON.parse(seenRow?.value ?? "{}") as Record<string, number>;
  const vocabulary = publicVocabulary(index);
  const terms = [...gaps.zero, ...gaps.few]
    .map((gap) => gap.term)
    .filter((term, at, all) => all.indexOf(term) === at)
    .filter((term) => allowedLearnedTerm(term, vocabulary));
  for (const query of terms) {
    if (out.queries >= LEARN_QUERIES_PER_RUN || budget.remaining < 1) break;
    if (now - (seen[query] ?? 0) < LEARN_REPEAT_MS) continue;
    seen[query] = now;
    const hits = search(index, query, LEARN_CANDIDATES);
    if (hits.length === 0) continue;
    const ids = hits.map((hit) => hit.id);
    const { results } = await db
      .prepare(`SELECT mod_id, title, category, desc_excerpt FROM catalog_mods WHERE mod_id IN (${ids.map(() => "?").join(",")})`)
      .bind(...ids)
      .all<{ mod_id: number; title: string; category: string; desc_excerpt: string }>();
    const candidates: Candidate[] = ids.flatMap((id) => {
      const row = results.find((candidate) => candidate.mod_id === id);
      return row === undefined ? [] : [{ id, title: row.title, category: row.category, description: row.desc_excerpt }];
    });
    if (candidates.length === 0) continue;
    const result = await askJev(net, budget, key, learnRequest(query, candidates), LEARN_STATE_KEYS);
    out.queries += 1;
    out.tokens += result.inputTokens;
    for (const [at, candidate] of candidates.entries()) {
      const fit = scored(result.answers[`fit${at}`], LEVELS.length);
      if (fit === null) continue;
      if (await addLearned(db, candidate.id, query, fit.s, fit.c, now)) out.added += 1;
    }
  }
  await db
    .prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES ('learned_queries', ?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at")
    .bind(JSON.stringify(seen), now)
    .run();
  return out;
}

/** Keeps each judged term with its score; publishing applies the threshold. */
async function addLearned(db: D1Database, modId: number, term: string, s: number, c: number, now: number): Promise<boolean> {
  const row = await db.prepare("SELECT data FROM mod_tags WHERE mod_id = ?1 AND judgment = 'learned'").bind(modId).first<{ data: string }>();
  const data = (row === null ? { terms: [] } : JSON.parse(row.data)) as { terms: { term: string; s: number; c: number; at: number }[] } & Stored;
  data.terms = [...data.terms.filter((entry) => entry.term !== term), { term, s: round(s), c: round(c), at: now }].slice(-40);
  await db
    .prepare(
      `INSERT INTO mod_tags (mod_id, judgment, version, source, model, data, enriched_at) VALUES (?1, 'learned', ?2, 'learned', ?3, ?4, ?5)
       ON CONFLICT(mod_id, judgment) DO UPDATE SET data = excluded.data, enriched_at = excluded.enriched_at, version = excluded.version, model = excluded.model`,
    )
    .bind(modId, LEARN_VERSION, JEV_MODEL, JSON.stringify(data), now)
    .run();
  return s >= 0.67;
}
