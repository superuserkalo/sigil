# 0019 Community crosshair library

Status: accepted (Worker routes built and tested with migration 0007; not deployed, migration not
applied remotely, the app does not call it yet)
Date: 2026-11-25

## Context

Players make crosshairs in Sigil's crosshair editor (`rust/crates/deadlock-core/src/crosshair.rs`)
and share them today only as console lines or `DL.` share codes pasted by hand. Deadlock Mod
Manager has a browsable community crosshair library; Sigil builds for parity (ADR 0011). The owner
approved (2026-11-25) a library on Sigil's own Worker: a player publishes a crosshair (a name and
its settings) from the app, others browse the list and apply one. No accounts.

The library was designed from that description only. No competitor code or data was read or
copied.

## Decision

The library lives on the existing Worker (`sigil-telemetry`, `services/telemetry/src/crosshairs/`)
and its D1 database (`sigil`), migration `0007_crosshairs.sql`.

- Publishing is the player's own action, one crosshair at a time. Browsing and applying need no
  consent: the list is public and the same for everyone.
- No accounts and no identifiers. A row holds the name, the canonical config, its SHA-256, the use
  and report counts, the `official` and `hidden` flags, the created time and day, and a SHA-256 of
  a delete token. The token is returned once, to the publishing app, which keeps it to remove the
  crosshair later.
- Limits and dedupe use a network bucket: HMAC-SHA256 under the `PRODUCT_BUCKET_KEY` Worker secret
  of the UTC day and the caller's network prefix (IPv4 /24, IPv6 /48), cut to 16 hex characters.
  It appears only in three small tables (`crosshair_uses`, `crosshair_reports`,
  `crosshair_publishes`), whose rows the daily job deletes 2 days after they were written. The
  publish table does not say which crosshair was published. The bucket changes every day, so
  nothing links one day's network to the next.
- Moderation without a model: strict validation, a short slur blocklist on a normalised name,
  reports from distinct networks that hide a row automatically at 3, and owner-only admin routes.
- 21 official crosshairs ship in the migration, flagged `official` and shown as by Sigil. They
  pass the same validator (a test checks every row), cannot be deleted by token and are not hidden
  by reports.

## API contract

Base: `https://sigil-telemetry.gantchevkaloyan.workers.dev`. Every request needs
`x-sigil-client: 1` and a semver `x-sigil-version`, otherwise `400 {"error":"bad_client"}`. Every
route is behind the Worker's rate limiter (60 requests per 60 seconds per location; over it
`429 {"error":"rate_limited"}`). A wrong method gets `405 {"error":"method_not_allowed"}` with an
`allow` header. Bodies are JSON (`content-type: application/json`); responses are JSON. Errors are
`{"error": "<code>"}` plus the extra fields named below.

### Ids

- Community crosshair: 16 lower case hex characters (`^[0-9a-f]{16}$`), random.
- Official crosshair: `sigil-` and 1 to 32 of `a-z`, `0-9`, `-` (`^sigil-[a-z0-9-]{1,32}$`).

A path with any other id is `404 {"error":"not_found"}`.

### The config object

Exactly these 19 keys, all required, the camelCase names the app's `Crosshair` serialises to:

| Key | Type | Range |
| --- | --- | --- |
| `pipGap` | integer | 0 to 64 |
| `pipGapStatic` | boolean | |
| `pipHeight` | integer | 0 to 64 |
| `pipWidth` | integer | 0 to 64 |
| `pipOpacity` | number | 0 to 1 |
| `pipOutlineBorder` | integer | 0 to 16 |
| `pipOutlineGap` | integer | 0 to 16 |
| `pipOutlineOpacity` | number | 0 to 1 |
| `dotOpacity` | number | 0 to 1 |
| `dotSize` | integer | 0 to 64 |
| `dotOutlineBorder` | integer | 0 to 16 |
| `dotOutlineGap` | integer | 0 to 16 |
| `dotOutlineOpacity` | number | 0 to 1 |
| `colorR`, `colorG`, `colorB` | integer | 0 to 255 |
| `outlineColorR`, `outlineColorG`, `outlineColorB` | integer | 0 to 255 |

- Integers must be JSON numbers with no fraction (`5` or `5.0`, never `"5"` or `5.5`). Booleans
  must be JSON `true` or `false` (never `1` or `"true"`). `null` is never accepted.
- `disableHeroSpecificCrosshairs` may be present as a boolean. It is a player preference, not part
  of the look (share codes leave it out too), so it is accepted and dropped, never stored or
  returned. The app applies a library crosshair without changing the player's own value.
- Any other key, a missing key, a wrong type or a value out of range fails the whole config with
  `400 {"error":"bad_config","field":"<the first key that failed>"}`. `field` is `config` when the
  value is not an object.
- Stored canonical: keys in the order of the table, opacities rounded to hundredths (as the app
  rounds them and the game writes them), no whitespace. The config hash is the lower case hex
  SHA-256 of that text. Two configs that look the same in game (different key order, opacity
  `0.999` and `1`) are the same config.

### The name

- NFC, trimmed, each run of whitespace collapsed to one space; then 2 to 32 characters.
- Only ASCII letters, the Latin-1 and Latin Extended-A letters (U+00C0 to U+017F without `×` and
  `÷`), digits `0-9`, space and `. - _ ' ! +`.
- A dot may not be followed directly by a letter or digit (`Dr. Aim` is fine, `x.com` is not). With
  no `/`, `:` or `@` allowed, a name cannot spell a link or an address.
- At least one letter or digit.
- Failing any of these: `400 {"error":"bad_name"}`.
- On the blocklist (`services/telemetry/src/crosshairs/blocklist.ts`): `400 {"error":"blocked_name"}`.
  Matched on a normalised form: lower case, accents stripped, `0 3 4 5 6 7 8 9 ! +` read as
  `o e a s g t b g i t`, `1` read as both `i` and `l`, letters allowed to repeat. Stems match
  anywhere once spaces and punctuation are removed; short words match only as a whole word or the
  whole name.

### POST /v1/crosshairs

Body (at most 2 KiB): `{"name": string, "config": object}`. No other key
(`400 {"error":"bad_body"}`).

| Status | Body | When |
| --- | --- | --- |
| 201 | `{"id": string, "delete_token": string}` | Published. `delete_token` is 43 base64url characters (32 random bytes); only its SHA-256 is stored, so it cannot be shown again. |
| 400 | `bad_body`, `invalid_json`, `invalid_utf8`, `bad_name`, `blocked_name`, `bad_config` (+ `field`), `no_network` | Rejected. `no_network` only when the request carries no usable caller address. |
| 409 | `{"error":"duplicate","id": string}` | The same canonical config is already in the library (community or official, visible or hidden). Nothing is stored. |
| 413 | `body_too_large` | Over 2 KiB. |
| 429 | `publish_limit` | This network already published 3 crosshairs today (UTC). |
| 429 | `daily_limit` | 300 crosshairs were published today across everyone. |
| 503 | `not_configured` | `PRODUCT_BUCKET_KEY` is missing or shorter than 32 characters. Fails closed: nothing is stored. |
| 507 | `library_full` | The library holds 20,000 rows. |

Checks run in this order: secret, body, name, config, duplicate, limits. A deleted crosshair still
counts toward the day's limits.

### GET /v1/crosshairs?sort=used|new&cursor=<opaque>

`sort` defaults to `used` (most uses first, ties by id ascending); `new` is newest first (ties by
id descending). `cursor` is the `next_cursor` of the previous page with the same `sort`; treat it as
opaque.

`200`, `cache-control: public, max-age=300`:

```json
{
  "crosshairs": [
    {
      "id": "sigil-dot",
      "name": "Dot",
      "config": { "pipGap": 5, "pipGapStatic": true, "...": "all 19 keys" },
      "uses": 0,
      "official": true,
      "created_day": "2026-11-25"
    }
  ],
  "next_cursor": "eyJzIjoidXNlZCIsInUiOjAsImkiOiJzaWdpbC10aGluIn0"
}
```

- At most 60 per page; `next_cursor` is `null` on the last page.
- Hidden rows never appear. Every query filters on `hidden = 0`; a cursor holds only a position
  (a use count or a time, and an id), so no cursor, forged or not, can return a hidden row.
- Pages are keyset pages: no row repeats within one walk, but a row whose use count changes
  between two requests can move across the cursor.
- Errors: `400 bad_sort`, `400 bad_cursor` (malformed, or made for the other sort).
- No secret needed; the list works without `PRODUCT_BUCKET_KEY`.

### POST /v1/crosshairs/<id>/delete

Body (at most 256 bytes): `{"delete_token": string}`.

| Status | Body | When |
| --- | --- | --- |
| 200 | `{"deleted": true}` | Removed, with its use and report dedupe rows. |
| 400 | `bad_token` | Missing or not 43 base64url characters. |
| 403 | `wrong_token` | The token does not match (compared in constant time), or the row is official. |
| 404 | `not_found` | No such row (already deleted or removed). |

### POST /v1/crosshairs/<id>/use

Counts one apply. The body is ignored (send `{}`). Once per network per crosshair per UTC day.

- `200 {"counted": true}` the first time, `200 {"counted": false}` after.
- `404 not_found` for a missing or hidden row; `503 not_configured` without the bucket key;
  `400 no_network` without a caller address.

### POST /v1/crosshairs/<id>/report

Reports a crosshair. The body is ignored (send `{}`). Once per network per crosshair per UTC day.
When a community row reaches 3 counted reports it is hidden until the owner decides. Reports on an
official row answer `{"counted": false}` and change nothing.

- `200 {"counted": true | false}`; `404 not_found` for a missing or hidden row; `503`, `400` as
  for use.

### Admin routes

`authorization: Bearer <PRODUCT_ADMIN_KEY>`, compared in constant time. Without the secret (or
shorter than 32 characters) `503 not_configured`; a missing or wrong key `401 unauthorized`.

| Method and path | Body | Answer |
| --- | --- | --- |
| `GET /v1/crosshairs/admin/flagged` | | `200 {"crosshairs": [ ...list fields, "reports": int, "hidden": bool ]}`, rows that are hidden or have reports, at most 500, hidden first. |
| `POST /v1/crosshairs/admin/<id>/remove` | | `200 {"removed": true}` or `404`. Deletes the row and its dedupe rows. |
| `POST /v1/crosshairs/admin/<id>/restore` | | `200 {"restored": true}` or `404`. Shows the row again with reports back to 0. |
| `POST /v1/crosshairs/admin/official` | `{"id": "sigil-...", "name", "config"}` | `200 {"id"}`. Inserts or replaces an official row through the same name and config rules; `400 bad_id`, `bad_name`, `blocked_name`, `bad_config`; `409 duplicate` (+ `id`) when another row has that config. |

## What is stored

| Table | Columns | Kept |
| --- | --- | --- |
| `crosshairs` | `id`, `name`, `config`, `config_hash` (unique), `uses`, `reports`, `official`, `hidden`, `created_at`, `created_day`, `delete_token_hash` (NULL for official rows) | Until deleted by its token or removed by the owner. |
| `crosshair_uses` | `bucket`, `crosshair_id`, `created_at` | 2 days. |
| `crosshair_reports` | `bucket`, `crosshair_id`, `created_at` | 2 days. |
| `crosshair_publishes` | `bucket`, `day`, `created_at` | 2 days. |

No install id, player id, Steam id, address or user agent is read into any of them. The use and
report counts on a row stay after their dedupe rows go.

## Seeds

Migration 0007 inserts 21 official rows: the eight built-in presets of `crosshair.rs` (Sigil
Default, Game Default, Dot, Small Cross, Cross and Dot, Wide Gap, Thin, Bold) and thirteen distinct
variations (Red Dot, Micro Dot, Big Dot No Outline, Boxed Dot, Wide Cross, Tight Plus, Thick,
Hairline, Clean Green, Heavy Outline, Dynamic Spread, Soft Pink, Long Lines). The game draws four
equal pips, a square dot and square outlines, so there is no T shape and no circle: Boxed Dot
(a dot inside a square outline ring) stands in for circle with dot, and no T shape ships.

## Consequences

- The app builder implements against the contract above: publish from the crosshair editor with a
  confirm that names what becomes public, keep each delete token locally, list with both sorts,
  apply, count the apply, report.
- A network can publish 3 crosshairs a day; a shared network (a school, a carrier NAT) shares that
  allowance. A determined player on many networks can still publish; the daily and total caps
  bound the damage.
- Reports dedupe per network per day, not forever (the bucket rotates daily so nothing is kept
  longer). One network alone could reach 3 reports over three days; the owner's restore covers it.
- The blocklist is short by design. Reports and the admin routes handle everything else.
- Deploying needs migration 0007 applied remotely and the Worker redeployed; `PRODUCT_BUCKET_KEY`
  and `PRODUCT_ADMIN_KEY` already exist for ADR 0017.

## Alternatives

- Accounts or an install id per publisher: rejected. A delete token gives the author control
  without tying a crosshair to anyone.
- A model to judge names: rejected. A character allowlist and a short blocklist are predictable
  and need no third party.
- Storing the raw address or a long-lived network hash for dedupe: rejected. A daily HMAC in
  tables that live 2 days is enough to stop casual spam.
- Using `DL.` share codes as the stored form: rejected. Decoding needs deadlock-api; a validated
  JSON config is checked on the Worker and applied by the app directly.
