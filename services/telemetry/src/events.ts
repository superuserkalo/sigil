/**
 * The usage event allowlist.
 *
 * An event the app sends but this list does not name is dropped, never stored: the Worker accepts
 * no free text event name. Adding a name here is a deliberate act with a privacy review, because
 * every stored name is a statement about real players (`docs/privacy.md`).
 *
 * A page view is also an event (`page:browse`, `page:profiles` and so on), so the event name and
 * the `page` field stay in step and one dashboard can count both.
 */

/** Screens a player can be on. Used for `page:<name>` events and the `page` field. */
export const PAGE_NAMES = [
  "browse",
  "installed",
  "conflicts",
  "updates",
  "profiles",
  "autoexec",
  "stats",
  "locker",
  "launch",
  "servers",
  "crosshair",
  "performance",
  "agent",
  "projects",
  "index",
  "patches",
  "impact",
  "release",
  "toolchain",
  "settings",
  "downloads",
  "character",
  "cursors",
  "divine",
  "about",
] as const;

export type PageName = (typeof PAGE_NAMES)[number];

/** Things a player does. The name says the action, never the content of it. */
export const FEATURE_EVENTS = [
  "install:start",
  "install:done",
  "install:fail",
  "profile:apply",
  "repair:apply",
  "deploy:plan",
  "deploy:apply",
  "pack:done",
  "validate:fail",
  "launch:modded",
  "launch:vanilla",
  "update:check",
  "update:install",
  "publish:prepare",
  "index:build",
  "mod:create",
  "migrate:apply",
  "watch:run",
  "server:join",
  "diagnostics:send",
  "crash:send",
  "perf:apply",
  "app:start",
  "browse:search",
  "browse:filter",
  "browse:open",
  "browse:download",
  "install:failed",
  "install:cancel",
  "identify:lookup",
  "identify:contribute",
  "profile:create",
  "profile:share",
  "mods:toggle",
  "mods:reorder",
  "deploy:start",
  "deploy:done",
  "launch:restore",
  "crosshair:apply",
  "autoexec:apply",
  "performance:apply",
  "backup:restore",
  "cleanup:run",
  "settings:change",
  "agent:chat",
  "publish:start",
] as const;

export type FeatureEvent = (typeof FEATURE_EVENTS)[number];
export type PageEvent = `page:${PageName}`;
export type EventName = FeatureEvent | PageEvent;

const PAGE_EVENTS: readonly PageEvent[] = PAGE_NAMES.map((page): PageEvent => `page:${page}`);

/** Every name the Worker stores: the screens plus the actions. It must cover
 * `deadlock_core::telemetry::EVENTS`; a test checks that. */
export const EVENT_NAMES: readonly EventName[] = [...FEATURE_EVENTS, ...PAGE_EVENTS];

const EVENT_NAME_SET: ReadonlySet<string> = new Set<string>(EVENT_NAMES);
const PAGE_NAME_SET: ReadonlySet<string> = new Set<string>(PAGE_NAMES);

export function isEventName(value: string): value is EventName {
  return EVENT_NAME_SET.has(value);
}

export function isPageName(value: string): value is PageName {
  return PAGE_NAME_SET.has(value);
}
