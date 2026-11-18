/**
 * Request validation for the five endpoints.
 *
 * Every parser takes untrusted JSON and returns either a fully typed value or a short snake_case
 * reason. Reasons are stable words because the app logs them and tests assert on them; they never
 * echo a client value back.
 *
 * Rules that are about size live in `body.ts`; this file is about shape.
 */

import { isPageName } from "./events";

export const MAX_EVENT_COUNT = 50;
export const MAX_SHA256_BYTES = 8 * 1024 * 1024 * 1024;
export const MAX_MESSAGE_CHARS = 8 * 1024;
export const MAX_BACKTRACE_CHARS = 48 * 1024;
export const MAX_SUMMARY_CHARS = 4 * 1024;

const MAX_VERSION_CHARS = 64;
const MAX_INSTALL_ID_CHARS = 36;
const MAX_EVENT_NAME_CHARS = 64;
const MAX_PAGE_CHARS = 32;
const MAX_LOG_CHARS = 1024 * 1024;
const MAX_GAMEBANANA_ID = 1_000_000_000;

const UUID_PATTERN: RegExp =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SEMVER_PATTERN: RegExp =
  /^\d{1,5}\.\d{1,5}\.\d{1,5}(?:-[0-9A-Za-z.-]{1,32})?(?:\+[0-9A-Za-z.-]{1,32})?$/;
const SHA256_PATTERN: RegExp = /^[0-9a-f]{64}$/i;
const SIZE_PATTERN: RegExp = /^\d{1,12}$/;

export type ParseResult<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: string };

export type OsName = "windows" | "macos" | "linux" | "other";

const OS_NAMES: readonly OsName[] = ["windows", "macos", "linux", "other"];

export type EventInput = {
  readonly name: string;
  /** Empty when the app sent no page, or a page name this Worker does not know. */
  readonly page: string;
  /** Milliseconds since the epoch, clamped to the server clock. */
  readonly ts: number;
};

export type EventsBody = {
  readonly installId: string;
  readonly version: string;
  readonly os: OsName;
  readonly events: readonly EventInput[];
};

export type CrashBody = {
  readonly installId: string;
  readonly version: string;
  readonly os: OsName;
  readonly message: string;
  readonly backtrace: string;
};

export type DiagnosticsBody = {
  readonly installId: string;
  readonly version: string;
  readonly os: OsName;
  readonly summary: string;
  readonly log: string;
};

export type HashesBody = {
  readonly installId: string;
  readonly sha256: string;
  readonly size: number;
  readonly gamebananaModId: number;
  readonly fileId: number;
};

export type IdentifyQuery = {
  readonly sha256: string;
  readonly size: number;
};

function ok<T>(value: T): ParseResult<T> {
  return { ok: true, value };
}

function fail<T>(error: string): ParseResult<T> {
  return { ok: false, error };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function boundedString(value: unknown, max: number, min = 1): string | null {
  if (typeof value !== "string") return null;
  if (value.length < min || value.length > max) return null;
  return value;
}

function parseOs(value: unknown): OsName | null {
  if (typeof value !== "string") return null;
  return (OS_NAMES as readonly string[]).includes(value) ? (value as OsName) : null;
}

function parseInstallId(value: unknown): string | null {
  const text = boundedString(value, MAX_INSTALL_ID_CHARS);
  if (text === null || !UUID_PATTERN.test(text)) return null;
  return text.toLowerCase();
}

/** True for a version like `0.4.1`, `1.0.0-beta.2` or `2.1.3+build.7`. */
export function isSemver(value: string): boolean {
  return SEMVER_PATTERN.test(value);
}

function parseVersion(value: unknown): string | null {
  const text = boundedString(value, MAX_VERSION_CHARS);
  if (text === null || !isSemver(text)) return null;
  return text;
}

function parsePositiveInteger(value: unknown, max: number): number | null {
  if (typeof value !== "number") return null;
  if (!Number.isSafeInteger(value) || value < 1 || value > max) return null;
  return value;
}

export function parseEvents(raw: unknown, now: number): ParseResult<EventsBody> {
  if (!isRecord(raw)) return fail("bad_body");

  const installId = parseInstallId(raw.installId);
  if (installId === null) return fail("bad_install_id");

  const version = parseVersion(raw.version);
  if (version === null) return fail("bad_version");

  const os = parseOs(raw.os);
  if (os === null) return fail("bad_os");

  const events = raw.events;
  if (!Array.isArray(events)) return fail("bad_events");
  if (events.length === 0 || events.length > MAX_EVENT_COUNT) return fail("bad_event_count");

  const parsed: EventInput[] = [];
  for (const entry of events) {
    if (!isRecord(entry)) return fail("bad_event");

    const name = boundedString(entry.name, MAX_EVENT_NAME_CHARS);
    if (name === null) return fail("bad_event");

    let page = "";
    if (entry.page !== undefined) {
      const raw = boundedString(entry.page, MAX_PAGE_CHARS, 0) ?? "";
      // The app may send `page:browse` or `browse`; store the bare name.
      const candidate = raw.startsWith("page:") ? raw.slice(5) : raw;
      page = isPageName(candidate) ? candidate : "";
    }

    const stamp = entry.ts;
    if (typeof stamp !== "number" || !Number.isFinite(stamp) || stamp < 0) return fail("bad_event");
    const ts = Math.min(Math.floor(stamp), now);

    parsed.push({ name, page, ts });
  }

  return ok({ installId, version, os, events: parsed });
}

export function parseCrash(raw: unknown): ParseResult<CrashBody> {
  if (!isRecord(raw)) return fail("bad_body");

  const installId = parseInstallId(raw.installId);
  if (installId === null) return fail("bad_install_id");

  const version = parseVersion(raw.version);
  if (version === null) return fail("bad_version");

  const os = parseOs(raw.os);
  if (os === null) return fail("bad_os");

  const message = boundedString(raw.message, MAX_MESSAGE_CHARS);
  if (message === null) return fail("bad_message");

  const backtrace = boundedString(raw.backtrace, MAX_BACKTRACE_CHARS, 0);
  if (backtrace === null) return fail("bad_backtrace");

  return ok({ installId, version, os, message, backtrace });
}

export function parseDiagnostics(raw: unknown): ParseResult<DiagnosticsBody> {
  if (!isRecord(raw)) return fail("bad_body");

  const installId = parseInstallId(raw.installId);
  if (installId === null) return fail("bad_install_id");

  const version = parseVersion(raw.version);
  if (version === null) return fail("bad_version");

  const os = parseOs(raw.os);
  if (os === null) return fail("bad_os");

  const summary = boundedString(raw.summary, MAX_SUMMARY_CHARS);
  if (summary === null) return fail("bad_summary");

  const log = boundedString(raw.log, MAX_LOG_CHARS, 0);
  if (log === null) return fail("bad_log");

  return ok({ installId, version, os, summary, log });
}

export function parseHashes(raw: unknown): ParseResult<HashesBody> {
  if (!isRecord(raw)) return fail("bad_body");

  const installId = parseInstallId(raw.installId);
  if (installId === null) return fail("bad_install_id");

  const sha256 = boundedString(raw.sha256, 64);
  if (sha256 === null || !SHA256_PATTERN.test(sha256)) return fail("bad_sha256");

  const size = parsePositiveInteger(raw.size, MAX_SHA256_BYTES);
  if (size === null) return fail("bad_size");

  const gamebananaModId = parsePositiveInteger(raw.gamebananaModId, MAX_GAMEBANANA_ID);
  if (gamebananaModId === null) return fail("bad_gamebanana_mod_id");

  const fileId = parsePositiveInteger(raw.fileId, MAX_GAMEBANANA_ID);
  if (fileId === null) return fail("bad_file_id");

  return ok({ installId, sha256: sha256.toLowerCase(), size, gamebananaModId, fileId });
}

export function parseIdentify(search: URLSearchParams): ParseResult<IdentifyQuery> {
  const sha256 = search.get("sha256") ?? "";
  if (!SHA256_PATTERN.test(sha256)) return fail("bad_sha256");

  const sizeText = search.get("size") ?? "";
  if (!SIZE_PATTERN.test(sizeText)) return fail("bad_size");

  const size = Number(sizeText);
  if (!Number.isSafeInteger(size) || size < 1 || size > MAX_SHA256_BYTES) return fail("bad_size");

  return ok({ sha256: sha256.toLowerCase(), size });
}
