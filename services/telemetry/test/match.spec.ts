import { describe, expect, it } from "vitest";
import { aliasTable } from "../src/catalog/aliases";
import { search, tokenMatch, withinOneEdit } from "../src/catalog/match";
import { buildIndex, type JudgmentRows } from "../src/catalog/publish";

const HEROES = ["Venator", "McGinnis", "Haze", "Lady Geist", "Mirage", "Abrams"];

type Fixture = { id: number; title: string; sub?: string; terms?: [string, "reference" | "alias" | "visual" | "theme"][]; kind?: string; styles?: string[] };

const MODS: Fixture[] = [
  { id: 1, title: "CS 1.6 GIGN Venator skin", sub: "Venator", terms: [["counter strike", "reference"], ["cs", "alias"], ["gign", "alias"], ["counter terrorist", "alias"], ["gas mask", "visual"], ["soldier", "visual"], ["military", "theme"]], kind: "model_swap" },
  { id: 2, title: "Counter Strike", sub: "HUD" },
  { id: 3, title: "Strike Team Haze", sub: "Haze", terms: [["tactical", "theme"]] },
  { id: 4, title: "Gojo for Mirage", sub: "Mirage", terms: [["jujutsu kaisen", "reference"], ["jjk", "alias"], ["gojo satoru", "reference"], ["blindfold", "visual"]], styles: ["anime"] },
  { id: 5, title: "Neo trenchcoat Lady Geist", sub: "Lady Geist", terms: [["the matrix", "reference"], ["matrix", "alias"], ["trench coat", "visual"], ["sunglasses", "visual"]] },
  { id: 6, title: "Big Floppa McGinnis", sub: "McGinnis", terms: [["big floppa", "reference"], ["floppa", "alias"], ["caracal", "visual"], ["cat", "visual"]], styles: ["meme"] },
  { id: 7, title: "Plague Doctor Abrams", sub: "Abrams", terms: [["gas mask", "visual"], ["plague doctor", "visual"], ["horror", "theme"]], styles: ["horror"] },
  { id: 8, title: "Minimal HUD", sub: "HUD", kind: "hud", styles: ["minimal"] },
  { id: 9, title: "Venator recolor black", sub: "Venator", kind: "recolour", styles: ["dark"] },
];

function index() {
  const rows = new Map<number, JudgmentRows>();
  for (const mod of MODS) {
    rows.set(mod.id, {
      ...(mod.terms === undefined ? {} : { terms: { terms: mod.terms.map(([term, cls]) => ({ term, cls, p: 0.95 })) } }),
      ...(mod.kind === undefined ? {} : { kind: { kind: mod.kind, p: 0.95, fits: 0.95 } }),
      ...(mod.styles === undefined ? {} : { style: Object.fromEntries(mod.styles.map((style) => [style, 0.95])) }),
    });
  }
  return buildIndex(
    MODS.map((mod) => ({ id: mod.id, title: mod.title, subcategory: mod.sub ?? "", enrichedAt: 1 })),
    rows,
    HEROES,
    1,
  );
}

const INDEX = index();
const top = (query: string, n = 3) => search(INDEX, query, n).map((hit) => hit.id);

describe("local matcher", () => {
  it("finds the owner's example by franchise words the title never uses", () => {
    expect(top("counter strike", 5)).toContain(1);
    expect(top("counterstrike")).toContain(1);
    expect(top("cs go venator")[0]).toBe(1);
  });

  it("puts an exact title match first, then title tokens, then references", () => {
    expect(top("counter strike")[0]).toBe(2);
    expect(new Set(top("strike", 2))).toEqual(new Set([2, 3]));
    expect(top("strike", 3)[2]).toBe(1);
  });

  it("finds references in other directions", () => {
    expect(top("jujutsu kaisen")[0]).toBe(4);
    expect(top("jjk")[0]).toBe(4);
    expect(top("matrix")[0]).toBe(5);
    expect(top("floppa")[0]).toBe(6);
  });

  it("finds visual queries", () => {
    expect(new Set(top("gas mask", 2))).toEqual(new Set([1, 7]));
    expect(top("sunglasses")[0]).toBe(5);
  });

  it("tolerates one typo and a prefix", () => {
    expect(top("conter strike", 5)).toContain(1);
    expect(top("jujutsu kaisne")[0]).toBe(4);
    expect(top("floppa mcgin")[0]).toBe(6);
  });

  it("uses hero nicknames, tag synonyms and a few German words", () => {
    expect(top("mcg")[0]).toBe(6);
    expect(top("geist")[0]).toBe(5);
    expect(top("recolor venator")[0]).toBe(9);
    expect(top("gasmaske", 2)).toContain(7);
    expect(top("katze")[0]).toBe(6);
  });

  it("requires every word, and falls back to any word when nothing has all", () => {
    expect(top("gas mask abrams")).toEqual([7]);
    expect(top("gas mask unicorn").length).toBeGreaterThan(0);
  });

  it("edit distance helpers", () => {
    expect(withinOneEdit("strike", "strik")).toBe(true);
    expect(withinOneEdit("kaisen", "kaisne")).toBe(true);
    expect(withinOneEdit("venator", "vindicta")).toBe(false);
    expect(tokenMatch("ven", "venator")).toBe(0.8);
    expect(tokenMatch("ve", "venator")).toBe(0);
  });

  it("ships the alias table", () => {
    expect(aliasTable(HEROES).mcg).toBe("mcginnis");
    expect(INDEX.aliases.geist).toBe("lady geist");
  });
});
