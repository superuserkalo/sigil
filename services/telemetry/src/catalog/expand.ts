/**
 * Document expansion: a generative model with world knowledge proposes search terms for one mod
 * from its public title, description, category and main screenshot (Cloudflare Workers AI through
 * the `AI` binding). Jev then checks every proposed term (`judgments.ts`, `terms`), which is what
 * stops invented references. Only kept terms reach the catalog index.
 *
 * HARD RULE, same as TypeSafe: only public GameBanana content goes in. `expansionInput` builds the
 * one message from a fixed set of fields and is the only way in. No player text, ever.
 *
 * Fail closed: without the binding, or on any error, there is no expansion and the rest of the
 * enrichment runs as before.
 */

import type { ListedMod, Profile } from "./gamebanana";
import type { Picture } from "./images";
import { normalize } from "./aliases";

/** Natively multimodal, 131k context, JSON schema output. $0.27 / M input, $0.85 / M output. */
export const EXPAND_MODEL = "@cf/meta/llama-4-scout-17b-16e-instruct";
export const EXPAND_USD_IN = 0.27 / 1_000_000;
export const EXPAND_USD_OUT = 0.85 / 1_000_000;
/** Bumping it re-runs the generator and the `terms` judgment. */
export const EXPAND_VERSION = 1;

export const MAX_REFERENCES = 8;
export const MAX_ALIASES = 5;
export const MAX_VISUAL = 12;
export const MAX_THEMES = 6;
export const MAX_TERMS = 40;
export const MAX_TERM_CHARS = 40;

export type TermClass = "reference" | "alias" | "visual" | "theme";

export type Term = { readonly term: string; readonly cls: TermClass; readonly of?: string };

export type Expansion = { readonly model: string; readonly terms: readonly Term[]; readonly usd: number };

export type AiRunner = { run: (model: string, input: Record<string, unknown>) => Promise<unknown> };

const SCHEMA = {
  type: "object",
  properties: {
    references: {
      type: "array",
      maxItems: MAX_REFERENCES,
      items: {
        type: "object",
        properties: {
          name: { type: "string", maxLength: MAX_TERM_CHARS },
          aliases: { type: "array", maxItems: MAX_ALIASES, items: { type: "string", maxLength: MAX_TERM_CHARS } },
        },
        required: ["name", "aliases"],
      },
    },
    visual: { type: "array", maxItems: MAX_VISUAL, items: { type: "string", maxLength: MAX_TERM_CHARS } },
    themes: { type: "array", maxItems: MAX_THEMES, items: { type: "string", maxLength: MAX_TERM_CHARS } },
  },
  required: ["references", "visual", "themes"],
} as const;

const SYSTEM = [
  "You write search keywords for a mod of the video game Deadlock, so players can find it by what they type.",
  "Return JSON only, every string in lower case, short phrases, no sentences.",
  "references: outside games, franchises, shows, films, anime, memes, real brands and characters this mod clearly takes from or names. For each, the common name and up to five other names players type: abbreviations, short forms, character or faction names, other spellings, transliterations and common misspellings. Example for a Counter-Strike 1.6 GIGN skin: name \"counter-strike\", aliases [\"cs\", \"cs 1.6\", \"counter strike\", \"gign\", \"counter-terrorist\"].",
  "visual: what the main picture and description show in plain words: objects, clothing, colours, materials, art style (for example gas mask, soldier, maid outfit, neon, pixel art).",
  "themes: moods and themes (for example horror, cozy, cyberpunk, military, royal).",
  "Do not invent references the title, description or picture do not support. Leave a list empty when nothing fits. Do not repeat the Deadlock hero name or the word deadlock.",
].join("\n");

/** The only content the generator sees. Built from public mod page fields only. */
export function expansionInput(mod: ListedMod, profile: Profile): string {
  return JSON.stringify({
    title: mod.title,
    category: [mod.category, mod.subcategory].filter((part) => part.length > 0).join(" / "),
    description: [profile.summary, profile.text].filter((part) => part.length > 0).join("\n").slice(0, 1500),
  });
}

export function aiBinding(env: Env): AiRunner | null {
  const ai = (env as Env & { readonly AI?: AiRunner }).AI;
  return ai !== undefined && typeof ai.run === "function" ? ai : null;
}

export async function expand(ai: AiRunner, mod: ListedMod, profile: Profile, picture: Picture | null): Promise<Expansion | null> {
  const text = expansionInput(mod, profile);
  const content: unknown =
    picture === null
      ? text
      : [
          { type: "text", text },
          { type: "image_url", image_url: { url: `data:image/${picture.format === "png" ? "png" : "jpeg"};base64,${picture.base64}` } },
        ];
  const raw = await ai.run(EXPAND_MODEL, {
    messages: [
      { role: "system", content: SYSTEM },
      { role: "user", content },
    ],
    response_format: { type: "json_schema", json_schema: SCHEMA },
    max_tokens: 700,
    temperature: 0.2,
  });
  const terms = parseExpansion(raw);
  if (terms === null) return null;
  const usage = (raw as { usage?: { prompt_tokens?: number; completion_tokens?: number } } | null)?.usage;
  const usd =
    (usage?.prompt_tokens ?? Math.ceil(text.length / 4) + (picture === null ? 0 : 1600)) * EXPAND_USD_IN +
    (usage?.completion_tokens ?? 400) * EXPAND_USD_OUT;
  return { model: EXPAND_MODEL, terms, usd };
}

/** Strict reading: unknown shapes give null, strings are normalised and capped, totals bounded. */
export function parseExpansion(raw: unknown): Term[] | null {
  let value: unknown = raw;
  if (typeof value === "object" && value !== null && "response" in value) value = (value as { response: unknown }).response;
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return null;
    }
  }
  if (typeof value !== "object" || value === null) return null;
  const record = value as { references?: unknown; visual?: unknown; themes?: unknown };
  const seen = new Set<string>(["deadlock"]);
  const out: Term[] = [];
  const add = (text: unknown, cls: TermClass, of?: string): string | null => {
    if (typeof text !== "string" || out.length >= MAX_TERMS) return null;
    const term = normalize(text).slice(0, MAX_TERM_CHARS).trim();
    if (term.length < 2 || seen.has(term)) return null;
    seen.add(term);
    out.push(of === undefined ? { term, cls } : { term, cls, of });
    return term;
  };
  const references = Array.isArray(record.references) ? record.references.slice(0, MAX_REFERENCES) : [];
  for (const reference of references) {
    if (typeof reference !== "object" || reference === null) continue;
    const { name, aliases } = reference as { name?: unknown; aliases?: unknown };
    const kept = add(name, "reference");
    if (kept === null) continue;
    for (const alias of Array.isArray(aliases) ? aliases.slice(0, MAX_ALIASES) : []) add(alias, "alias", kept);
  }
  for (const visual of Array.isArray(record.visual) ? record.visual.slice(0, MAX_VISUAL) : []) add(visual, "visual");
  for (const theme of Array.isArray(record.themes) ? record.themes.slice(0, MAX_THEMES) : []) add(theme, "theme");
  return out;
}
