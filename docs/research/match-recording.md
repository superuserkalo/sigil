# Local match recording and AI review

Status: proposed. Researched 2026-11-16.
Updated: 2026-11-16

Idea: when the public API lacks a stat, record the user's own matches locally, build a private
dataset, and let a model review the verified summary. This is the fallback for positions, duels,
objective timing and other fields the API does not expose.

This is feasible without reading game memory. Do not touch process memory or inject into the game.

## What already exists

- **Source 2 demo files** for Deadlock can be parsed by open-source tools:
  - `source2-demo`, a Rust crate with a Deadlock feature.
  - `boon-deadlock`, a Rust/Python parser with match metadata, entities, events and Polars
    datasets.
  - `deadem`, JavaScript libraries and an in-browser demo viewer.
  - `deadlock-api` has a demo analyzer that streams events and metadata from `.dem` files.
- **Overwolf game events** expose Deadlock game and match info to Overwolf apps. That is a separate
  runtime and permission model, not this app.
- **GSI**: Valve has documented Game State Integration for CS2 and Dota 2. A Deadlock forum thread
  asks whether Deadlock supports it, with no confirmed public config. Do not depend on it.

## Why demos first

- No memory reading, no anti-cheat risk, no injected overlay.
- The demo contains the match as the server saw it: players, positions, kills, objectives, items,
  souls, timers and round state.
- It is reproducible. The same file can be re-parsed when the schema improves.
- It is local-first. A demo can stay on disk and never be uploaded.

## Pipeline

```text
demo file
  -> copy into <data>/matches/<match-id>/match.dem
  -> parse header and metadata
  -> stream ticks and events
  -> events.jsonl + positions.bin + summary.json
  -> deterministic stats and heatmaps
  -> LLM coach reads summary.json only
  -> review.md with actions and timestamps
```

### Storage layout

```text
<data>/matches/<match-id>/
  match.dem
  meta.json          match id, map, duration, build, players, result
  events.jsonl       kills, deaths, assists, objectives, items, souls
  positions.bin      compact per-tick player positions and team
  summary.json       derived stats, sample counts, heatmap aggregates
  review.md          generated coaching narrative
  provenance.json    parser version, source, created at, hashes
```

### Extraction targets

- Match metadata: map, duration, build, game mode, result.
- Players: hero, team, lane, net worth, items.
- Combat: kills, deaths, assists, damage, healing, objective damage.
- Economy: souls over time, item purchases and timings.
- Objectives: mid boss, urn, walkers, shrines, patron phases.
- Positions: per-second X/Y/Z, alive state, team.
- Events: tower/objective takedowns, team fights, large soul swings.

### Derived statistics

All computed locally, with formulas in code:

- KDA, damage per minute, objective damage per minute, healing per minute.
- Lane phase soul differential.
- Death locations and time-of-death clusters.
- Kill participation and trade rate.
- Objective timing and conversion.
- Heatmap cells: deaths, kills, farm, team-fight density.
- Map control by area over time, if positions are reliable.

Every stat carries its sample count and whether it is above or below a benchmark.

## LLM review contract

The model never sees raw ticks. It receives `summary.json` plus a small set of event windows:

```json
{
  "match": {"map": "...", "duration": 2410, "result": "loss"},
  "player": {"hero": "...", "kda": [7, 4, 9], "gpm": 612},
  "weaknesses": [
    {"metric": "deaths_before_10m", "value": 5, "benchmark": 2.1},
    {"metric": "objective_damage_per_min", "value": 180, "benchmark": 320}
  ],
  "events": [{"at": 842, "type": "death", "area": "mid", "gold_swing": -900}]
}
```

Rules:

- The model may explain, prioritize and suggest.
- It may not invent a number not present in the summary.
- Every recommendation names the timestamp or metric that supports it.
- The prompt says "if the data does not support a claim, omit it."
- A deterministic linter checks generated numbers against the summary.

## Privacy

- Local only by default. No match upload, no telemetry.
- Sharing a review is opt-in and excludes positions and player names unless the user includes them.
- A demo may contain other players' names. Treat it as personal data.
- No provider call until the user asks for a review.

## Phases

1. Import a `.dem`, store provenance, parse metadata and players.
2. Extract events and positions into the local format.
3. Compute summary and heatmap aggregates.
4. Build the Stats screen views: player header, match list, map heatmap, review.
5. LLM coach from the verified summary.
6. Watch a demo folder and auto-import new matches.
7. Optional: live event capture through a supported official surface, never memory reading.

## Open questions

- Where does Deadlock write demos, and are they downloadable after a match?
- Does `source2-demo` or `boon-deadlock` cover the current Deadlock demo version and build?
- How large is a typical demo and how long does parsing take?
- Does the demo contain reliable positions for all players, or only the local player?
- Which fields are missing from deadlock-api that demos actually solve?

## Sources

- https://github.com/Rupas1k/source2-demo
- https://github.com/Igor-Losev/deadem
- https://deadem.com/
- https://pypi.org/project/boon-deadlock/
- https://deepwiki.com/deadlock-api/deadlock-api/3.5-demo-analyzer
- https://dev.overwolf.com/ow-native/live-game-data-gep/supported-games/deadlock/
- https://forums.playdeadlock.com/threads/is-game-state-integration-gsi-available.185655/
- https://developer.valvesoftware.com/wiki/Counter-Strike:_Global_Offensive_Game_State_Integration
