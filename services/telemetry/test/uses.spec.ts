/**
 * What product data gives back (ADR 0017): mod health, frame rates on PCs like yours, and the
 * owner's search gaps. Every new kind goes through the same epoch floor as `pair`; these tests
 * hold each kind to it, hold the admin route to its secret, and check that no response carries an
 * install hash or a bucket.
 */

import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import worker from "../src/index";
import {
  closeEpochs,
  contribute,
  cpuTier,
  EPOCH_MS,
  epochOf,
  fpsFor,
  keysOf,
  median,
  modHealth,
  resolutionBucket,
  rising,
  searchGaps,
  type KeyContext,
} from "../src/aggregates";
import { installHash, parseRecord, type ProductRecord } from "../src/product";
import { call, clearTables, INSTALL_ID, readJson, VERSION } from "./helpers";

const SALT = (env as typeof env & { readonly PRODUCT_SALT: string }).PRODUCT_SALT;
const KEY = (env as typeof env & { readonly PRODUCT_BUCKET_KEY: string }).PRODUCT_BUCKET_KEY;
const DAY = 24 * 60 * 60 * 1000;
const ADMIN = "test-only-admin-key-0123456789abcdef-xyz";
/** A closed epoch that is recent today, so the routes (which read `Date.now()`) see it. */
const RECENT = (epochOf(Date.now()) - 2) * EPOCH_MS;
const SIX_NETWORKS = (n: number): string => `10.${n % 6}.1.${n + 1}`;
const HEX_ID = /[0-9a-f]{16,}/;

beforeEach(clearTables);

function rec(raw: Record<string, unknown>): ProductRecord {
  const parsed = parseRecord(raw);
  if (parsed === null) throw new Error(`bad record ${JSON.stringify(raw)}`);
  return parsed;
}

/**
 * `installs` installs, each on `network(n)` and on two days (`n % 3`, `n % 3 + 1`) of the epoch
 * starting at `base`, each sending `records(n)`.
 */
async function seed(
  base: number,
  installs: number,
  network: (n: number) => string,
  records: (n: number) => ProductRecord[],
  ctx: (n: number) => KeyContext = () => ({ gameBuild: 6100 }),
): Promise<void> {
  for (let n = 0; n < installs; n += 1) {
    const hash = await installHash(crypto.randomUUID(), SALT);
    for (const day of [n % 3, (n % 3) + 1]) {
      await contribute(env.DB, KEY, network(n), hash, records(n), base + day * DAY + 3_600_000, ctx(n));
    }
  }
}

async function get(path: string, extra: Record<string, unknown> = {}, headers: Record<string, string> = {}): Promise<Response> {
  return worker.fetch(
    new Request(`http://telemetry.test${path}`, {
      headers: { "x-sigil-client": "1", "x-sigil-version": VERSION, "cf-connecting-ip": "203.0.113.77", ...headers },
    }),
    { ...env, ...extra } as typeof env,
  );
}

const SET = rec({ type: "enabled_set", mods: [{ mod_id: 100, slot: 1 }, { mod_id: 200, slot: 2 }] });
const FAIL = rec({ type: "install_result", mod_id: 100, ok: false, reason: "nothing_installable" });
const OK = rec({ type: "install_result", mod_id: 100, ok: true, reason: null });

describe("keys of the new kinds", () => {
  it("counts only failures that point at the mod, and attempts without the player's own trouble", () => {
    const kinds = (r: ProductRecord) => keysOf(r, { gameBuild: 6100 }).map((k) => `${k.kind}:${k.key}`);
    expect(kinds(FAIL)).toEqual(["attempt:100", "health:100|install_nothing_installable", "health:100|install_failed"]);
    expect(kinds(OK)).toEqual(["attempt:100"]);
    expect(kinds(rec({ type: "install_result", mod_id: 100, ok: false, reason: "cancelled" }))).toEqual(["health:100|install_cancelled"]);
    expect(kinds(rec({ type: "off_after_crash", mod_id: 100 }))).toEqual(["health:100|off_after_crash|6100"]);
    expect(kinds(rec({ type: "mod_pair", a: 300, b: 100, kind: "overlap" }))).toEqual(["conflict:100|300"]);
    expect(kinds(SET)).toContain("use:100|6100");
    expect(kinds(rec({ type: "browse_query", sort: null, filters: [], search: "kelvin hat", results: 0 }))).toEqual([
      "search:kelvin hat",
      "search_zero:kelvin hat",
    ]);
    expect(kinds(rec({ type: "browse_query", sort: null, filters: [], search: "haze", results: 40 }))).toEqual(["search:haze"]);
  });

  it("files a benchmark under the hardware class, config version and resolution bucket", () => {
    const bench = rec({ type: "benchmark", avg_fps: 144, low_fps: 90, scene: "street_test", width: 2560, height: 1440, preset: "optilock" });
    const ctx: KeyContext = { gameBuild: null, fps: { gpu: "NVIDIA GeForce RTX 3060", tier: "mid", versions: new Map([["optilock", 3]]) } };
    expect(keysOf(bench, ctx).find((k) => k.kind === "fps")).toEqual({
      kind: "fps",
      key: "NVIDIA GeForce RTX 3060|mid|optilock|3|1440p",
      value: 144,
      value2: 90,
    });
    expect(keysOf(bench).some((k) => k.kind === "fps")).toBe(false);
  });

  it("puts CPUs into coarse tiers and heights into buckets", () => {
    expect(cpuTier("AMD Ryzen 7 5800X3D")).toBe("high");
    expect(cpuTier("AMD Ryzen 5 3600")).toBe("mid");
    expect(cpuTier("AMD Ryzen 3 2200G")).toBe("low");
    expect(cpuTier("Intel Core i5-12400F")).toBe("mid");
    expect(cpuTier("Intel Core i9-13900K")).toBe("high");
    expect(cpuTier("Intel Core i7-4790K")).toBe("low");
    expect(cpuTier("Intel Core Ultra 7 265K")).toBe("high");
    expect(cpuTier(null)).toBe("other");
    expect([resolutionBucket(720), resolutionBucket(1080), resolutionBucket(1440), resolutionBucket(2160)]).toEqual([
      "720p",
      "1080p",
      "1440p",
      "2160p",
    ]);
    expect(median([3, 1, 2])).toBe(2);
    expect(median([1, 2, 3, 4])).toBe(3);
  });
});

describe("mod health", () => {
  it("publishes install failures, broken builds and conflicts that met the floor, with their bases", async () => {
    await seed(RECENT, 10, SIX_NETWORKS, (n) => [
      SET,
      n < 10 ? FAIL : OK,
      rec({ type: "compat_finding", mod_id: 100, finding: "gone", game_build: 6100 }),
      rec({ type: "mod_pair", a: 100, b: 200, kind: "conflict" }),
    ]);
    // Successful installs on more networks raise the base, not the failures.
    await seed(RECENT, 12, (n) => `10.${(n % 6) + 10}.2.${n + 1}`, () => [OK]);
    await closeEpochs(env.DB, Date.now());

    const response = await get("/v1/product/health?mods=100,200,999");
    expect(response.status).toBe(200);
    const text = await response.text();
    expect(text).not.toMatch(HEX_ID);
    const body = JSON.parse(text) as { mods: { modId: number }[] };
    expect(body.mods.map((m) => m.modId)).toEqual([100, 200]);
    const until = (RECENT + EPOCH_MS) / 1000;
    expect(body.mods[0]).toEqual({
      modId: 100,
      install: { failed: 10, attempts: 22, reasons: [{ reason: "nothing_installable", installs: 10 }], until },
      broken: [{ build: 6100, reports: 10, users: 10, until }],
      conflicts: [{ modId: 200, reports: 10, together: 10, until }],
    });
    expect(body.mods[1]).toEqual({ modId: 200, install: null, broken: [], conflicts: [{ modId: 100, reports: 10, together: 10, until }] });
  });

  it("publishes nothing from nine installs, from one network, or from the open epoch", async () => {
    const records = (): ProductRecord[] => [SET, FAIL, rec({ type: "crash_suspect", mod_id: 100 }), rec({ type: "mod_pair", a: 100, b: 200, kind: "conflict" })];
    await seed(RECENT, 9, SIX_NETWORKS, records);
    await seed(RECENT + EPOCH_MS, 40, () => "198.51.100.7", records);
    await closeEpochs(env.DB, Date.now());
    // The open epoch, with plenty of installs.
    await seed(epochOf(Date.now()) * EPOCH_MS, 12, SIX_NETWORKS, records);
    expect(await modHealth(env.DB, KEY, [100, 200], Date.now())).toEqual([]);
    const totals = await env.DB.prepare("SELECT COUNT(*) AS n FROM product_totals").first<{ n: number }>();
    expect(totals?.n).toBe(0);
  });

  it("forgets reports older than the recent epochs", async () => {
    const old = (epochOf(Date.now()) - 12) * EPOCH_MS;
    await seed(old, 10, SIX_NETWORKS, () => [SET, FAIL]);
    await closeEpochs(env.DB, Date.now());
    expect(await modHealth(env.DB, KEY, [100], old + 2 * EPOCH_MS + 3 * DAY)).toHaveLength(1);
    expect(await modHealth(env.DB, KEY, [100], Date.now())).toEqual([]);
  });

  it("rejects bad mod lists", async () => {
    for (const mods of ["", "abc", "1,,2", "0", Array.from({ length: 41 }, (_, n) => n + 1).join(",")]) {
      expect((await get(`/v1/product/health?mods=${mods}`)).status).toBe(400);
    }
  });
});

describe("frame rates on PCs like yours", () => {
  const GPU = "NVIDIA GeForce RTX 3060";
  const ctx = (): KeyContext => ({ gameBuild: null, fps: { gpu: GPU, tier: "mid", versions: new Map([["optilock", 3]]) } });
  const bench = (n: number): ProductRecord[] => [
    rec({ type: "benchmark", avg_fps: 100 + n * 2, low_fps: 60 + n, scene: "street_test", width: 2560, height: 1440, preset: "optilock" }),
  ];

  it("publishes medians per config and version behind the floor, and no single install's value", async () => {
    await seed(RECENT, 11, SIX_NETWORKS, bench, ctx);
    await closeEpochs(env.DB, Date.now());
    const response = await get(`/v1/product/fps?gpu=${encodeURIComponent(GPU)}&cpu=${encodeURIComponent("AMD Ryzen 5 3600")}&height=1440`);
    expect(response.status).toBe(200);
    const text = await response.text();
    expect(text).not.toMatch(HEX_ID);
    // Averages 100..120 step 2: median 110. Lows 60..70: median 65.
    expect(JSON.parse(text)).toEqual({ configs: [{ config: "optilock", version: 3, fps: 110, low: 65, installs: 11 }] });
    // Another resolution, another class: nothing.
    expect(await fpsFor(env.DB, KEY, GPU, "mid", "1080p")).toEqual([]);
    expect(await fpsFor(env.DB, KEY, GPU, "high", "1440p")).toEqual([]);
    // Only the medians stay.
    const rows = await env.DB.prepare("SELECT median, low, installs FROM product_totals WHERE kind = 'fps'").all();
    expect(rows.results).toEqual([{ median: 110, low: 65, installs: 11 }]);
    const left = await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib WHERE kind = 'fps'").first<{ n: number }>();
    expect(left?.n).toBe(0);
  });

  it("publishes nothing below the floor", async () => {
    await seed(RECENT, 9, SIX_NETWORKS, bench, ctx);
    await closeEpochs(env.DB, Date.now());
    expect(await fpsFor(env.DB, KEY, GPU, "mid", "1440p")).toEqual([]);
  });

  it("files a sent benchmark under the install's own hardware and config", async () => {
    const id = crypto.randomUUID();
    const send = (records: unknown[]) =>
      call("/v1/product", { body: { installId: id, version: VERSION, os: "windows", records }, ip: "10.40.0.1" });
    await send([
      { type: "hardware", gpu: GPU, cpu: "Intel Core i5-12400F", ram_gb: 16, width: 2560, height: 1440, refresh_hz: 0 },
      { type: "perf_config", config: "optilock", version: 4 },
    ]);
    await send([{ type: "benchmark", avg_fps: 130, low_fps: 80, scene: "street_test", width: 2560, height: 1440, preset: "optilock" }]);
    const row = await env.DB.prepare("SELECT key, value, value2 FROM product_contrib WHERE kind = 'fps'").first();
    expect(row).toEqual({ key: `${GPU}|mid|optilock|4|1440p`, value: 130, value2: 80 });
    // Delete my data takes the open-epoch value with it.
    await call("/v1/product/delete", { body: { installId: id } });
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_contrib").first<{ n: number }>())?.n).toBe(0);
  });

  it("rejects a query that is not a hardware class", async () => {
    for (const query of ["", "gpu=RTX&height=abc", "gpu=%2Fhome%2Fme&height=1080", "gpu=RTX%203060&height=99999"]) {
      expect((await get(`/v1/product/fps?${query}`)).status).toBe(400);
    }
  });
});

describe("search gaps (owner only)", () => {
  const search = (text: string, results: number) => rec({ type: "browse_query", sort: null, filters: [], search: text, results });

  it("answers 503 without the admin key, 401 with a wrong one, and the gaps with the right one", async () => {
    await seed(RECENT, 10, SIX_NETWORKS, () => [search("kelvin hat", 0), search("yamato sword", 2), search("haze", 50)]);
    // A rare zero-result term from two installs stays unpublished.
    await seed(RECENT, 2, (n) => `10.50.${n}.1`, () => [search("my secret term", 0)]);
    await closeEpochs(env.DB, Date.now());

    expect((await get("/v1/product/admin/search-gaps", {}, { authorization: `Bearer ${ADMIN}` })).status).toBe(503);
    expect((await get("/v1/product/admin/search-gaps", { PRODUCT_ADMIN_KEY: "short" }, { authorization: "Bearer short" })).status).toBe(503);
    expect((await get("/v1/product/admin/search-gaps", { PRODUCT_ADMIN_KEY: ADMIN }, { authorization: "Bearer wrong-key" })).status).toBe(401);
    expect((await get("/v1/product/admin/search-gaps", { PRODUCT_ADMIN_KEY: ADMIN })).status).toBe(401);
    const ok = await get("/v1/product/admin/search-gaps", { PRODUCT_ADMIN_KEY: ADMIN }, { authorization: `Bearer ${ADMIN}` });
    expect(ok.status).toBe(200);
    const text = await ok.text();
    expect(text).not.toMatch(HEX_ID);
    expect(JSON.parse(text)).toEqual({ zero: [{ term: "kelvin hat", installs: 10 }], few: [{ term: "yamato sword", installs: 10 }] });
    expect(await searchGaps(env.DB, undefined, 10)).toEqual({ zero: [], few: [] });
  });
});

describe("the new routes without their secrets", () => {
  const cases: [string, Record<string, unknown>][] = [
    ["no salt", { PRODUCT_SALT: undefined }],
    ["no bucket key", { PRODUCT_BUCKET_KEY: undefined }],
    ["a short bucket key", { PRODUCT_BUCKET_KEY: "short-key" }],
  ];
  for (const [name, override] of cases) {
    it(`answers 503 not_configured with ${name}`, async () => {
      for (const path of [
        "/v1/product/health?mods=100",
        "/v1/product/fps?gpu=RTX%203060&height=1080",
        "/v1/product/admin/search-gaps",
      ]) {
        const response = await get(path, { ...override, PRODUCT_ADMIN_KEY: ADMIN }, { authorization: `Bearer ${ADMIN}` });
        expect(response.status).toBe(503);
        expect(await readJson<{ error: string }>(response)).toEqual({ error: "not_configured" });
      }
    });
  }

  it("needs the client headers like every route", async () => {
    const response = await worker.fetch(new Request("http://telemetry.test/v1/product/health?mods=100"), env);
    expect(response.status).toBe(400);
    expect(INSTALL_ID).toMatch(/-/);
  });
});

describe("rising by real installs", () => {
  const installed = (mod: number) => rec({ type: "mod_action", mod_id: mod, action: "installed" });

  /** `installs` installs of `mod`, each first sending on `day` of the epoch at `base` and once more. */
  async function installsOn(base: number, mod: number, installs: number, day: (n: number) => number, net = 0): Promise<void> {
    for (let n = 0; n < installs; n += 1) {
      const hash = await installHash(crypto.randomUUID(), SALT);
      const network = `10.${60 + net + (n % 6)}.0.${n + 1}`;
      const first = day(n);
      await contribute(env.DB, KEY, network, hash, [installed(mod)], base + first * DAY + 3_600_000);
      await contribute(env.DB, KEY, network, hash, [], base + Math.min(6, first + 1) * DAY + 3_600_000);
    }
  }

  it("is empty until a closed epoch has data", async () => {
    const response = await get("/v1/product/rising");
    expect(response.status).toBe(200);
    expect(await readJson(response)).toEqual({ mods: [] });
    // The open epoch alone is never read.
    await installsOn(epochOf(Date.now()) * EPOCH_MS, 500, 12, (n) => n % 3);
    await closeEpochs(env.DB, Date.now());
    expect(await rising(env.DB, KEY)).toEqual([]);
  });

  it("ranks the newest closed week against the week before, behind the floor", async () => {
    const week = (epochOf(Date.now()) - 2) * EPOCH_MS;
    const before = week - EPOCH_MS;
    // 600 grows: 3 installs the week before (below the floor, so 0), 14 in the newest week.
    await installsOn(before, 600, 3, (n) => n % 3);
    await installsOn(week, 600, 14, (n) => n % 3);
    // 700 is steady: 12 then 12. 800 is under the floor in the newest week.
    await installsOn(before, 700, 12, (n) => n % 3, 10);
    await installsOn(week, 700, 12, (n) => n % 3, 10);
    await installsOn(week, 800, 9, (n) => n % 3, 20);
    await closeEpochs(env.DB, Date.now());
    const response = await get("/v1/product/rising");
    const text = await response.text();
    expect(text).not.toMatch(HEX_ID);
    expect(JSON.parse(text)).toEqual({ mods: [{ modId: 600, recent: 14 }, { modId: 700, recent: 12 }] });
  });

  it("publishes nothing from one network over many weeks", async () => {
    for (let k = 1; k <= 6; k += 1) {
      const base = (epochOf(Date.now()) - k) * EPOCH_MS;
      for (let n = 0; n < 30; n += 1) {
        const hash = await installHash(crypto.randomUUID(), SALT);
        for (const day of [n % 3, (n % 3) + 1]) {
          await contribute(env.DB, KEY, `198.51.100.${n + 1}`, hash, [installed(900), SET, FAIL], base + day * DAY + 3_600_000);
        }
      }
    }
    await closeEpochs(env.DB, Date.now());
    expect(await rising(env.DB, KEY)).toEqual([]);
    expect(await modHealth(env.DB, KEY, [100, 900], Date.now())).toEqual([]);
    expect((await env.DB.prepare("SELECT COUNT(*) AS n FROM product_totals").first<{ n: number }>())?.n).toBe(0);
  });

  it("answers 503 without its secrets", async () => {
    const response = await get("/v1/product/rising", { PRODUCT_BUCKET_KEY: undefined });
    expect(response.status).toBe(503);
  });
});
