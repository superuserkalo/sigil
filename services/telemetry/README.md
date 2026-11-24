# Sigil telemetry Worker

Status: deployed at `https://sigil-telemetry.gantchevkaloyan.workers.dev` (migrations 0001 and 0002
applied, 2026-11-20). The desktop app sends only behind the Settings > Privacy switches or a press. The decision is `docs/decisions/0014-opt-in-telemetry.md`; the player-facing
description is `docs/privacy.md`.

One Cloudflare Worker, `sigil-telemetry`, owns every opt-in upload Sigil makes. There is no third
party: usage events go to Analytics Engine, everything else to D1.

## Endpoints

| Method | Path | Body limit | What it stores |
| --- | --- | --- | --- |
| POST | `/v1/events` | 32 KiB | One Analytics Engine point per allowlisted event. Unknown names are dropped, never stored. |
| POST | `/v1/crash` | 96 KiB | One D1 row per fingerprint, with a repeat count, the kind and the newest log tail. |
| POST | `/v1/diagnostics` | 1 MiB | One D1 row, returns a support code such as `SGL-7K4M-9QZT`. |
| GET | `/v1/identify?sha256=<hex>&size=<n>` | none | Nothing. Reads the hash index. |
| POST | `/v1/hashes` | 4 KiB | Upserts one `mod_hashes` row and counts reports. |

Every request needs `x-sigil-client: 1` and a semver `x-sigil-version`, otherwise 400. The limiter
allows 60 requests per 60 seconds per Cloudflare location, keyed by a salted hash of
`cf-connecting-ip`; the salt lives in memory and is replaced at UTC midnight. No address, user
agent, install id or key is stored, and structured log lines carry only method, path, status and
duration.

### Usage events

```json
{
  "installId": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
  "version": "0.4.1",
  "os": "macos",
  "events": [{ "name": "page:browse", "page": "browse", "ts": 1795000000000 }]
}
```

At most 50 events. `installId` is a random UUID the app makes, not a machine id; only its SHA-256 is
used, as the Analytics Engine sampling index. `os` is `windows`, `macos`, `linux` or `other`. Names
come from `src/events.ts` (20 `page:<screen>` names and 22 actions). `page` is optional and is
dropped when it is not a known screen.

Each stored event is one data point: blobs `[name, page, version, os]`, doubles `[1]`, index the
install id hash. The dataset is `sigil_events` behind the `SIGIL_EVENTS` binding, for example:

```sql
SELECT blob1 AS event, SUM(_sample_interval * double1) AS uses
FROM sigil_events
WHERE timestamp > NOW() - INTERVAL '7' DAY
GROUP BY event
ORDER BY uses DESC
```

### Crashes

`{installId, version, os, message, backtrace, kind?, context?}`. `kind` is `panic` (default),
`unclean_exit` (the app found its session marker at the next start without a panic) or `native` (the
Windows exception filter). `context` is the session's scrubbed log tail, at most 16 KiB; a repeat of
the same fingerprint keeps the newest non-empty one (migration `0002_crash_context.sql`). Scrubbed again on the server (`src/scrub.ts`): home
paths, user names in paths, Steam ids, emails, provider and GitHub keys, bearer tokens, JWTs and the
install id itself. The fingerprint is the first message line plus the first five frames with
addresses, line numbers and columns removed (`src/fingerprint.ts`), so a repeating crash is one row
with `repeats`. Kept 90 days.

### Diagnostics

`{installId, version, os, summary, log}`, 1 MiB. Scrubbed the same way and stored with a support
code from `crypto.getRandomValues` over an alphabet without `0`, `O`, `1` and `I`. Kept 30 days.
The player sends this only by pressing a button.

### Hash index

`POST /v1/hashes` takes the SHA-256 and size of a VPK the app just installed from GameBanana along
with the mod and file ids, and counts one report per install. `GET /v1/identify` returns
`{matches: [{gamebananaModId, fileId, reports}]}` sorted by reports, best first, at most five. A
lookup stores nothing about the caller. Several submissions can ship the same file, so one hash can
map to several mods and the report count decides the order.

## Layout

```text
src/index.ts        routes, handlers, logging
src/events.ts       the event and page allowlist
src/body.ts         capped streaming body reader
src/validate.ts     shape validation for every body and query
src/scrub.ts        server side scrubbing
src/fingerprint.ts  crash fingerprints
src/codes.ts        support codes
src/limiter.ts      daily salt and limiter key
src/retention.ts    the cron cleanup
migrations/         D1 schema
test/               Workers runtime tests
```

## Develop and test

```bash
npm install
npx vitest run          # Workers runtime, real D1, Analytics Engine and limiter bindings
npm run typecheck
npm run types           # regenerate worker-configuration.d.ts after a binding change
npm run migrate:local   # apply migrations to the local D1 database
npm run dev             # wrangler dev on a local port
```

Tests use `@cloudflare/vitest-plugin`, the renamed successor to `@cloudflare/vitest-pool-workers`.
The older package pins a workerd binary that refuses a `2026-11-18` compatibility date, so it cannot
run this Worker's configuration. The test API (`cloudflare:test`, `applyD1Migrations`, the pool
options) is the same.

`nodejs_compat` is not enabled: the Worker uses only Workers APIs (`crypto.subtle`, streams,
`fetch`, D1, Analytics Engine).

## Deploy

The owner deploys. Nothing here writes to a game install.

1. `npx wrangler login` once on the machine that deploys.
2. `npx wrangler d1 create sigil` and paste the returned `database_id` over the placeholder
   `"TO_BE_CREATED"` in `wrangler.jsonc`. Local development and the tests ignore the id.
3. `npx wrangler d1 migrations apply sigil --remote` to create the tables.
4. `npx wrangler deploy`. The Worker answers on its `workers.dev` address or on a route added to
   `wrangler.jsonc`.
5. Record the address in `docs/privacy.md` and add the host to the network allowlist in
   `docs/architecture.md`, then point the desktop app at it once the client exists.

The cron trigger runs at 03:17 UTC daily: retention first (diagnostics older than 30 days, crashes
older than 90, raw product data and free-text product totals older than 13 months, product
contributions older than 75 days), then it closes finished 30-day product epochs. Analytics Engine
retention is Cloudflare's own; query it with the SQL API.

### Help build Sigil (product data, ADR 0017)

Before deploying the product routes, set two secrets once. Never put them in the repo or in
`wrangler.jsonc`:

1. `npx wrangler secret put PRODUCT_SALT`: a long random string (for example
   `openssl rand -hex 32`). It salts the install id hash. Set it before the first deploy of these
   routes and never change it: a new salt makes Delete my data miss older rows.
2. `npx wrangler secret put PRODUCT_BUCKET_KEY`: another long random string (at least 16
   characters). It keys the network bucket. Without it the Worker stores raw records but writes and
   publishes no aggregate. Changing it only starts the buckets of the current epoch over.
3. `npx wrangler d1 migrations apply sigil --remote` applies `0003_product_data.sql`.
4. `npx wrangler deploy`.

A mistake in the data is fixed by deleting rows from D1, not by deploying a key.

## What the apply step (desktop side) still has to do

This repository part is the receiving end only. The desktop session that builds the client must:

- keep every kind (usage events, crash reports, unknown-mod lookup) off until the player agrees, and
  keep diagnostics manual;
- generate the install id once, store it in the app data directory, and never reuse a machine id;
- scrub before sending, reuse the same rules as `src/scrub.ts`, and cap bodies to the limits above;
- send `x-sigil-client: 1` and the app version, and respect 429 by waiting;
- write the hash contribution only after a GameBanana install the player asked for.
