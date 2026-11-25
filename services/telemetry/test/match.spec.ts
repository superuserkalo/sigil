import { describe, expect, it } from "vitest";
import { normalize, search, tokenMatch, withinOneEdit } from "../src/catalog/match";
import { buildIndex, type JudgmentRows } from "../src/catalog/publish";

type Fixture = { id: number; title: string; cat?: string; sub?: string; author?: string; description?: string; terms?: string[] };

const MODS: Fixture[] = [
  { id: 1, title: "GIGN Venator", sub: "Venator", description: "Counter-Strike 1.6 GIGN model with a gas mask and goggles." },
  { id: 2, title: "Counter Strike", cat: "HUD", sub: "Character HUD", description: "Kill feed like an old shooter." },
  { id: 3, title: "Strike Team Haze", sub: "Haze", description: "Tactical vest and helmet." },
  { id: 4, title: "Gojo for Mirage", sub: "Mirage", description: "Satoru Gojo from Jujutsu Kaisen, blindfold included." },
  { id: 5, title: "Neo trenchcoat Lady Geist", sub: "Lady Geist", description: "Long black coat and sunglasses.", author: "matrixfan" },
  { id: 6, title: "Big Floppa McGinnis", sub: "McGinnis", description: "The caracal meme cat." },
  { id: 7, title: "Plague Doctor Abrams", sub: "Abrams", description: "Beak gas mask, horror outfit." },
  { id: 8, title: "Minimal HUD", cat: "HUD", sub: "Gameplay", description: "Clean minimal interface." },
  { id: 9, title: "Venator recolor black", sub: "Venator", description: "All black recolour." },
  // The owner's case: the page never says "Counter-Strike"; only expansion knows CS is that game.
  { id: 10, title: "CS 1.6 Vindicta skin", sub: "Vindicta", description: "Sniper from the classic.", terms: ["counter strike", "cs", "counter strike 1 6"] },
  { id: 11, title: "Vindicta but pink", sub: "Vindicta", description: "Pink." },
];

function index(terms: boolean) {
  const rows = new Map<number, JudgmentRows>();
  for (const mod of MODS) {
    if (mod.terms !== undefined) rows.set(mod.id, { terms: { terms: mod.terms.map((term) => ({ term, cls: "reference", p: 0.95 })) } });
  }
  return buildIndex(
    MODS.map((mod) => ({
      id: mod.id,
      title: mod.title,
      category: mod.cat ?? "Skins",
      subcategory: mod.sub ?? "",
      author: mod.author ?? "someone",
      description: mod.description ?? "",
      enrichedAt: 1,
    })),
    rows,
    1,
    terms,
  );
}

const PLAIN = index(false);
const EXPANDED = index(true);
const top = (query: string, n = 3, built = PLAIN) => search(built, query, n).map((hit) => hit.id);

describe("local matcher over public text", () => {
  it("puts an exact title match first, then title words, then description words", () => {
    expect(top("counter strike")[0]).toBe(2);
    expect(top("counter strike", 5)).toContain(1);
    expect(new Set(top("strike", 2))).toEqual(new Set([2, 3]));
  });

  it("finds description, author and hero words", () => {
    expect(top("jujutsu kaisen")[0]).toBe(4);
    expect(top("sunglasses")[0]).toBe(5);
    expect(top("matrixfan")[0]).toBe(5);
    expect(new Set(top("gas mask", 2))).toEqual(new Set([1, 7]));
    expect(top("vindicta pink")[0]).toBe(11);
  });

  it("tolerates one typo and a prefix", () => {
    expect(top("jujutsu kaisne")[0]).toBe(4);
    expect(top("conter strike", 5)).toContain(1);
    expect(top("floppa mcgin")[0]).toBe(6);
    expect(top("counterstrike", 5)).toContain(2);
  });

  it("covers short forms and spelling variants by prefix and one edit, with no alias table", () => {
    expect(top("mcg")[0]).toBe(6);
    expect(top("geist")[0]).toBe(5);
    expect(top("recolour venator")[0]).toBe(9);
    expect(top("vindict pink")[0]).toBe(11);
  });

  it("requires every word, and falls back to any word when nothing has all", () => {
    expect(top("gas mask abrams")).toEqual([7]);
    expect(top("gas mask unicorn").length).toBeGreaterThan(0);
  });

  it("reports the owner's case both ways: counter strike finds the CS 1.6 Vindicta skin only with expansion", () => {
    expect(top("counter strike", 5)).not.toContain(10);
    expect(top("counter strike", 5, EXPANDED)).toContain(10);
    expect(top("counter strike vindicta", 3, EXPANDED)[0]).toBe(10);
    // "cs vindicta" works on the title alone.
    expect(top("cs vindicta")[0]).toBe(10);
  });

  it("edit distance helpers", () => {
    expect(withinOneEdit("strike", "strik")).toBe(true);
    expect(withinOneEdit("kaisen", "kaisne")).toBe(true);
    expect(withinOneEdit("venator", "vindicta")).toBe(false);
    expect(tokenMatch("ven", "venator")).toBe(0.8);
    expect(tokenMatch("ve", "venator")).toBe(0);
  });

  it("ships no alias table and maps no word to another", () => {
    expect(Object.keys(PLAIN).sort()).toEqual(["built", "mods", "strings", "v"]);
    // Without a hand-made list a German word finds only mods that use it.
    expect(top("katze")).toEqual([]);
  });

  it("normalises case, accents and punctuation", () => {
    expect(normalize("  Mo & Krill: Café-Noir!! ")).toBe("mo and krill cafe noir");
  });
});
