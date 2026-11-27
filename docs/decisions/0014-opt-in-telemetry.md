# 0014 Opt-in telemetry on Sigil's own Cloudflare Worker

Status: accepted
Date: 2026-11-18

## Context

AGENTS.md banned telemetry, analytics and heartbeats outright. That kept Sigil blind: no crash
reports from players, no way to see which features are used, and no way to identify an unknown
VPK a player dropped in. Deadlock Mod Manager sends usage events to Google Analytics after a
first-start consent, runs its own Cloudflare-fronted API for VPK hash lookups, and never uploads
logs or crash dumps. The owner asked for the same kind of data, on infrastructure Sigil controls,
and approved changing the rule on 2026-11-18.

## Decision

- Sigil runs one Cloudflare Worker, `sigil-telemetry` (`services/telemetry/`), with Analytics
  Engine for usage events and D1 for crashes, diagnostics and the hash index. No third party.
- Nothing is sent until the player agrees. Until 0.1.45 that was a first-start dialog; it opened
  right after the setup's own Help build Sigil question, so players were asked twice in a row.
  Since 0.1.46 (2026-11-27) the dialog no longer opens by itself: the switches start off and are
  turned on in Settings > Privacy. Settings > Privacy has one
  switch per kind: usage events, crash reports, unknown-mod lookup. Diagnostics are sent only when
  the player presses Send, every time.
- Usage events carry an allowlisted event name, page, app version and OS, keyed by the SHA-256 of
  a random install id (not a machine id). Crash reports and diagnostics are scrubbed on the device
  and again on the server: home paths, user names, API keys and tokens, emails.
- The Worker stores no IP address or user agent; rate limiting uses a salted hash kept in memory.
  Diagnostics are deleted after 30 days, crashes after 90.
- The hash index is filled by apps that just installed a file from GameBanana (with lookup
  consent); a lookup sends only the hash and size.
- `docs/privacy.md` lists every destination and field and is updated with any change.

## Consequences

- One more host on the network allowlist, opt-in.
- The owner operates the Worker and its costs; the code is in the repo so players can read it.
- BYOK keys and Steam ids are never part of events or crashes; scrubbing is tested on both sides.

## Amendment 2026-11-20: crash reports after a restart

Owner-approved on 2026-11-20 ("after a crash and we restart Sigil we should detect that and offer
on startup a popup to ask the user if they want to send that crash report").

- Sigil keeps a session marker and finds a run that did not exit cleanly at the next start. The
  report kinds are `panic`, `unclean_exit` (no panic recorded: abort, stack overflow, driver crash,
  forced quit) and `native` (a Windows unhandled exception record). A report now also carries the
  session's scrubbed log tail (`context`, at most 16 KiB). The Worker stores `kind` and `context`
  (migration `0002_crash_context.sql`).
- With crash reports on, the report is sent at start and a toast says so. With crash reports off,
  or the first-start question never answered, a dialog shows exactly what would be sent and asks.
  Pressing Send report is one-off consent for that report only, the same rule as Send diagnostics;
  "Always send crash reports" in the dialog is the Settings > Privacy switch. Nothing is sent
  without a press or the switch.
- Limits on the device: the same crash at most once a day, at most 3 reports an hour and 10 a day.
  A crash the player declined is not asked about again for a day.
