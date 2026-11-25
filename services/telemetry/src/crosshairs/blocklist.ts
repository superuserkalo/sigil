/**
 * The short list of names a community crosshair may not carry (ADR 0019). Only obvious slurs, kept
 * in this one file. No model is used: `name.ts` matches these against a normalised form of the name
 * (lower case, accents stripped, common digit-for-letter swaps), and a letter repeated in the name
 * still matches ("fagggot"). The list is not meant to be complete: reports and the admin routes
 * handle the rest.
 */

/**
 * Matched anywhere in the name once spaces and punctuation are removed. Only stems long or rare
 * enough that an innocent name does not contain them.
 */
export const BLOCKED_STEMS: readonly string[] = [
  "nigger",
  "nigga",
  "faggot",
  "retard",
  "tranny",
  "wetback",
  "raghead",
  "kike",
  "chink",
];

/** Matched only as a whole word, or as the whole name once spaces are removed. */
export const BLOCKED_WORDS: readonly string[] = ["fag", "fags", "spic", "spics", "coon", "coons", "gook", "gooks", "beaner", "dyke"];
