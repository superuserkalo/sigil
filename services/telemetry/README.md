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
| POST | `/v1/product` | 32 KiB | Help build Sigil records (ADR 0017), raw rows and open-epoch contributions. |
| POST | `/v1/product/delete` | 1 KiB | Deletes one install's raw rows and open-epoch contributions. |
| GET | `/v1/product/related?mod=<id>` | none | Nothing. Co-installed mods above the floor. |
| GET | `/v1/product/health?mods=<id,id,...>` | none | Nothing. Mod health above the floor, at most 40 mods. |
| GET | `/v1/product/fps?gpu=<model>&cpu=<model>&height=<px>` | none | Nothing. Frame rates per config on that hardware class. |
| GET | `/v1/product/rising` | none | Nothing. Up to 50 mods whose installs grew most from one closed week to the next. |
| GET | `/v1/product/admin/search-gaps` | none | Nothing. Owner only, needs `PRODUCT_ADMIN_KEY`. |

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
older than 90, product contributions older than 21 days), then it unlinks the raw product records
of installs that sent nothing for 24 months, then it closes finished weekly product epochs (7 days, `EPOCH_DAYS`). Raw
product records and product totals are otherwise kept (ADR 0017). Analytics Engine
retention is Cloudflare's own; query it with the SQL API.

### Help build Sigil (product data, ADR 0017)

Before deploying the product routes, set two secrets once. Never put them in the repo or in
`wrangler.jsonc`:

1. `npx wrangler secret put PRODUCT_SALT`: a random string of at least 32 characters (for example
   `openssl rand -hex 32`). It salts the install id hash. Set it before the first deploy of these
   routes and never change it: a new salt makes Delete my data miss older rows.
2. `npx wrangler secret put PRODUCT_BUCKET_KEY`: another random string of at least 32 characters.
   It keys the network bucket. Changing it only starts the buckets of the current epoch over.

Without both secrets (or with one shorter than 32 characters) `POST /v1/product`,
`POST /v1/product/delete` and `GET /v1/product/related` answer 503 `not_configured` and store
nothing. The app keeps its queue within its usual limits and tries again later.
3. `npx wrangler d1 migrations apply sigil --remote` applies `0003_product_data.sql`.
4. `npx wrangler deploy`.

A mistake in the data is fixed by deleting rows from D1, not by deploying a key.

### What players get back (migration 0004)

Every answer below is built only from `product_totals`: closed epochs whose key met the floor (10
eligible installs, 6 network buckets, 3 days). No install hash, bucket or single install's value
leaves the Worker. The routes are rate limited like every other route.

- `GET /v1/product/related?mod=<id>`: `{mods: [{modId, installs}]}`, at most 20, damped cosine.
- `GET /v1/product/health?mods=<id,...>`: per mod, from the last 8 closed weekly epochs only (so a
  fixed mod stops carrying old reports): `install` (`failed` installs whose reason
  points at the mod, `attempts` from the same epochs, `reasons`), `broken` per game build
  (`reports`: the strongest of moved, gone, skeleton, turned off after a crash and crash suspect;
  `users`: installs that had the mod on at that build) and `conflicts` (`reports` of overlap or
  conflict pairs, `together`: installs that had both on). Kinds: `health`, `attempt`, `use`,
  `conflict`. The app decides what is strong enough to show (`deadlock_core::product_data`).
- `GET /v1/product/fps?gpu=<model>&cpu=<model>&height=<px>`: `{configs: [{config, version, fps,
  low, installs}]}`. The hardware class is the reduced GPU model and a CPU tier (`cpuTier`: high,
  mid, low, other); `height` picks the bucket (720p, 1080p, 1440p, 2160p). A benchmark is filed
  under kind `fps` with the install's latest `hardware` and `perf_config` records; closing an
  epoch writes the median average and the median 1% low of the eligible installs (`median`,
  `low`) and deletes the per-install values with the rest of the epoch.

- `GET /v1/product/rising`: `{mods: [{modId, recent}]}`, at most 50. Kind `install_day` (from
  `mod_action` installed), one key per mod. The route reads the newest closed week: a mod must meet
  the floor there (`late`); the week before counts when it met the floor too (`early`), else 0.
  Ranked by `((late + 1) / (early + 1)) * late / (late + 10)`, at least 5 late installs. Empty
  until a closed week has data. The `weeks` column from 0004 is no longer written.

### Search gaps (owner only)

Browse searches that found nothing (`search_zero`) or at most 5 mods (`search_few`), above the
same floor. Set the key once; without it (or with one shorter than 32 characters) the route answers
503 and reads nothing, and a wrong key gets 401:

```bash
npx wrangler secret put PRODUCT_ADMIN_KEY   # openssl rand -hex 32; keep it out of the repo
curl -s https://sigil-telemetry.gantchevkaloyan.workers.dev/v1/product/admin/search-gaps?limit=100 \
  -H 'x-sigil-client: 1' -H 'x-sigil-version: 0.1.0' \
  -H "authorization: Bearer $PRODUCT_ADMIN_KEY"
# {"zero":[{"term":"kelvin hat","installs":14}],"few":[{"term":"yamato sword","installs":11}]}
```

### Weekly epochs (migration 0005)

Epochs are 7 days (`EPOCH_DAYS` in `src/aggregates.ts`, the one setting). Contributions live their
week plus 2 days of grace, 21 days at most. Health and related mods read the last 8 closed weeks.
`0005_weekly_epochs.sql` deletes any contribution or total numbered under the old 30-day epochs
(epoch below 1500); the tables were empty when the change landed, so it is a guard. Apply it after
deploying the weekly code:

```bash
npx wrangler deploy
npx wrangler d1 migrations apply sigil --remote   # 0005_weekly_epochs.sql
```

### Deploying migration 0004

```bash
npx wrangler d1 migrations apply sigil --remote   # 0004_product_uses.sql: five columns, one index
npx wrangler secret put PRODUCT_ADMIN_KEY         # only for the search gaps route
npx wrangler deploy
```

Contributions sent before 0004 keep counting: the new kinds start from the deploy, and a
benchmark sent before it has no frame rate row.

## What the apply step (desktop side) still has to do

This repository part is the receiving end only. The desktop session that builds the client must:

- keep every kind (usage events, crash reports, unknown-mod lookup) off until the player agrees, and
  keep diagnostics manual;
- generate the install id once, store it in the app data directory, and never reuse a machine id;
- scrub before sending, reuse the same rules as `src/scrub.ts`, and cap bodies to the limits above;
- send `x-sigil-client: 1` and the app version, and respect 429 by waiting;
- write the hash contribution only after a GameBanana install the player asked for.
