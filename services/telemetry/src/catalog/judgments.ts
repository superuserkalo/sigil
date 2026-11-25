/**
 * The two Jev judgments still under evaluation, each behind its own flag and off by default
 * (ADR 0018). Each has a name and a version: bumping one re-runs only that judgment. Answers are
 * stored with their probabilities; thresholds live in `publish.ts`.
 *
 * - `terms` (`CATALOG_EXPANSION = "1"`): one Noul per search term the generator proposed, so an
 *   invented reference never reaches the index.
 * - `files` (`CATALOG_FILE_LABELS = "1"`): for a mod with several files, which is the main one and
 *   a label per file. Only ever a pre-selection hint for the app's file picker.
 *
 * GameBanana text and generated terms are untrusted: they sit in the request state as data and
 * the questions point at them by path, never quote them.
 */

import type { Expansion } from "./expand";
import type { ListedMod, ModText } from "./gamebanana";
import type { Answer, Question } from "./typesafe";

export type ModInput = {
  readonly mod: ListedMod;
  readonly text: ModText;
  readonly expansion: Expansion | null;
};

export type Read = (id: string) => Answer | undefined;

export type Stored = Record<string, unknown>;

export type Judgment = {
  readonly name: "terms" | "files";
  readonly version: number;
  /**
   * A stored row made with an older version that is still good enough, so a version bump re-runs
   * only the rows it affects. Absent: only the current version is current.
   */
  readonly accepts?: (version: number, data: Stored) => boolean;
  /** False when there is nothing to judge; nothing is asked then. */
  readonly applies: (input: ModInput) => boolean;
  readonly ask: (input: ModInput) => Record<string, Question>;
  readonly read: (answer: Read, input: ModInput) => Stored | null;
};

export const MAX_FILES = 8;

function noul(instructions: string): Question {
  return { type: "noul", instructions };
}

function choice(instructions: string, criteria: Record<string, string>): Question {
  return { type: "choice", instructions, criteria };
}

export function yes(answer: Answer | undefined): number | null {
  return answer?.type === "noul" && Number.isFinite(answer.noul) ? round(answer.noul) : null;
}

export function picked(answer: Answer | undefined): { choice: string; p: number } | null {
  if (answer?.type !== "choice") return null;
  return { choice: answer.choice, p: round(answer.probabilities[answer.choice] ?? 0) };
}

export function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}

/** Generated terms shorter than this are never asked about and never published ("hd", "cs"). */
export const MIN_TERM_CHARS = 3;

function hasAlias(data: Stored): boolean {
  return Array.isArray(data.terms) && (data.terms as { cls?: unknown }[]).some((term) => term.cls === "alias");
}

/**
 * The generator proposes, Jev checks (the docs' cascade). Every reference, visual and theme term
 * gets one Noul. Every alias gets its own Noul ("is it a name players commonly type for its
 * reference"), asked in the same request, and is kept only when it and its reference both pass.
 *
 * Version 5 added the alias check. A version 4 row with no alias is unchanged by it and stays
 * current; only rows that hold an alias are asked again.
 */
const terms: Judgment = {
  name: "terms",
  version: 5,
  accepts: (version, data) => version === 4 && !hasAlias(data),
  applies: (input) => input.expansion !== null && input.expansion.terms.length > 0,
  ask: (input) => {
    const out: Record<string, Question> = {};
    const list = input.expansion?.terms ?? [];
    list.forEach((term, at) => {
      if (term.term.length < MIN_TERM_CHARS) return;
      const ref = `\`terms[${at}]\``;
      if (term.cls === "alias") {
        const parent = list.findIndex((other) => other.cls !== "alias" && other.term === term.of);
        if (parent < 0 || (list[parent]?.term.length ?? 0) < MIN_TERM_CHARS) return;
        out[`a${at}`] = noul(`Is ${ref} a name players would commonly type for \`terms[${parent}]\`?`);
      } else if (term.facet === "visual") out[`t${at}`] = noul(`Does the title or description say the mod shows ${ref}?`);
      else if (term.facet === "theme") out[`t${at}`] = noul(`Does ${ref} describe this mod's theme or mood, judging by its title and description?`);
      else if (term.facet === "character") out[`t${at}`] = noul(`Does this mod depict the character ${ref}, judging by its title and description?`);
      else out[`t${at}`] = noul(`Is this mod based on, or does it clearly reference, ${ref}, judging by its title and description?`);
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
    return {
      model: expansion.model,
      terms: expansion.terms.map((term, at) => ({
        term: term.term,
        cls: term.cls,
        facet: term.facet,
        ...(term.of === undefined ? {} : { of: term.of }),
        // An alias keeps its reference's answer in `p` and its own check in `pa`.
        p: term.cls === "alias" ? (own.get(term.of ?? "") ?? 0) : (own.get(term.term) ?? 0),
        ...(term.cls === "alias" ? { pa: yes(answer(`a${at}`)) ?? 0 } : {}),
      })),
    };
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
  applies: (input) => input.text.files.length >= 2,
  ask: (input) => {
    const list = input.text.files.slice(0, MAX_FILES);
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
    const list = input.text.files.slice(0, MAX_FILES);
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

export const JUDGMENTS: readonly Judgment[] = [terms, files];

/** Separator in question ids: `<judgment>__<question>`. */
export const SEP = "__";
