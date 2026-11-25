/**
 * A small curated supplement to the generated search terms: hero nicknames, mod-kind and style
 * synonyms, and a few German words for common queries. Outside franchises and characters are not
 * listed here (see ADR 0018).
 *
 * Every entry maps what a player may type to a canonical term that the index already uses.
 */

/** Lower case, accents dropped, punctuation to spaces, spaces collapsed. Shared with the matcher. */
export function normalize(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/&/g, " and ")
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

/** Hero nicknames players use. Keys are canonical hero names (normalised). */
export const HERO_NICKNAMES: Readonly<Record<string, readonly string[]>> = {
  abrams: ["bull", "abe"],
  "grey talon": ["talon", "gt", "gray talon", "orion"],
  "lady geist": ["geist", "lg"],
  mcginnis: ["mcg", "mcginis", "mcginniss"],
  "mo and krill": ["mo krill", "mo", "krill", "mnk"],
  infernus: ["inf", "infernis"],
  vindicta: ["vindi", "vindy"],
  viscous: ["goo", "viscus", "viscious"],
  wraith: ["wraif"],
  yamato: ["yama"],
  seven: ["7"],
  pocket: ["pockets"],
  paradox: ["dox"],
  dynamo: ["dyna"],
  "deadman danny": ["danny", "deadman"],
  "nurse harrow": ["harrow", "nurse"],
  "rat king": ["rat"],
  holliday: ["holiday", "holly"],
  kelvin: ["kel"],
  warden: ["ward"],
  sinclair: ["sinc"],
  calico: ["cali"],
  doorman: ["door"],
  bebop: ["bop", "be bop"],
  shiv: ["shivv"],
  haze: ["haze"],
  ivy: ["ivy"],
  lash: ["lash"],
  mirage: ["mira"],
  vyper: ["viper"],
  victor: ["vic"],
};

/** Synonyms for the closed kinds and styles. Keys are the canonical term the index uses. */
export const TAG_SYNONYMS: Readonly<Record<string, readonly string[]>> = {
  recolour: ["recolor", "retexture", "texture", "colour", "color", "recolored", "recoloured"],
  "model swap": ["model", "model replacement", "replacement", "swap"],
  skin: ["outfit", "costume", "skins"],
  hud: ["ui", "interface", "menu", "icons", "hud mod"],
  sound: ["sounds", "voice", "voice lines", "sfx", "audio"],
  music: ["soundtrack", "song", "songs", "ost"],
  animation: ["animations", "emote", "emotes", "pose"],
  effects: ["vfx", "particles", "particle", "effect"],
  map: ["environment", "lighting", "sky", "world"],
  crosshair: ["reticle", "cursor"],
  "quality of life": ["qol", "config", "tweak", "settings"],
  anime: ["manga", "weeb", "waifu"],
  realistic: ["real", "realism", "lifelike"],
  cute: ["kawaii", "adorable", "chibi"],
  horror: ["scary", "creepy", "spooky"],
  meme: ["joke", "funny", "shitpost"],
  festive: ["christmas", "halloween", "holiday", "seasonal", "xmas"],
  crossover: ["from another game"],
  minimal: ["clean", "minimalist", "simple"],
  colourful: ["colorful", "vibrant", "rainbow"],
  dark: ["black", "edgy", "gothic"],
  variants: ["variant", "options", "colours", "colors"],
};

/** Common German query words, cheap to ship, mapped to the English the terms use. */
export const GERMAN: Readonly<Record<string, string>> = {
  maske: "mask",
  gasmaske: "gas mask",
  soldat: "soldier",
  schwarz: "black",
  weiss: "white",
  rot: "red",
  blau: "blue",
  grun: "green",
  gelb: "yellow",
  rosa: "pink",
  lila: "purple",
  gold: "gold",
  katze: "cat",
  hund: "dog",
  niedlich: "cute",
  suss: "cute",
  gruselig: "horror",
  weihnachten: "christmas",
  musik: "music",
  waffe: "weapon",
  waffen: "weapons",
  kleid: "dress",
  rustung: "armor",
  ritter: "knight",
  hexe: "witch",
  zombie: "zombie",
  roboter: "robot",
  feuer: "fire",
  eis: "ice",
  schwert: "sword",
  hut: "hat",
  brille: "glasses",
  anzug: "suit",
  uniform: "uniform",
  geraeusche: "sounds",
  stimme: "voice",
  fadenkreuz: "crosshair",
};

/** Flat alias table, alias to canonical term, in the shape the catalog output ships. */
export function aliasTable(heroes: readonly string[] = []): Record<string, string> {
  const table: Record<string, string> = {};
  const known = new Set(heroes.map(normalize));
  for (const [hero, nicknames] of Object.entries(HERO_NICKNAMES)) {
    if (known.size > 0 && !known.has(hero)) continue;
    for (const nickname of nicknames) if (nickname !== hero) table[normalize(nickname)] = hero;
  }
  for (const [tag, synonyms] of Object.entries(TAG_SYNONYMS)) {
    for (const synonym of synonyms) table[normalize(synonym)] ??= tag;
  }
  for (const [german, english] of Object.entries(GERMAN)) table[german] ??= english;
  return table;
}
