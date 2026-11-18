import { env } from "cloudflare:workers";
import { createScheduledController } from "cloudflare:test";
import { beforeEach, describe, expect, it } from "vitest";
import worker from "../src/index";
import { call, clearTables, eventsBody, INSTALL_ID, readJson, VERSION } from "./helpers";

const DAY_MS = 24 * 60 * 60 * 1000;

type ErrorBody = { readonly error: string };

type CodeRow = { readonly code: string };
type FingerprintRow = { readonly fingerprint: string };

async function insertDiagnostics(createdAt: number, code: string): Promise<void> {
  await env.DB.prepare(
    `INSERT INTO diagnostics (created_at, code, version, os, summary, log)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)`,
  )
    .bind(createdAt, code, VERSION, "linux", "old bundle", "log")
    .run();
}

async function insertCrash(createdAt: number, fingerprint: string): Promise<void> {
  await env.DB.prepare(
    `INSERT INTO crashes (created_at, version, os, message, backtrace, fingerprint, repeats, last_seen_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?1)`,
  )
    .bind(createdAt, VERSION, "macos", "Error: old", "", fingerprint)
    .run();
}

beforeEach(clearTables);

describe("retention", () => {
  it("deletes diagnostics after 30 days and crashes after 90", async () => {
    const now = Date.now();
    await insertDiagnostics(now - 31 * DAY_MS, "SGL-2222-AAAA");
    await insertDiagnostics(now - 29 * DAY_MS, "SGL-2222-BBBB");
    await insertCrash(now - 91 * DAY_MS, "old-crash");
    await insertCrash(now - 89 * DAY_MS, "new-crash");

    await worker.scheduled(createScheduledController({ cron: "17 3 * * *" }), env);

    const diagnostics = await env.DB.prepare("SELECT code FROM diagnostics ORDER BY code").all<CodeRow>();
    expect(diagnostics.results.map((row) => row.code)).toEqual(["SGL-2222-BBBB"]);

    const crashes = await env.DB.prepare("SELECT fingerprint FROM crashes ORDER BY fingerprint").all<FingerprintRow>();
    expect(crashes.results.map((row) => row.fingerprint)).toEqual(["new-crash"]);
  });

  it("runs on an empty database", async () => {
    await worker.scheduled(createScheduledController({ cron: "17 3 * * *" }), env);

    const diagnostics = await env.DB.prepare("SELECT COUNT(*) AS rows FROM diagnostics").first<{ rows: number }>();
    const crashes = await env.DB.prepare("SELECT COUNT(*) AS rows FROM crashes").first<{ rows: number }>();
    expect(diagnostics?.rows).toBe(0);
    expect(crashes?.rows).toBe(0);
  });
});

describe("rate limiting", () => {
  it("allows 60 requests a minute from one caller and then answers 429", async () => {
    const ip = "198.51.100.200";
    const statuses: number[] = [];

    for (let i = 0; i < 61; i += 1) {
      const response = await call("/v1/events", {
        body: eventsBody([{ name: "page:browse", ts: Date.now() }], { installId: INSTALL_ID }),
        ip,
      });
      statuses.push(response.status);
    }

    expect(statuses.slice(0, 60)).toEqual(Array.from({ length: 60 }, () => 202));
    expect(statuses[60]).toBe(429);

    const blocked = await call("/v1/events", {
      body: eventsBody([{ name: "page:browse", ts: Date.now() }]),
      ip,
    });
    expect(blocked.status).toBe(429);
    expect((await readJson<ErrorBody>(blocked)).error).toBe("rate_limited");

    // A second caller is not punished for the first one's burst.
    const neighbour = await call("/v1/events", {
      body: eventsBody([{ name: "page:browse", ts: Date.now() }]),
      ip: "198.51.100.201",
    });
    expect(neighbour.status).toBe(202);
  });
});
