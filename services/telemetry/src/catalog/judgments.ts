/**
 * Small named judgments about one GameBanana mod. Each has its own version: bumping one re-runs
 * only that judgment. Code decides which apply, builds Jev questions over one shared state, and
 * reads the typed answers into a small stored record with the raw probabilities. Thresholds live
 * in `publish.ts`, so changing one needs no new inference.
 *
 * All questions for one mod go in one request (the docs' speculative fan-out): the state is sent
 * once and questions run in parallel.
 */

import type { Expansion } from "./expand";
import type { ListedMod, ModComment, ModUpdate, Profile } from "./gamebanana";
import type { Picture } from "./images";
import type { Answer, Question } from "./typesafe";

export type Part = "text" | "files" | "screenshots" | "updates" | "comments" | "others";

export type Other = { readonly id: number; readonly title: string; readonly category: string; readonly description: string };

export type ModInput = {
  readonly mod: ListedMod;
  readonly profile: Profile;
  readonly updates: readonly ModUpdate[];
  /** Newest first, already cut to the window. */
  readonly comments: readonly ModComment[];
  /** Pictures that loaded, with their index among the mod's screenshots. */
  readonly shots: readonly { readonly index: number; readonly picture: Picture }[];
  readonly others: readonly Other[];
  readonly heroes: readonly string[];
  readonly expansion: Expansion | null;
  readonly now: number;
};

export type Read = (id: string) => Answer | undefined;

export type Stored = Record<string, unknown>;

export type Judgment = {
  readonly name: string;
  readonly version: number;
  readonly needs: readonly Part[];
  /** False when there is nothing to judge (no files, too few comments); nothing is stored then. */
  readonly applies: (input: ModInput) => boolean;
  readonly ask: (input: ModInput) => Record<string, Question>;
  readonly read: (answer: Read, input: ModInput) => Stored | null;
};

export const MAX_SHOTS = 4;
export const MAX_FILES = 8;
export const MAX_UPDATES = 3;
export const MAX_COMMENTS = 20;
export const COMMENT_WINDOW_DAYS = 120;
export const MAX_OTHERS = 3;

// ---------------------------------------------------------------------------------------------
// Question and answer helpers
// ---------------------------------------------------------------------------------------------

function noul(instructions: string, criteria?: { true: string; false: string }): Question {
  return criteria === undefined ? { type: "noul", instructions } : { type: "noul", instructions, criteria };
}

function choice(instructions: string, criteria: Record<string, string>): Question {
  return { type: "choice", instructions, criteria };
}

function score(instructions: string, levels: string[]): Question {
  return { type: "score", instructions, criteria: levels };
}

function box(instructions: string): Question {
  return { type: "box", instructions };
}

export function yes(answer: Answer | undefined): number | null {
  return answer?.type === "noul" && Number.isFinite(answer.noul) ? round(answer.noul) : null;
}

export function picked(answer: Answer | undefined): { choice: string; p: number } | null {
  if (answer?.type !== "choice") return null;
  return { choice: answer.choice, p: round(answer.probabilities[answer.choice] ?? 0) };
}

/** A Score as a 0 to 1 position, with its confidence. */
export function scored(answer: Answer | undefined, levels: number): { s: number; c: number } | null {
  if (answer?.type !== "score" || levels < 2) return null;
  return { s: round(answer.score / (levels - 1)), c: round(answer.confidence) };
}

function boxed(answer: Answer | undefined): [number, number, number, number] | null {
  if (answer?.type !== "box") return null;
  const { left, top, right, bottom } = answer.box;
  if (![left, top, right, bottom].every((edge) => Number.isFinite(edge) && edge >= 0 && edge <= 1)) return null;
  if (left >= right || top >= bottom) return null;
  return [round(left), round(top), round(right), round(bottom)];
}

export function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function slug(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "") || "x";
}

// ---------------------------------------------------------------------------------------------
// The judgments
// ---------------------------------------------------------------------------------------------

export const KINDS: Readonly<Record<string, string>> = {
  recolour: "Recolours or retextures an existing hero, weapon, item or effect without changing its shape.",
  model_swap: "Replaces a hero or object model with a different model, for example a character from another game or show.",
  skin: "A new outfit, costume or skin for a hero that keeps the hero recognisable.",
  hud: "Changes the HUD, menus, icons, fonts, minimap or other interface parts.",
  sound: "Replaces or adds sound effects or voice lines.",
  music: "Replaces or adds music.",
  animation: "Changes animations, poses or emotes.",
  effects: "Changes visual effects, particles, ability visuals, tracers or glows.",
  map: "Changes the map, environment, lighting, sky or world props.",
  crosshair: "A crosshair or reticle.",
  quality_of_life: "A quality-of-life change, config or tweak that changes behaviour or readability rather than looks.",
};

const kind: Judgment = {
  name: "kind",
  version: 1,
  needs: ["text", "screenshots"],
  applies: () => true,
  ask: () => ({
    kind: choice("What kind of Deadlock mod is this, judging by its title, category, description and screenshots?", { ...KINDS }),
    fits: noul(
      "Does this mod clearly belong to one of these kinds: recolour or retexture, model swap, outfit or skin, HUD or interface, sound or voice, music, animation, effects or particles, map or environment, crosshair, quality of life or config?",
      {
        true: "One of those kinds describes the mod well.",
        false: "It is something else, such as a tool, a pack of many unrelated mods, a gameplay mode, or the page does not say.",
      },
    ),
  }),
  read: (answer) => {
    const chosen = picked(answer("kind"));
    const fits = yes(answer("fits"));
    return chosen === null || fits === null ? null : { kind: chosen.choice, p: chosen.p, fits };
  },
};

export const STYLES: Readonly<Record<string, string>> = {
  anime: "Is the look or theme of this mod anime or manga styled, or based on an anime or manga character?",
  realistic: "Does this mod aim for a realistic, true-to-life look?",
  cute: "Is this mod cute or adorable in style (chibi, pastel, plush, kawaii)?",
  horror: "Is this mod horror themed: creepy, gory or meant to be scary?",
  meme: "Is this mod a meme or a joke, made to be funny?",
  festive: "Is this mod festive or seasonal, made for a holiday such as Halloween, Christmas or Lunar New Year?",
  crossover: "Does this mod bring in a character, outfit or item from another game, show, film or franchise?",
  minimal: "Is this mod minimal or clean in style: simplified, decluttered, flat?",
  colourful: "Is this mod notably colourful: bright, saturated or rainbow colours?",
  dark: "Is this mod notably dark in look: black, gothic, shadowy or edgy?",
};

const style: Judgment = {
  name: "style",
  version: 1,
  needs: ["text", "screenshots"],
  applies: () => true,
  ask: () => Object.fromEntries(Object.entries(STYLES).map(([id, question]) => [id, noul(question)])),
  read: (answer) => {
    const out: Stored = {};
    for (const id of Object.keys(STYLES)) {
      const p = yes(answer(id));
      if (p === null) return null;
      out[id] = p;
    }
    return out;
  },
};

/** The hero a mod's subcategory names, when it is one: an exact lookup kept in code. */
export function categoryHero(mod: ListedMod, heroes: readonly string[]): string | null {
  const sub = mod.subcategory.trim().toLowerCase();
  return heroes.find((hero) => hero.toLowerCase() === sub) ?? null;
}

const hero: Judgment = {
  name: "hero",
  version: 1,
  needs: ["text", "screenshots"],
  applies: (input) => input.heroes.length > 0,
  ask: (input) => {
    const options: Record<string, string> = {};
    for (const name of input.heroes) options[`h_${slug(name)}`] = `The Deadlock hero ${name}.`;
    options.none = "No specific hero: the mod changes the interface, map, items, sounds for everything, or similar.";
    options.several = "Several heroes, for example a pack for many heroes.";
    return { hero: choice("Which Deadlock hero does this mod change?", options) };
  },
  read: (answer, input) => {
    const chosen = picked(answer("hero"));
    if (chosen === null) return null;
    const name = input.heroes.find((candidate) => `h_${slug(candidate)}` === chosen.choice);
    return {
      hero: name ?? chosen.choice,
      p: chosen.p,
      category: categoryHero(input.mod, input.heroes),
    };
  },
};

const nsfw: Judgment = {
  name: "nsfw",
  version: 1,
  needs: ["text", "screenshots"],
  applies: () => true,
  ask: () => ({
    nsfw: noul("Does this mod show or describe sexual content or nudity, including outfits made to be sexually revealing?"),
  }),
  read: (answer, input) => {
    const p = yes(answer("nsfw"));
    return p === null ? null : { p, gb: input.mod.rated };
  },
};

const CARD_LEVELS = [
  "The change cannot be made out at thumbnail size: tiny, dark, blurry, or mostly text or a logo.",
  "The change is visible but small or cluttered; a viewer has to look closely.",
  "The change is clear at a glance: one main subject, well lit.",
];

const BANNER_LEVELS = [
  "Unusable as a wide banner: mostly text, a collage of panels, blurry or tiny, or nothing to show.",
  "Weak: busy, several small subjects, big text over the picture, or soft.",
  "Good: a clear subject with some room around it, sharp enough.",
  "Excellent: one striking, sharp subject with space at the sides, little or no text, not a collage.",
];

const cover: Judgment = {
  name: "cover",
  version: 1,
  needs: ["screenshots"],
  applies: (input) => input.shots.length > 0,
  ask: (input) => {
    const out: Record<string, Question> = {};
    input.shots.forEach((_, at) => {
      const ref = `\`screenshots[${at}]\``;
      out[`card${at}`] = score(`How clearly does ${ref} show what this mod changes when shown as a small card thumbnail about 300 pixels wide?`, CARD_LEVELS);
      out[`banner${at}`] = score(`How well would ${ref} work as a wide 21:9 banner across the top of a page, cropped to a wide strip?`, BANNER_LEVELS);
      out[`text${at}`] = noul(`Does ${ref} have large text baked into the picture, such as a title, a logo with words, or captions covering much of it?`);
      out[`box${at}`] = box(`the main subject of ${ref}: the hero, model, item or scene the mod is showing off`);
    });
    return out;
  },
  read: (answer, input) => {
    const shots: Stored[] = [];
    for (let at = 0; at < input.shots.length; at += 1) {
      const shot = input.shots[at];
      const card = scored(answer(`card${at}`), CARD_LEVELS.length);
      const banner = scored(answer(`banner${at}`), BANNER_LEVELS.length);
      const text = yes(answer(`text${at}`));
      if (shot === undefined || card === null || banner === null || text === null) return null;
      shots.push({
        i: shot.index,
        w: shot.picture.width,
        h: shot.picture.height,
        card: card.s,
        banner: banner.s,
        bannerC: banner.c,
        text,
        box: boxed(answer(`box${at}`)),
      });
    }
    return { shots };
  },
};

const INSTALL_KINDS: Record<string, string> = {
  manual_files: "Files have to be placed or renamed by hand.",
  extra_tool: "An extra tool or program is needed to install or use it.",
  incompatible_mod: "It does not work together with another named mod, which has to be removed.",
};

const DEV_STATES: Record<string, string> = {
  finished: "Released and presented as complete or maintained.",
  wip: "Described as a work in progress, beta or unfinished, or more is coming.",
  abandoned: "Described as abandoned, discontinued or no longer updated.",
};

const facts: Judgment = {
  name: "facts",
  version: 1,
  needs: ["text", "files"],
  applies: () => true,
  ask: (input) => {
    const out: Record<string, Question> = {
      requires: noul("Does the description say this mod needs another mod installed to work (a requirement, not only a recommendation)?"),
      variants: noul("Does this mod come in several variants or colour options to choose from?"),
      model: noul("Does this mod replace a hero's 3D model, not only colours or textures?"),
      sounds: noul("Does this mod include sounds or voice lines?"),
      abilities: noul("Does this mod change how abilities, projectiles or their effects look?"),
      install: noul("Does the description mention install steps beyond a normal install, such as placing files by hand, using an extra tool, or removing another mod?"),
      installKind: choice("Which kind of special install step does the description mention?", INSTALL_KINDS),
      state: choice("How does the page present this mod's state?", DEV_STATES),
    };
    if (input.profile.links.length > 0) {
      const options: Record<string, string> = {};
      input.profile.links.forEach((_, at) => {
        // Link text is untrusted page content: the option points at it in the state, never quotes it.
        options[`link${at}`] = `The mod linked as \`links[${at}].label\`.`;
      });
      options.none = "None of the linked mods is required.";
      out.which = choice("Which linked mod does this mod require?", options);
    }
    return out;
  },
  read: (answer, input) => {
    const out: Stored = {};
    for (const id of ["requires", "variants", "model", "sounds", "abilities", "install"]) {
      const p = yes(answer(id));
      if (p === null) return null;
      out[id] = p;
    }
    out.installKind = picked(answer("installKind"));
    out.state = picked(answer("state"));
    const which = picked(answer("which"));
    if (which !== null) {
      const at = which.choice.startsWith("link") ? Number(which.choice.slice(4)) : -1;
      out.which = { id: input.profile.links[at]?.id ?? 0, p: which.p };
    }
    return out;
  },
};

export const FILE_LABELS: Record<string, string> = {
  main: "The main download most players want.",
  variant: "An alternative version of the same mod, such as another colour or style.",
  addon: "An optional add-on to use together with the main file.",
  old: "An older version kept for reference.",
  source: "Source or project files for modders, not for playing.",
};

const files: Judgment = {
  name: "files",
  version: 1,
  needs: ["text", "files"],
  applies: (input) => input.profile.files.length >= 2,
  ask: (input) => {
    const list = input.profile.files.slice(0, MAX_FILES);
    const options: Record<string, string> = {};
    list.forEach((_, at) => {
      options[`file${at}`] = `\`files[${at}]\``;
    });
    const out: Record<string, Question> = {
      main: choice("Which of `files` is the main download most players should pick?", options),
    };
    list.forEach((_, at) => {
      out[`label${at}`] = choice(`What is \`files[${at}]\` compared with the other files of this mod?`, FILE_LABELS);
    });
    return out;
  },
  read: (answer, input) => {
    const list = input.profile.files.slice(0, MAX_FILES);
    const main = picked(answer("main"));
    if (main === null) return null;
    const mainFile = list[Number(main.choice.replace("file", ""))];
    return {
      main: mainFile === undefined ? null : { id: mainFile.id, p: main.p },
      labels: list.map((file, at) => {
        const label = picked(answer(`label${at}`));
        return { id: file.id, label: label?.choice ?? null, p: label?.p ?? 0 };
      }),
    };
  },
};

export const UPDATE_KINDS: Record<string, string> = {
  game_fix: "Fixes or updates the mod after a game update broke it.",
  content: "Adds new content: models, variants, features.",
  bugfix: "Fixes bugs in the mod itself.",
  cosmetic: "Small visual or cosmetic tweaks.",
};

const updates: Judgment = {
  name: "updates",
  version: 1,
  needs: ["updates"],
  applies: (input) => input.updates.length > 0,
  ask: (input) => {
    const out: Record<string, Question> = {};
    input.updates.slice(0, MAX_UPDATES).forEach((_, at) => {
      out[`kind${at}`] = choice(`What does the mod update \`updates[${at}]\` mainly do?`, UPDATE_KINDS);
      out[`compat${at}`] = noul(`Does \`updates[${at}]\` say the mod works again with, or was updated for, the latest game patch?`);
    });
    return out;
  },
  read: (answer, input) => {
    const entries = input.updates.slice(0, MAX_UPDATES).map((update, at) => ({
      at: update.added,
      kind: picked(answer(`kind${at}`)),
      compat: yes(answer(`compat${at}`)),
    }));
    return entries.some((entry) => entry.kind === null || entry.compat === null) ? null : { entries };
  },
};

/** Distinct commenters in the window. Counted in code; names are never stored or sent. */
export function commenters(comments: readonly ModComment[]): number {
  return new Set(comments.map((comment) => comment.poster)).size;
}

export const MIN_COMMENTERS = 3;

const health: Judgment = {
  name: "health",
  version: 1,
  needs: ["comments"],
  applies: (input) => commenters(input.comments) >= MIN_COMMENTERS,
  ask: () => ({
    broken: noul("Do several different commenters in `comments` report that the mod is broken, crashing or not working since a game update?"),
    works: noul("Do commenters in `comments` report that the mod works after the latest game patch?"),
    conflict: noul("Does a commenter in `comments` report a conflict with another named mod?"),
  }),
  read: (answer, input) => {
    const broken = yes(answer("broken"));
    const works = yes(answer("works"));
    const conflict = yes(answer("conflict"));
    if (broken === null || works === null || conflict === null) return null;
    const times = input.comments.map((comment) => comment.added);
    return {
      broken,
      works,
      conflict,
      comments: input.comments.length,
      commenters: commenters(input.comments),
      from: Math.min(...times),
      to: Math.max(...times),
    };
  },
};

const dupes: Judgment = {
  name: "dupes",
  version: 1,
  needs: ["text", "others"],
  applies: (input) => input.others.length > 0,
  ask: (input) =>
    Object.fromEntries(
      input.others.map((_, at) => [
        `same${at}`,
        noul(`Is \`others[${at}]\` the same mod as this one uploaded again (by the same or another author), rather than a different mod?`),
      ]),
    ),
  read: (answer, input) => {
    const pairs = input.others.map((other, at) => ({ id: other.id, p: yes(answer(`same${at}`)) }));
    return pairs.some((pair) => pair.p === null) ? null : { pairs };
  },
};

/**
 * The verifier of generated search terms (the docs' cascade: a generator proposes, Jev checks
 * each term against the public text and the pictures). Other names of a reference (abbreviations,
 * misspellings) share its answer and are not asked.
 */
const terms: Judgment = {
  name: "terms",
  version: 3,
  needs: ["text", "screenshots"],
  applies: (input) => input.expansion !== null && input.expansion.terms.length > 0,
  ask: (input) => {
    const out: Record<string, Question> = {};
    const pictures = input.shots.length > 0;
    (input.expansion?.terms ?? []).forEach((term, at) => {
      if (term.cls === "alias") return;
      // Generated terms come from untrusted page content: they sit in the state as data and the
      // question points at them by path.
      const quoted = `\`terms[${at}]\``;
      if (term.facet === "visual") {
        out[`t${at}`] = noul(pictures ? `Do \`screenshots\` or the description show ${quoted}?` : `Does the description say the mod shows ${quoted}?`);
      } else if (term.facet === "theme") {
        out[`t${at}`] = noul(`Does ${quoted} describe this mod's theme or mood?`);
      } else if (term.facet === "character") {
        out[`t${at}`] = noul(`Does this mod depict the character ${quoted}, judging by its title, description and screenshots?`);
      } else {
        out[`t${at}`] = noul(`Is this mod based on, or does it clearly reference, ${quoted}, judging by its title, description and screenshots?`);
      }
    });
    return out;
  },
  read: (answer, input) => {
    const expansion = input.expansion;
    if (expansion === null) return null;
    const own = new Map<string, number>();
    expansion.terms.forEach((term, at) => {
      if (term.cls !== "alias") own.set(term.term, yes(answer(`t${at}`)) ?? 0);
    });
    const kept = expansion.terms.map((term) => ({
      term: term.term,
      cls: term.cls,
      facet: term.facet,
      src: term.src,
      ...(term.of === undefined ? {} : { of: term.of }),
      p: term.cls === "alias" ? (own.get(term.of ?? "") ?? 0) : (own.get(term.term) ?? 0),
    }));
    return { model: expansion.model, terms: kept };
  },
};

/** In request order. `terms` last: it depends on the generator's output. */
export const JUDGMENTS: readonly Judgment[] = [kind, style, hero, nsfw, cover, facts, files, updates, health, dupes, terms];

/** Separator in question ids: `<judgment>__<question>`. */
export const SEP = "__";
