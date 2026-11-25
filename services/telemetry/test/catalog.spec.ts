import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { scheduledJob } from "../src/cron";
import { runEnrichment, buildAndStoreIndex, loadIndex, RUN_SUBREQUESTS } from "../src/catalog/enrich";
import { decodeEntities, htmlToText, modLinks } from "../src/catalog/gamebanana";
import { learnRequest } from "../src/catalog/learn";
import { LEARN_STATE_KEYS } from "../src/catalog/typesafe";
import { parseExpansion, type AiRunner } from "../src/catalog/expand";
import { imageSize } from "../src/catalog/images";
import { JUDGMENTS } from "../src/catalog/judgments";
import { lineHero, patchLines, publishPatch } from "../src/catalog/patches";
import {
  coverPicks,
  devState,
  dupeOf,
  healthChip,
  keptTerms,
  nsfwHint,
  publishedHero,
  publishedKind,
  publishedStyles,
  requiresMod,
  T,
} from "../src/catalog/publish";
import { assertPublicOnly, BODY_KEYS, STATE_KEYS, TYPESAFE_URL, type SystemOneBody } from "../src/catalog/typesafe";
import type { Net } from "../src/catalog/net";
import { call, clearTables, readJson } from "./helpers";

const KEY = "test-only-typesafe-key-0123456789";
const NOW = Date.UTC(2026, 10, 25, 12);
const SENTINEL = "PLAYER-SECRET-SEARCH-TEXT";
const COMMENTER = "SecretCommenterName";

type Calls = { gb: string[]; typesafe: SystemOneBody[]; headers: Headers[]; steam: number };

/** A 600x340 PNG header: enough for the size reader; the fake Jev never decodes it. */
function pngBytes(width = 600, height = 340): Uint8Array {
  const bytes = new Uint8Array(64);
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52]);
  const view = new DataView(bytes.buffer);
  view.setUint32(16, width);
  view.setUint32(20, height);
  return bytes;
}

type ModFixture = { id: number; updated: number; title?: string; sub?: string; posts?: number };

function listBody(mods: ModFixture[]): unknown {
  return {
    _aRecords: mods.map((mod) => ({
      _idRow: mod.id,
      _sModelName: "Mod",
      _sName: mod.title ?? `Mod ${mod.id}`,
      _tsDateAdded: mod.updated - 100,
      _tsDateModified: mod.updated,
      _aRootCategory: { _sName: "Skins" },
      _aSubCategory: { _sName: mod.sub ?? "Venator" },
      _aSubmitter: { _idRow: 77, _sName: "author" },
      _bHasContentRatings: false,
      _nPostCount: mod.posts ?? 3,
    })),
  };
}

const PROFILE = {
  _sDescription: "CS 1.6 GIGN as Venator",
  _sText: 'Counter-Strike 1.6 GIGN model. Needs <a href="https://gamebanana.com/mods/123">Base pack</a>.',
  _aFiles: [
    { _idRow: 1, _sFile: "gign_main.zip", _nFilesize: 10485760, _sDescription: "main" },
    { _idRow: 2, _sFile: "gign_blue.zip", _nFilesize: 10485760, _sDescription: "blue variant" },
  ],
  _aPreviewMedia: {
    _aImages: [
      { _sType: "screenshot", _sBaseUrl: "https://images.gamebanana.com/img/ss/mods", _sFile: "a.png", _sFile530: "530-90_a.png", _wFile530: 530, _hFile530: 300 },
    ],
  },
  _nUpdatesCount: 1,
};

const POSTS = {
  _aRecords: [1, 2, 3].map((poster) => ({
    _tsDateAdded: NOW / 1000 - 86400 * poster,
    _sText: `<p>broken since the update ${poster}</p>`,
    _aPoster: { _idRow: 9000 + poster, _sName: COMMENTER },
  })),
};

const UPDATES = { _aRecords: [{ _sName: "Fixed for patch", _sText: "works again", _aChangeLog: [{ cat: "Bugfix", text: "fixed" }], _tsDateAdded: NOW / 1000 - 86400 }] };

const HEROES_ROOT = [{ _idRow: 33295, _sName: "Skins" }];
const HEROES = [{ _idRow: 1, _sName: "Venator" }, { _idRow: 2, _sName: "Abrams" }];

/** Answers every question the request asked: Nouls at `noul`, Choices on their first option. */
function jevAnswer(body: SystemOneBody, noul = 0.9): unknown {
  const answers: Record<string, unknown> = {};
  for (const [id, question] of Object.entries(body.questions)) {
    if (question.type === "noul") answers[id] = { type: "noul", noul };
    if (question.type === "choice") {
      const first = Object.keys(question.criteria)[0] ?? "x";
      answers[id] = { type: "choice", choice: first, confidence: 0.85, probabilities: { [first]: 0.9 } };
    }
    if (question.type === "score") answers[id] = { type: "score", score: question.criteria.length - 1, confidence: 0.9, probabilities: {} };
    if (question.type === "box") answers[id] = { type: "box", box: { left: 0.3, top: 0.1, right: 0.7, bottom: 0.9 }, confidence: 0.8 };
  }
  return { model: "jev-1.14.0", answers, usage: { input_tokens: 2000, output_tokens: 10 } };
}

function fakeNet(
  mods: ModFixture[],
  calls: Calls,
  options: { gbStatus?: number; typesafeStatus?: number; noul?: number; profiles?: Record<number, unknown> } = {},
): Net {
  return {
    sleep: async () => {},
    fetch: async (url, init) => {
      if (url === TYPESAFE_URL) {
        const body = JSON.parse(String(init?.body)) as SystemOneBody;
        calls.typesafe.push(body);
        calls.headers.push(new Headers(init?.headers));
        if (options.typesafeStatus !== undefined) return new Response("{}", { status: options.typesafeStatus });
        return Response.json(jevAnswer(body, options.noul));
      }
      if (url.startsWith("https://api.steampowered.com/")) {
        calls.steam += 1;
        return Response.json({
          appnews: {
            newsitems: [{ gid: "555", title: "Gameplay Update", date: 1795000000, tags: ["patchnotes"], contents: "[p]- Venator: model updated[/p][p]- Mirage Tornado cooldown up[/p]" }],
          },
        });
      }
      calls.gb.push(url);
      if (options.gbStatus !== undefined) return new Response("no", { status: options.gbStatus });
      if (url.startsWith("https://images.gamebanana.com/")) return new Response(pngBytes());
      if (url.includes("_idGameRow=")) return Response.json(HEROES_ROOT);
      if (url.includes("_idCategoryRow=")) return Response.json(HEROES);
      if (url.includes("/Mod/Index")) return Response.json(listBody(url.includes("_nPage=1&") ? mods : []));
      if (url.endsWith("/ProfilePage")) {
        const id = Number(/Mod\/(\d+)\//.exec(url)?.[1]);
        const custom = options.profiles?.[id];
        if (custom === 404) return new Response("gone", { status: 404 });
        return Response.json(custom ?? PROFILE);
      }
      if (url.includes("/Posts")) return Response.json(POSTS);
      if (url.includes("/Updates")) return Response.json(UPDATES);
      return new Response("unknown", { status: 404 });
    },
  };
}

const fakeAi = (calls: { ai: number; input?: string }): AiRunner => ({
  run: async (_model, input) => {
    calls.ai += 1;
    calls.input = JSON.stringify(input);
    return {
      response: {
        franchises: [{ name: "Counter-Strike", aliases: ["cs", "cs 1.6", "counter strike", "gign"] }],
        characters: [],
        visual: ["gas mask", "soldier"],
        themes: ["military"],
      },
      usage: { prompt_tokens: 1500, completion_tokens: 120 },
    };
  },
});

function newCalls(): Calls {
  return { gb: [], typesafe: [], headers: [], steam: 0 };
}

function testEnv(extra: Record<string, unknown> = {}): Env {
  return { ...env, TYPESAFE_API_KEY: KEY, AI: undefined, ...extra } as unknown as Env;
}

async function clearCatalog(): Promise<void> {
  await env.DB.batch(["catalog_mods", "mod_tags", "patch_tags", "enrich_state"].map((table) => env.DB.prepare(`DELETE FROM ${table}`)));
}

beforeEach(async () => {
  await clearTables();
  await clearCatalog();
});

describe("what is sent to TypeSafe", () => {
  it("holds only allowlisted public fields and nothing from the product tables", async () => {
    await env.DB.prepare(
      `INSERT INTO product_events (created_at, install_hash, type, version, os, os_build, game_build, payload) VALUES (?1, ?2, 'search', '0.1.0', 'linux', '', '', ?3)`,
    )
      .bind(NOW, "install-hash-should-never-leave", JSON.stringify({ query: SENTINEL }))
      .run();
    const calls = newCalls();
    const ai = { ai: 0 } as { ai: number; input?: string };
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));

    expect(calls.typesafe.length).toBeGreaterThan(0);
    for (const body of calls.typesafe) {
      expect(() => assertPublicOnly(body)).not.toThrow();
      for (const key of Object.keys(body)) expect(BODY_KEYS.has(key)).toBe(true);
      const text = JSON.stringify(body);
      expect(text).not.toContain(SENTINEL);
      expect(text).not.toContain("install-hash-should-never-leave");
      expect(text).not.toContain(COMMENTER);
      expect(text).not.toContain("9001");
      const walk = (value: unknown): void => {
        if (Array.isArray(value)) value.forEach(walk);
        else if (typeof value === "object" && value !== null) {
          for (const [key, inner] of Object.entries(value)) {
            expect(STATE_KEYS.has(key), key).toBe(true);
            walk(inner);
          }
        }
      };
      walk(body.state);
    }
    // The generator saw only the public title, category and description.
    expect(ai.ai).toBe(2);
    expect(ai.input).not.toContain(SENTINEL);
    expect(ai.input).not.toContain(COMMENTER);
    // The key travels only as the bearer header; images switch on the documented header.
    expect(calls.headers[0]?.get("authorization")).toBe(`Bearer ${KEY}`);
    expect(calls.headers[0]?.get("typesafe-danger-zone-images")).toBe("true");
  });

  it("refuses a body with a field outside the allowlist", () => {
    const body = { model: "jev-1.14.0", state: { title: "x", installHash: "abc" }, questions: {} } as unknown as SystemOneBody;
    expect(() => assertPublicOnly(body)).toThrow(/installHash/);
    const extra = { model: "jev-1.14.0", state: {}, questions: {}, user: "x" } as unknown as SystemOneBody;
    expect(() => assertPublicOnly(extra)).toThrow(/user/);
  });

  it("pins the allowlists", () => {
    expect([...BODY_KEYS].sort()).toEqual(["assets", "model", "questions", "state"]);
    expect([...STATE_KEYS].sort()).toEqual(
      ["age_days", "category", "changes", "commenter", "comments", "description", "files", "label", "lines", "links", "name", "note", "others", "screenshots", "size_mb", "subcategory", "terms", "text", "title", "updates"].sort(),
    );
  });
});

describe("fail closed", () => {
  it("does nothing at all without the key, and the route still answers", async () => {
    const calls = newCalls();
    const report = await runEnrichment({ ...env, TYPESAFE_API_KEY: undefined } as unknown as Env, NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(report.skipped).toBe("no_typesafe_key");
    expect(calls.gb).toEqual([]);
    expect(calls.typesafe).toEqual([]);
    const response = await call("/v1/catalog/tags", { method: "GET" });
    expect(response.status).toBe(200);
    expect((await readJson<{ mods: unknown[] }>(response)).mods).toEqual([]);
  });

  it("runs every other judgment without the AI binding", async () => {
    const calls = newCalls();
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    const { results } = await env.DB.prepare("SELECT judgment FROM mod_tags WHERE mod_id = 1 ORDER BY judgment").all<{ judgment: string }>();
    const names = results.map((row) => row.judgment);
    expect(names).toContain("kind");
    expect(names).toContain("cover");
    expect(names).not.toContain("terms");
  });

  it("keeps going when the generator fails", async () => {
    const calls = newCalls();
    const broken: AiRunner = { run: async () => { throw new Error("model down"); } };
    await runEnrichment(testEnv({ AI: broken }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(calls.typesafe.length).toBeGreaterThan(0);
  });
});

describe("re-enrichment", () => {
  it("asks again only when the page or a judgment version changes", async () => {
    const calls = newCalls();
    const mods = [{ id: 1, updated: 1795000000 }];
    await runEnrichment(testEnv(), NOW, fakeNet(mods, calls));
    const first = calls.typesafe.filter((body) => "title" in (body.state as object) && !("lines" in (body.state as object))).length;
    expect(first).toBe(1);

    await runEnrichment(testEnv(), NOW + 1000, fakeNet(mods, calls));
    expect(calls.typesafe.filter((body) => !("lines" in (body.state as object))).length).toBe(1);

    const edited = { profiles: { 1: { ...PROFILE, _sText: "Now with a red variant." } } };
    await runEnrichment(testEnv(), NOW + 2000, fakeNet([{ id: 1, updated: 1795000500 }], calls, edited));
    expect(calls.typesafe.filter((body) => !("lines" in (body.state as object))).length).toBe(2);

    // An older version of one judgment re-runs that judgment alone.
    await env.DB.prepare("UPDATE mod_tags SET version = 0 WHERE mod_id = 1 AND judgment = 'style'").run();
    await runEnrichment(testEnv(), NOW + 3000, fakeNet([{ id: 1, updated: 1795000500 }], calls, edited));
    const last = calls.typesafe.filter((body) => !("lines" in (body.state as object))).at(-1);
    const groups = new Set(Object.keys(last?.questions ?? {}).map((id) => id.split("__")[0]));
    expect([...groups]).toEqual(["style"]);
  });
});

describe("bounded work", () => {
  it("never makes more outside calls than the run budget", async () => {
    const calls = newCalls();
    const mods = Array.from({ length: 20 }, (_, at) => ({ id: at + 1, updated: 1795000000 + at }));
    const report = await runEnrichment(testEnv(), NOW, fakeNet(mods, calls), 20);
    expect(calls.gb.length + calls.typesafe.length + calls.steam).toBeLessThanOrEqual(20);
    expect(report.mods).toBeGreaterThan(0);
    expect(report.mods).toBeLessThan(20);
    expect(RUN_SUBREQUESTS).toBeLessThanOrEqual(50);
  });

  it("does not overlap a run that holds the lease", async () => {
    await env.DB.prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES ('lease', ?1, ?2)").bind(String(NOW + 60_000), NOW).run();
    const calls = newCalls();
    const report = await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1 }], calls));
    expect(report.skipped).toBe("lease_held");
    expect(calls.gb).toEqual([]);
  });
});

describe("errors in the cron", () => {
  async function seedOldDiagnostics(): Promise<void> {
    await env.DB.prepare(`INSERT INTO diagnostics (created_at, code, version, os, summary, log) VALUES (?1, 'SGL-OLD1-AAAA', '0.1.0', 'linux', 's', 'l')`)
      .bind(NOW - 40 * 86400_000)
      .run();
  }

  for (const [label, options] of [
    ["GameBanana", { gbStatus: 503 }],
    ["TypeSafe", { typesafeStatus: 500 }],
  ] as const) {
    it(`a ${label} error leaves retention and epoch closing running`, async () => {
      await seedOldDiagnostics();
      let closed = 0;
      const calls = newCalls();
      await scheduledJob(
        testEnv(),
        NOW,
        "17 3 * * *",
        async () => {
          closed += 1;
          return [];
        },
        (runEnv, now) => runEnrichment(runEnv, now, fakeNet([{ id: 1, updated: 1795000000 }], calls, options)),
      );
      expect(closed).toBe(1);
      const left = await env.DB.prepare("SELECT COUNT(*) AS n FROM diagnostics").first<{ n: number }>();
      expect(left?.n).toBe(0);
      const lease = await env.DB.prepare("SELECT value FROM enrich_state WHERE key = 'lease'").first<{ value: string }>();
      expect(lease?.value).toBe("0");
    });
  }

  it("a throwing enrichment step leaves the others running", async () => {
    await seedOldDiagnostics();
    let closed = 0;
    await scheduledJob(
      testEnv(),
      NOW,
      "17 3 * * *",
      async () => {
        closed += 1;
        return [];
      },
      async () => {
        throw new Error("boom");
      },
    );
    expect(closed).toBe(1);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM diagnostics").first<{ n: number }>())?.n).toBe(0);
  });
});

describe("routes", () => {
  it("serve the index with an ETag, a few hours of caching, 304s and a delta", async () => {
    const ai = { ai: 0 };
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000, title: "CS 1.6 GIGN Venator skin" }], newCalls()));
    await buildAndStoreIndex(env.DB, NOW);
    const response = await call("/v1/catalog/tags", { method: "GET" });
    expect(response.status).toBe(200);
    expect(response.headers.get("cache-control")).toBe("public, max-age=10800");
    const etag = response.headers.get("etag") ?? "";
    expect(etag).toMatch(/^"[0-9a-f]{32}"$/);
    const index = await readJson<{ strings: string[]; mods: { i: number; x: number[]; h?: string; k?: string }[]; aliases: Record<string, string> }>(response);
    expect(index.mods[0]?.i).toBe(1);
    expect(index.mods[0]?.h).toBe("Venator");
    const terms = (index.mods[0]?.x ?? []).map((packed) => index.strings[Math.floor(packed / 8)]);
    expect(terms).toContain("counter strike");
    expect(terms).toContain("gas mask");
    expect(index.aliases.bull).toBe("abrams");

    const again = await call("/v1/catalog/tags", { method: "GET", headers: { "if-none-match": etag } });
    expect(again.status).toBe(304);

    const empty = await call(`/v1/catalog/tags?since=${NOW + 1}`, { method: "GET" });
    expect((await readJson<{ mods: unknown[] }>(empty)).mods).toEqual([]);
    const full = await call("/v1/catalog/tags?since=0", { method: "GET" });
    const delta = await readJson<{ strings: string[]; mods: { x: number[] }[] }>(full);
    expect(delta.mods.length).toBe(1);
    expect(Math.max(...(delta.mods[0]?.x ?? [0]).map((packed) => Math.floor(packed / 8)))).toBeLessThan(delta.strings.length);

    expect((await call("/v1/catalog/tags?since=abc", { method: "GET" })).status).toBe(400);
  });

  it("serve per-mod detail and patch classes", async () => {
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }], newCalls()));
    const detail = await call("/v1/catalog/mod/1", { method: "GET" });
    expect(detail.status).toBe(200);
    expect(detail.headers.get("etag")).not.toBeNull();
    const body = await readJson<{ files: { main: number | null }; health: { commenters: number }; updates: unknown[] }>(detail);
    expect(body.files.main).toBe(1);
    expect(body.health.commenters).toBe(3);
    expect(body.updates.length).toBe(1);

    const patch = await call("/v1/catalog/patch/latest", { method: "GET" });
    expect(patch.status).toBe(200);
    const patchBody = await readJson<{ id: string; heroes: { hero: string }[]; complete: boolean }>(patch);
    expect(patchBody.id).toBe("555");
    expect(patchBody.complete).toBe(true);
    expect(patchBody.heroes.map((hero) => hero.hero)).toContain("Venator");
    expect((await call("/v1/catalog/patch/999", { method: "GET" })).status).toBe(404);
  });

  it("are rate limited like the other routes", async () => {
    const statuses: number[] = [];
    for (let at = 0; at < 61; at += 1) statuses.push((await call("/v1/catalog/tags", { method: "GET", ip: "198.51.100.77" })).status);
    expect(statuses.at(-1)).toBe(429);
  });
});

describe("thresholds", () => {
  it("publishes a kind only when a kind fits and the pick is clear", () => {
    expect(publishedKind({ kind: "skin", p: 0.9, fits: 0.9 })).toBe("skin");
    expect(publishedKind({ kind: "skin", p: T.kindP - 0.01, fits: 0.9 })).toBeNull();
    expect(publishedKind({ kind: "skin", p: 0.9, fits: 0.5 })).toBeNull();
    expect(publishedKind({ kind: "skin", p: 0.9, fits: 0.1 })).toBe("other");
  });

  it("styles, hero, nsfw hint and facts", () => {
    expect(publishedStyles({ anime: 0.8, cute: 0.74, dark: 0.1 })).toEqual(["anime"]);
    const meta = { id: 1, title: "x", subcategory: "Haze", enrichedAt: 0 };
    expect(publishedHero({ hero: "Abrams", p: 0.99 }, meta, ["Haze", "Abrams"])).toBe("Haze");
    expect(publishedHero({ hero: "Abrams", p: 0.71 }, { ...meta, subcategory: "HUD" }, ["Haze", "Abrams"])).toBe("Abrams");
    expect(publishedHero({ hero: "Abrams", p: 0.6 }, { ...meta, subcategory: "HUD" }, ["Abrams"])).toBeNull();
    expect(publishedHero({ hero: "none", p: 0.95 }, { ...meta, subcategory: "HUD" }, ["Abrams"])).toBeNull();
    expect(nsfwHint({ p: 0.79, gb: false })).toBe(false);
    expect(nsfwHint({ p: 0.81, gb: false })).toBe(true);
    expect(requiresMod({ requires: 0.9, which: { id: 123, p: 0.7 } })).toBe(123);
    expect(requiresMod({ requires: 0.5, which: { id: 123, p: 0.9 } })).toBeNull();
    expect(devState({ state: { choice: "wip", p: 0.8 } })).toBe("wip");
    expect(devState({ state: { choice: "finished", p: 0.99 } })).toBeNull();
  });

  it("cover picks skip baked-in text for the banner", () => {
    const picks = coverPicks({
      shots: [
        { i: 0, w: 530, h: 300, card: 1, banner: 1, text: 0.9, box: [0, 0, 1, 1] },
        { i: 1, w: 530, h: 300, card: 0.5, banner: 0.7, text: 0.1, box: [0.2, 0.1, 0.8, 0.9] },
      ],
    });
    expect(picks?.card).toBe(0);
    expect(picks?.banner).toEqual([1, 0.7, 0.2, 0.1, 0.8, 0.9]);
    expect(coverPicks({ shots: [{ i: 0, w: 1, h: 1, card: 0, banner: 0.3, text: 0, box: null }] })?.banner).toBeNull();
  });

  it("health needs enough commenters; duplicates need a strong yes; terms need their class threshold", () => {
    expect(healthChip({ broken: 0.9, works: 0.1, conflict: 0, commenters: 2 })).toBeNull();
    expect(healthChip({ broken: 0.9, works: 0.1, conflict: 0, commenters: 3 })?.broken).toBe(true);
    expect(dupeOf({ pairs: [{ id: 5, p: 0.8 }, { id: 6, p: 0.9 }] })).toBe(6);
    expect(
      keptTerms({
        terms: [
          { term: "counter strike", cls: "reference", p: 0.71 },
          { term: "cs", cls: "alias", p: 0.71 },
          { term: "gas mask", cls: "visual", p: 0.74 },
          { term: "military", cls: "theme", p: 0.8 },
        ],
      }).map((term) => term.term),
    ).toEqual(["counter strike", "cs", "military"]);
  });

  it("every judgment has a name, a version and a question", () => {
    expect(new Set(JUDGMENTS.map((judgment) => judgment.name)).size).toBe(JUDGMENTS.length);
    for (const judgment of JUDGMENTS) expect(judgment.version).toBeGreaterThan(0);
  });
});

describe("parsers", () => {
  it("reads image sizes from PNG and JPEG headers", () => {
    expect(imageSize(pngBytes(800, 450))).toMatchObject({ format: "png", width: 800, height: 450 });
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 17, 8, 0x01, 0x2c, 0x02, 0x12, 3, 0, 0, 0, 0]);
    expect(imageSize(jpeg)).toMatchObject({ format: "jpg", width: 530, height: 300, progressive: false });
  });

  it("bounds and normalises generated terms", () => {
    const terms = parseExpansion({
      response: JSON.stringify({
        franchises: [{ name: "Counter-Strike", aliases: ["CS", "cs", "Counter Strike", "x".repeat(80)] }],
        visual: Array.from({ length: 30 }, (_, at) => `thing ${at}`),
        themes: ["Deadlock", "military"],
      }),
    });
    expect(terms?.[0]).toEqual({ term: "counter strike", cls: "reference", facet: "franchise", src: "text" });
    expect(terms?.filter((term) => term.cls === "visual").length).toBe(10);
    expect(terms?.some((term) => term.term === "deadlock")).toBe(false);
    expect(terms?.every((term) => term.term.length <= 40)).toBe(true);
    expect(parseExpansion("not json")).toBeNull();
  });

  it("splits patch notes and finds heroes in code", () => {
    const lines = patchLines("[p]- Apollo: Riposte now disarms[/p][p]- Healbane: stacks diminish[/p][p]x[/p]");
    expect(lines).toEqual(["Apollo: Riposte now disarms", "Healbane: stacks diminish"]);
    expect(lineHero("Apollo: Riposte now disarms", ["Apollo", "Haze"])).toBe("Apollo");
    expect(lineHero("Fixed Haze dagger", ["Apollo", "Haze"])).toBe("Haze");
    expect(lineHero("Healbane: stacks", ["Apollo"])).toBeNull();
    const published = publishPatch("1", "t", 0, [
      { hero: "Apollo", system: "model", systemP: 0.9, visual: 0.9 },
      { hero: null, system: "abilities", systemP: 0.5, visual: 0.1 },
    ]);
    expect(published.systems).toEqual({ model: 1 });
    expect(published.heroes).toEqual([{ hero: "Apollo", lines: 1, visual: 1 }]);
  });
});

describe("untrusted page content", () => {
  it("decodes only valid code points", () => {
    expect(decodeEntities("a&#99999999;b&#xD800;c&#x41;&#66;&#x110000;")).toBe("a\ufffdb\ufffdcAB\ufffd");
  });

  it("scans hostile HTML in linear time", () => {
    const hostile = "<a href=".repeat(50_000) + "<".repeat(200_000) + "<a href='https://gamebanana.com/mods/1'>".repeat(20_000);
    const started = Date.now();
    htmlToText(hostile);
    modLinks(hostile);
    expect(Date.now() - started).toBeLessThan(500);
    expect(modLinks('<p>x</p><a href="https://gamebanana.com/mods/42">Base</a>')).toEqual([{ id: 42, label: "Base" }]);
    expect(htmlToText("<p>one</p><br>two &amp; <b>three</b>")).toBe("one\ntwo & three");
  });

  it("never quotes page text or generated terms in a question", async () => {
    const calls = newCalls();
    const ai = { ai: 0 };
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    const questions = JSON.stringify(calls.typesafe.map((body) => body.questions));
    expect(questions).not.toContain("Base pack");
    expect(questions).not.toContain("counter");
    expect(questions).toContain("`terms[");
  });

  it("skips one broken mod page and carries on", async () => {
    const calls = newCalls();
    const report = await runEnrichment(
      testEnv(),
      NOW,
      fakeNet([{ id: 1, updated: 1795000000 }, { id: 2, updated: 1795000001 }], calls, { profiles: { 2: { _sText: 5, _aFiles: "x", _aPreviewMedia: { _aImages: [{ _sBaseUrl: {}, _sFile: [] }] } } } }),
    );
    expect(report.mods).toBeGreaterThanOrEqual(1);
    expect(report.stopped).toBeUndefined();
  });
});

describe("upkeep", () => {
  it("drops withdrawn mods from the index", async () => {
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }, { id: 2, updated: 1795000001 }], newCalls()));
    await runEnrichment(testEnv(), NOW + 1000, fakeNet([{ id: 1, updated: 1795000000 }, { id: 2, updated: 1795000999 }], newCalls(), { profiles: { 2: 404 } }));
    await buildAndStoreIndex(env.DB, NOW + 2000);
    expect((await loadIndex(env.DB))?.mods.map((mod) => mod.i)).toEqual([1]);
  });

  it("spends nothing when a page bump leaves the content the same", async () => {
    const calls = newCalls();
    const modCalls = () => calls.typesafe.filter((body) => !("lines" in (body.state as object)) && !("query" in (body.state as object))).length;
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    const before = modCalls();
    const report = await runEnrichment(testEnv(), NOW + 1000, fakeNet([{ id: 1, updated: 1795000500 }], calls));
    expect(modCalls()).toBe(before);
    expect(report.unchanged).toBe(1);
    expect(report.mods).toBe(0);
  });

  it("stops at the per-run mod cap and the daily spend cap", async () => {
    const mods = Array.from({ length: 10 }, (_, at) => ({ id: at + 1, updated: 1795000000 + at }));
    const capped = await runEnrichment(testEnv({ ENRICH_RUN_MODS: "2" }), NOW, fakeNet(mods, newCalls()));
    expect(capped.mods).toBe(2);
    expect(capped.stopped).toBe("run_mods");
    await env.DB.prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES ('spend_day', ?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
      .bind(JSON.stringify({ day: new Date(NOW).toISOString().slice(0, 10), usd: 30 }), NOW)
      .run();
    const calls = newCalls();
    const blocked = await runEnrichment(testEnv(), NOW + 1000, fakeNet(mods, calls));
    expect(blocked.skipped).toBe("daily_cap");
    expect(calls.gb).toEqual([]);
  });
});

describe("learning from search gaps", () => {
  it("sends only floor-passed terms and adds learned terms to matching mods", async () => {
    const ai = { ai: 0 };
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000, title: "GIGN Vindicta" }], newCalls()));
    await buildAndStoreIndex(env.DB, NOW);
    await env.DB.batch([
      env.DB.prepare("INSERT INTO product_totals (epoch, kind, key, installs, buckets, days, created_at) VALUES (1, 'search_zero', 'counter strike', 12, 7, 4, 0)"),
      env.DB.prepare("INSERT INTO product_totals (epoch, kind, key, installs, buckets, days, created_at) VALUES (1, 'search_zero', 'below floor secret', 3, 1, 1, 0)"),
      env.DB.prepare("UPDATE enrich_state SET value = '0' WHERE key = 'learn_at'"),
    ]);
    const calls = newCalls();
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW + 25 * 3600_000, fakeNet([], calls));
    const learn = calls.typesafe.filter((body) => "query" in (body.state as object));
    expect(learn.length).toBe(1);
    expect((learn[0]?.state as { query: string }).query).toBe("counter strike");
    expect(JSON.stringify(calls.typesafe)).not.toContain("below floor secret");
    for (const key of Object.keys(learn[0]?.state as object)) expect(LEARN_STATE_KEYS.has(key)).toBe(true);
    const row = await env.DB.prepare("SELECT data FROM mod_tags WHERE mod_id = 1 AND judgment = 'learned'").first<{ data: string }>();
    expect(JSON.parse(row?.data ?? "{}").terms[0].term).toBe("counter strike");
  });

  it("builds a request with the query as data and candidates as public fields", () => {
    const body = learnRequest("gas mask", [{ id: 1, title: "t", category: "c", description: "d" }]);
    expect(() => assertPublicOnly(body, LEARN_STATE_KEYS)).not.toThrow();
    expect(() => assertPublicOnly(body)).toThrow(/query/);
    expect(JSON.stringify(body.questions)).not.toContain("gas mask");
  });
});

describe("learned terms and stale tags", () => {
  it("never publishes a gap word that is not already public vocabulary", async () => {
    const ai = { ai: 0 };
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000, title: "GIGN Vindicta" }], newCalls()));
    await buildAndStoreIndex(env.DB, NOW);
    await env.DB.batch([
      env.DB.prepare("INSERT INTO product_totals (epoch, kind, key, installs, buckets, days, created_at) VALUES (1, 'search_zero', 'gign soldier', 12, 7, 4, 0)"),
      env.DB.prepare("INSERT INTO product_totals (epoch, kind, key, installs, buckets, days, created_at) VALUES (1, 'search_zero', 'gign zorblax', 12, 7, 4, 0)"),
      env.DB.prepare("INSERT INTO product_totals (epoch, kind, key, installs, buckets, days, created_at) VALUES (1, 'search_zero', 'gign 123456', 12, 7, 4, 0)"),
      env.DB.prepare("UPDATE enrich_state SET value = '0' WHERE key = 'learn_at'"),
    ]);
    const calls = newCalls();
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW + 25 * 3600_000, fakeNet([], calls));
    const sent = JSON.stringify(calls.typesafe);
    expect(sent).not.toContain("zorblax");
    expect(sent).not.toContain("123456");
    // Off by default: nothing learned reaches the index; on, only the vocabulary term does.
    await buildAndStoreIndex(env.DB, NOW + 26 * 3600_000);
    const off = await loadIndex(env.DB);
    expect(JSON.stringify(off)).not.toContain("gign soldier");
    await buildAndStoreIndex(env.DB, NOW + 27 * 3600_000, { learned: true });
    const on = JSON.stringify(await loadIndex(env.DB));
    expect(on).toContain("gign soldier");
    expect(on).not.toContain("zorblax");
  });

  it("retries a mod whose enrichment failed midway and marks its old tags stale", async () => {
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }], newCalls()));
    const edited = { profiles: { 1: { ...PROFILE, _sText: "Now NSFW." } } };
    const failing = await runEnrichment(testEnv(), NOW + 1000, fakeNet([{ id: 1, updated: 1795000900 }], newCalls(), { ...edited, typesafeStatus: 503 }));
    expect(failing.stopped).toBe("typesafe_503");
    await buildAndStoreIndex(env.DB, NOW + 2000);
    expect((await loadIndex(env.DB))?.mods[0]?.st).toBe(1);
    // The old rows were not moved to the new page version.
    const kind = await env.DB.prepare("SELECT source FROM mod_tags WHERE mod_id = 1 AND judgment = 'kind'").first<{ source: string }>();
    expect(kind?.source).toBe("u:1795000000");
    const calls = newCalls();
    const retry = await runEnrichment(testEnv(), NOW + 3000, fakeNet([{ id: 1, updated: 1795000900 }], calls, edited));
    expect(retry.mods).toBe(1);
    await buildAndStoreIndex(env.DB, NOW + 4000);
    expect((await loadIndex(env.DB))?.mods[0]?.st).toBeUndefined();
  });
});
