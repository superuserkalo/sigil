# Deadlock player stats: API, derived metrics and visualizations

Status: active. Researched and verified 2026-11-16 against the live API.
Updated: 2026-11-17

This document records the public data source, the endpoints the workbench uses, the formulas behind
every derived number, what the API does not expose, and the chart choices for the future Stats
screen. The companion case study is `docs/research/stats-case-study.md`.

## Data source

- Base URL: `https://api.deadlock-api.com`
- OpenAPI document: `https://api.deadlock-api.com/openapi.json` (version 0.1.0, 125 paths)
- Documentation: `https://api.deadlock-api.com/docs`
- Source and license: `https://github.com/deadlock-api/deadlock-api`, MIT
- The project states it is not endorsed by Valve. Treat it as a community data source, not an
  official game API.

The endpoints used here are read only and need no account or API key. An optional key only raises
the shared rate limits. The client reads `DEADLOCK_API_KEY` and sends it as the `X-API-KEY` header.
The key is never logged, never included in JSON output, and never written to disk.

Requests use the user agent `dah/0.0.0`, a 45 second global timeout and
`http_status_as_error(false)` so the client can report the status, `Retry-After` and a short body
snippet. There is no automatic retry. The API asks clients not to send many uncached requests in
parallel.

### Rate limits that apply to this client

Limits below are the IP limits from the live OpenAPI document. A key raises most of them.

| Endpoint | IP limit | Notes |
| --- | --- | --- |
| `GET /v1/players/steam` | 100 req/s read, 3 req/min and 15 req/h with `refresh=true` | The client never sends `refresh=true`. |
| `GET /v1/players/{account_id}/match-history` | 100 req/s stored history, 10 req/h bot friends, 1 req/h with `force_refetch=true` | The client never forces a refetch. |
| `GET /v1/players/hero-stats` | 100 req/s | Defaults to normal game mode and ranked plus unranked. |
| `GET /v1/players/{account_id}/rank` | 10,000 accounts/min, 100,000 accounts/h | Shared by all rank endpoints. |
| `GET /v1/players/rank/distribution` | 5 req/min | The tightest limit used here. |
| `GET /v1/leaderboard/{region}` | 100 req/s | Valve refreshes the leaderboard hourly. |
| `GET /v1/analytics/kill-death-stats` | 200 req/min | Shared by all analytics endpoints. |
| `GET /v1/assets/heroes` | No table in the OpenAPI document | About 1.9 MB. |
| `GET /v1/assets/ranks` | No table in the OpenAPI document | About 37 KB. |
| `GET /v1/assets/map` | No table in the OpenAPI document | About 111 KB. |

Load shedding is documented and visible in practice:

- `503 Service Unavailable` with `Retry-After` when the database queue is busy.
- `504 Gateway Timeout` when a query exceeds its time limit. The same query then fails fast for a
  few minutes. Narrow the filter instead of retrying unchanged.
- `429 Too Many Requests` with `Retry-After`.
- `X-Truncated: true` on the SQL endpoint only, which this client does not use.

## Endpoints used

| Function | Endpoint | Real fields used |
| --- | --- | --- |
| `fetch_player` | `GET /v1/players/steam?account_ids=...` | Steam name, profile URL, avatars, country, last update, 30-day match count, friend count. |
| `fetch_rank` | `GET /v1/players/{account_id}/rank` | Badge, tier, subrank, last ranked match metadata, progress change. |
| `fetch_rank_distribution` | `GET /v1/players/rank/distribution` | Player count per badge for the API's default 30-day window. |
| `fetch_matches` | `GET /v1/players/{account_id}/match-history` | Result, hero, K/D/A, net worth, last hits, denies, duration, mode, team, ranked badge and delta. |
| `fetch_hero_stats` | `GET /v1/players/hero-stats?account_ids=...` | Career per-hero matches, wins, kills, deaths, assists, time played, damage and rate fields. |
| `fetch_hero_names` | `GET /v1/assets/heroes` | Hero id and display name only. |
| `fetch_rank_names` | `GET /v1/assets/ranks` | Tier and tier name. |
| `fetch_leaderboard` | `GET /v1/leaderboard/{region}` | Place, account name, possible account ids, top hero ids. |
| `fetch_kill_heatmap` | `GET /v1/analytics/kill-death-stats?account_ids=...` | Kill and death positions, killer team, counts, for the API's default 30-day window. |
| `fetch_peer_benchmarks` | `GET /v1/analytics/hero-stats?min_average_badge=&max_average_badge=&min_unix_timestamp=` | Per-hero wins, losses, matches, kills, deaths, assists and net worth for the player's rank tier (subranks 1 to 6) over the last 30 days. |
| `search_players` | `GET /v1/players/steam-search?search_query=&limit=` | Steam profiles by name, for a pasted name or vanity link. Only profiles with recent matches come back. |
| `fetch_map_objectives` | `GET /v1/assets/map` | `objective_positions` only: Guardians (`tier1`), Walkers (`tier2`), Patrons (`titan`) and bases (`core`) per team and lane. |

`fetch_snapshot` composes the profile, rank, rank benchmark, hero rows and match window. It makes
about seven requests. Hero names are best effort: if the 1.9 MB asset request fails, rows keep their
hero ids and the statistics remain usable. The rank benchmark is also best effort because the
distribution endpoint allows only 5 requests per minute.

### Match history has no limit parameter

The match-history response is the full stored history. There is no cursor, page or limit parameter.
`fetch_matches` sorts newest first and truncates locally, which caps memory and screen rows but not
network use. Measured on 2026-11-16:

| Account | Rows | Payload |
| --- | --- | --- |
| `80488228` (soÿer) | 4,060 | 2.4 MB |
| `138006262` (사악한) | 673 | 409 KB |

Fetch the window once per screen and reuse it. Do not poll this endpoint.

Two `player_match_outcome` details matter for derived stats:

- `0` means invalid. It is common on older rows: `80488228` had 3,221 invalid rows out of 4,060.
- Only `1` (win) and `2` (loss) count towards win rate. `3` and `4` are penalized, and `5` is not
  scored. Those rows stay in the sample as unscored.

Career totals use the `wins` field from the hero-stats endpoint, which has its own default filters.
The match window uses `player_match_outcome`. The two sample sizes can therefore differ, and the
JSON reports both scopes instead of presenting one blended number.

## Derived metrics and formulas

Every derived field has a formula. None of them invent a game value.

| Field | Formula |
| --- | --- |
| `winRatePercent` | `100 * wins / (wins + losses)`. Unscored rows are excluded. |
| `kda` | `(kills + assists) / max(deaths, 1)`. Standard MOBA convention, not a ratio to infinity. |
| `averageMatchSeconds` | `timePlayedSeconds / matchesPlayed`. |
| `damagePerMinute` (career) | `sum(hero.damagePerMinute * hero.timePlayedSeconds / 60) / careerMinutes`. |
| `bossDamage` (career) | Sum of `total_boss_damage` from each hero row. |
| `objectiveDamage` (career) | `sum(hero.objectiveDamagePerMinute * hero.timePlayedSeconds / 60)`. The API has no career objective total, so this is a time-weighted estimate and is labeled as such. |
| `averageDurationSeconds` (window) | Mean duration over matches with `durationSeconds > 0`. |
| `averageNetWorth` (window) | Mean final net worth over the window. |
| `recentForm` | Up to 10 newest decisive matches in the window, mapped to W or L. |
| `recentWinRatePercent` | Win rate over those 10 entries. No smoothing. |
| `winRateLowPercent`, `winRateHighPercent` | 95 percent Wilson score interval for the binomial win rate. |
| `rank.benchmark.percentile` | `100 * sum(players with 0 < badge <= playerBadge) / sum(all ranked players)`. |

Wilson interval, z = 1.959963984540054:

```text
center = (p + z^2 / 2n) / (1 + z^2 / n)
margin = z * sqrt(p(1 - p) / n + z^2 / 4n^2) / (1 + z^2 / n)
low    = max(0, center - margin) * 100
high   = min(1, center + margin) * 100
```

Wilson is used because hero samples can be small. A 5-game hero at 60 percent is not the same claim
as a 300-game hero at 60 percent, and the interval makes that visible. The CLI prints the interval
next to the win rate.

The rank benchmark compares the player's badge with the current ranked population. It is not an MMR
number and not a skill rating. `badge = 0` is unranked and gets no benchmark. The distribution
endpoint's default window is the last 30 days, and the benchmark records that window.

There is deliberately no impact score, aim rating or positioning rating. See the gaps below.

## What the API does not support

| Claim | Reality | Decision |
| --- | --- | --- |
| Per-match position stream | Match history has no positions. Match metadata carries `death_details[].death_pos` and `killer_pos`, but one match metadata response measured 1.2 MB. The bulk metadata endpoint returned `404 No matches found` for a match the single endpoint served. The demo query API exposes `CCitadelPlayerPawn` body coordinates per tick, but it is asynchronous: about 13 seconds average, 200 requests/hour per IP, and the demo must be available. | Not implemented. Per-match movement paths need a caching demo-query lane. The synchronous kill/death raster is implemented instead, and it is aggregate only. |
| Position timeline for one match | Only through the asynchronous demo query (`POST /v1/matches/demo/query`, poll `/demo/query/{job_id}`). | Not implemented. Do not show a per-match path or positioning score. |
| Aim rating | Hero stats has aggregate `accuracy` and `crit_shot_rate`. Match metadata has shots hit and missed in the per-player time series. There is no published aim model for Deadlock and no shot-level telemetry in the history payload. | Do not invent an aim rating. Accuracy can be shown as a raw hero statistic with a sample count. |
| Positioning rating | Needs per-tick positions plus a model of good positions. Neither is in the history payload. | Do not invent one. The kill/death heatmap is evidence, not a rating. |
| Impact rating | Match history has K/D/A, net worth, last hits and denies, but no per-match player damage, healing, objective damage or trades. Match metadata has a per-player time series (`player_damage`, `player_healing`, `player_damage_taken`, shots) and a damage matrix, but each match costs about 1.2 MB and one request. | Not implemented. A Leetify-style impact rating needs metadata or demo ingestion first. KDA is labeled KDA, not impact. |
| Numeric MMR | The API exposes badge, subrank, progress change and rank distribution, not a public MMR number. | Show badge, tier, progress and percentile. Never display a made-up MMR. |
| Per-player hero counters and synergy | Global `/v1/analytics/hero-counter-stats` and `/v1/analytics/hero-synergy-stats` exist. Per-player `/v1/players/{account_id}/enemy-stats` and `/mate-stats` return player-level records, not hero-level matchups, and each response measured about 700 KB. | Deferred. Do not relabel player records as hero counters. |
| Build and item recommendations | `/v1/builds` and hero build analytics exist. | Separate feature, not part of the stats backend. |
| Objective positions per match | `/v1/assets/map` has normalized objective positions. Match metadata has objective destruction events with times. | Future timeline work, not in this module. |
| Match timeline | Metadata has objective events, death details, pauses and a damage matrix. The match-history payload does not. | Future review surface, not in this module. |
| Leaderboard rank as badge | Leaderboard `rank` is a place, not a rank badge. `possible_account_ids` can be wrong because Steam names are not unique. | The CLI prints candidate ids and asks for the id, rather than picking one. |

Map data is available for heatmap rendering: `/v1/assets/map` returned a radius of 10752, minimap,
frame and tunnel images, and normalized `left_relative` and `top_relative` positions for cores,
titans and other objectives. Fetch the map asset instead of hard-coding image URLs because the map
can change between patches.

## What the case study changes

`docs/research/stats-case-study.md` studied Leetify, Blitz.gg and Scope.gg. The conclusions that
affect this backend:

Adopted now:

- A benchmark, not a raw number: rank percentile against the current ranked population.
- Sample size and uncertainty: Wilson intervals on every win rate, plus the match count.
- Recent form and a window that can differ from career totals, with both scopes labeled.
- Hero pool sorted by sample size, with the interval visible.
- A real heatmap with a legend, a stated window and a sample count, or no heatmap at all.
- A short list of what is unavailable, so the screen does not imply missing data exists.

Deferred until the data lane exists:

- Impact, aim and positioning ratings. The API does not carry the needed telemetry in the cheap
  payloads, and inventing formulas would fail the case study's data honesty rules.
- Per-match review, demo clips and comparison views. These need demo or metadata ingestion.
- In-game overlays. The case study explicitly keeps them out of the first stats screen.

Rejected:

- A single composite rating with no published formula.
- A per-match heatmap built from the 30-day aggregate.
- Color as the only carrier of win or loss, above or below benchmark.
- Treating a 1-game hero as a win-rate story.

## Visualization recommendations

The screen should read as one summary, then evidence. Use a strong player header, then charts and
tables, not ten equal cards.

### 1. Player header

- Avatar, name, rank badge and tier, rank percentile, career record, recent form.
- Rank progress from the last ranked match and the change it awarded.
- Label the career scope: hero-stats filters (normal mode, ranked plus unranked) differ from the
  match window.

### 2. Rank progression

- Step line of `ranked_badge` from match history over `start_time`, newest at the right.
- A step chart, not a smooth line, because badges are discrete.
- Annotate promotions and demotions using `ranked_delta`.
- Filter to ranked matches. Unranked rows have `ranked_badge = null`.
- Do not plot badge 0 as a real tier. Break the line or start after the first ranked badge.

A line chart suits change over time. The FT Visual Vocabulary classifies change over time as a
line family and warns against using bars for a continuous sequence.

### 3. Hero pool

- Horizontal bars for win rate, sorted by matches played.
- Wilson interval whiskers on each bar.
- Always show `n`. Put low-sample heroes behind a "show all" control and start with a minimum of
  5 or 10 matches.
- A second small chart for KDA or damage per minute; do not put five measures on one bar.
- Sort by sample size, not by win rate, so a 1-game 100 percent hero cannot dominate the screen.

Data to Viz notes that exact values are hard to read from color alone, so keep the numeric value
next to the bar.

### 4. Recent form

- Ten W/L chips plus the exact record.
- A rolling 20-match win-rate line with the Wilson band, if the window is large enough.
- Do not draw a trend line through ten binary outcomes. It suggests signal that is not there.
- Mark unscored and penalized matches distinctly and do not count them as losses.

### 5. Match list

- One row per match: date, hero, result, K/D/A, KDA, net worth, duration, ranked badge and delta.
- Result needs text or a shape as well as color.
- Do not add a net-worth sparkline: the history payload has only the final net worth, not a series.
- Expanding a row can show the fields that exist. It must not show a damage chart until the
  metadata or demo lane is implemented.

### 6. Kill and death heatmap

- Render `KillHeatPoint` on the map from `/v1/assets/map`.
- World units: the map radius is 10752, so normalize with `(x + radius) / (2 * radius)` and the
  matching transform for `y`. Confirm orientation against the minimap image before shipping.
- Draw team 0 and team 1 separately, or use one hue with two shapes. The API gives `killer_team` for
  every cell.
- The raster is sparse: 2,586 cells held 1,598 kills and 1,438 deaths for one account. A gaussian
  blur or hexbin is appropriate; a raw scatter of 2,586 points is not.
- Legend must state counts, the 30-day window and the cell count. State that it is not filtered to
  one match.
- Do not draw player movement paths. Those are not in this data.

Data to Viz calls out normalization and color palette as the two common heatmap mistakes. Normalize
per layer or per fixed count scale, and pick a sequential palette with a colorbar.

### 7. Benchmarks and color

- Rank percentile is the only population benchmark implemented. Show it as "top X percent" or
  "ahead of X percent", with the ranked player count and 30-day window.
- When more benchmarks arrive, use green above, amber at, and red below, plus an arrow and the
  numeric value. WCAG 1.4.1 does not allow color alone to carry meaning.
- Keep benchmark color out of the raw stat text. Use a colored trend chip plus a label.

### 8. What not to draw yet

- A radar chart of aim, positioning, impact and utility. The data does not exist.
- A per-match map with paths. The data is asynchronous and not fetched.
- A hero counter matrix from per-player enemy records. Those are player records.
- A single "performance score". There is no published formula.

## Stats screen

Status: implemented in `rust/crates/desktop/src/screens/stats/`, verified with the fixture scenes
`stats-profile`, `stats-drill`, `stats-entry`, `stats-board`, `stats-people` and `stats-tracked`
(`dah-desktop --list-scenes`). Updated 2026-11-17.

The screen loads one account with `fetch_snapshot(account, 200)`, then the peer benchmark, the
kill and death raster and the map objectives. Match history has no paging, so 200 rows cost the
same single request as 20 and give the rank chart and the form comparisons enough games.

Order, top to bottom, and the rule each part follows:

1. **The brief.** One headline for what changed and one "Next" item for what to work on, from
   `deadlock_core::stats::insight::brief`. Up to two more notes, positive ones first.
2. **Your last 20.** Win rate, KDA, deaths a match and souls a minute for the newest 20 matches,
   against the matches before them in the window, with the tier average where the API has one and
   a sparkline of the last 30 matches (rolling mean). Change carries an arrow and a signed number,
   not colour alone. A change under 3 percent reads as level.
3. **Progress.** Rank as a step line over the ranked matches in the window (badges are discrete),
   or the rolling 20 match win rate shaded green above even and red below. Under it, the last 30
   results as beads: a win stands up, a loss hangs down.
4. **Heroes.** Sorted by matches. Win rate on a 0 to 100 track with its 95 percent Wilson range,
   the rank tier's win rate on the hero as a tick (when that hero has at least 100 tier matches),
   and the gap as a signed number. Heroes under 10 matches are dimmed and say "few".
5. **Matches by sitting.** A sitting ends after a 3 hour break. Each sitting heads its matches with
   the record and the rank points it moved. A row opens to the fields the history holds, the match
   against the player's average on that hero, the way to the scoreboard for that match and the way
   to a demo review.
6. **The scoreboard.** Opened from a match row, on its own worker: both teams in net worth order,
   each player's K/D/A, souls, damage and healing from the last sample, the rank badge the match was
   played at, the MVP vote, the final build (items still held at the end, in shop order weapon,
   vitality, spirit) and the soul lead every three minutes. One metadata response per match, kept
   for a week because a finished match never changes. Player names come from one profile request
   for the twelve account ids. Item names come from `/v1/assets/items/by-type/upgrade`; item art
   would need `assets-bucket.deadlock-api.com`, which is not on the allowlist, so builds are text.
7. **Players you meet.** Teammates and opponents with at least three shared matches, best teammates
   by win rate (five games or more), frequent opponents by games, the win rate with and against
   them, and a pin for rows that were queued in the same party. One press loads the three responses
   (about 700 KB each); a row opens that player's page. Names are one profile request for the top
   fifty rows.
8. **Leaderboard.** Region (Europe, NAmerica, SAmerica, Asia, Oceania) and season from
   `/v1/assets/ranked-seasons`. The region defaults to the profile's country. The player's row is the
   entry whose `possible_account_ids` hold their account id; it is marked in place on the first page
   and repeated under the list when it is further down. An empty current board falls back to the
   previous season.
9. **Where fights happen.** The 30 day kill and death raster on a 48 by 48 grid, drawn in the
   minimap frame with a lane skeleton through the objectives. Deaths or kills, one layer at a time.
   The three busiest areas (3 by 3 cell sums, kept 4 cells apart) are numbered and named after the
   nearest objective: "Near the enemy Guardian, middle lane". The team for "your" and "the enemy"
   comes from `killer_team` in the raster.

World units map to the minimap frame with `left = (x + r) / 2r` and `top = (r - y) / 2r`,
`r = 10752`. The camp positions in `/v1/assets/map` (world position and relative position side by
side) confirm the transform. The earlier screen plotted y without the flip.

### Insight rules

`rust/crates/deadlock-core/src/stats/insight.rs`. Thresholds follow the Deadlock Mod Manager study
(15 decided matches per side, 6 points of win rate) so a sentence never rests on a handful of games.

| Insight | Fires when | Wording |
| --- | --- | --- |
| Rank move | The badge changed over the week ending at the newest ranked match. | "Up to Eternus 5 this week." |
| Win streak | 3 or more wins in a row, unscored rows skipped. | "6 wins in a row." plus personal best and the last rank delta. |
| Loss streak | 3 or more losses in a row. Always the "Next" item. | Quotes the player's own win rate after 3 straight losses (10 or more cases), else how often such runs happen. |
| Form up or down | Newest 20 against the rest of the window, both 15 or more decided, gap 6 or more points. | "Winning 70% lately, up from 57%." |
| Deaths up or down | Deaths a match changed by 15 percent (up) or 13 percent (down), 15 or more matches each side. | "Dying more: 6.4 a match over your last 20." |
| Comfort or trap hero | A hero with 20 or more career matches, played in the last 60 days, 6 or more points above or below the career rate. | "Paradox is costing you." with the tier's rate on the hero. |
| Long sittings | Games 3 and later of a sitting win 6 or more points less than games 1 and 2, 15 or more each. | "Long sittings cost you." |
| Peer gap | A hero played 3 or more times in the last 20 whose career rate trails the tier's rate on it. | "Grey Talon trails your rank." |
| Last sitting | Always, as the fallback headline. | "Last sitting: 4 W 2 L, +930 rank points." |

"Next" is the focus item with the largest effect times the square root of its sample. A loss streak
always wins, because it is the one the player can act on before the next queue.

### Account entry

`parse_account_input` accepts a SteamID3, a SteamID64 (minus `76561197960265728`), `[U:1:n]`, and
`steamcommunity.com/profiles/<id64>` links. A `steamcommunity.com/id/<vanity>` link or a plain name
becomes a `steam-search` query and the screen lists the matching active players to pick from.
Resolving a vanity URL exactly needs a Steam Web API key, which the app does not hold.

Loaded accounts are remembered, most recent first, in `stats-accounts.json` in the app data
directory (account id and persona name only, at most six). The last one loads when the screen opens.
The switcher in the page head lists them and can forget one.

### Cache

`deadlock_core::stats::cache::ResponseCache` keeps one file per request under the app data
directory (`cache/stats`). A response inside its lifetime is used without a request; an older copy
answers when the API cannot be reached and the page says "Showing an older copy". Manual refresh
waits 60 seconds.

| Endpoint | Lifetime |
| --- | --- |
| Match history, hero stats, kill and death raster | 10 minutes |
| Rank | 30 minutes |
| Rank distribution, peer benchmarks | 12 hours |
| Teammates, opponents, leaderboard | 1 hour |
| Steam profiles | 24 hours |
| Match metadata, hero, rank, season and item assets, map | 7 days |

### Tracked players

The account field takes a SteamID3, a SteamID64, `[U:1:n]` and both profile link forms. Loading an
account tracks it (at most six, primary first, in `stats-accounts.json`). One is the primary account
and the page opens on it. `deadlock_core::steam_users` reads `config/loginusers.vdf` from the Steam
roots discovery already knows, and the panel offers those accounts as a one-press add. Detection
runs on the stats worker, never on the UI thread.

### Fixture

`rust/crates/desktop/fixtures/stats-profile.json` is a capture of the raw API responses for a public
leaderboard account captured on 2026-11-17, with the name, account id and avatar
replaced. Match history keeps 200 rows and the used fields; the kill and death raster is
re-aggregated to 384 world unit cells and stored as `[x, y, killer_team, kills, deaths]` rows.
`deadlock_core::stats::snapshot_from_bundle` parses it through the same code as a live load. The
generator is a local script (`.scratch/ui-hill/stats2/make-fixture.py`).

### Not done

- Steam avatars are not shown: `avatars.steamstatic.com` is not on the network allowlist. The page
  shows the persona initial.
- Rank badge art from `/v1/assets/ranks` lives on `assets-bucket.deadlock-api.com`, also not on the
  allowlist. Ranks are text.
- Hero chip icons come from the local Grimoire art (`hero_portraits/chip-icons`). The packaging
  script does not copy that folder yet, so a packaged build shows letters.
- Item icons are not drawn, for the same reason: `assets-bucket.deadlock-api.com` is not on the
  network allowlist.
- The live match view is designed but not built: `docs/design/stats-live.md`.


## CLI surface

The core lives in `rust/crates/deadlock-core/src/stats.rs`. The command module is
`rust/crates/dah-rs/src/commands/stats.rs` with public `run_*` functions:

```text
dah stats player <account-id> [--recent N] [--json]
dah stats matches <account-id> [--limit N] [--json]
dah stats heroes <account-id> [--json]
dah stats heatmap <account-id> [--json]
dah stats leaderboard <region> [--limit N] [--json]
```

Regions: `Europe`, `Asia`, `NAmerica`, `SAmerica`, `Oceania`.

JSON output is wrapped in an object with `ok`, `source` and the payload. The player payload is the
full `StatsSnapshot`. Match and hero payloads include the derived window or career block so a UI
does not have to recompute formulas.

Wiring status: `commands/mod.rs` and `main.rs` were outside this change, so the commands are not
reachable from the `dah` binary yet. The core module also needs `pub mod stats;` in
`rust/crates/deadlock-core/src/lib.rs`. Until both lines and the clap arms exist, describe this as
an implemented backend, not an available command.

## Live verification

Verified on 2026-11-16 with the public account `138006262` (사악한, Europe leaderboard place 5,
Eternus 4). No API key was set.

```text
사악한 (account 138006262)
Steam: https://steamcommunity.com/id/hlias1275/
Steam profile refreshed: 2026-11-16 02:13  Friends: 20  Matches last 30 days: 217
Rank: Eternus 4, badge 114  |  last ranked 2026-11-15 15:44  progress -350  match 137990616  |  top 0.01% of 409,569 ranked players
Career: 664 matches, 391 W / 273 L (58.9%, 95% CI 55.1-62.6%) 6272/3770/7410 K/D/A at 3.63 KDA, 27,383,515 damage (6,883,882 objective), avg 33m 22s
Heroes: 12 played, led by Mina

Recent window: newest 20 match(es)
  Record: 15 W / 5 L (75.0%, 95% CI 53.1-88.8%)  Unscored: 0  K/D/A: 130/152/261 at 2.57 KDA
  Average duration: 29m 57s  Average net worth: 39,248
  Recent form (last 10): L W W L W W L W W W  70.0%

Top heroes by matches:
  Mina                 354 matches    59.0% win       53.8-64.0%    4.31 KDA  2026-11-15 12:05
  Lady Geist           120 matches    64.2% win       55.3-72.2%    2.47 KDA  2026-11-15 13:26
  Haze                  74 matches    59.5% win       48.1-69.9%    3.58 KDA  2026-11-15 15:44
```

The same command on public account `80488228` (soÿer, Europe leaderboard place 2) exercised the
unranked path:

```text
Rank: unranked (Obscurus, badge 0)  |  last ranked 2026-11-15 04:56  progress unknown  match 137739924
Career: 3,178 matches, 1,773 W / 1,405 L (55.8%, 95% CI 54.1-57.5%) 23477/20507/44081 K/D/A at 3.29 KDA, 98,149,519 damage (21,005,726 objective), avg 31m 30s
```

Heatmap command for `138006262`:

```text
Kill and death heatmap for 138006262 (last 30 days)
Cells: 2,586  Kills: 1,598  Deaths: 1,438

Busiest cells:
  (   128,    896)  team 0  2 kills  3 deaths
  (   256,   -384)  team 0  4 kills  0 deaths
  (   512,      0)  team 1  4 kills  0 deaths
```

Leaderboard command for Europe returned place 1 MikaelS with 11 candidate account ids, place 2
soÿer with one id (`80488228`), and place 5 사악한 with one id (`138006262`). This is why the CLI
prints candidate ids instead of choosing one.

Verification commands:

```bash
cargo check --manifest-path rust/Cargo.toml -p deadlock-core
```

The CLI module was compiled in an isolated check because `commands/mod.rs` is not wired. It
compiles with no errors; the only warnings are dead-code warnings for the unused command surface.

## Sources

- Deadlock API OpenAPI: <https://api.deadlock-api.com/openapi.json>
- Deadlock API docs: <https://api.deadlock-api.com/docs>
- Deadlock API repository and licence: <https://github.com/deadlock-api/deadlock-api>
- Deadlock API data lake notes: <https://deadlock-api.com/data-dumps>
- Deadlock hero assets: <https://api.deadlock-api.com/v1/assets/heroes>
- Deadlock ranks: <https://api.deadlock-api.com/v1/assets/ranks>
- Deadlock map asset: <https://api.deadlock-api.com/v1/assets/map>
- Case study, Leetify, Blitz.gg and Scope.gg: `docs/research/stats-case-study.md`
- Leetify: <https://leetify.com/>
- Leetify rating explanation: <https://leetify.com/blog/leetify-rating-explained/>
- Leetify statistics glossary: <https://leetify.com/blog/leetify-stats-glossary/>
- Leetify benchmarks: <https://leetify.com/blog/cs2-benchmarks/>
- Blitz Deadlock: <https://blitz.gg/deadlock>
- Scope CS2 dashboard: <https://scope.gg/cs2-dashboard/>
- Scope: <https://scope.gg/>
- Tracklock Deadlock: <https://tracklock.gg/>
- Deadlock Tracker: <https://deadlocktracker.gg/>
- OpenDota API, a reference MOBA player and match API: <https://docs.opendota.com/>
- Data to Viz, heatmap definition and common mistakes: <https://www.data-to-viz.com/graph/heatmap.html>
- FT Visual Vocabulary, chart choice by data relationship:
  <https://ft-interactive.github.io/visual-vocabulary/>
- WCAG 2.2, Use of Color: <https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html>
