/**
 * Thresholds and the compact catalog output. Stored judgments keep raw probabilities; this file
 * decides what is published, so a threshold change needs no new inference (bump PUBLISH_VERSION).
 *
 * Starting thresholds follow the TypeSafe docs (Noul: 0.7 reads as yes; Choice: act on the chosen
 * option's probability) and are raised where a wrong visible tag costs more than a missing one.
 * `.scratch/typesafe/eval.md` records the hand-checked sample used to confirm them.
 */

import { aliasTable, normalize } from "./aliases";
import { categoryHero, STYLES, type Stored } from "./judgments";

export const PUBLISH_VERSION = 1;

export const T = {
  kindFitsYes: 0.7,
  kindFitsNo: 0.3,
  kindP: 0.6,
  style: 0.75,
  heroP: 0.7,
  nsfwHint: 0.8,
  fact: 0.7,
  whichP: 0.6,
  installKindP: 0.6,
  stateP: 0.7,
  fileMainP: 0.6,
  fileLabelP: 0.6,
  updateKindP: 0.6,
  compat: 0.7,
  health: 0.7,
  dupe: 0.85,
  termReference: 0.7,
  termVisual: 0.75,
  termTheme: 0.75,
  learned: 0.67,
  bannerMin: 0.5,
  bannerText: 0.7,
} as const;

/**
 * Term classes in the index, which say where a term came from so the matcher can weight it:
 * a reference (franchise or character, and its other names) proposed from the page text, a tag
 * (hero, kind, style, fact), a visual term, a theme, a reference proposed from the cover picture,
 * and a term learned from floor-passed search gaps. Packed as `stringId * PACK + class`.
 */
export const CLS = { reference: 0, tag: 1, visual: 2, theme: 3, picture: 4, learned: 5 } as const;
export const PACK = 8;

export type JudgmentRows = Readonly<Record<string, Stored>>;

export type ModMeta = { readonly id: number; readonly title: string; readonly subcategory: string; readonly enrichedAt: number };

const FACT_TERMS: Record<string, string> = {
  variants: "variants",
  model: "model swap",
  sounds: "sound",
  abilities: "ability effects",
};

const KIND_TERMS: Record<string, string> = {
  recolour: "recolour",
  model_swap: "model swap",
  skin: "skin",
  hud: "hud",
  sound: "sound",
  music: "music",
  animation: "animation",
  effects: "effects",
  map: "map",
  crosshair: "crosshair",
  quality_of_life: "quality of life",
};

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function pick(value: unknown): { choice: string; p: number } | null {
  if (typeof value !== "object" || value === null) return null;
  const { choice, p } = value as { choice?: unknown; p?: unknown };
  return typeof choice === "string" ? { choice, p: num(p) } : null;
}

export function publishedKind(row: Stored | undefined): string | null {
  if (row === undefined) return null;
  if (num(row.fits) < T.kindFitsNo) return "other";
  if (num(row.fits) >= T.kindFitsYes && num(row.p) >= T.kindP && typeof row.kind === "string") return row.kind;
  return null;
}

export function publishedHero(row: Stored | undefined, meta: ModMeta, heroes: readonly string[]): string | null {
  const fromCategory = categoryHero({ subcategory: meta.subcategory } as never, heroes);
  if (fromCategory !== null) return fromCategory;
  if (row === undefined || num(row.p) < T.heroP || typeof row.hero !== "string") return null;
  if (row.hero === "none") return null;
  return row.hero;
}

export function publishedStyles(row: Stored | undefined): string[] {
  if (row === undefined) return [];
  return Object.keys(STYLES).filter((id) => num(row[id]) >= T.style);
}

/** Only ever adds a hint; never clears GameBanana's own flag. */
export function nsfwHint(row: Stored | undefined): boolean {
  return row !== undefined && num(row.p) >= T.nsfwHint;
}

type Shot = { i: number; w: number; h: number; card: number; banner: number; text: number; box: [number, number, number, number] | null };

export function coverPicks(row: Stored | undefined): { card: number; banner: [number, number, ...number[]] | null } | null {
  const shots = Array.isArray(row?.shots) ? (row.shots as Shot[]) : [];
  if (shots.length === 0) return null;
  const card = shots.reduce((best, shot) => (shot.card > best.card ? shot : best));
  const usable = shots.filter((shot) => shot.text < T.bannerText);
  const best = usable.reduce<Shot | null>((top, shot) => (top === null || shot.banner > top.banner ? shot : top), null);
  const banner = best !== null && best.banner >= T.bannerMin ? ([best.i, best.banner, ...(best.box ?? [])] as [number, number, ...number[]]) : null;
  return { card: card.i, banner };
}

export function publishedFacts(row: Stored | undefined): string[] {
  if (row === undefined) return [];
  const out = Object.keys(FACT_TERMS).filter((id) => num(row[id]) >= T.fact);
  if (num(row.install) >= T.fact) {
    const kind = pick(row.installKind);
    out.push(kind !== null && kind.p >= T.installKindP ? `install:${kind.choice}` : "install");
  }
  return out;
}

export function requiresMod(row: Stored | undefined): number | null {
  if (row === undefined || num(row.requires) < T.fact) return null;
  const which = row.which as { id?: unknown; p?: unknown } | undefined;
  return which !== undefined && num(which.p) >= T.whichP && num(which.id) > 0 ? num(which.id) : null;
}

export function devState(row: Stored | undefined): "wip" | "abandoned" | null {
  const state = pick(row?.state);
  if (state === null || state.p < T.stateP) return null;
  return state.choice === "wip" || state.choice === "abandoned" ? state.choice : null;
}

/** The newest classified update: [time, kind or "", 1 when it says it restores compatibility]. */
export function latestUpdate(row: Stored | undefined): [number, string, number] | null {
  const entries = Array.isArray(row?.entries) ? (row.entries as { at: number; kind: unknown; compat: unknown }[]) : [];
  const newest = entries.reduce<(typeof entries)[number] | null>((top, entry) => (top === null || entry.at > top.at ? entry : top), null);
  if (newest === null) return null;
  const kind = pick(newest.kind);
  return [newest.at, kind !== null && kind.p >= T.updateKindP ? kind.choice : "", num(newest.compat) >= T.compat ? 1 : 0];
}

export function healthChip(row: Stored | undefined): { broken: boolean; works: boolean; conflict: boolean } | null {
  if (row === undefined || num(row.commenters) < 3) return null;
  return { broken: num(row.broken) >= T.health, works: num(row.works) >= T.health, conflict: num(row.conflict) >= T.health };
}

export function dupeOf(row: Stored | undefined): number | null {
  const pairs = Array.isArray(row?.pairs) ? (row.pairs as { id: number; p: number }[]) : [];
  const best = pairs.filter((pair) => pair.p >= T.dupe).sort((a, b) => b.p - a.p)[0];
  return best?.id ?? null;
}

export type KeptTerm = { term: string; cls: number; facet: string };

export function keptTerms(row: Stored | undefined): KeptTerm[] {
  const terms = Array.isArray(row?.terms) ? (row.terms as { term: string; cls: string; facet?: string; src?: string; p: number }[]) : [];
  return terms.flatMap((term): KeptTerm[] => {
    const facet = term.facet ?? (term.cls === "visual" || term.cls === "theme" ? term.cls : "franchise");
    if ((term.cls === "reference" || term.cls === "alias") && term.p >= T.termReference) {
      return [{ term: term.term, cls: term.src === "picture" ? CLS.picture : CLS.reference, facet }];
    }
    if (term.cls === "visual" && term.p >= T.termVisual) return [{ term: term.term, cls: CLS.visual, facet }];
    if (term.cls === "theme" && term.p >= T.termTheme) return [{ term: term.term, cls: CLS.theme, facet }];
    return [];
  });
}

/** Typed facets: the verified franchise and character names (not their other spellings). */
export function facets(row: Stored | undefined): { franchises: string[]; characters: string[] } {
  const terms = Array.isArray(row?.terms) ? (row.terms as { term: string; cls: string; facet?: string; p: number }[]) : [];
  const named = (facet: string) => terms.filter((term) => term.cls === "reference" && (term.facet ?? "franchise") === facet && term.p >= T.termReference).map((term) => term.term);
  return { franchises: named("franchise"), characters: named("character") };
}

/** Learned terms from search gaps that Jev judged relevant to this mod. */
export function learnedTerms(row: Stored | undefined): string[] {
  const terms = Array.isArray(row?.terms) ? (row.terms as { term: string; s: number }[]) : [];
  return terms.filter((term) => term.s >= T.learned).map((term) => term.term);
}

export type IndexMod = {
  i: number;
  ti: string;
  u: number;
  x: number[];
  k?: string;
  h?: string;
  n?: 1;
  c?: number;
  b?: number[];
  r?: number;
  w?: string;
  up?: [number, string, number];
  he?: string;
  d?: number;
  /** Verified franchise and character names, as string ids. */
  f?: number[];
  ch?: number[];
  /** Optional document embedding, int8, base64 (behind the index embeddings flag). */
  e?: string;
};

export type CatalogIndex = {
  v: number;
  built: number;
  strings: string[];
  aliases: Record<string, string>;
  mods: IndexMod[];
};

/** The whole tag index: one shared string table, ids per mod, aliases for local matching. */
export function buildIndex(
  mods: readonly ModMeta[],
  rows: ReadonlyMap<number, JudgmentRows>,
  heroes: readonly string[],
  now: number,
  embeddings = false,
): CatalogIndex {
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
    const judged = rows.get(meta.id) ?? {};
    const terms = new Map<number, number>();
    const add = (text: string, cls: number): void => {
      if (normalize(text).length === 0) return;
      const id = intern(text);
      const known = terms.get(id);
      if (known === undefined || cls < known) terms.set(id, cls);
    };
    const record: IndexMod = { i: meta.id, ti: meta.title, u: meta.enrichedAt, x: [] };
    const kind = publishedKind(judged.kind);
    if (kind !== null) {
      record.k = kind;
      if (kind !== "other") add(KIND_TERMS[kind] ?? kind, CLS.tag);
    }
    const hero = publishedHero(judged.hero, meta, heroes);
    if (hero !== null) {
      record.h = hero;
      if (hero !== "several") add(hero, CLS.tag);
    }
    for (const id of publishedStyles(judged.style)) add(id, CLS.tag);
    for (const fact of publishedFacts(judged.facts)) if (FACT_TERMS[fact] !== undefined) add(FACT_TERMS[fact] ?? fact, CLS.tag);
    for (const term of keptTerms(judged.terms)) add(term.term, term.cls);
    for (const term of learnedTerms(judged.learned)) add(term, CLS.learned);
    const named = facets(judged.terms);
    if (named.franchises.length > 0) record.f = named.franchises.map(intern);
    if (named.characters.length > 0) record.ch = named.characters.map(intern);
    if (embeddings && typeof judged.embed?.v === "string") record.e = judged.embed.v;
    if (nsfwHint(judged.nsfw)) record.n = 1;
    const cover = coverPicks(judged.cover);
    if (cover !== null) {
      record.c = cover.card;
      if (cover.banner !== null) record.b = cover.banner;
    }
    const requires = requiresMod(judged.facts);
    if (requires !== null) record.r = requires;
    const state = devState(judged.facts);
    if (state !== null) record.w = state;
    const update = latestUpdate(judged.updates);
    if (update !== null) record.up = update;
    const chip = healthChip(judged.health);
    if (chip !== null && (chip.broken || chip.works || chip.conflict)) {
      record.he = [chip.broken ? "broken" : "", chip.works ? "works" : "", chip.conflict ? "conflict" : ""].filter((part) => part.length > 0).join(",");
    }
    const dupe = dupeOf(judged.dupes);
    if (dupe !== null) record.d = dupe;
    record.x = [...terms.entries()].map(([id, cls]) => id * PACK + cls);
    out.push(record);
  }
  // Canonical names for the alias table's targets are interned too, so the app can resolve them.
  const aliases = aliasTable(heroes);
  for (const target of new Set(Object.values(aliases))) intern(target);
  return { v: PUBLISH_VERSION, built: now, strings, aliases, mods: out };
}

/** The per-mod detail: files, update notes, health counts, facts, kept terms. */
export function modDetail(id: number, judged: JudgmentRows): Record<string, unknown> {
  const files = judged.files;
  const labels = Array.isArray(files?.labels) ? (files.labels as { id: number; label: string | null; p: number }[]) : [];
  const main = files?.main as { id: number; p: number } | null | undefined;
  const updates = Array.isArray(judged.updates?.entries) ? (judged.updates.entries as { at: number; kind: unknown; compat: unknown }[]) : [];
  const health = judged.health;
  const facts = judged.facts;
  return {
    id,
    files:
      files === undefined
        ? null
        : {
            main: main !== null && main !== undefined && main.p >= T.fileMainP ? main.id : null,
            labels: labels.map((label) => ({ id: label.id, label: label.p >= T.fileLabelP ? label.label : null })),
          },
    updates: updates.map((entry) => {
      const kind = pick(entry.kind);
      return { at: entry.at, kind: kind !== null && kind.p >= T.updateKindP ? kind.choice : null, restoresCompat: num(entry.compat) >= T.compat };
    }),
    health:
      health === undefined
        ? null
        : {
            ...healthChip(health),
            comments: num(health.comments),
            commenters: num(health.commenters),
            from: num(health.from),
            to: num(health.to),
          },
    facts: facts === undefined ? null : { flags: publishedFacts(facts), requires: requiresMod(facts), state: devState(facts) },
    terms: keptTerms(judged.terms),
  };
}
