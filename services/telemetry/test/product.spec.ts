import { createScheduledController } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import worker from "../src/index";
import { installHash, modelName, PRODUCT_TYPES, scrubSearch } from "../src/product";
import { call, clearTables, INSTALL_ID, readJson, VERSION } from "./helpers";

type Accepted = { readonly accepted: number; readonly dropped: number };
type EventRow = {
  readonly install_hash: string;
  readonly type: string;
  readonly os_build: string;
  readonly game_build: number | null;
  readonly payload: string;
};
type Related = { readonly mods: readonly { readonly modId: number; readonly installs: number }[] };

const DAY_MS = 24 * 60 * 60 * 1000;

function body(records: readonly unknown[], extra: Readonly<Record<string, unknown>> = {}): Record<string, unknown> {
  return { installId: INSTALL_ID, version: VERSION, os: "windows", osBuild: "10.0.22631", gameBuild: 5911, records, ...extra };
}

async function rows(): Promise<EventRow[]> {
  const { results } = await env.DB.prepare(
    "SELECT install_hash, type, os_build, game_build, payload FROM product_events ORDER BY id",
  ).all<EventRow>();
  return results;
}

beforeEach(clearTables);

describe("POST /v1/product", () => {
  it("stores allowlisted records under a salted hash, never the raw id", async () => {
    const response = await call("/v1/product", {
      body: body([
        { type: "mod_action", mod_id: 512345, action: "installed" },
        { type: "launch", modded: true },
      ]),
    });
    expect(response.status).toBe(202);
    expect(await readJson<Accepted>(response)).toEqual({ accepted: 2, dropped: 0 });

    const stored = await rows();
    expect(stored.map((row) => row.type)).toEqual(["mod_action", "launch"]);
    expect(stored[0]?.install_hash).toBe(await installHash(INSTALL_ID, undefined));
    expect(stored[0]?.install_hash).not.toContain(INSTALL_ID);
    expect(stored[0]?.os_build).toBe("10.0.22631");
    expect(stored[0]?.game_build).toBe(5911);
    expect(JSON.parse(stored[0]?.payload ?? "")).toEqual({ mod_id: 512345, action: "installed" });

    const dump = JSON.stringify(await env.DB.prepare("SELECT * FROM product_events").all());
    expect(dump).not.toContain(INSTALL_ID);
    expect(dump).not.toContain("192.0.2.");
  });

  it("drops unknown types and unknown fields", async () => {
    const response = await call("/v1/product", {
      body: body([
        { type: "steam_profile", steam_id: "76561198012345678" },
        { type: "launch", modded: false, path: "C:\\Users\\kalo", steamId: 76561198012345678 },
        { type: "crash_suspect", mod_id: 0 },
        { type: "crash_suspect", mod_id: 76561198012345678 },
        { type: "settings", theme: "C:/Users/kalo", density: "compact", language: "de", mature: false },
      ]),
    });
    expect(await readJson<Accepted>(response)).toEqual({ accepted: 1, dropped: 4 });
    const stored = await rows();
    expect(stored.map((row) => row.type)).toEqual(["launch"]);
    expect(JSON.parse(stored[0]?.payload ?? "")).toEqual({ modded: false });
  });

  it("scrubs search text again and bounds lists", async () => {
    const filters = Array.from({ length: 20 }, (_, n) => `filter_${String.fromCharCode(97 + n)}`);
    await call("/v1/product", {
      body: body([
        { type: "browse_query", sort: "new", filters, search: "  Haze SKIN ", results: 41 },
        { type: "browse_query", sort: null, filters: [], search: "me@example.com", results: 0 },
        { type: "browse_query", sort: null, filters: [], search: "C:\\Users\\kalo", results: 0 },
        { type: "browse_query", sort: null, filters: [], search: "76561198012345678", results: 0 },
        {
          type: "hardware",
          gpu: "NVIDIA GeForce RTX 3060/PCIe/SSE2",
          cpu: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz",
          ram_gb: 16,
          width: 1920,
          height: 1080,
          refresh_hz: 144,
        },
      ]),
    });
    const payloads = (await rows()).map((row) => JSON.parse(row.payload) as Record<string, unknown>);
    expect(payloads[0]?.search).toBe("haze skin");
    expect((payloads[0]?.filters as string[]).length).toBe(8);
    expect(payloads.slice(1, 4).map((p) => p.search)).toEqual([null, null, null]);
    expect(payloads[4]).toMatchObject({ gpu: "NVIDIA GeForce RTX 3060", cpu: "Intel Core i7-9700K" });
  });

  it("rejects a bad envelope and an oversized body", async () => {
    expect((await call("/v1/product", { body: body([], {}) })).status).toBe(400);
    expect((await call("/v1/product", { body: body([{ type: "launch", modded: true }], { installId: "nope" }) })).status).toBe(400);
    const many = Array.from({ length: 51 }, () => ({ type: "launch", modded: true }));
    expect((await call("/v1/product", { body: body(many) })).status).toBe(400);
    const huge = "x".repeat(40 * 1024);
    expect((await call("/v1/product", { rawBody: JSON.stringify(body([{ type: "launch", modded: true, pad: huge }])) })).status).toBe(413);
  });

  it("knows the same types as the app", () => {
    expect(PRODUCT_TYPES.length).toBe(21);
    expect(new Set(PRODUCT_TYPES).size).toBe(PRODUCT_TYPES.length);
  });
});

describe("scrubbers", () => {
  it("search text follows ADR 0017", () => {
    expect(scrubSearch("  Haze   SKIN\t")).toBe("haze skin");
    expect(scrubSearch("a ".repeat(100))?.length).toBeLessThanOrEqual(64);
    for (const text of ["me@x.y", "/Users/kalo", "https://x", "www.x", "gamebanana.com", "12345", "[U:1:52345]", "765 611 980 123"]) {
      expect(scrubSearch(text)).toBeNull();
    }
  });

  it("model names lose marks, clocks and serials", () => {
    expect(modelName("AMD Ryzen 7 5800X 8-Core Processor")).toBe("AMD Ryzen 7 5800X");
    expect(modelName("GPU SN 123456789")).toBeNull();
    expect(modelName("C:\\Users\\kalo")).toBeNull();
  });
});

describe("POST /v1/product/delete", () => {
  it("removes every raw row for that install and only that install", async () => {
    const other = crypto.randomUUID();
    await call("/v1/product", { body: body([{ type: "launch", modded: true }, { type: "enabled_set", mods: [{ mod_id: 1, slot: 1 }, { mod_id: 2, slot: 2 }] }]) });
    await call("/v1/product", { body: body([{ type: "launch", modded: false }], { installId: other }) });

    const response = await call("/v1/product/delete", { body: { installId: INSTALL_ID } });
    expect(response.status).toBe(200);
    expect(await readJson<{ deleted: number }>(response)).toEqual({ deleted: 3 });

    const left = await rows();
    expect(left.length).toBe(1);
    expect(left[0]?.install_hash).toBe(await installHash(other, undefined));
    const pairs = await env.DB.prepare("SELECT COUNT(*) AS n FROM product_pairs").first<{ n: number }>();
    expect(pairs?.n).toBe(0);
  });

  it("needs a real install id", async () => {
    expect((await call("/v1/product/delete", { body: { installId: "x" } })).status).toBe(400);
    expect((await call("/v1/product/delete", { body: {} })).status).toBe(400);
  });
});

describe("GET /v1/product/related", () => {
  it("answers only for pairs at least 10 installs share", async () => {
    const set = { type: "enabled_set", mods: [{ mod_id: 100, slot: 1 }, { mod_id: 200, slot: 2 }, { mod_id: 300, slot: 3 }] };
    for (let n = 0; n < 9; n += 1) {
      await call("/v1/product", { body: body([set], { installId: crypto.randomUUID() }) });
    }
    // The same install twice still counts once.
    await call("/v1/product", { body: body([set]) });
    await call("/v1/product", { body: body([set]) });
    let response = await call("/v1/product/related?mod=100", { method: "GET" });
    expect(response.status).toBe(200);
    // 9 random installs + INSTALL_ID = 10.
    expect(await readJson<Related>(response)).toEqual({
      mods: [
        { modId: 200, installs: 10 },
        { modId: 300, installs: 10 },
      ],
    });

    await call("/v1/product", { body: body([{ type: "enabled_set", mods: [{ mod_id: 100, slot: 1 }, { mod_id: 400, slot: 2 }] }]) });
    response = await call("/v1/product/related?mod=400", { method: "GET" });
    expect(await readJson<Related>(response)).toEqual({ mods: [] });
  });

  it("rejects a bad mod id", async () => {
    for (const mod of ["", "0", "abc", "76561198012345678"]) {
      expect((await call(`/v1/product/related?mod=${mod}`, { method: "GET" })).status).toBe(400);
    }
  });
});

describe("product retention", () => {
  it("deletes raw rows older than 13 months", async () => {
    const now = Date.now();
    for (const [age, type] of [
      [400, "old"],
      [380, "new"],
    ] as const) {
      await env.DB.prepare(
        `INSERT INTO product_events (created_at, install_hash, type, version, os, payload) VALUES (?1, 'h', ?2, '0.1.0', 'windows', '{}')`,
      )
        .bind(now - age * DAY_MS, type)
        .run();
      await env.DB.prepare(`INSERT INTO product_pairs (mod_a, mod_b, install_hash, created_at, updated_at) VALUES (1, ?1, 'h', ?2, ?2)`)
        .bind(age, now - age * DAY_MS)
        .run();
    }
    await worker.scheduled(createScheduledController({ cron: "17 3 * * *" }), env);
    expect((await rows()).map((row) => row.type)).toEqual(["new"]);
    const pairs = await env.DB.prepare("SELECT mod_b FROM product_pairs").all<{ mod_b: number }>();
    expect(pairs.results.map((row) => row.mod_b)).toEqual([380]);
  });
});
