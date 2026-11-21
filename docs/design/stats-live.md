# A live match view

Status: built 2026-11-20, real game unverified. The console lane, the bridge builder, the panel and
the settings are built and tested on fixtures and headless scenes; nobody has watched a real match
with the bridge yet (no Deadlock install on the build Mac). Fills the Stats row "Live match view"
in `docs/design/feature-parity.md`.

Deadlock Mod Manager has a Live tab: both teams as the in-game top bar shows them, read from the
game's console log, which a small HUD mod (its "live bridge") writes into. Grimoire has nothing
like it. Sigil builds the same two lanes, from DMM's behaviour (GPL-3.0, no code or text taken).

## The two lanes

### 1. Console log (`-condebug`)

`deadlock_core::live_match` follows `game/citadel/console.log` by offset. Local, no network.

| What | Console line | Use |
| --- | --- | --- |
| Searching | `k_EMsgClientToGCStartMatchmaking` / `...StopMatchmaking` | "Searching for a match, 1:23" |
| Match found | `Lobby <lobby> for Match <match> created` | The match id, a fresh board |
| In progress | `OnGameStateChanged: GameInProgress (7)` | The clock "since the horn" when there is no bridge |
| Match over | `Lobby <lobby> for Match <match> destroyed`, or the game closing | The last board stays until the next queue |
| Hero, mode, party | the presence rules in `game_session` | "In a match as Haze, Standard (6v6), party of 2" |

The hideout's own server logs `ChangeGameState` lines straight through to `GameInProgress`; a state
only counts while a lobby is open, so the hideout never reads as a match. A lobby left in hero select
is not kept as a finished match.

The first read takes up to 16 MB from the end of the log (a match writes a few MB at most), so
opening Sigil in the middle of a match still finds its lobby. Later reads take only the appended
bytes. A log that shrank or was created again (the game restarted) starts over.

### 2. The live bridge (an opt-in HUD mod)

Panorama cannot write files, so the first design here (a `sigil-live.json` the mod writes) was
wrong. What works, and what DMM ships: a script on the top bar prints the numbers to the console
with `$.Msg`, and `-condebug` puts them in `console.log` next to the game's own lines.

`deadlock_core::live_bridge` builds the mod on the player's computer from their own game:

- `panorama/layout/citadel_hud_top_bar_team.vxml_c`: the game's top bar team layout from
  `pak01_dir.vpk` with one script include added (a `SCRIPTS` section shaped like the layout's own
  `STYLES` section, or one more include in an existing `SCRIPTS` section). KV3 patching is
  morphic's (vpkmerge, MIT).
- `panorama/scripts/sigil_live_bridge.vjs_c`: `live_bridge.js` in a compiled script container.
  The container is the smallest of the game's own `.vjs_c` files whose `DATA` block is plain text
  and whose other blocks are only edit info, with the `DATA` swapped. No Valve compiler, nothing
  from the game shipped with Sigil.

The script runs once per top bar team panel. Every two seconds it reads, for each player panel,
the dialog variables the top bar already binds (`player_name`, `hero_name`, `hero_id`, `gold`,
`player_level`, `kills`, `deaths`, `assists`, `objective_damage`) and the `LocalPlayer` and `Dead`
classes, plus the clock, and prints one line when anything changed (and every 20 seconds anyway, so
a quiet board is told from a dead bridge):

```text
11/20 20:20:02 [PanoramaScript] [SIGIL-LIVE] {"v":1,"side":"friendly","clock":"12:34","players":[{"name":"Wren","hero":"Haze","hero_id":13,"souls":"12.3k","level":14,"k":5,"d":1,"a":7,"obj":4210,"me":true,"dead":false}]}
11/20 20:04:01 [PanoramaScript] [SIGIL-LIVE-ROSTER] {"v":1,"teams":[[{"name":"Wren","id":"76561198..."}],[...]]}
```

The roster line comes from the versus screen before the match, the one place the HUD ties a name to
an account. `TeamFriendly` and `TeamEnemy` are fixed slots (teams 2 and 3, Hidden King and
Archmother), not relative to the player: the team holding the `LocalPlayer` is "your team". Lines
from another wire version are skipped; `v` is bumped with every change, in both files, and a test
checks they agree.

What the bridge cannot give: other players' items, cooldowns and positions, objective health,
camp and rune timers. The top bar does not show them, so the script does not read them.

## Installing, updating, removing

- Off by default. Settings > Game > Live match > "Live bridge (HUD mod)", or "Set up the live
  bridge" on the Stats panel, opens one explanation (what it reads, that it draws nothing and sends
  nothing, that it needs `-condebug`, that another top bar mod can hide it) and Install.
- Install builds into the workspace, then goes through the normal install pipeline into the lowest
  free slot, switched on, with a library entry "Sigil live bridge". If `-condebug` is missing from
  the Steam launch options, the usual launch options confirm opens to add it.
- Every change waits for Deadlock to close (the game keeps its addons open).
- `live_bridge::check` finds the bridge (a VPK holding exactly its two files), compares it byte for
  byte with a build from the current game, and reports: not installed, off, current, outdated
  (Rebuild), incompatible (the top bar changed shape: Remove), or unknown (the game could not be
  read). It also lists other switched-on mods that replace the same layout; one in a lower slot wins
  and silences the bridge, and the panel says so.
- After a game update (2026-11-21): while the bridge is on, Sigil reads the game build every
  minute on a worker (`live_bridge::game_build`: `steam.inf` ClientVersion and `pak01_dir.vpk`'s
  size and time) and checks the bridge again when it changed. An outdated bridge is rebuilt once
  Deadlock is closed, never while it runs: by itself with Settings > Game > Live match > "Rebuild
  the bridge after game updates" (off by default, a toast says it happened), else one question per
  update with Rebuild. The build it was last found current for is kept in the options.
- Rebuild replaces the installed file in place, and refuses a file that holds anything but the
  bridge. Remove moves the slot to Sigil's trash.

## The panel

At the top of Stats while Deadlock runs, and for two hours after the last match when it does not:

| State | Shows |
| --- | --- |
| Console log off | "Add -condebug to the launch options" |
| Hideout or menus | "No match running" |
| Searching | the queue time |
| Match found | loading in, the match id |
| In a match, no bridge | hero, mode, party, the clock since the horn, "Set up the live bridge" |
| Bridge installed, no report | "Waiting for the live bridge" and why (restart the game, top bar mod) |
| Bridge reporting | soul lead, team souls, kills, your souls and souls a minute, the soul lead curve from the bridge's own samples, both teams sorted by souls with hero, level, K/D/A, souls and objective damage, respawning marks, "From the live bridge, 2 s ago" or "No signal" after 45 s |
| Match over | the last board, match length, Scoreboard (once deadlock-api has the match) and Statlocker |

Earlier match ids of the session are listed under the panel. Bridge warnings (off, outdated,
incompatible, shadowed) sit in the panel with their one action.

Scenes: `stats-live-idle`, `-console`, `-bridge`, `-waiting`, `-ended`, `-confirm`, `-condebug`, and
`settings-live`.

## Network and budget

| Lane | When | Cost |
| --- | --- | --- |
| Console tail | every 2 s, only while Stats is open and Deadlock runs | one stat, a seek and the new bytes |
| Bridge check | when Stats or Settings opens, after a bridge action, then every 10 minutes | reads the addons' directories and `pak01`'s directory |
| Player lookup | once per match, only with "Look up players in the live match" on (off by default) and Player stats allowed | one `/v1/players/steam` request for the lobby's account ids, then `/v1/players/{id}/rank` per account, all on `api.deadlock-api.com` |

Nothing else leaves the computer. Sigil does not draw over the game, read its memory, hook its
renderer or watch its traffic.

## To verify on a real game (Windows)

1. Install the bridge from Settings with Deadlock closed; check a slot holds a two-file VPK.
2. Launch with `-condebug`, play a bot match: `console.log` gets `[SIGIL-LIVE]` lines, the panel
   fills both teams, the clock follows the top bar, the top bar itself looks unchanged.
3. Check the roster line names accounts (the avatar property is a guess from DMM's script).
4. After a game update: Sigil asks (or, with the switch on, rebuilds by itself once the game is
   closed), and the rebuilt bridge reports again.
