# A live match view

Status: design only. Nothing here is built. The rows it would fill in
`docs/design/feature-parity.md` (Stats: "Live match view") stay missing until the two data lanes
below exist. Updated 2026-11-17.

Deadlock Mod Manager shows a live match overlay (souls, timers, net worth). Grimoire does not. The
row is marked L because no public endpoint carries a player's own live state, and reading it needs
two things Sigil does not have yet: the game's console log written by `-condebug`, and a HUD bridge
mod that exposes the pause-screen numbers. This note fixes what those are, what the screen would
show, and what it refuses to do.

## What exists today

- `deadlock_core::game_session` already tails `game/citadel/console.log` (with `-condebug` on) and
  reads the phase, the map, the hero codename, the party size, the player count and bots. The
  Discord presence and Settings > Game watching use it. Polling is incremental: `Tail::poll` reads
  only the new bytes.
- `GET /v1/matches/active?account_ids=<id>` returns the active matches the game's Watch tab lists
  (the top 200 by spectators). A row carries the match id, the game mode, the two team net worths,
  the twelve account ids, the hero ids and the objectives mask. It is anonymous and has no
  per-player items, positions, cooldowns or timer detail.
- `GET /v1/matches/{id}/metadata` is the same payload the scoreboard reads, but only after the match
  ends. It is not a live feed.

So: the console tells Sigil *that* a match is running and with which hero; the API occasionally
tells it the ceiling (is this match in the Watch tab); neither tells it the player's own souls,
items or cooldowns.

## The two lanes

### 1. Console log (`-condebug`)

Read-only, local, no network. Already implemented and tested in `game_session`.

| What | Source line | Use |
| --- | --- | --- |
| In a match, hero | `[Server] Loaded hero N/hero_<codename>`, map and game-state changes | Show the live strip only while `Phase::InMatch`, name the hero |
| Party size | `party ... members` | "Duo", "party of 4" |
| Mode | map name (`street_test`), bots flag | Standard, Street Brawl, bot match |
| Match end | `Lobby N for Match M destroyed`, disconnect lines | Close the strip and offer the scoreboard |

Limits, stated on the screen: the log has no souls, items, cooldowns, positions or objective health.
A `-condebug` game writes a file that grows; `Tail` already caps its replay at 4 MB and reads from
the end.

### 2. HUD bridge mod (the missing half)

A normal Sigil mod (a `pakNN_dir.vpk` in `citadel/addons`) that replaces the pause screen HUD layout
and, once per second while the match runs, writes one small JSON file:

```json
{
  "match_id": 138737576,
  "time_s": 942,
  "hero": "hero_haze",
  "souls": 28450,
  "souls_per_min": 1812,
  "items": [548943648, 3452399392],
  "kills": 7, "deaths": 3, "assists": 11,
  "team_souls": [163473, 165228]
}
```

- Written to `game/citadel/sigil-live.json`, overwritten atomically (write a `.part` file, then
  rename) so Sigil never reads half a line.
- Panorama can read the local player's stats through the HUD's own panel data. It cannot read the
  other eleven players' items or cooldowns; those stay unavailable until Valve exposes them.
- The mod is a normal addon: Sigil packs it, installs it into an addon slot from a user action, and
  the user can remove it. Nothing about it is special-cased in the agent tools.
- It is opt-in: a switch in Settings > Game watching, "Live match numbers (HUD mod)", off by
  default, because it changes the HUD and because a third-party HUD may interact with how Valve
  treats a session. The screen says so in one line, not a paragraph.

The mod is the one place Sigil would ship game-facing code. Until it exists and is verified in a
real match, the live view stays unbuilt: a strip that shows only "you are in a match" is not worth
the row.

## The screen

Shown at the top of Stats only while `Phase::InMatch`, above the brief, and gone when the match
ends. One row, 52 points tall, no card:

- Hero chip and name, mode, party size, and the match clock from the bridge (or the console's last
  state change when the bridge is absent).
- Souls and souls a minute; the team soul lead from `/v1/matches/active` when the match is in the
  Watch tab, otherwise a dash with "not in the Watch tab".
- A single action: "Open live on deadlock-api" is wrong (there is no live page); instead "Show this
  match when it ends" sets a marker so the Stats page opens the scoreboard for that match id the
  next time it appears in the history.

When the match ends, the strip becomes one line for a minute: "Match ended, 31 minutes. Scoreboard
when the API has it." with a button that opens the scoreboard once `/v1/matches/{id}/metadata`
answers.

Nothing here is an overlay. Sigil does not draw over the game, read game memory, hook DirectX, or
sniff packets. Anything that needs that is out of scope, not deferred.

## Polling and budget

| Lane | Frequency | Cost |
| --- | --- | --- |
| Console tail | every 500 ms while Deadlock runs | one stat and a seek, no network |
| `/v1/matches/active` | once at match start, then every 60 s, at most 45 times per match | one request, about 200 KB |
| HUD bridge file | every second, read only when its length or modified time changed | one stat, no network |

No request carries an account id anywhere except `api.deadlock-api.com`, which is already on the
network allowlist for Stats. Offline mode disables the active-match lane and leaves the console
strip working.

## What stays unavailable

- Other players' souls, items and cooldowns in real time. The Watch tab data has team totals only,
  and it only covers the top 200 matches.
- Positions and objective health. The demo query lane is asynchronous and takes 13 seconds.
- Last-hit and deny timers, camp timers and rune timers: these are game client knowledge, not in any
  public endpoint, and the HUD mod would have to read them from panels that do not expose them.

The screen shows what it has and says what it does not. It never estimates a live number.

## Steps to build it

1. Verify the HUD panel data in one real match: which values the pause-screen panel exposes and
   whether a mod can write a file from Panorama. If it cannot, stop; do not guess a schema.
2. Write the bridge mod, package it with Sigil, and add the opt-in switch.
3. Add a `deadlock-core` reader for `sigil-live.json` with unit tests against a captured file, and a
   `StatsClient::fetch_active_matches(account_id)` parsed into the fields listed above.
4. Add the strip to the Stats page with a scene fixture for each state: in match with the bridge, in
   match without it, match ended, and stale bridge file (older than 10 seconds reads as "no signal").
5. Update this note, `docs/research/stats.md` and the feature-parity row when the first real match
   has been watched end to end.
