import { createScheduledController } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import worker from "../src/index";
import { closeEpochs, contribute, EPOCH_MS, epochOf, networkPrefix, publishedKeys } from "../src/aggregates";
import { installHash, modelName, parseRecord, PRODUCT_TYPES, scrubSearch, type ProductRecord } from "../src/product";
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
  it("removes the install's raw rows and open-epoch contributions, and only that install's", async () => {
    const other = crypto.randomUUID();
    await call("/v1/product", { body: body([{ type: "launch", modded: true }, { type: "enabled_set", mods: [{ mod_id: 1, slot: 1 }, { mod_id: 2, slot: 2 }] }]) });
    await call("/v1/product", { body: body([{ type: "launch", modded: false }], { installId: other }) });
    const mine = await installHash(INSTALL_ID, undefined);
    const before = await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib WHERE install_hash = ?1").bind(mine).first<{ n: number }>();
    // seen, two mods and one pair.
    expect(before?.n).toBe(4);

    const response = await call("/v1/product/delete", { body: { installId: INSTALL_ID } });
    expect(response.status).toBe(200);
    // Two events and four contributions.
    expect(await readJson<{ deleted: number }>(response)).toEqual({ deleted: 6 });

    const left = await rows();
    expect(left.length).toBe(1);
    expect(left[0]?.install_hash).toBe(await installHash(other, undefined));
    const contrib = await env.DB.prepare("SELECT DISTINCT install_hash FROM product_contrib").all<{ install_hash: string }>();
    expect(contrib.results.map((r) => r.install_hash)).toEqual([await installHash(other, undefined)]);
  });

  it("needs a real install id", async () => {
    expect((await call("/v1/product/delete", { body: { installId: "x" } })).status).toBe(400);
    expect((await call("/v1/product/delete", { body: {} })).status).toBe(400);
  });
});

const SET = { type: "enabled_set", mods: [{ mod_id: 100, slot: 1 }, { mod_id: 200, slot: 2 }, { mod_id: 300, slot: 3 }] };

const KEY = (env as typeof env & { readonly PRODUCT_BUCKET_KEY: string }).PRODUCT_BUCKET_KEY;
const DAY = 24 * 60 * 60 * 1000;
/** The start of a fixed epoch, so tests do not depend on today's date. */
const E0 = (epochOf(Date.UTC(2026, 0, 1)) + 1) * EPOCH_MS;

function records(): ProductRecord[] {
  return [SET, { type: "browse_query", sort: null, filters: [], search: "haze", results: 3 }].map((raw) => parseRecord(raw) as ProductRecord);
}

/**
 * `installs` installs in epoch `epoch`; install n is on the network `network(n)` and sends on two
 * days, `n % 3` and `n % 3 + 1`, so an epoch with three or more installs spans three days.
 */
async function sendEpoch(epoch: number, installs: number, network: (n: number) => string, ids?: string[]): Promise<string[]> {
  const made: string[] = [];
  for (let n = 0; n < installs; n += 1) {
    const hash = ids?.[n] ?? (await installHash(crypto.randomUUID(), undefined));
    made.push(hash);
    for (const day of [n % 3, (n % 3) + 1]) {
      await contribute(env.DB, KEY, network(n), hash, records(), E0 + epoch * EPOCH_MS + day * DAY + 3_600_000);
    }
  }
  return made;
}

async function closeAll(): Promise<void> {
  await closeEpochs(env.DB, E0 + 40 * EPOCH_MS);
}

async function relatedTo(mod: number, envOverride?: Record<string, unknown>): Promise<Related> {
  const response =
    envOverride === undefined
      ? await call(`/v1/product/related?mod=${mod}`, { method: "GET" })
      : await worker.fetch(
          new Request(`http://telemetry.test/v1/product/related?mod=${mod}`, {
            headers: { "x-sigil-client": "1", "x-sigil-version": VERSION, "cf-connecting-ip": "203.0.113.1" },
          }),
          { ...env, ...envOverride } as typeof env,
        );
  expect(response.status).toBe(200);
  return readJson<Related>(response);
}

const SIX_NETWORKS = (n: number): string => `10.${n % 6}.0.${n + 1}`;

describe("the distinct-contributor floor", () => {
  it("publishes a pair ten installs on six networks share on three days of one epoch", async () => {
    await sendEpoch(0, 10, SIX_NETWORKS);
    // The open epoch is never published.
    expect(await relatedTo(100)).toEqual({ mods: [] });
    await closeAll();
    expect(await relatedTo(100)).toEqual({
      mods: [
        { modId: 200, installs: 10 },
        { modId: 300, installs: 10 },
      ],
    });
    expect(await publishedKeys(env.DB, KEY, "search", 10)).toEqual([{ key: "haze", installs: 10 }]);
  });

  it("publishes nothing for fifty made-up ids on one network over three epochs", async () => {
    for (const epoch of [0, 1, 2]) await sendEpoch(epoch, 50, (n) => `198.51.100.${n + 1}`);
    await closeAll();
    expect(await relatedTo(100)).toEqual({ mods: [] });
    expect(await publishedKeys(env.DB, KEY, "search", 10)).toEqual([]);
    const totals = await env.DB.prepare("SELECT MAX(installs) AS n FROM product_totals").first<{ n: number }>();
    expect(totals?.n).toBe(3);
  });

  it("publishes nothing from two networks", async () => {
    await sendEpoch(0, 40, (n) => `172.16.${n % 2}.${n + 1}`);
    await closeAll();
    expect(await relatedTo(100)).toEqual({ mods: [] });
  });

  it("does not let a flood from one network block real installs in the next epoch", async () => {
    await sendEpoch(0, 30, () => "10.0.0.1");
    await sendEpoch(1, 10, SIX_NETWORKS);
    await closeAll();
    expect((await relatedTo(100)).mods.map((m) => m.modId)).toEqual([200, 300]);
  });

  it("frees a bucket slot and lowers the counts when an install is deleted in the open epoch", async () => {
    const network = (): string => "10.9.9.9";
    const ids = await sendEpoch(0, 3, network);
    const fourth = await installHash(crypto.randomUUID(), undefined);
    expect(await contribute(env.DB, KEY, "10.9.9.10", fourth, records(), E0 + 3_600_000)).toBe(false);
    await env.DB.prepare("DELETE FROM product_contrib WHERE install_hash = ?1").bind(ids[0]).run();
    expect(await contribute(env.DB, KEY, "10.9.9.10", fourth, records(), E0 + 3_600_000)).toBe(true);
    const counted = await env.DB.prepare("SELECT COUNT(DISTINCT install_hash) AS n FROM product_contrib WHERE kind = 'mod' AND key = '100'").first<{ n: number }>();
    expect(counted?.n).toBe(3);
  });

  it("a delete through the endpoint removes the install from the open epoch", async () => {
    const id = crypto.randomUUID();
    await call("/v1/product", { body: body([SET], { installId: id }), ip: "10.7.7.7" });
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib").first<{ n: number }>())?.n).toBeGreaterThan(0);
    await call("/v1/product/delete", { body: { installId: id } });
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib").first<{ n: number }>())?.n).toBe(0);
  });

  it("counts an IPv6 /48 as one network", () => {
    expect(networkPrefix("2001:db8:abcd:12::1")).toBe(networkPrefix("2001:db8:abcd:ffff::9"));
    expect(networkPrefix("2001:db8:abcd::1")).not.toBe(networkPrefix("2001:db8:abce::1"));
    expect(networkPrefix("203.0.113.9")).toBe(networkPrefix("203.0.113.200"));
    expect(networkPrefix("not an address")).toBeNull();
  });

  it("publishes nothing and writes no contribution without the bucket secret", async () => {
    await sendEpoch(0, 10, SIX_NETWORKS);
    await closeAll();
    expect(await relatedTo(100, { PRODUCT_BUCKET_KEY: undefined })).toEqual({ mods: [] });
    expect(await publishedKeys(env.DB, undefined, "search", 10)).toEqual([]);

    await clearTables();
    expect(await contribute(env.DB, undefined, "203.0.113.5", "h", records(), Date.now())).toBe(false);
    const response = await worker.fetch(
      new Request("http://telemetry.test/v1/product", {
        method: "POST",
        headers: { "x-sigil-client": "1", "x-sigil-version": VERSION, "cf-connecting-ip": "203.0.113.5", "content-type": "application/json" },
        body: JSON.stringify(body([SET])),
      }),
      { ...env, PRODUCT_BUCKET_KEY: undefined } as typeof env,
    );
    expect(response.status).toBe(202);
    expect((await rows()).length).toBe(1);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib").first<{ n: number }>())?.n).toBe(0);
  });

  it("does not count installs seen on one day only", async () => {
    for (let n = 0; n < 12; n += 1) {
      const hash = await installHash(crypto.randomUUID(), undefined);
      await contribute(env.DB, KEY, SIX_NETWORKS(n), hash, records(), E0 + (n % 3) * DAY);
    }
    await closeAll();
    expect(await relatedTo(100)).toEqual({ mods: [] });
  });

  it("leaves no install hash or bucket outside raw events once an epoch closes", async () => {
    await sendEpoch(0, 10, SIX_NETWORKS);
    await call("/v1/product", { body: body([SET]) });
    await closeAll();
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib WHERE epoch <= ?1").bind(epochOf(E0)).first<{ n: number }>())?.n).toBe(0);
    const columns = await env.DB.prepare("SELECT name FROM pragma_table_info('product_totals')").all<{ name: string }>();
    expect(columns.results.map((c) => c.name)).toEqual(["epoch", "kind", "key", "mod_a", "mod_b", "installs", "buckets", "days"]);
    const totals = JSON.stringify(await env.DB.prepare("SELECT * FROM product_totals").all());
    expect(totals).not.toMatch(/[0-9a-f]{16}/);
    const eventColumns = await env.DB.prepare("SELECT name FROM pragma_table_info('product_events')").all<{ name: string }>();
    expect(eventColumns.results.map((c) => c.name)).not.toContain("bucket");
  });

  it("never stores an address or a network prefix", async () => {
    await call("/v1/product", { body: body([SET]), ip: "198.51.100.9" });
    for (const table of ["product_events", "product_contrib", "product_totals"]) {
      const dump = JSON.stringify(await env.DB.prepare(`SELECT * FROM ${table}`).all());
      expect(dump).not.toContain("198.51.100");
    }
  });

  it("rejects a bad mod id", async () => {
    for (const mod of ["", "0", "abc", "76561198012345678"]) {
      expect((await call(`/v1/product/related?mod=${mod}`, { method: "GET" })).status).toBe(400);
    }
  });
});

describe("product retention", () => {
  it("deletes raw rows older than 13 months and stray contributions, and keeps totals", async () => {
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
    }
    await env.DB.prepare(
      `INSERT INTO product_contrib (epoch, kind, key, install_hash, bucket, day, created_at) VALUES (1, 'seen', 'd', 'h', 'b', 'd', ?1)`,
    )
      .bind(now - 45 * DAY_MS)
      .run();
    await env.DB.prepare(`INSERT INTO product_totals (epoch, kind, key, installs, buckets, days) VALUES (1, 'search', 'haze', 12, 7, 4)`).run();
    await worker.scheduled(createScheduledController({ cron: "17 3 * * *" }), env);
    expect((await rows()).map((row) => row.type)).toEqual(["new"]);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib").first<{ n: number }>())?.n).toBe(0);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_totals").first<{ n: number }>())?.n).toBe(1);
  });
});
