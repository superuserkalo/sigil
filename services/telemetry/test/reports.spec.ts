import { env } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { CODE_PATTERN } from "../src/codes";
import { call, clearTables, INSTALL_ID, readJson, VERSION } from "./helpers";

type ErrorBody = { readonly error: string };
type CrashResponse = { readonly repeats: number };
type DiagnosticsResponse = { readonly code: string };

type CrashRow = {
  readonly message: string;
  readonly backtrace: string;
  readonly repeats: number;
  readonly version: string;
  readonly os: string;
};

type DiagnosticsRow = {
  readonly code: string;
  readonly summary: string;
  readonly log: string;
};

function crashBody(message: string, backtrace: string): Record<string, unknown> {
  return { installId: INSTALL_ID, version: VERSION, os: "windows", message, backtrace };
}

function diagnosticsBody(summary: string, log: string): Record<string, unknown> {
  return { installId: INSTALL_ID, version: VERSION, os: "linux", summary, log };
}

beforeEach(clearTables);

const SECRETS = [
  "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
  "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
  "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
  "76561198012345678",
  "alice@example.com",
];

describe("crash reports", () => {
  it("scrubs home paths and secrets before storing", async () => {
    const message = [
      "panic at /Users/alice/sigil/rust/crates/desktop/src/main.rs:42:9",
      "C:\\Users\\Alice\\AppData\\Local\\sigil\\logs\\main.log",
      ...SECRETS,
    ].join("\n");
    const backtrace = [
      "Error: boom",
      "  at install (/Users/alice/sigil/rust/crates/desktop/src/deploy.rs:120:5)",
      "  at run (0x7ff8ab12cd34)",
    ].join("\n");

    const response = await call("/v1/crash", { body: crashBody(message, backtrace) });
    expect(response.status).toBe(202);
    expect(await readJson<CrashResponse>(response)).toEqual({ repeats: 1 });

    const row = await env.DB.prepare(
      "SELECT message, backtrace, repeats, version, os FROM crashes",
    ).first<CrashRow>();

    expect(row).not.toBeNull();
    const stored = row as CrashRow;
    expect(stored.version).toBe(VERSION);
    expect(stored.os).toBe("windows");
    expect(stored.message).not.toContain("alice");
    expect(stored.message).not.toContain("Alice");
    expect(stored.backtrace).not.toContain("alice");
    expect(stored.message).toContain("/Users/<user>");
    expect(stored.message).toContain("C:\\Users\\<user>");
    for (const secret of SECRETS) expect(stored.message).not.toContain(secret);
    expect(stored.message).toContain("<key>");
    expect(stored.message).toContain("<steamid64>");
    expect(stored.message).toContain("<email>");
  });

  it("stores one row per fingerprint and counts repeats", async () => {
    const first = crashBody(
      "Error: material not found",
      "  at load (/Users/alice/game/materials/x.vmat:12:3)\n  at run (0x1234)",
    );
    const second = crashBody(
      "Error: material not found",
      "  at load (/Users/bob/game/materials/x.vmat:99:7)\n  at run (0xdeadbeef)",
    );

    expect((await call("/v1/crash", { body: first })).status).toBe(202);
    const response = await call("/v1/crash", { body: second });
    expect(await readJson<CrashResponse>(response)).toEqual({ repeats: 2 });

    const count = await env.DB.prepare("SELECT COUNT(*) AS rows FROM crashes").first<{ rows: number }>();
    expect(count?.rows).toBe(1);
  });

  it("counts a crash with another message separately", async () => {
    await call("/v1/crash", { body: crashBody("Error: one", "") });
    await call("/v1/crash", { body: crashBody("Error: two", "") });

    const count = await env.DB.prepare("SELECT COUNT(*) AS rows FROM crashes").first<{ rows: number }>();
    expect(count?.rows).toBe(2);
  });

  it("rejects a crash body over 96 KiB", async () => {
    const response = await call("/v1/crash", {
      body: { ...crashBody("Error: huge", "x".repeat(40 * 1024)), context: "y".repeat(60 * 1024) },
    });

    expect(response.status).toBe(413);
    expect((await readJson<ErrorBody>(response)).error).toBe("body_too_large");
  });

  it("rejects an empty message", async () => {
    const response = await call("/v1/crash", { body: crashBody("", "  at run") });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_message");
  });

  it("stores the kind and the scrubbed log tail, keeping the newest tail on a repeat", async () => {
    const first = {
      ...crashBody("Sigil did not exit cleanly (no panic was recorded)", "no panic was recorded\nlast log line: INFO  Opened the 3D view"),
      kind: "unclean_exit",
      context: "last log lines:\nINFO  Library at C:\\Users\\Alice\\Deadlock\nINFO  key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789",
    };
    expect((await call("/v1/crash", { body: first })).status).toBe(202);
    const row = await env.DB.prepare("SELECT kind, context FROM crashes").first<{ kind: string; context: string }>();
    expect(row?.kind).toBe("unclean_exit");
    expect(row?.context).toContain("C:\\Users\\<user>");
    expect(row?.context).not.toContain("Alice");
    expect(row?.context).not.toContain("sk-ant-api03");

    const second = { ...first, context: "last log lines:\nINFO  newer session" };
    expect(await readJson<CrashResponse>(await call("/v1/crash", { body: second }))).toEqual({ repeats: 2 });
    const third = { ...first, context: undefined };
    expect(await readJson<CrashResponse>(await call("/v1/crash", { body: third }))).toEqual({ repeats: 3 });
    const kept = await env.DB.prepare("SELECT context FROM crashes").first<{ context: string }>();
    expect(kept?.context).toContain("newer session");
  });

  it("reads a report without a kind as a panic and rejects an unknown kind", async () => {
    expect((await call("/v1/crash", { body: crashBody("Error: old app", "") })).status).toBe(202);
    const row = await env.DB.prepare("SELECT kind, context FROM crashes").first<{ kind: string; context: string }>();
    expect(row?.kind).toBe("panic");
    expect(row?.context).toBe("");

    const bad = await call("/v1/crash", { body: { ...crashBody("Error: x", ""), kind: "meltdown" } });
    expect(bad.status).toBe(400);
    expect((await readJson<ErrorBody>(bad)).error).toBe("bad_kind");
    const long = await call("/v1/crash", { body: { ...crashBody("Error: x", ""), context: "z".repeat(17 * 1024) } });
    expect((await readJson<ErrorBody>(long)).error).toBe("bad_context");
  });

  it("accepts a crash without a backtrace", async () => {
    const response = await call("/v1/crash", { body: crashBody("Error: alone", "") });

    expect(response.status).toBe(202);
  });
});

describe("diagnostics bundles", () => {
  it("returns a readable support code and stores the scrubbed bundle", async () => {
    const summary = "Install failed for /Users/alice/sigil-cache/pak01_dir.vpk";
    const log = [
      "2026-11-18 12:00:00 [info] starting",
      "home /home/bob/.local/share/sigil",
      "Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
      "token=abcdef123456",
      "C:\\Users\\Alice\\AppData\\Local\\sigil\\logs\\main.log",
      ...SECRETS,
    ].join("\n");

    const response = await call("/v1/diagnostics", { body: diagnosticsBody(summary, log) });
    expect(response.status).toBe(202);

    const { code } = await readJson<DiagnosticsResponse>(response);
    expect(code).toMatch(CODE_PATTERN);

    const row = await env.DB.prepare("SELECT code, summary, log FROM diagnostics").first<DiagnosticsRow>();
    expect(row?.code).toBe(code);
    expect(row?.summary).not.toContain("alice");
    expect(row?.summary).toContain("/Users/<user>");
    expect(row?.log).not.toContain("bob");
    expect(row?.log).not.toContain("Alice");
    for (const secret of SECRETS) expect(row?.log).not.toContain(secret);
    expect(row?.log).toContain("<token>");
    expect(row?.log).toContain("<redacted>");
  });

  it("gives every bundle its own code", async () => {
    const codes = new Set<string>();
    for (let i = 0; i < 5; i += 1) {
      const response = await call("/v1/diagnostics", {
        body: diagnosticsBody(`summary ${i}`, `log ${i}`),
      });
      codes.add((await readJson<DiagnosticsResponse>(response)).code);
    }

    expect(codes.size).toBe(5);
  });

  it("rejects a bundle over 1 MiB", async () => {
    const response = await call("/v1/diagnostics", {
      body: diagnosticsBody("big", "x".repeat(1024 * 1024 + 1024)),
    });

    expect(response.status).toBe(413);
    expect((await readJson<ErrorBody>(response)).error).toBe("body_too_large");
  });

  it("rejects a bundle with no summary", async () => {
    const response = await call("/v1/diagnostics", { body: diagnosticsBody("", "log") });

    expect(response.status).toBe(400);
    expect((await readJson<ErrorBody>(response)).error).toBe("bad_summary");
  });
});
