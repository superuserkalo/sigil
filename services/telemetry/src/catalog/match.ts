/**
 * Reference matcher for the catalog search index: what the app runs locally and offline over the
 * index it downloaded. Nothing typed by a player leaves the PC. The Rust app ports this file; the
 * algorithm is written out in docs/decisions/0018-typesafe-catalog-tags.md.
 *
 * 1. Normalise the query (lower case, accents and punctuation dropped) and split it into words.
 *    Stop words (the, a, of, mod...) are scored but not required.
 * 2. Each word is a unit. There is no alias or synonym table of any kind: misspellings and
 *    short forms are covered by the tolerance below, outside names by the flagged verified terms.
 * 3. A unit matches a token exactly (1.0), as a prefix of at least 3 letters (0.8), or one edit
 *    away for words of 4 letters or more (0.7). A unit also matches a multi-word term written
 *    without spaces ("counterstrike").
 * 4. Fields and weights: title 3, verified search term 2.5 (only with the flagged expansion), hero
 *    (GameBanana's hero subcategory) 2, category 1.5, author 1.5, description word 1. Each field is weighted by idf = ln(1 + (N - df + 0.5) / (df + 0.5)).
 *    A multi-word field hit by only some of its words counts at 0.6.
 * 5. A mod scores, per unit, its best field. Exact title equality adds 1000; every unit found in
 *    the title adds 30, so title matches lead.
 * 6. Every non-stop unit must match somewhere. When no mod passes, any match counts instead.
 */

import { PACK, type CatalogIndex } from "./publish";

/** Lower case, accents dropped, punctuation to spaces, spaces collapsed. Shared with the index builder. */
export function normalize(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

export const WEIGHTS = { title: 3, term: 2.5, hero: 2, category: 1.5, author: 1.5, description: 1, visual: 0.5 } as const;
/** By field class (`publish.ts` CLS): description word, category, author, hero, verified term. */
const CLASS_WEIGHT = [WEIGHTS.description, WEIGHTS.category, WEIGHTS.author, WEIGHTS.hero, WEIGHTS.term, WEIGHTS.visual];
const STOP = new Set(["the", "a", "an", "of", "for", "and", "mod", "mods", "with", "der", "die", "das", "und", "fur"]);

type Field = { readonly tokens: readonly string[]; readonly joined: string; readonly weight: number; readonly key: string };
type Prepared = { readonly mods: readonly { readonly id: number; readonly title: string; readonly fields: readonly Field[] }[]; readonly idf: ReadonlyMap<string, number> };

type Unit = { readonly word: string; readonly required: boolean };

export type Hit = { readonly id: number; readonly score: number };

const prepared = new WeakMap<CatalogIndex, Prepared>();

function prepare(index: CatalogIndex): Prepared {
  const cached = prepared.get(index);
  if (cached !== undefined) return cached;
  const df = new Map<string, number>();
  const mods = index.mods.map((mod) => {
    const title = normalize(mod.ti);
    const fields: Field[] = [{ tokens: title.split(" ").filter(Boolean), joined: title.replace(/ /g, ""), weight: WEIGHTS.title, key: "" }];
    for (const packed of mod.x) {
      const text = index.strings[Math.floor(packed / PACK)];
      if (text === undefined) continue;
      fields.push({ tokens: text.split(" "), joined: text.replace(/ /g, ""), weight: CLASS_WEIGHT[packed % PACK] ?? 1, key: text });
    }
    for (const field of fields) {
      if (field.key.length > 0) df.set(field.key, (df.get(field.key) ?? 0) + 1);
    }
    return { id: mod.i, title, fields };
  });
  const n = Math.max(1, mods.length);
  const idf = new Map<string, number>();
  for (const [key, count] of df) idf.set(key, Math.log(1 + (n - count + 0.5) / (count + 0.5)));
  const out = { mods, idf };
  prepared.set(index, out);
  return out;
}

/** Levenshtein distance, stopping early past 1. */
export function withinOneEdit(a: string, b: string): boolean {
  if (a === b) return true;
  if (Math.abs(a.length - b.length) > 1) return false;
  let i = 0;
  let j = 0;
  let edits = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      i += 1;
      j += 1;
      continue;
    }
    edits += 1;
    if (edits > 1) return false;
    if (a.length > b.length) i += 1;
    else if (b.length > a.length) j += 1;
    else if (a[i + 1] === b[j] && a[i] === b[j + 1]) {
      // A swap of two neighbours counts as one edit.
      i += 2;
      j += 2;
    } else {
      i += 1;
      j += 1;
    }
  }
  return edits + (a.length - i) + (b.length - j) <= 1;
}

export function tokenMatch(query: string, token: string): number {
  if (query === token) return 1;
  if (query.length >= 3 && token.startsWith(query)) return 0.8;
  if (query.length >= 4 && token.length >= 4 && withinOneEdit(query, token)) return 0.7;
  return 0;
}

function units(query: string): Unit[] {
  return normalize(query)
    .split(" ")
    .filter(Boolean)
    .map((word) => ({ word, required: !STOP.has(word) }));
}

function unitQuality(unit: Unit, field: Field): number {
  let best = unit.word.length >= 4 && field.tokens.length > 1 ? tokenMatch(unit.word, field.joined) : 0;
  for (const token of field.tokens) best = Math.max(best, tokenMatch(unit.word, token));
  return best;
}

export function search(index: CatalogIndex, query: string, limit = 20): Hit[] {
  const ready = prepare(index);
  const list = units(query);
  if (list.length === 0) return [];
  const whole = normalize(query);
  const required = list.filter((unit) => unit.required);
  const scoreAll = (mode: "all" | "any"): Hit[] => {
    const hits: Hit[] = [];
    for (const mod of ready.mods) {
      let total = 0;
      let covered = 0;
      let inTitle = 0;
      const coveredRequired = new Set<Unit>();
      // Units per field, for the partial-phrase factor.
      const perField = new Map<Field, number>();
      const best: { field: Field | null; value: number; quality: number }[] = list.map(() => ({ field: null, value: 0, quality: 0 }));
      list.forEach((unit, u) => {
        for (const field of mod.fields) {
          const quality = unitQuality(unit, field);
          if (quality === 0) continue;
          const idf = field.key.length > 0 ? (ready.idf.get(field.key) ?? 1) : 1;
          const value = field.weight * quality * idf;
          const slot = best[u];
          if (slot !== undefined && value > slot.value) best[u] = { field, value, quality };
        }
        const slot = best[u];
        if (slot?.field != null) perField.set(slot.field, (perField.get(slot.field) ?? 0) + 1);
      });
      list.forEach((unit, u) => {
        const slot = best[u];
        if (slot === undefined || slot.field === null) return;
        const field = slot.field;
        const wholeTerm = field.tokens.length <= 1 || withinOneEdit(unit.word, field.joined);
        const partial = field.key.length > 0 && field.tokens.length > 1 && (perField.get(field) ?? 0) < field.tokens.length && !wholeTerm ? 0.6 : 1;
        total += slot.value * partial;
        covered += 1;
        if (unit.required) coveredRequired.add(unit);
        if (field.key.length === 0) inTitle += 1;
      });
      if (covered === 0) continue;
      if (mode === "all" && coveredRequired.size < required.length) continue;
      if (mod.title === whole) total += 1000;
      if (inTitle === list.length) total += 30;
      hits.push({ id: mod.id, score: Math.round(total * 1000) / 1000 });
    }
    return hits;
  };
  let hits = scoreAll("all");
  if (hits.length === 0) hits = scoreAll("any");
  return hits.sort((a, b) => b.score - a.score || a.id - b.id).slice(0, limit);
}
