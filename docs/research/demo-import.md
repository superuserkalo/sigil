# Local demo import and the merged match summary

Status: active. The container parser, local store, merged summary and CLI are implemented and wired.
No real Deadlock `.dem` has been parsed on this host yet; the framing is verified against the
`source2-demo` implementation and against synthetic files. Updated: 2026-11-16.

This is the depth half of the stats product. `docs/research/stats.md` covers the public API (breadth
and population context). This document covers local demo ingestion, what the container parser can
prove, what it cannot, and how the API record is merged without hiding a conflict.

The product rules that apply:

- Local first. A demo never leaves the machine during import. No telemetry, no upload.
- A demo may contain other players' names. Treat the store as personal data.
- Never show a number a parse did not produce. Every derived field carries its source, confidence
  and sample count.
- The importer never writes to a game install. It copies a file into the workspace and writes
  derived JSON next to it.

## In plain language

A Deadlock match recording is a Source 2 demo: a container full of compressed command frames. The
container alone carries the match header, the playback length and string table names. It does not
carry "player 3 died at mid at 12:04". That needs the game's network serializer and the Citadel
message schemas, because entity state is delta encoded and most game facts live inside user
messages.

So the importer is split in two:

1. `container`: always available, decodes framing and metadata, never invents players or positions.
2. `source2-demo` or another serializer aware backend: not linked today. It would add entities,
   events, positions and Citadel match metadata through the same `DemoExtractor` seam.

The merged summary states which fields are present, partial or absent. A heatmap is only drawn from
position samples that a backend actually produced.

## Source 2 demo format as verified

The layout below is what `source2-demo` implements for Source 2 and what the Rust importer now
mirrors. The reference files are `source2-demo/src/stream/reader/msg.rs`,
`source2-demo/src/parser/mod.rs` and `source2-demo-protobufs/protos/common/demo.proto`.

| Offset or field | Meaning |
| --- | --- |
| Byte 0, 8 bytes | Magic `PBDEMS2\0` (`50 42 44 45 4d 53 32 00`). |
| Byte 8, 4 bytes LE | Offset to the `CDemoFileInfo` command. |
| Byte 12, 4 bytes LE | Second offset. Its meaning is undocumented in the sources used, so the importer records it and does not interpret it. |
| Byte 16 | Start of the command stream. |

Every command frame is:

```text
varint command
varint tick
varint payload size
payload bytes
```

The command value has a compression flag at bit 6 (`64`). When set, the payload is a raw Snappy
block. The importer implements Snappy literal and copy decoding in `demo.rs`, so compressed
metadata commands are readable without adding a dependency. Unknown commands are counted by raw id,
not treated as a parse failure.

Decoder coverage today:

| Command | Name | What is decoded |
| --- | --- | --- |
| 0 | `DEM_Stop` | End of stream marker. |
| 1 | `DEM_FileHeader` | `CDemoFileHeader`: `demo_file_stamp`, `patch_version`, server and client name, map name, game directory, fullpackets version, addons, demo version name and GUID, build number, game, server start tick. |
| 2 | `DEM_FileInfo` | `playback_time` (seconds), `playback_ticks`, `playback_frames`, nested `CGameInfo` byte length. |
| 6 | `DEM_StringTables` | Table names, item counts, clientside counts, data byte totals and up to 16 sample keys per table. |
| 7, 8 | `DEM_Packet`, `DEM_SignonPacket` | `CDemoPacket.data` (field 3), then the packet message framing. |
| 10 | `DEM_CustomData` | Callback index, byte length and a printable preview. |
| 11 | `DEM_CustomDataCallbacks` | Save ids. |
| 13 | `DEM_FullPacket` | Nested string tables and packet. |
| 18 | `DEM_Recovery` | Counted only. |

`CDemoUserMessage` is not a standalone command in the verified proto. Source 2 frames user messages
inside `CDemoPacket`, as `ubit_var message id`, `varint size`, payload. The importer decodes that
framing and keeps an inventory: raw id, count and payload bytes. The only id it names is
`316`, `k_EUserMsg_PostMatchDetails`, which is verified in the Deadlock proto. The payload is counted,
not decoded. That is where Deadlock match metadata and the player list will come from once a
serializer aware backend is linked.

`playback_time` is a float in seconds, so duration does not need a tick-rate assumption. If a demo
reports ticks but no playback time, the summary keeps `durationTicks` and marks
`match.durationSeconds` as partial. The importer never divides ticks by a guessed tick rate.

## Store layout

```text
<workspace>/matches/<match-id>/
  match.dem          the copied Source 2 demo
  meta.json          compact list row and import timestamps
  events.jsonl       one JSON object per extracted event
  summary.json       merged fields, coverage, heatmap, disagreements
  provenance.json    importer version, source path, byte length, CRC32, parse time, extractor, missing fields
  api.json           optional cached raw deadlock-api response
  review.md          optional coaching review, never deleted by an import
```

- The match id comes from `--id`, else a sanitized file stem, else `demo-<crc32>`.
- Allowed id characters are ASCII letters, digits, dot, hyphen and underscore. `.` and `..` are
  rejected, so a supplied id cannot escape `matches/`.
- The magic is checked before the match folder is created. A wrong file cannot leave a partial store.
- Re-import increments `revision`, keeps the first `importedAt`, refreshes `updatedAt`, and preserves
  `review.md`. Files the importer does not own are never removed.
- A fixture import writes `fixture.json` instead of `match.dem` and records `source.kind: "fixture"`.
- `events.jsonl` and `meta.json` are written first; `summary.json` and `provenance.json` last.
- `provenance.missingFields` describes what the extractor did not produce. It can still list
  `players` after `api.json` supplied them, because provenance is about the demo parse, not about
  the merged summary. `summary.coverage` is the merged view.

## Extractor seam

```rust
pub trait DemoExtractor {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn extract(&self, demo: &DemoFile) -> Result<DemoExtract>;
}
```

`DemoExtract` carries:

- `match_facts`: map, duration seconds and ticks, result, start time, mode, build, server name,
  rank, MMR, population percentile.
- `players`: account id, name, hero, hero id, team, K/D/A, net worth, badge, MMR, result.
- `events`: tick, time, kind, actor, target, assists, team, position, detail.
- `positions`: time, player, account id, team, kind (`position`, `death`, `kill`), x, y, z.
- `derived_kda`: counts keyed by account id or normalized name, plus resolved and unresolved kill
  samples.
- `missing_fields` and `notes`.

A backend that returns events and positions is enough. It does not touch the store or the merge.

`backend_status()` reports the lanes without pretending:

- `container`: available. Metadata and inventory only.
- `fixture`: available, marked `testOnly`. Documented JSON for tests and UI development.
- `source2-demo`: unavailable. It is a Rust library, so wiring it is a dependency decision, not an
  install step. Its Deadlock proto set must be kept in sync with the game build.
- `deadlock-api demo analyzer`: unavailable. It would upload the demo to a third party, which the
  local first rule forbids without explicit opt in.

## What is real and what is not

Real today, from the container:

- Magic and framing, command counts, compressed and unknown command counts.
- File header fields listed above, when `DEM_FileHeader` is present in the stream.
- `playbackTimeSeconds`, `playbackTicks`, `playbackFrames`.
- String table names, item counts and sample keys.
- Packet counts and a raw user message id inventory, including whether message 316 appears.
- CRC32, byte length, source path, extractor name, importer version and parse timestamp.
- Coverage and the list of fields that were not extracted.

Not real today, and recorded as absent:

- Players, heroes, teams, account ids.
- Kills, deaths, assists, damage, healing, net worth.
- Objectives, items, souls, timers.
- Event timeline and per-player evidence.
- Positions, movement, heatmap cells.
- Match result, start time, game mode and rank, unless a cached `api.json` provides them.

The summary never turns an absent field into zero. A heatmap is `available: false` with a reason and
`coverage.fields.heatmap: "absent"`.

## Merged API plus demo summary

`api.json` is read from the match folder when present. The parser is tolerant about shape because
the cached response comes from the public API and can be wrapped. It looks at the root, `data`,
`match`, `match_info`, `matchInfo` and `metadata`, then accepts common key spellings.

Join rules from `match-recording.md`:

- Primary key is the match id.
- If `api.json` has no match id, the join uses the folder id and is marked `inferred`.
- If the demo row has no account id, a normalized name join is used and marked
  `joinInferred: true` on that player.
- Demo wins event level facts. API wins rank, MMR, population percentile and fields the demo does
  not carry.
- Every merged field carries `source` (`demo`, `api`, `fixture`, `derived`) and `confidence`
  (`high`, `medium`, `low`).
- A conflict is written to `disagreements` with both values, the kept value and an optional note.
  It is never dropped silently.
- If `api.json` has a different match id, its fields are not merged at all. The conflict is recorded
  as `match.id` and the summary stays demo only.

The summary shape:

```json
{
  "schema": "dah.match.summary/1",
  "matchId": "42",
  "source": "demo+api",
  "join": { "key": "42", "inferred": false, "note": "Joined on the match id." },
  "matchInfo": {
    "map": { "value": "dl_midtown", "source": "demo", "confidence": "high" },
    "durationSeconds": { "value": 1234, "source": "demo", "confidence": "high" },
    "startTimeUnix": { "value": 1760000000, "source": "api", "confidence": "medium" },
    "rank": { "value": "average badge 55", "source": "api", "confidence": "high" }
  },
  "players": [],
  "timeline": [],
  "heatmap": { "available": false, "reason": "No position samples..." },
  "coverage": { "fields": { "match.map": "present", "positions": "absent" } },
  "disagreements": []
}
```

KDA is `(kills + assists) / max(deaths, 1)`, the same convention as `stats.rs`. Counts derived from
resolved kill events get confidence `medium`; the KDA ratio itself is `source: "derived"`. If a kill
event cannot be attributed to a player, `players.kda` becomes `partial` and the unresolved count is
recorded.

Heatmap cells are 512 world units. The map asset reports a radius of 10752. If death positions
exist, the basis is `deaths`; otherwise the basis is `positions`. Cells are sorted by sample count
then coordinates, so the output is deterministic.

## CLI surface

The command module is `rust/crates/dah-rs/src/commands/demo.rs` and it is wired in `main.rs`:

```text
dah demo import <FILE> [--id ID] [--workspace DIR] [--json]
dah demo list [--workspace DIR] [--json]
dah demo show <ID> [--workspace DIR] [--json]
dah demo heatmap <ID> [--workspace DIR] [--json]
dah demo backends [--json]
```

`import` refuses a file without the `PBDEMS2` magic before writing anything:

```text
$ dah demo import .scratch/demo-fixture/not-a-demo.dem --id nope
dah: .scratch/demo-fixture/not-a-demo.dem is not a Source 2 demo (the file is 54 bytes). Expected the magic PBDEMS2\0 (50 42 44 45 4d 53 32 00) at byte 0, found 48 4c 32 44 45 4d 4f 00 ("HL2DEMO."). That is a Source 1 demo. Deadlock records Source 2 demos. A Deadlock match recording is a Source 2 .dem written by the game.
```

Container import of a synthetic demo:

```text
$ dah demo import .scratch/demo-fixture/synthetic.dem --id scratch-1
Imported scratch-1 into .scratch/demo-workspace/matches/scratch-1
  Source:     .scratch/demo-fixture/synthetic.dem (136 B, crc32 4893c15d)
  Extractor:  container (container only)
  Map: dl_midtown (demo, high)
  Duration: 20m 34s (1234 s, demo, high)
  Playback ticks: 37035
  Result: absent
  Players: 0  Events: 0  Heatmap cells: 0
  Coverage: 5 present, 0 partial, 11 absent
  Absent:   events.timeline, heatmap, match.gameMode, match.mmr, match.populationPercentile, match.rank, match.result, match.startTimeUnix, +3 more
  Not extracted: assists, deaths, events, heatmap, kills, players, positions, result
  No player, event or position rows were produced. The container extractor reads metadata only.
  Heatmap: absent. A heatmap is only drawn from real position samples. Run `dah demo backends`.
```

After a cached `api.json` is added and the demo is re-imported with the same id:

```text
$ dah demo show 42
Match 42  [demo+api]
  Join: 42 (exact match id)
  Join note: Joined on the match id.
  Map: dl_midtown (demo, high)
  Duration: 20m 34s (1234 s, demo, high)
  Playback ticks: 37035
  Result: absent
  Start time: 1760000000 (api, medium)
  Game mode: 1 (api, medium)
  Build: 9999 (demo, high)
  Rank: average badge 55 (api, high)
  MMR: absent
  Population percentile: absent

Players: 1
  Alice              Haze           team 0  7/2/5     [api]  KDA 6.00   [derived]  net 30,000    evidence 0e/0p

Timeline: 0 event(s)

Heatmap for 42: absent
  Reason: No position samples were produced by the extractor. The container backend cannot recover entity state, so no heatmap is drawn.
  Position coverage: absent
```

`dah demo backends` prints the lanes and reasons shown above. `--json` wraps the payload in an
object with `ok: true`.

## Fixture format

Fixtures exist for tests and UI development only. The CLI does not accept them, because
`dah demo import` must refuse a file without the Source 2 magic. The library entry point is
`deadlock_core::demo::import_fixture`.

```json
{
  "schema": "dah.match.fixture/1",
  "matchId": "fixture-1",
  "map": "dl_midtown",
  "durationSeconds": 1800,
  "result": "win",
  "gameMode": "standard",
  "players": [
    { "accountId": 1, "name": "Alice", "hero": "Haze", "team": 0 },
    { "accountId": 2, "name": "Bob", "hero": "Mina", "team": 1 }
  ],
  "events": [
    {
      "tick": 100,
      "timeSeconds": 10.5,
      "kind": "kill",
      "actor": "Alice",
      "target": "Bob",
      "assists": ["Alice"],
      "x": 1024,
      "y": 2048
    }
  ],
  "positions": [
    { "timeSeconds": 10.5, "x": 1000, "y": 2000, "team": 1, "player": "Bob", "kind": "death" }
  ]
}
```

Rules:

- Every field is recorded with `source: "fixture"`. `meta.json`, `summary.json` and
  `provenance.json` say the record is not a real capture.
- A fixture with no map, players, events or positions is rejected.
- A player needs an account id, a name or a hero.
- An event needs a kind. A position needs finite x and y.
- Fixture K/D/A from kill events wins over declared counts, so the same event counting path is
  exercised.
- The file is stored as `fixture.json`; `match.dem` is not created.

## Verification

Run on this macOS host, 2026-11-16:

```bash
cargo check --manifest-path rust/Cargo.toml -p deadlock-core -p dah-rs
cargo test -p deadlock-core --test demo
```

Results: `cargo check` passes for both crates. The demo test target passes 4 tests:

- synthetic container import, store files, provenance CRC32, then a re-import that merges a cached
  API record, keeps `review.md`, keeps the first `importedAt` and bumps `revision`.
- refusal of a non-Source-2 file with the magic, found bytes and Source 1 hint in the message.
- a cached API record with a different match id is not merged and the conflict is recorded.
- fixture import produces events, positions, a death basis heatmap and derived KDA with sources.

Unit tests inside `demo.rs` cover varint boundaries and truncation, all four `ubit_var` buckets,
Snappy literal and copy decoding, protobuf file header and file info decoding, string table
inventory, packet message inventory for id 316, match id traversal rejection, synthetic framing,
non-demo refusal, the container extractor's empty output, fixture KDA and death heatmap selection.

The CLI was run against a generated `PBDEMS2` file and against a plain text file. Both outputs are
pasted above. `dah demo list`, `show`, `heatmap`, `backends` and `--json` were exercised.

Not verified:

- A real Deadlock `.dem`. No install and no recording on this host.
- Whether the first command in a real Deadlock demo is `DEM_FileHeader`. If it is not, the map name
  and build stay absent and coverage says so. The command stream is still parsed in order.
- The meaning of the second 32 bit offset at byte 12.
- Snappy streams that use the four byte copy form or lengths above the decode cap. The decoder
  implements all four tag forms, but only the literal and one byte copy forms are covered by a unit
  test.
- Any entity, position, event or Citadel match metadata extraction.

## Remaining gaps

1. Link a serializer aware backend. `source2-demo` behind a feature flag is the shortest path. It
   brings entities, serializers, Citadel game events and post-match details. It is a dependency and
   proto sync decision, not a CLI flag.
2. Decode `k_EUserMsg_PostMatchDetails` (316) into match metadata and players. The container already
   detects the message. Decoding needs the Citadel proto shape and a field mapping test against a
   real demo.
3. Add a real demo fixture set from a captured match, with permission, and verify map, duration,
   players, events and positions against a known review.
4. Add a `demo reindex` command that re-parses stored demos after a backend upgrade. The store
   layout already supports it because `match.dem` is kept.
5. Add API fetch and cache. Today `api.json` must already exist. The public client lives in
   `stats.rs`; the app or CLI can write the raw response into the match folder before a re-import.
6. Decide whether the desktop app labels container only matches differently in the match list. The
   data is present in `coverage` and `provenance.unsupported`.

## Sources

- `source2-demo` repository: <https://github.com/Rupas1k/source2-demo>
  - `source2-demo/src/parser/mod.rs` (magic check, offset at byte 8, stream at byte 16)
  - `source2-demo/src/stream/reader/msg.rs` (command varints, compression bit, Snappy)
  - `source2-demo-protobufs/protos/common/demo.proto` (field numbers used here)
- `deadem` deadlock `demo.proto`, used to confirm the same command and field numbers:
  <https://github.com/Igor-Losev/deadem>
- Deadlock API demo analyzer notes: `docs/research/match-recording.md`
- Deadlock API statistics client: `docs/research/stats.md`
- Snappy block format: <https://github.com/google/snappy/blob/main/format_description.txt>
- Valve Source 2 `ReadUBitVar` behavior, mirrored by `source2-demo`:
  <https://github.com/Rupas1k/source2-demo/blob/master/source2-demo/src/stream/bits.rs>
