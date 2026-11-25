/**
 * The compact search index and its thresholds. The index is built from GameBanana's own public
 * text (title, description words, category, hero subcategory, author) plus, when the flagged
 * expansion runs, the search terms Jev verified. Stored judgments keep raw probabilities; this
 * file decides what is published, so a threshold change needs no new inference (bump
 * PUBLISH_VERSION).
 */

import { normalize } from "./match";
import type { Stored } from "./judgments";

/** 3: the alias table left the index (no hand-made alias list of any kind). */
export const PUBLISH_VERSION = 3;

/**
 * Thresholds. A Noul at 0.7 reads as yes (TypeSafe docs); a Choice acts on the chosen option's
 * probability. Not yet confirmed on live answers: see `.scratch/typesafe/eval.md`.
 */
export const T = {
  termReference: 0.7,
  termVisual: 0.75,
  termTheme: 0.75,
  fileMainP: 0.6,
  fileLabelP: 0.6,
} as const;

/**
 * Field classes in the index, packed as `stringId * PACK + class`: a description word, a category
 * name, the author, the hero (from GameBanana's hero subcategory), a verified search term.
 */
export const CLS = { description: 0, category: 1, author: 2, hero: 3, term: 4 } as const;
export const PACK = 8;

/** At most this many distinct description words per mod. */
export const MAX_DESCRIPTION_WORDS = 60;

const DESCRIPTION_STOP = new Set([
  "the", "and", "for", "with", "this", "that", "from", "you", "your", "are", "was", "but", "not", "all", "can", "has", "have",
  "its", "will", "just", "also", "into", "out", "use", "any", "one", "mod", "mods", "deadlock", "credit", "credits", "thanks",
  "https", "http", "www", "com",
]);

export type ModMeta = {
  readonly id: number;
  readonly title: string;
  readonly category: string;
  readonly subcategory: string;
  readonly author: string;
  readonly description: string;
  readonly enrichedAt: number;
  /** The page changed after its judgments were made and they have not been redone yet. */
  readonly stale?: boolean;
};

export type JudgmentRows = Readonly<Record<string, Stored>>;

export type IndexMod = {
  i: number;
  ti: string;
  u: number;
  x: number[];
  /** 1 when the page changed since its verified terms were made. */
  st?: 1;
};

export type CatalogIndex = {
  v: number;
  built: number;
  strings: string[];
  mods: IndexMod[];
};

/** Distinct description words worth indexing, in order of first use. */
export function descriptionWords(text: string): string[] {
  const out: string[] = [];
  for (const word of normalize(text).split(" ")) {
    if (word.length < 2 || word.length > 24 || DESCRIPTION_STOP.has(word) || /^\d+$/.test(word) && word.length > 4) continue;
    if (!out.includes(word)) out.push(word);
    if (out.length >= MAX_DESCRIPTION_WORDS) break;
  }
  return out;
}

export function keptTerms(row: Stored | undefined): string[] {
  const terms = Array.isArray(row?.terms) ? (row.terms as { term: string; cls: string; p: number }[]) : [];
  return terms
    .filter((term) => {
      if (term.cls === "reference" || term.cls === "alias") return term.p >= T.termReference;
      if (term.cls === "visual") return term.p >= T.termVisual;
      if (term.cls === "theme") return term.p >= T.termTheme;
      return false;
    })
    .map((term) => term.term);
}

/** The whole index: one shared string table, per mod the title and packed fields. */
export function buildIndex(mods: readonly ModMeta[], rows: ReadonlyMap<number, JudgmentRows>, now: number, terms = false): CatalogIndex {
  const strings: string[] = [];
  const ids = new Map<string, number>();
  const intern = (text: string): number => {
    const key = normalize(text);
    let id = ids.get(key);
    if (id === undefined) {
      id = strings.length;
      strings.push(key);
      ids.set(key, id);
    }
    return id;
  };
  const out: IndexMod[] = [];
  for (const meta of mods) {
    const fields = new Map<number, number>();
    const add = (text: string, cls: number): void => {
      if (normalize(text).length === 0) return;
      const id = intern(text);
      const known = fields.get(id);
      // A string in several fields keeps its strongest class (the larger weight in the matcher).
      if (known === undefined || CLASS_RANK[cls]! > CLASS_RANK[known]!) fields.set(id, cls);
    };
    for (const word of descriptionWords(meta.description)) add(word, CLS.description);
    for (const part of [meta.category, meta.subcategory]) add(part, CLS.category);
    add(meta.author, CLS.author);
    if (meta.category.toLowerCase() === "skins" && meta.subcategory.length > 0) add(meta.subcategory, CLS.hero);
    if (terms) for (const term of keptTerms(rows.get(meta.id)?.terms)) add(term, CLS.term);
    const record: IndexMod = { i: meta.id, ti: meta.title, u: meta.enrichedAt, x: [...fields.entries()].map(([id, cls]) => id * PACK + cls) };
    if (meta.stale === true) record.st = 1;
    out.push(record);
  }
  return { v: PUBLISH_VERSION, built: now, strings, mods: out };
}

/** Matcher order of field strength, by class. */
const CLASS_RANK: Readonly<Record<number, number>> = { 0: 0, 1: 2, 2: 1, 3: 3, 4: 4 };

/** Per-mod detail: the file hint, when the flagged file labels ran. */
export function modDetail(id: number, judged: JudgmentRows): Record<string, unknown> {
  const files = judged.files;
  const labels = Array.isArray(files?.labels) ? (files.labels as { id: number; label: string | null; p: number }[]) : [];
  const main = files?.main as { id: number; p: number } | null | undefined;
  return {
    id,
    files:
      files === undefined
        ? null
        : {
            // A hint for pre-selecting in the picker; the app still shows the picker.
            main: main !== null && main !== undefined && main.p >= T.fileMainP ? main.id : null,
            labels: labels.map((label) => ({ id: label.id, label: label.p >= T.fileLabelP ? label.label : null })),
          },
  };
}
