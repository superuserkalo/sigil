/**
 * Official Deadlock patch notes from the public Steam news feed (the same feed the app reads),
 * split into lines in code. The hero a line is about is an exact lookup in code (lines start with
 * "Hero:"); Jev classifies each line's system and whether it changes how something looks or
 * sounds, so the app can say which installed mods a patch probably touches.
 */

import { htmlToText } from "./gamebanana";
import { Budget, BudgetSpent, RemoteError, type Net } from "./net";
import type { Question, SystemOneBody } from "./typesafe";
import { JEV_MODEL } from "./typesafe";
import { round, yes, picked, type Read } from "./judgments";

export const STEAM_NEWS_URL = "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid=1422450&count=8&maxlength=0&feeds=steam_community_announcements";
export const PATCH_VERSION = 1;
/** Lines per Jev request: two short questions each keeps a request far under the context limits. */
export const PATCH_CHUNK = 80;
export const MAX_PATCH_LINES = 400;

export const SYSTEMS: Record<string, string> = {
  model: "A hero's model, skin, outfit or animation.",
  abilities: "A hero ability or an item: its numbers, behaviour or effects.",
  ui: "The interface: HUD, menus, icons, minimap or text.",
  sounds: "Sounds, voice lines or music.",
  map: "The map, lanes, objectives, environment or lighting.",
  other: "Matchmaking, economy, bug fixes or anything else.",
};

export type PatchItem = { readonly id: string; readonly title: string; readonly date: number; readonly lines: readonly string[] };

export async function fetchPatches(net: Net, budget: Budget): Promise<PatchItem[]> {
  if (!budget.take()) throw new BudgetSpent("subrequest budget spent");
  const response = await net.fetch(STEAM_NEWS_URL, { headers: { "user-agent": "Sigil (patch tags)", accept: "application/json" } });
  if (!response.ok) {
    await response.body?.cancel();
    throw new RemoteError("steam", response.status);
  }
  const body = (await response.json()) as { appnews?: { newsitems?: unknown[] } };
  const items = Array.isArray(body.appnews?.newsitems) ? body.appnews.newsitems : [];
  return items.flatMap((raw) => {
    if (typeof raw !== "object" || raw === null) return [];
    const item = raw as { gid?: unknown; title?: unknown; date?: unknown; contents?: unknown; tags?: unknown };
    const title = typeof item.title === "string" ? item.title : "";
    const tags = Array.isArray(item.tags) ? item.tags.map(String) : [];
    const isPatch = tags.some((tag) => tag.toLowerCase().includes("patch")) || /patch|update|hotfix/i.test(title);
    if (!isPatch || typeof item.gid !== "string") return [];
    return [{ id: item.gid, title, date: typeof item.date === "number" ? item.date : 0, lines: patchLines(String(item.contents ?? "")) }];
  });
}

/** BBCode and HTML dropped, one line per change, bullets removed. */
export function patchLines(contents: string): string[] {
  const text = htmlToText(contents.replace(/\[\/?(p|list|\*|li)\]/gi, "\n"));
  return text
    .split("\n")
    .map((line) => line.replace(/^[-*•\s]+/, "").trim())
    .filter((line) => line.length >= 6 && line.length <= 400)
    .slice(0, MAX_PATCH_LINES);
}

/** The hero a line names first ("Apollo: Riposte ..."), else any hero named in it. Code only. */
export function lineHero(line: string, heroes: readonly string[]): string | null {
  const head = line.split(":")[0]?.trim().toLowerCase() ?? "";
  const direct = heroes.find((hero) => hero.toLowerCase() === head);
  if (direct !== undefined) return direct;
  const lower = ` ${line.toLowerCase()} `;
  return heroes.find((hero) => new RegExp(`[^a-z]${hero.toLowerCase().replace(/[^a-z ]/g, ".")}[^a-z]`).test(lower)) ?? null;
}

export function patchRequest(item: PatchItem, from: number): SystemOneBody {
  const lines = item.lines.slice(from, from + PATCH_CHUNK);
  const questions: Record<string, Question> = {};
  lines.forEach((_, at) => {
    questions[`system${at}`] = { type: "choice", instructions: `What part of the game does \`lines[${at}]\` change?`, criteria: SYSTEMS };
    questions[`visual${at}`] = {
      type: "noul",
      instructions: `Does \`lines[${at}]\` change how something looks or sounds in the game (models, textures, effects, interface, audio), rather than only numbers or rules?`,
    };
  });
  // Public Valve patch notes only.
  return { model: JEV_MODEL, state: { title: item.title, lines }, questions };
}

export type PatchLine = { readonly hero: string | null; readonly system: string | null; readonly systemP: number; readonly visual: number };

export function readPatch(answer: Read, item: PatchItem, from: number, heroes: readonly string[]): PatchLine[] {
  return item.lines.slice(from, from + PATCH_CHUNK).map((line, at) => {
    const system = picked(answer(`system${at}`));
    return { hero: lineHero(line, heroes), system: system?.choice ?? null, systemP: system?.p ?? 0, visual: yes(answer(`visual${at}`)) ?? 0 };
  });
}

export const PATCH_T = { systemP: 0.6, visual: 0.7 } as const;

/** The public view of one patch: which heroes and systems it changes visibly. */
export function publishPatch(id: string, title: string, date: number, lines: readonly PatchLine[]): Record<string, unknown> {
  const heroes = new Map<string, { lines: number; visual: number }>();
  const systems: Record<string, number> = {};
  const out = lines.map((line, at) => {
    const system = line.systemP >= PATCH_T.systemP ? line.system : null;
    const visual = line.visual >= PATCH_T.visual;
    if (line.hero !== null) {
      const entry = heroes.get(line.hero) ?? { lines: 0, visual: 0 };
      entry.lines += 1;
      if (visual) entry.visual += 1;
      heroes.set(line.hero, entry);
    }
    if (system !== null) systems[system] = (systems[system] ?? 0) + 1;
    return { n: at, h: line.hero, s: system, vis: visual ? 1 : 0, p: round(line.visual) };
  });
  return {
    id,
    title,
    date,
    heroes: [...heroes.entries()].map(([hero, counts]) => ({ hero, ...counts })),
    systems,
    lines: out,
  };
}
