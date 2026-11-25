/**
 * Community crosshair names (ADR 0019).
 *
 * - NFC, then trimmed, with every run of whitespace collapsed to one space.
 * - 2 to 32 characters.
 * - Only letters (ASCII and the Latin-1 and Latin Extended-A letters, so "é", "ß", "ł"), digits,
 *   space and `. - _ ' ! +`. There is no `/`, `:` or `@`, and a dot may not be followed directly
 *   by a letter or digit, so a name cannot spell a link ("x.com") or an address.
 * - At least one letter or digit.
 * - Not on the blocklist (`blocklist.ts`), matched on a normalised form.
 */

import { BLOCKED_STEMS, BLOCKED_WORDS } from "./blocklist";

export const NAME_MIN = 2;
export const NAME_MAX = 32;

const LETTER = "A-Za-zÀ-ÖØ-öø-ſ";
const ALLOWED = new RegExp(`^[${LETTER}0-9 .\\-_'!+]+$`);
const HAS_ALNUM = new RegExp(`[${LETTER}0-9]`);
const DOT_THEN_ALNUM = new RegExp(`\\.[${LETTER}0-9]`);

export type NameResult = { readonly ok: true; readonly name: string } | { readonly ok: false; readonly error: "bad_name" | "blocked_name" };

/** Letters NFD does not split into a base letter and a mark. */
const FOLD: Readonly<Record<string, string>> = {
  ß: "ss",
  æ: "ae",
  œ: "oe",
  ø: "o",
  đ: "d",
  ð: "d",
  ł: "l",
  þ: "th",
  ı: "i",
  ŋ: "n",
  ĸ: "k",
  ŀ: "l",
  ŉ: "n",
  ſ: "s",
};

const SWAPS: Readonly<Record<string, string>> = { "0": "o", "3": "e", "4": "a", "5": "s", "6": "g", "7": "t", "8": "b", "9": "g", "!": "i", "+": "t" };

/**
 * The forms the blocklist is matched against: lower case, accents stripped, digits read as the
 * letters they stand for. `1` is read both as `i` and as `l`, so there are two forms.
 */
export function blocklistForms(name: string): string[] {
  const base = Array.from(name.toLowerCase().normalize("NFD").replace(/\p{M}/gu, ""))
    .map((c) => FOLD[c] ?? SWAPS[c] ?? c)
    .join("");
  return [base.replaceAll("1", "i"), base.replaceAll("1", "l")];
}

/** `word` with each letter allowed to repeat: "fag" matches "faag" but "niger" never matches "nigger". */
function repeatable(word: string): string {
  return Array.from(word, (c) => `${c}+`).join("");
}

const STEM_PATTERNS = BLOCKED_STEMS.map((stem) => new RegExp(repeatable(stem)));
const WORD_PATTERNS = BLOCKED_WORDS.map((word) => new RegExp(`^${repeatable(word)}$`));

export function isBlocked(name: string): boolean {
  for (const form of blocklistForms(name)) {
    const words = form.split(/[^a-z]+/).filter(Boolean);
    const compact = words.join("");
    if (STEM_PATTERNS.some((pattern) => pattern.test(compact))) return true;
    for (const candidate of [...words, compact]) {
      if (WORD_PATTERNS.some((pattern) => pattern.test(candidate))) return true;
    }
  }
  return false;
}

export function parseName(raw: unknown): NameResult {
  if (typeof raw !== "string" || raw.length > 256) return { ok: false, error: "bad_name" };
  const name = raw.normalize("NFC").trim().split(/\s+/).filter(Boolean).join(" ");
  const length = Array.from(name).length;
  if (length < NAME_MIN || length > NAME_MAX) return { ok: false, error: "bad_name" };
  if (!ALLOWED.test(name) || !HAS_ALNUM.test(name) || DOT_THEN_ALNUM.test(name)) return { ok: false, error: "bad_name" };
  if (isBlocked(name)) return { ok: false, error: "blocked_name" };
  return { ok: true, name };
}
