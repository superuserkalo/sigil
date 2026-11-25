import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { scheduledJob } from "../src/cron";
import { buildAndStoreIndex, loadIndex, runEnrichment, RUN_SUBREQUESTS, DEFAULT_DAILY_USD } from "../src/catalog/enrich";
import { parseExpansion, withoutHeroNames, type AiRunner } from "../src/catalog/expand";
import { decodeEntities, htmlToText } from "../src/catalog/gamebanana";
import { JUDGMENTS } from "../src/catalog/judgments";
import { descriptionWords, keptTerms, modDetail, PACK, PUBLISH_VERSION, T } from "../src/catalog/publish";
import { assertPublicOnly, BODY_KEYS, STATE_KEYS, TYPESAFE_URL, type SystemOneBody } from "../src/catalog/typesafe";
import type { Net } from "../src/catalog/net";
import { call, clearTables, readJson } from "./helpers";

const KEY = "test-only-typesafe-key-0123456789";
const NOW = Date.UTC(2026, 10, 25, 12);
const SENTINEL = "PLAYER-SECRET-SEARCH-TEXT";

type Calls = { gb: string[]; typesafe: SystemOneBody[]; headers: Headers[] };

type ModFixture = { id: number; updated: number; title?: string; sub?: string; cat?: string };

function listBody(mods: ModFixture[]): unknown {
  return {
    _aRecords: mods.map((mod) => ({
      _idRow: mod.id,
      _sModelName: "Mod",
      _sName: mod.title ?? `Mod ${mod.id}`,
      _tsDateAdded: mod.updated - 100,
      _tsDateModified: mod.updated,
      _aRootCategory: { _sName: mod.cat ?? "Skins" },
      _aSubCategory: { _sName: mod.sub ?? "Vindicta" },
      _aSubmitter: { _idRow: 77, _sName: "pyro shark" },
    })),
  };
}

const TEXT = {
  _sDescription: "CS 1.6 GIGN as Vindicta",
  _sText: 'Counter-Strike 1.6 GIGN model with a gas mask. Needs <a href="https://gamebanana.com/mods/123">Base pack</a>.',
  _aFiles: [
    { _idRow: 1, _sFile: "gign_main.zip", _nFilesize: 10485760, _sDescription: "main" },
    { _idRow: 2, _sFile: "gign_blue.zip", _nFilesize: 10485760, _sDescription: "blue variant" },
  ],
};

/** Answers every question: Nouls at 0.9, Choices on their first option. */
function jevAnswer(body: SystemOneBody): unknown {
  const answers: Record<string, unknown> = {};
  for (const [id, question] of Object.entries(body.questions)) {
    if (question.type === "noul") answers[id] = { type: "noul", noul: 0.9 };
    if (question.type === "choice") {
      const first = Object.keys(question.criteria)[0] ?? "x";
      answers[id] = { type: "choice", choice: first, confidence: 0.85, probabilities: { [first]: 0.9 } };
    }
  }
  return { model: "jev-1.14.0", answers, usage: { input_tokens: 1500, output_tokens: 10 } };
}

function fakeNet(
  mods: ModFixture[],
  calls: Calls,
  options: { gbStatus?: number; typesafeStatus?: number; texts?: Record<number, unknown> } = {},
): Net {
  return {
    sleep: async () => {},
    fetch: async (url, init) => {
      if (url === TYPESAFE_URL) {
        const body = JSON.parse(String(init?.body)) as SystemOneBody;
        calls.typesafe.push(body);
        calls.headers.push(new Headers(init?.headers));
        if (options.typesafeStatus !== undefined) return new Response("{}", { status: options.typesafeStatus });
        return Response.json(jevAnswer(body));
      }
      calls.gb.push(url);
      if (options.gbStatus !== undefined) return new Response("no", { status: options.gbStatus });
      if (url.includes("/Mod/Index")) return Response.json(listBody(url.includes("_nPage=1&") ? mods : []));
      if (url.includes("/Mod/Multi")) {
        const ids = (/_csvRowIds=([\d,]+)/.exec(url)?.[1] ?? "").split(",").map(Number);
        return Response.json(
          ids.flatMap((id) => {
            const custom = options.texts?.[id];
            if (custom === "gone") return [];
            return [{ _idRow: id, ...((custom as object | undefined) ?? TEXT) }];
          }),
        );
      }
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
        characters: [{ name: "Vindicta", aliases: [] }],
        visual: ["gas mask"],
        themes: ["military"],
      },
      usage: { prompt_tokens: 600, completion_tokens: 120 },
    };
  },
});

const newCalls = (): Calls => ({ gb: [], typesafe: [], headers: [] });

function testEnv(extra: Record<string, unknown> = {}): Env {
  // The model flags start off here whatever wrangler.jsonc ships, so each test says what it turns on.
  return { ...env, CATALOG_INDEX: "1", CATALOG_EXPANSION: "0", CATALOG_FILE_LABELS: "0", TYPESAFE_API_KEY: KEY, AI: undefined, ...extra } as unknown as Env;
}

const ON = { CATALOG_EXPANSION: "1", CATALOG_FILE_LABELS: "1" };

async function clearCatalog(): Promise<void> {
  await env.DB.batch(["catalog_mods", "mod_tags", "enrich_state"].map((table) => env.DB.prepare(`DELETE FROM ${table}`)));
}

beforeEach(async () => {
  await clearTables();
  await clearCatalog();
});

describe("the index from public text", () => {
  it("indexes title, description words, category, hero subcategory and author with no model at all", async () => {
    const calls = newCalls();
    const report = await runEnrichment({ ...env, CATALOG_INDEX: "1", CATALOG_EXPANSION: "0", TYPESAFE_API_KEY: undefined } as unknown as Env, NOW, fakeNet([{ id: 1, updated: 1795000000, title: "GIGN Vindicta" }], calls));
    expect(report.refreshed).toBe(1);
    expect(calls.typesafe).toEqual([]);
    await buildAndStoreIndex(env.DB, NOW);
    const index = await loadIndex(env.DB);
    const fields = (index?.mods[0]?.x ?? []).map((packed) => index?.strings[Math.floor(packed / PACK)]);
    expect(fields).toEqual(expect.arrayContaining(["counter", "strike", "gas", "mask", "skins", "vindicta", "pyro shark"]));
    // Model-made terms are absent while the flag is off.
    expect(fields).not.toContain("counter strike");
  });

  it("drops withdrawn mods and stops reading unchanged ones", async () => {
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000 }, { id: 2, updated: 1795000001 }], newCalls()));
    const calls = newCalls();
    const report = await runEnrichment(testEnv(), NOW + 1000, fakeNet([{ id: 1, updated: 1795000000 }, { id: 2, updated: 1795000999 }], calls, { texts: { 2: "gone" } }));
    expect(report.withdrawn).toBe(1);
    // Mod 1's timestamp did not move: its text is not read again.
    expect(calls.gb.filter((url) => url.includes("/Mod/Multi")).join()).not.toMatch(/RowIds=1\b/);
    await buildAndStoreIndex(env.DB, NOW + 2000);
    expect((await loadIndex(env.DB))?.mods.map((mod) => mod.i)).toEqual([1]);
  });

  it("indexes description words without stop words, capped", () => {
    expect(descriptionWords("The GIGN mod, with a gas mask! https://x.com")).toEqual(["gign", "gas", "mask"]);
    expect(descriptionWords(Array.from({ length: 200 }, (_, at) => `word${at}`).join(" ")).length).toBe(60);
  });
});

describe("what is sent to TypeSafe (flags on)", () => {
  it("holds only allowlisted public fields and nothing from the product tables", async () => {
    await env.DB.prepare(
      `INSERT INTO product_events (created_at, install_hash, type, version, os, os_build, game_build, payload) VALUES (?1, ?2, 'search', '0.1.0', 'linux', '', '', ?3)`,
    )
      .bind(NOW, "install-hash-should-never-leave", JSON.stringify({ query: SENTINEL }))
      .run();
    const calls = newCalls();
    const ai = { ai: 0 } as { ai: number; input?: string };
    await runEnrichment(testEnv({ ...ON, AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(calls.typesafe.length).toBe(1);
    for (const body of calls.typesafe) {
      expect(() => assertPublicOnly(body)).not.toThrow();
      for (const key of Object.keys(body)) expect(BODY_KEYS.has(key)).toBe(true);
      const text = JSON.stringify(body);
      expect(text).not.toContain(SENTINEL);
      expect(text).not.toContain("install-hash-should-never-leave");
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
      // Page text and generated terms are data in the state, never quoted in a question.
      const questions = JSON.stringify(body.questions);
      expect(questions).not.toContain("counter");
      expect(questions).not.toContain("gign_main");
    }
    expect(ai.ai).toBe(1);
    expect(ai.input).not.toContain(SENTINEL);
    expect(calls.headers[0]?.get("authorization")).toBe(`Bearer ${KEY}`);
  });

  it("refuses a body with a field outside the allowlist, and pins the lists", () => {
    expect(() => assertPublicOnly({ model: "jev-1.14.0", state: { title: "x", installHash: "abc" }, questions: {} } as unknown as SystemOneBody)).toThrow(/installHash/);
    expect(() => assertPublicOnly({ model: "jev-1.14.0", state: {}, questions: {}, user: "x" } as unknown as SystemOneBody)).toThrow(/user/);
    expect([...BODY_KEYS].sort()).toEqual(["model", "questions", "state"]);
    expect([...STATE_KEYS].sort()).toEqual(["category", "description", "files", "name", "note", "size_mb", "subcategory", "terms", "title"]);
  });
});

describe("flags and fail closed", () => {
  it("does nothing while the upkeep is switched off", async () => {
    const calls = newCalls();
    expect((await runEnrichment(testEnv({ CATALOG_INDEX: "0" }), NOW, fakeNet([{ id: 1, updated: 1 }], calls))).skipped).toBe("off");
    expect(calls.gb).toEqual([]);
  });

  it("makes no model call with the flags off, even with the key", async () => {
    const ai = { ai: 0 };
    const calls = newCalls();
    await runEnrichment(testEnv({ AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(calls.typesafe).toEqual([]);
    expect(ai.ai).toBe(0);
  });

  it("makes no model call without the key, and the route still answers", async () => {
    const ai = { ai: 0 };
    const calls = newCalls();
    await runEnrichment({ ...env, ...ON, CATALOG_INDEX: "1", TYPESAFE_API_KEY: undefined, AI: fakeAi(ai) } as unknown as Env, NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(calls.typesafe).toEqual([]);
    expect(ai.ai).toBe(0);
    expect((await call("/v1/catalog/tags", { method: "GET" })).status).toBe(200);
  });

  it("runs file labels without the AI binding, and keeps going when the generator fails", async () => {
    const calls = newCalls();
    await runEnrichment(testEnv(ON), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(Object.keys(calls.typesafe[0]?.questions ?? {}).every((id) => id.startsWith("files__"))).toBe(true);
    const broken: AiRunner = {
      run: async () => {
        throw new Error("model down");
      },
    };
    await clearCatalog();
    const again = newCalls();
    await runEnrichment(testEnv({ ...ON, AI: broken }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], again));
    expect(again.typesafe.length).toBe(1);
  });

  it("stops at the daily spend cap, $1 by default, and the free index upkeep still runs", async () => {
    expect(DEFAULT_DAILY_USD).toBe(1);
    await env.DB.prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES ('spend_day', ?1, ?2)")
      .bind(JSON.stringify({ day: new Date(NOW).toISOString().slice(0, 10), usd: 1.2 }), NOW)
      .run();
    const calls = newCalls();
    const report = await runEnrichment(testEnv(ON), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(report.stopped).toBe("daily_cap");
    expect(calls.typesafe).toEqual([]);
    expect(report.refreshed).toBe(1);
  });
});

describe("re-judging", () => {
  it("asks again only when the content or a judgment version changes", async () => {
    const calls = newCalls();
    await runEnrichment(testEnv(ON), NOW, fakeNet([{ id: 1, updated: 1795000000 }], calls));
    expect(calls.typesafe.length).toBe(1);
    // Timestamp moved, content the same: nothing is asked.
    await runEnrichment(testEnv(ON), NOW + 1000, fakeNet([{ id: 1, updated: 1795000500 }], calls));
    expect(calls.typesafe.length).toBe(1);
    // Content changed: asked again.
    const edited = { texts: { 1: { ...TEXT, _sText: "Now with a red variant." } } };
    await runEnrichment(testEnv(ON), NOW + 2000, fakeNet([{ id: 1, updated: 1795000900 }], calls, edited));
    expect(calls.typesafe.length).toBe(2);
    // An older version of one judgment re-runs that one alone.
    await env.DB.prepare("UPDATE mod_tags SET version = 0 WHERE mod_id = 1 AND judgment = 'files'").run();
    await runEnrichment(testEnv(ON), NOW + 3000, fakeNet([{ id: 1, updated: 1795000900 }], calls, edited));
    expect(new Set(Object.keys(calls.typesafe.at(-1)?.questions ?? {}).map((id) => id.split("__")[0]))).toEqual(new Set(["files"]));
  });

  it("retries a mod whose judging failed midway and marks its old terms stale", async () => {
    const ai = { ai: 0 };
    await runEnrichment(testEnv({ ...ON, AI: fakeAi(ai) }), NOW, fakeNet([{ id: 1, updated: 1795000000 }], newCalls()));
    const edited = { texts: { 1: { ...TEXT, _sText: "Something else entirely." } } };
    const failing = await runEnrichment(testEnv({ ...ON, AI: fakeAi(ai) }), NOW + 1000, fakeNet([{ id: 1, updated: 1795000900 }], newCalls(), { ...edited, typesafeStatus: 503 }));
    expect(failing.stopped).toBe("typesafe_503");
    await buildAndStoreIndex(env.DB, NOW + 2000, { expansion: true });
    expect((await loadIndex(env.DB))?.mods[0]?.st).toBe(1);
    const retry = await runEnrichment(testEnv({ ...ON, AI: fakeAi(ai) }), NOW + 3000, fakeNet([{ id: 1, updated: 1795000900 }], newCalls(), edited));
    expect(retry.judged).toBe(1);
    await buildAndStoreIndex(env.DB, NOW + 4000, { expansion: true });
    expect((await loadIndex(env.DB))?.mods[0]?.st).toBeUndefined();
  });
});

describe("bounded work", () => {
  it("never makes more outside calls than the run budget", async () => {
    const calls = newCalls();
    const mods = Array.from({ length: 30 }, (_, at) => ({ id: at + 1, updated: 1795000000 + at }));
    const report = await runEnrichment(testEnv(ON), NOW, fakeNet(mods, calls), 12);
    expect(calls.gb.length + calls.typesafe.length).toBeLessThanOrEqual(12);
    expect(report.judged).toBeGreaterThan(0);
    expect(RUN_SUBREQUESTS).toBeLessThanOrEqual(50);
  });

  it("does not overlap a run that holds the lease", async () => {
    await env.DB.prepare("INSERT INTO enrich_state (key, value, updated_at) VALUES ('lease', ?1, ?2)").bind(String(NOW + 60_000), NOW).run();
    const calls = newCalls();
    expect((await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1 }], calls))).skipped).toBe("lease_held");
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
      await scheduledJob(
        testEnv(ON),
        NOW,
        "17 3 * * *",
        async () => {
          closed += 1;
          return [];
        },
        (runEnv, now) => runEnrichment(runEnv, now, fakeNet([{ id: 1, updated: 1795000000 }], newCalls(), options)),
      );
      expect(closed).toBe(1);
      expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM diagnostics").first<{ n: number }>())?.n).toBe(0);
      expect((await env.DB.prepare("SELECT value FROM enrich_state WHERE key = 'lease'").first<{ value: string }>())?.value).toBe("0");
    });
  }

  it("a throwing upkeep step leaves the others running", async () => {
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
    await runEnrichment(testEnv(), NOW, fakeNet([{ id: 1, updated: 1795000000, title: "CS 1.6 Vindicta skin" }], newCalls()));
    await buildAndStoreIndex(env.DB, NOW);
    const response = await call("/v1/catalog/tags", { method: "GET" });
    expect(response.status).toBe(200);
    expect(response.headers.get("cache-control")).toBe("public, max-age=10800");
    const etag = response.headers.get("etag") ?? "";
    expect(etag).toMatch(/^"[0-9a-f]{32}"$/);
    const index = await readJson<{ v: number; mods: { i: number; ti: string }[] }>(response);
    expect(index.mods[0]?.ti).toBe("CS 1.6 Vindicta skin");
    expect(index.v).toBe(PUBLISH_VERSION);
    expect(index).not.toHaveProperty("aliases");
    expect((await call("/v1/catalog/tags", { method: "GET", headers: { "if-none-match": etag } })).status).toBe(304);
    const empty = await call(`/v1/catalog/tags?since=${1795000000 * 1000 + 1}`, { method: "GET" });
    expect((await readJson<{ mods: unknown[] }>(empty)).mods).toEqual([]);
    const delta = await readJson<{ strings: string[]; mods: { x: number[] }[] }>(await call("/v1/catalog/tags?since=0", { method: "GET" }));
    expect(delta.mods.length).toBe(1);
    expect(delta).not.toHaveProperty("aliases");
    expect(Math.max(...(delta.mods[0]?.x ?? [0]).map((packed) => Math.floor(packed / PACK)))).toBeLessThan(delta.strings.length);
    expect((await call("/v1/catalog/tags?since=abc", { method: "GET" })).status).toBe(400);
  });

  it("serve the file hint per mod", async () => {
    await runEnrichment(testEnv(ON), NOW, fakeNet([{ id: 1, updated: 1795000000 }], newCalls()));
    const detail = await call("/v1/catalog/mod/1", { method: "GET" });
    expect(detail.status).toBe(200);
    expect((await readJson<{ files: { main: number | null } }>(detail)).files.main).toBe(1);
  });

  it("are rate limited like the other routes", async () => {
    const statuses: number[] = [];
    for (let at = 0; at < 61; at += 1) statuses.push((await call("/v1/catalog/tags", { method: "GET", ip: "198.51.100.77" })).status);
    expect(statuses.at(-1)).toBe(429);
  });
});

describe("thresholds", () => {
  it("terms need their class threshold; aliases follow their reference", () => {
    expect(
      keptTerms({
        terms: [
          { term: "counter strike", cls: "reference", p: T.termReference },
          { term: "cs", cls: "alias", p: T.termReference },
          { term: "gas mask", cls: "visual", p: T.termVisual - 0.01 },
          { term: "military", cls: "theme", p: 0.8 },
          { term: "made up", cls: "reference", p: 0.2 },
        ],
      }),
    ).toEqual(["counter strike", "cs", "military"]);
  });

  it("file labels are a hint only above the threshold", () => {
    const detail = modDetail(1, { files: { main: { id: 7, p: T.fileMainP - 0.01 }, labels: [{ id: 7, label: "main", p: 0.9 }, { id: 8, label: "old", p: 0.3 }] } });
    expect(detail.files).toEqual({ main: null, labels: [{ id: 7, label: "main" }, { id: 8, label: null }] });
  });

  it("every judgment has a unique name and a version", () => {
    expect(new Set(JUDGMENTS.map((judgment) => judgment.name)).size).toBe(JUDGMENTS.length);
    for (const judgment of JUDGMENTS) expect(judgment.version).toBeGreaterThan(0);
  });
});

describe("untrusted page content", () => {
  it("decodes only valid code points", () => {
    expect(decodeEntities("a&#99999999;b&#xD800;c&#x41;&#66;&#x110000;")).toBe("a�b�cAB�");
  });

  it("scans hostile HTML in linear time", () => {
    const hostile = "<a href=".repeat(50_000) + "<".repeat(200_000) + "<a href='x'>".repeat(20_000);
    const started = Date.now();
    htmlToText(hostile);
    expect(Date.now() - started).toBeLessThan(500);
    expect(htmlToText("<p>one</p><br>two &amp; <b>three</b>")).toBe("one\ntwo & three");
  });

  it("bounds generated terms and drops Deadlock hero names", () => {
    const terms = parseExpansion({
      response: JSON.stringify({
        franchises: [{ name: "Counter-Strike", aliases: ["CS", "cs", "Counter Strike", "x".repeat(80)] }],
        characters: [],
        visual: Array.from({ length: 30 }, (_, at) => `thing ${at}`),
        themes: ["Deadlock", "military"],
      }),
    });
    expect(terms?.[0]).toEqual({ term: "counter strike", cls: "reference", facet: "franchise" });
    expect(terms?.filter((term) => term.cls === "visual").length).toBe(10);
    expect(terms?.some((term) => term.term === "deadlock")).toBe(false);
    expect(parseExpansion("not json")).toBeNull();
    const cut = withoutHeroNames(
      [
        { term: "seven", cls: "reference", facet: "character" },
        { term: "aston machan lady geist", cls: "reference", facet: "character" },
        { term: "machan", cls: "alias", facet: "character", of: "aston machan lady geist" },
      ],
      ["Seven", "Lady Geist"],
    );
    expect(cut.map((term) => [term.term, term.of ?? ""])).toEqual([
      ["aston machan", ""],
      ["machan", "aston machan"],
    ]);
  });
});
