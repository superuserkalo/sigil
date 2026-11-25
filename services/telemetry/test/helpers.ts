import { env } from "cloudflare:workers";
import worker from "../src/index";

export const INSTALL_ID = "3f2504e0-4f89-41d3-9a0c-0305e82c3301";
export const VERSION = "0.1.0";

const BASE = "http://telemetry.test";

let address = 0;

/**
 * A fresh caller address per call. The Rate Limiting binding is real in these tests, so a shared
 * address would let one test exhaust the allowance for the next.
 */
function nextAddress(): string {
  address += 1;
  if (address > 250) address = 1;
  return `192.0.2.${address}`;
}

export type CallOptions = {
  readonly method?: "GET" | "POST";
  readonly body?: unknown;
  readonly rawBody?: string;
  /** A `null` value removes the default header, which is how the header tests omit one. */
  readonly headers?: Readonly<Record<string, string | null>>;
  readonly ip?: string;
};

export async function call(path: string, options: CallOptions = {}): Promise<Response> {
  const method = options.method ?? "POST";
  const headers = new Headers();
  headers.set("x-sigil-client", "1");
  headers.set("x-sigil-version", VERSION);
  headers.set("cf-connecting-ip", options.ip ?? nextAddress());

  for (const [name, value] of Object.entries(options.headers ?? {})) {
    if (value === null) headers.delete(name);
    else headers.set(name, value);
  }

  const init: RequestInit = { method, headers };
  if (method === "POST") {
    headers.set("content-type", "application/json");
    init.body = options.rawBody ?? JSON.stringify(options.body ?? {});
  }

  return worker.fetch(new Request(`${BASE}${path}`, init), env);
}

export async function readJson<T>(response: Response): Promise<T> {
  return JSON.parse(await response.text()) as T;
}

/**
 * A request whose body arrives as a stream, so it carries no `content-length`. This is how the
 * tests prove the Worker stops reading at the cap instead of buffering whatever a client sends.
 */
export async function callStream(
  path: string,
  chunks: readonly Uint8Array[],
  ip = nextAddress(),
): Promise<Response> {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk);
      controller.close();
    },
  });

  const headers = new Headers({
    "x-sigil-client": "1",
    "x-sigil-version": VERSION,
    "cf-connecting-ip": ip,
    "content-type": "application/json",
  });

  return worker.fetch(
    new Request(`${BASE}${path}`, { method: "POST", headers, body: stream }),
    env,
  );
}

export function eventsBody(
  events: readonly unknown[],
  extra: Readonly<Record<string, unknown>> = {},
): Record<string, unknown> {
  return { installId: INSTALL_ID, version: VERSION, os: "macos", events, ...extra };
}

export const SAMPLE_HASH = "9f2c8d1b7a6e5f40312c8d1b7a6e5f40312c8d1b7a6e5f40312c8d1b7a6e5f40";

/**
 * Storage is isolated per test file, not per test, so each test starts from empty tables. The
 * schema itself is applied once per file by `test/apply-migrations.ts`.
 */
export async function clearTables(): Promise<void> {
  await env.DB.batch([
    env.DB.prepare("DELETE FROM crashes"),
    env.DB.prepare("DELETE FROM diagnostics"),
    env.DB.prepare("DELETE FROM mod_hashes"),
    env.DB.prepare("DELETE FROM hash_contributors"),
    env.DB.prepare("DELETE FROM product_events"),
    env.DB.prepare("DELETE FROM product_contrib"),
    env.DB.prepare("DELETE FROM product_totals"),
    // Sigil's seeded crosshairs stay; their counts and flags go back to the migration's.
    env.DB.prepare("DELETE FROM crosshairs WHERE official = 0"),
    env.DB.prepare("UPDATE crosshairs SET uses = 0, reports = 0, hidden = 0"),
    env.DB.prepare("DELETE FROM crosshair_uses"),
    env.DB.prepare("DELETE FROM crosshair_reports"),
    env.DB.prepare("DELETE FROM crosshair_daily"),
  ]);
}
