/**
 * The community crosshair library (ADR 0019): the config validator and its seeds, the name rules
 * and blocklist, duplicates, limits, delete tokens, use and report dedupe, auto-hide, hidden rows
 * in lists and cursors, the admin routes and retention.
 */

import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import worker from "../src/index";
import { canonicalText, configHash, parseConfig, type CrosshairConfig } from "../src/crosshairs/config";
import { blocklistForms, isBlocked, parseName } from "../src/crosshairs/name";
import { MAX_ROWS, PAGE_SIZE, PUBLISHES_PER_DAY } from "../src/crosshairs/routes";
import { runRetention } from "../src/retention";
import { call, clearTables, readJson, VERSION } from "./helpers";

const ADMIN = "test-only-admin-key-0123456789abcdef-xyz";
const DAY = 24 * 60 * 60 * 1000;

const BASE: CrosshairConfig = {
  pipGap: 5,
  pipGapStatic: true,
  pipHeight: 10,
  pipWidth: 2,
  pipOpacity: 1,
  pipOutlineBorder: 1,
  pipOutlineGap: 0,
  pipOutlineOpacity: 1,
  dotOpacity: 0,
  dotSize: 8,
  dotOutlineBorder: 2,
  dotOutlineGap: 0,
  dotOutlineOpacity: 0,
  colorR: 0,
  colorG: 255,
  colorB: 0,
  outlineColorR: 0,
  outlineColorG: 0,
  outlineColorB: 0,
};

/** A config no seed has: a distinct colour per `n`. */
function config(n: number, extra: Partial<Record<keyof CrosshairConfig, unknown>> = {}): Record<string, unknown> {
  return { ...BASE, colorR: n % 256, colorG: Math.floor(n / 256) % 256, colorB: 77, ...extra };
}

let net = 0;
/** A fresh /24 per call, so the per-network limit never leaks between tests. */
function freshIp(): string {
  net += 1;
  return `198.${18 + Math.floor(net / 250)}.${net % 250}.9`;
}

async function publish(body: unknown, ip = freshIp()): Promise<Response> {
  return call("/v1/crosshairs", { body, ip });
}

async function published(n: number): Promise<{ id: string; delete_token: string }> {
  const response = await publish({ name: `Aim ${n}`, config: config(n) });
  expect(response.status).toBe(201);
  return readJson(response);
}

async function get(path: string, extra: Record<string, unknown> = {}, headers: Record<string, string> = {}): Promise<Response> {
  return worker.fetch(
    new Request(`http://telemetry.test${path}`, {
      headers: { "x-sigil-client": "1", "x-sigil-version": VERSION, "cf-connecting-ip": freshIp(), ...headers },
    }),
    { ...env, ...extra } as typeof env,
  );
}

async function post(path: string, body: unknown, extra: Record<string, unknown> = {}, headers: Record<string, string> = {}): Promise<Response> {
  return worker.fetch(
    new Request(`http://telemetry.test${path}`, {
      method: "POST",
      headers: { "x-sigil-client": "1", "x-sigil-version": VERSION, "cf-connecting-ip": freshIp(), "content-type": "application/json", ...headers },
      body: JSON.stringify(body),
    }),
    { ...env, ...extra } as typeof env,
  );
}

type Listed = { id: string; name: string; config: Record<string, unknown>; uses: number; official: boolean; created_day: string };
type Page = { crosshairs: Listed[]; next_cursor: string | null };

async function listAll(sort: "used" | "new"): Promise<Listed[]> {
  const out: Listed[] = [];
  let cursor: string | null = null;
  for (let i = 0; i < 1000; i += 1) {
    const response = await get(`/v1/crosshairs?sort=${sort}${cursor === null ? "" : `&cursor=${cursor}`}`);
    expect(response.status).toBe(200);
    const page = await readJson<Page>(response);
    out.push(...page.crosshairs);
    cursor = page.next_cursor;
    if (cursor === null) break;
  }
  return out;
}

async function row(id: string): Promise<{ uses: number; reports: number; hidden: number } | null> {
  return env.DB.prepare("SELECT uses, reports, hidden FROM crosshairs WHERE id = ?1").bind(id).first();
}

beforeEach(async () => {
  await clearTables();
  await env.DB.prepare("DELETE FROM crosshairs WHERE id LIKE 'sigil-test-%'").run();
});

describe("config validation", () => {
  it("accepts every seeded crosshair and stores it canonical, with its hash", async () => {
    const { results } = await env.DB.prepare("SELECT id, name, config, config_hash, official, delete_token_hash FROM crosshairs ORDER BY id").all<{
      id: string;
      name: string;
      config: string;
      config_hash: string;
      official: number;
      delete_token_hash: string | null;
    }>();
    expect(results.length).toBeGreaterThanOrEqual(18);
    for (const seed of results) {
      const parsed = parseConfig(JSON.parse(seed.config));
      expect(parsed.ok, seed.id).toBe(true);
      if (!parsed.ok) continue;
      expect(canonicalText(parsed.config)).toBe(seed.config);
      expect(await configHash(parsed.config)).toBe(seed.config_hash);
      expect(parseName(seed.name)).toEqual({ ok: true, name: seed.name });
      expect(seed.official).toBe(1);
      expect(seed.delete_token_hash).toBeNull();
      expect(seed.id).toMatch(/^sigil-[a-z0-9-]+$/);
    }
  });

  it("rounds opacity to hundredths and drops the hero toggle", () => {
    const parsed = parseConfig({ ...BASE, pipOpacity: 0.7000000001, dotOpacity: 0.333, disableHeroSpecificCrosshairs: true });
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.config.pipOpacity).toBe(0.7);
    expect(parsed.config.dotOpacity).toBe(0.33);
    expect(canonicalText(parsed.config)).not.toContain("disableHero");
    // Key order in the input does not matter.
    const reversed = Object.fromEntries(Object.entries(BASE).reverse());
    const again = parseConfig(reversed);
    expect(again.ok && canonicalText(again.config)).toBe(canonicalText(BASE));
  });

  it("rejects out of range values, unknown keys, missing keys and wrong types", () => {
    const bad: [Record<string, unknown>, string][] = [
      [{ ...BASE, pipGap: -1 }, "pipGap"],
      [{ ...BASE, pipGap: 65 }, "pipGap"],
      [{ ...BASE, pipOutlineBorder: 17 }, "pipOutlineBorder"],
      [{ ...BASE, dotOutlineGap: 17 }, "dotOutlineGap"],
      [{ ...BASE, colorB: 256 }, "colorB"],
      [{ ...BASE, outlineColorR: -1 }, "outlineColorR"],
      [{ ...BASE, pipOpacity: 1.01 }, "pipOpacity"],
      [{ ...BASE, dotOutlineOpacity: -0.1 }, "dotOutlineOpacity"],
      [{ ...BASE, pipWidth: 2.5 }, "pipWidth"],
      [{ ...BASE, pipHeight: "10" }, "pipHeight"],
      [{ ...BASE, pipGapStatic: 1 }, "pipGapStatic"],
      [{ ...BASE, pipGapStatic: "true" }, "pipGapStatic"],
      [{ ...BASE, colorR: null }, "colorR"],
      [{ ...BASE, colorG: [255] }, "colorG"],
      [{ ...BASE, themed: false }, "themed"],
      [{ ...BASE, pip_gap: 5 }, "pip_gap"],
      [{ ...BASE, disableHeroSpecificCrosshairs: "yes" }, "disableHeroSpecificCrosshairs"],
    ];
    for (const [value, field] of bad) expect(parseConfig(value)).toEqual({ ok: false, field });
    const { dotSize: _dropped, ...missing } = BASE;
    expect(parseConfig(missing)).toEqual({ ok: false, field: "dotSize" });
    for (const value of [null, [], "x", 5, true]) expect(parseConfig(value)).toEqual({ ok: false, field: "config" });
  });

  it("answers 400 bad_config with the field, and 413 for an oversized body", async () => {
    const response = await publish({ name: "Fine Name", config: config(1, { dotSize: 99 }) });
    expect(response.status).toBe(400);
    expect(await readJson(response)).toEqual({ error: "bad_config", field: "dotSize" });
    const extra = await publish({ name: "Fine Name", config: config(1), owner: "me" });
    expect(await readJson(extra)).toEqual({ error: "bad_body" });
    const big = await publish({ name: "Fine Name", config: config(1), pad: "x".repeat(3000) });
    expect(big.status).toBe(413);
    expect((await call("/v1/crosshairs", { rawBody: "{not json" })).status).toBe(400);
  });
});

describe("names", () => {
  it("trims, collapses spaces and accepts letters with accents", () => {
    expect(parseName("  Héllo   Wörld  ")).toEqual({ ok: true, name: "Héllo Wörld" });
    expect(parseName("Łódź-Aim_2 'v'!+")).toEqual({ ok: true, name: "Łódź-Aim_2 'v'!+" });
    expect(parseName("Dr. Aim")).toEqual({ ok: true, name: "Dr. Aim" });
    expect(parseName("x".repeat(32))).toEqual({ ok: true, name: "x".repeat(32) });
    expect(parseName("42")).toEqual({ ok: true, name: "42" });
  });

  it("rejects bad lengths, other characters, links and names without a letter or digit", () => {
    for (const name of ["a", " a ", "x".repeat(33), "", "...", "-_-", "http://x", "x.com", "discord.gg invite", "me@x", "a/b", "a:b", "emoji 🎯", "tab\u0000x", "Привет", 5, null]) {
      expect(parseName(name), String(name)).toEqual({ ok: false, error: "bad_name" });
    }
  });

  it("blocks slurs through case, accents, spacing, repeats and digit swaps", () => {
    for (const name of ["Faggot", "f4gg0t", "N1GGER aim", "níggêr", "big fag", "F A G", "r3t4rd", "Faaaggot", "sp1c", "c00n"]) {
      expect(isBlocked(name), name).toBe(true);
      expect(parseName(name)).toEqual({ ok: false, error: "blocked_name" });
    }
    for (const name of ["Niger River", "Spice Aim", "Cocoon", "Fagin", "Raccoon Dot", "Shitake 3"]) {
      expect(isBlocked(name), name).toBe(false);
    }
    expect(blocklistForms("Ä1")).toEqual(["ai", "al"]);
  });
});

describe("publishing", () => {
  it("returns an id and a 43 character token, stores only the token's hash, and lists the row", async () => {
    const response = await publish({ name: "  My   Aim ", config: config(10, { pipOpacity: 0.456 }) });
    expect(response.status).toBe(201);
    const body = await readJson<{ id: string; delete_token: string }>(response);
    expect(Object.keys(body).sort()).toEqual(["delete_token", "id"]);
    expect(body.id).toMatch(/^[0-9a-f]{16}$/);
    expect(body.delete_token).toMatch(/^[A-Za-z0-9_-]{43}$/);
    const stored = await env.DB.prepare("SELECT * FROM crosshairs WHERE id = ?1").bind(body.id).first<Record<string, unknown>>();
    expect(stored?.name).toBe("My Aim");
    expect(JSON.stringify(stored)).not.toContain(body.delete_token);
    expect(String(stored?.config)).toContain('"pipOpacity":0.46');
    expect(Object.keys(stored ?? {}).sort()).toEqual(
      ["config", "config_hash", "created_at", "created_day", "delete_token_hash", "hidden", "id", "name", "official", "reports", "uses"].sort(),
    );
    const listed = (await listAll("new")).find((entry) => entry.id === body.id);
    expect(listed).toMatchObject({ name: "My Aim", uses: 0, official: false });
    expect(listed?.created_day).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("answers 409 with the existing id for a config already in the library", async () => {
    const first = await published(20);
    // Same look: different key order, opacity off by less than the rounding, the hero toggle.
    const again = await publish({ name: "Another Name", config: { ...config(20), pipOpacity: 0.999, disableHeroSpecificCrosshairs: true } });
    expect(again.status).toBe(409);
    expect(await readJson(again)).toEqual({ error: "duplicate", id: first.id });
    const seed = await publish({ name: "Copy", config: BASE });
    expect(await readJson(seed)).toEqual({ error: "duplicate", id: "sigil-default" });
  });

  it("allows 3 publishes per network per day", async () => {
    const ip = "203.0.113.10";
    for (let n = 0; n < 3; n += 1) expect((await publish({ name: `Mine ${n}`, config: config(100 + n) }, ip)).status).toBe(201);
    const fourth = await publish({ name: "Mine 4", config: config(104) }, "203.0.113.200");
    expect(fourth.status).toBe(429);
    expect(await readJson(fourth)).toEqual({ error: "publish_limit" });
    expect((await publish({ name: "Other", config: config(105) })).status).toBe(201);
  });

  it("allows 300 publishes a day overall", async () => {
    const day = new Date().toISOString().slice(0, 10);
    await env.DB.prepare(
      `WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < ?1)
       INSERT INTO crosshair_publishes (bucket, day, created_at) SELECT 'b' || x, ?2, ?3 FROM n`,
    )
      .bind(PUBLISHES_PER_DAY, day, Date.now())
      .run();
    const response = await publish({ name: "Late", config: config(200) });
    expect(response.status).toBe(429);
    expect(await readJson(response)).toEqual({ error: "daily_limit" });
  });

  it("stops at 20,000 rows", async () => {
    await env.DB.prepare(
      `WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < ?1)
       INSERT INTO crosshairs (id, name, config, config_hash, created_at, created_day)
       SELECT printf('%016x', x), 'Filler', '{}', 'h' || x, 1, '2026-01-01' FROM n`,
    )
      .bind(MAX_ROWS)
      .run();
    const response = await publish({ name: "Full", config: config(300) });
    expect(response.status).toBe(507);
    expect(await readJson(response)).toEqual({ error: "library_full" });
  });

  it("fails closed without the bucket key", async () => {
    for (const key of [undefined, "short"]) {
      const response = await post("/v1/crosshairs", { name: "Fine", config: config(400) }, { PRODUCT_BUCKET_KEY: key });
      expect(response.status).toBe(503);
      expect(await readJson(response)).toEqual({ error: "not_configured" });
      for (const action of ["use", "report"]) {
        expect((await post(`/v1/crosshairs/sigil-dot/${action}`, {}, { PRODUCT_BUCKET_KEY: key })).status).toBe(503);
      }
    }
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM crosshairs WHERE official = 0").first("n")).toBe(0);
  });

  it("needs the client headers", async () => {
    const response = await call("/v1/crosshairs", { body: { name: "Fine", config: config(401) }, headers: { "x-sigil-client": null } });
    expect(response.status).toBe(400);
  });
});

describe("deleting", () => {
  it("removes the row with the right token and refuses a wrong one", async () => {
    const mine = await published(500);
    const wrong = await call(`/v1/crosshairs/${mine.id}/delete`, { body: { delete_token: "A".repeat(43) } });
    expect(wrong.status).toBe(403);
    expect(await readJson(wrong)).toEqual({ error: "wrong_token" });
    expect((await call(`/v1/crosshairs/${mine.id}/delete`, { body: { delete_token: "short" } })).status).toBe(400);
    expect((await call(`/v1/crosshairs/${mine.id}/delete`, { body: {} })).status).toBe(400);
    expect(await row(mine.id)).not.toBeNull();
    await call(`/v1/crosshairs/${mine.id}/use`);

    const ok = await call(`/v1/crosshairs/${mine.id}/delete`, { body: { delete_token: mine.delete_token } });
    expect(ok.status).toBe(200);
    expect(await row(mine.id)).toBeNull();
    expect(await env.DB.prepare("SELECT COUNT(*) AS n FROM crosshair_uses WHERE crosshair_id = ?1").bind(mine.id).first("n")).toBe(0);
    expect((await call(`/v1/crosshairs/${mine.id}/delete`, { body: { delete_token: mine.delete_token } })).status).toBe(404);
  });

  it("never deletes one of Sigil's own crosshairs", async () => {
    const response = await call("/v1/crosshairs/sigil-dot/delete", { body: { delete_token: "A".repeat(43) } });
    expect(response.status).toBe(403);
    expect(await row("sigil-dot")).not.toBeNull();
  });
});

describe("uses and reports", () => {
  it("counts a use once per network per crosshair", async () => {
    const mine = await published(600);
    const ip = "203.0.113.50";
    expect(await readJson(await call(`/v1/crosshairs/${mine.id}/use`, { ip }))).toEqual({ counted: true });
    expect(await readJson(await call(`/v1/crosshairs/${mine.id}/use`, { ip: "203.0.113.51" }))).toEqual({ counted: false });
    expect(await readJson(await call(`/v1/crosshairs/${mine.id}/use`, { ip: "203.0.114.50" }))).toEqual({ counted: true });
    expect((await row(mine.id))?.uses).toBe(2);
    expect((await call("/v1/crosshairs/0123456789abcdef/use")).status).toBe(404);
  });

  it("counts a report once per network and hides the row at 3 networks", async () => {
    const mine = await published(700);
    expect(await readJson(await call(`/v1/crosshairs/${mine.id}/report`, { ip: "203.0.113.60" }))).toEqual({ counted: true });
    expect(await readJson(await call(`/v1/crosshairs/${mine.id}/report`, { ip: "203.0.113.61" }))).toEqual({ counted: false });
    await call(`/v1/crosshairs/${mine.id}/report`, { ip: "203.0.114.60" });
    expect(await row(mine.id)).toMatchObject({ reports: 2, hidden: 0 });
    await call(`/v1/crosshairs/${mine.id}/report`, { ip: "203.0.115.60" });
    expect(await row(mine.id)).toMatchObject({ reports: 3, hidden: 1 });

    expect((await listAll("used")).some((entry) => entry.id === mine.id)).toBe(false);
    expect((await listAll("new")).some((entry) => entry.id === mine.id)).toBe(false);
    expect((await call(`/v1/crosshairs/${mine.id}/use`)).status).toBe(404);
    expect((await call(`/v1/crosshairs/${mine.id}/report`)).status).toBe(404);
  });

  it("does not hide Sigil's own crosshairs on reports", async () => {
    for (let n = 0; n < 4; n += 1) await call("/v1/crosshairs/sigil-dot/report", { ip: `203.0.${120 + n}.1` });
    expect(await row("sigil-dot")).toMatchObject({ reports: 0, hidden: 0 });
  });

  it("keeps the dedupe rows 2 days, and keeps the counts", async () => {
    const mine = await published(800);
    await call(`/v1/crosshairs/${mine.id}/use`);
    await call(`/v1/crosshairs/${mine.id}/report`);
    await runRetention(env.DB, Date.now() + DAY);
    const count = async (table: string) => env.DB.prepare(`SELECT COUNT(*) AS n FROM ${table}`).first<number>("n");
    expect(await count("crosshair_uses")).toBe(1);
    await runRetention(env.DB, Date.now() + 2 * DAY + 60_000);
    for (const table of ["crosshair_uses", "crosshair_reports", "crosshair_publishes"]) expect(await count(table)).toBe(0);
    expect(await row(mine.id)).toMatchObject({ uses: 1, reports: 1 });
  });
});

describe("listing", () => {
  it("pages 60 at a time in a stable order, cacheable, with only the public fields", async () => {
    for (let n = 0; n < 70; n += 1) await env.DB.prepare(
      `INSERT INTO crosshairs (id, name, config, config_hash, uses, created_at, created_day)
       VALUES (printf('%016x', ?1 + 4096), 'Row ' || ?1, ?2, 'hash' || ?1, ?1 % 5, ?1, '2026-11-25')`,
    )
      .bind(n, canonicalText({ ...BASE, colorR: n }))
      .run();

    const first = await get("/v1/crosshairs");
    expect(first.headers.get("cache-control")).toBe("public, max-age=300");
    const page = await readJson<Page>(first);
    expect(page.crosshairs).toHaveLength(PAGE_SIZE);
    expect(Object.keys(page.crosshairs[0] ?? {}).sort()).toEqual(["config", "created_day", "id", "name", "official", "uses"]);
    expect(page.next_cursor).not.toBeNull();

    for (const sort of ["used", "new"] as const) {
      const all = await listAll(sort);
      const seeds = await env.DB.prepare("SELECT COUNT(*) AS n FROM crosshairs WHERE official = 1").first<number>("n");
      expect(all).toHaveLength(70 + (seeds ?? 0));
      expect(new Set(all.map((entry) => entry.id)).size).toBe(all.length);
    }
    const used = await listAll("used");
    for (let i = 1; i < used.length; i += 1) {
      const [a, b] = [used[i - 1], used[i]];
      if (a === undefined || b === undefined) continue;
      expect(a.uses > b.uses || (a.uses === b.uses && a.id < b.id)).toBe(true);
    }
  });

  it("never reaches a hidden row, even with a cursor built to point at it", async () => {
    const a = await published(900);
    const b = await published(901);
    await env.DB.prepare("UPDATE crosshairs SET hidden = 1 WHERE id = ?1").bind(a.id).run();
    const ids = (await listAll("new")).map((entry) => entry.id);
    expect(ids).toContain(b.id);
    expect(ids).not.toContain(a.id);

    const hidden = await env.DB.prepare("SELECT uses, created_at FROM crosshairs WHERE id = ?1").bind(a.id).first<{ uses: number; created_at: number }>();
    const forge = (value: unknown) => btoa(JSON.stringify(value)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    const cursors = [
      { sort: "new", cursor: forge({ s: "new", t: (hidden?.created_at ?? 0) + 1, i: a.id }) },
      { sort: "new", cursor: forge({ s: "new", t: hidden?.created_at ?? 0, i: "ffffffffffffffff" }) },
      { sort: "used", cursor: forge({ s: "used", u: (hidden?.uses ?? 0) + 1, i: a.id }) },
      { sort: "used", cursor: forge({ s: "used", u: hidden?.uses ?? 0, i: "0000000000000000" }) },
    ];
    for (const { sort, cursor } of cursors) {
      const page = await readJson<Page>(await get(`/v1/crosshairs?sort=${sort}&cursor=${cursor}`));
      expect(page.crosshairs.map((entry) => entry.id)).not.toContain(a.id);
    }
  });

  it("refuses a bad sort or cursor", async () => {
    expect((await get("/v1/crosshairs?sort=top")).status).toBe(400);
    const forge = (value: unknown) => btoa(JSON.stringify(value)).replace(/=+$/, "");
    for (const cursor of ["!!", "bm90IGpzb24", forge({ s: "new", t: 1, i: "x" }), forge({ s: "used", u: -1, i: "0123456789abcdef" }), forge({ s: "new", u: 1, i: "0123456789abcdef" })]) {
      expect((await get(`/v1/crosshairs?sort=used&cursor=${cursor}`)).status, cursor).toBe(400);
    }
  });
});

describe("admin", () => {
  const auth = { authorization: `Bearer ${ADMIN}` };
  const withKey = { PRODUCT_ADMIN_KEY: ADMIN };

  it("refuses every admin route without the key, with a short key, or with a wrong one", async () => {
    const routes: [string, "GET" | "POST"][] = [
      ["/v1/crosshairs/admin/flagged", "GET"],
      ["/v1/crosshairs/admin/sigil-dot/remove", "POST"],
      ["/v1/crosshairs/admin/sigil-dot/restore", "POST"],
      ["/v1/crosshairs/admin/official", "POST"],
    ];
    for (const [path, method] of routes) {
      const send = (extra: Record<string, unknown>, headers: Record<string, string>) =>
        method === "GET" ? get(path, extra, headers) : post(path, { id: "sigil-test-x", name: "X Aim", config: config(1) }, extra, headers);
      expect((await send({}, auth)).status, path).toBe(503);
      expect((await send({ PRODUCT_ADMIN_KEY: "short" }, { authorization: "Bearer short" })).status, path).toBe(503);
      expect((await send(withKey, { authorization: "Bearer wrong-key" })).status, path).toBe(401);
      expect((await send(withKey, {})).status, path).toBe(401);
    }
    expect(await row("sigil-dot")).not.toBeNull();
    expect(await row("sigil-test-x")).toBeNull();
  });

  it("lists flagged rows, restores and removes them", async () => {
    const mine = await published(1000);
    for (let n = 0; n < 3; n += 1) await call(`/v1/crosshairs/${mine.id}/report`, { ip: `203.0.${130 + n}.1` });
    const flagged = await readJson<{ crosshairs: { id: string; reports: number; hidden: boolean }[] }>(await get("/v1/crosshairs/admin/flagged", withKey, auth));
    expect(flagged.crosshairs).toEqual([expect.objectContaining({ id: mine.id, reports: 3, hidden: true })]);

    expect((await post(`/v1/crosshairs/admin/${mine.id}/restore`, {}, withKey, auth)).status).toBe(200);
    expect(await row(mine.id)).toMatchObject({ reports: 0, hidden: 0 });
    expect((await listAll("new")).some((entry) => entry.id === mine.id)).toBe(true);

    expect((await post(`/v1/crosshairs/admin/${mine.id}/remove`, {}, withKey, auth)).status).toBe(200);
    expect(await row(mine.id)).toBeNull();
    expect((await post(`/v1/crosshairs/admin/${mine.id}/remove`, {}, withKey, auth)).status).toBe(404);
  });

  it("upserts an official crosshair through the same validator", async () => {
    const make = (body: unknown) => post("/v1/crosshairs/admin/official", body, withKey, auth);
    expect((await make({ id: "sigil-test-one", name: "Test One", config: config(1100) })).status).toBe(200);
    expect(await readJson(await make({ id: "sigil-test-one", name: "Test One B", config: config(1101) }))).toEqual({ id: "sigil-test-one" });
    const stored = await env.DB.prepare("SELECT name, official FROM crosshairs WHERE id = 'sigil-test-one'").first();
    expect(stored).toEqual({ name: "Test One B", official: 1 });
    expect((await make({ id: "0123456789abcdef", name: "Test", config: config(1102) })).status).toBe(400);
    expect((await make({ id: "sigil-test-two", name: "Test", config: config(1102, { pipGap: 99 }) })).status).toBe(400);
    const dup = await make({ id: "sigil-test-two", name: "Test", config: BASE });
    expect(dup.status).toBe(409);
    expect(await readJson(dup)).toEqual({ error: "duplicate", id: "sigil-default" });
  });
});
