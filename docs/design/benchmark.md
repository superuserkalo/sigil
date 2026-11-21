# Sigil benchmark

Status: active. The core (run plan, marker and restore, parsers, metrics, comparisons,
mod cost search, store, exports) and the Performance > Benchmark page are built and tested on
fixtures. Nothing has run against a real Deadlock yet: the Windows checks at the end of this
document are open.
Updated: 2026-11-21

The owner asked for "our own crazy benchmark" (2026-11-21). This is the design and the record of
what Deadlock actually supports. Grimoire's unmerged `feat/deadlock-benchmarks-wip` (MIT) was read
for approach; Deadlock Mod Manager's `feature/perf-video-tracking` and `feature/game-presets`
(GPL) for behaviour only. Neither ships a benchmark today.

## What a player gets

- One button: Performance > Benchmark > Run. Sigil starts Deadlock, plays the same replay
  section as fast as the PC can draw it, Deadlock quits by itself, and the result is on screen:
  average FPS, 1% and 0.1% lows, p50/p95/p99 frame times, a frame time graph, stutters, and the
  machine and settings it ran on.
- Comparisons that matter to modders, measured the same way every time:
  - **Mods vs vanilla**: what all your mods cost.
  - **One mod**: what a single mod costs.
  - **Find the costliest mod**: a halving search over the enabled mods.
  - **Any two runs**: one profile against another, one Performance config against another, before
    and after a driver update or a game patch. Every run records the profile, the config, the
    video settings, the driver and the game build, so the comparison says what changed.
- History per replay, a shareable result card (PNG), and exports (JSON, CSV). All local.

## The repeatable scene

The facts below come from `SteamTracking/GameTracking-Deadlock` (`DumpSource2/convars.txt` and
`commands.txt`, build 6857, 2026-11-05) and the engine and client string tables of the same build.

| Option | What Deadlock supports | Verdict |
| --- | --- | --- |
| `timedemo` on a replay | `timedemo`, `timedemoquit`, `timedemo_start`, `timedemo_end`: release, no cheat flag. Prints `%i frames %5.3f seconds %5.2f fps (%5.2f ms/f) %5.3f fps variability, P5/50/95 frame time %.1f/%.1f/%.1f ms`. A per-frame file name `timedemo_profile.csv` with header `demo tick,frame start time,frame start delta,frame swap time,frame swap delta,FPS variability` sits next to it. | **Chosen.** Every run draws the same ticks from the same camera. |
| Map with a scripted camera | `map` works (Sigil already uses it). `setpos`, `setang`, `noclip`, `cl_drawhud` need `sv_cheats`. There is no `wait` command, so a cfg cannot sequence moves, and every camera path command is development-only. | Rejected: a cfg can place a camera once, never move it. |
| Hideout or Sandbox | Hideout commands are UI actions. The scene is light and does not stress what mods change (heroes, particles, the map). | Rejected as a benchmark scene. |
| Bot match | `citadel_start_local_bot_match` exists, `citadel_bot_record` / `citadel_bot_playrecording` replay bot inputs. A match is not deterministic, and bots need a server setup. | Not now. A recorded bot fight could be a second scene later. |
| Your own play | `cl_showfps 4` logs to `prof_%s.csv` (`Time,Player 1 Position,Smooth FPS,Frame FPS,Smooth MS,Frame MS,Server Frame MS`). | Built as a second scene, **Play session**: real numbers, never repeatable, never compared against a replay run. |

### Replays

The game downloads match replays to `replays/<match id>.dem` (client strings
`replays/%llu.dem`, `http://replay%u.%s/%u/%llu.dem.bz2`). No `.dem` ships with the game. Sigil
offers, in this order:

1. Replays the game already has in `game/citadel/replays/` (watch any match in game once).
2. Replays imported into Sigil's Stats (`<workspace>/matches/<id>/match.dem`).
3. A `.dem` file the player picks.

A replay outside `game/citadel/replays/` is copied in as `sigil_benchmark.dem` for the run and
removed after it. When a run starts Sigil takes the replay's SHA-256 and stores it with the run;
before every pass it hashes the file again (`session::verify_demo`). A replay the game downloaded
again, or a file that changed, stops the run: the passes would not be the same workload. History
can tell two runs on the same replay file apart from two runs on a re-download. (Grimoire's WIP
checks a bundled fixture against a manifest of hashes; Sigil bundles no replay, so it checks the
one the player picked.) Sigil reads each replay's header (`deadlock_core::demo::parse_demo`): map, build
and length. The engine refuses a replay from another build (there is no retail
`demo_allow_game_mismatch`), so a replay is tied to the build it was recorded on. The setup marks a
replay from an older build as "probably will not play", and history keys runs by replay and game
build. After a patch, a player picks a fresh replay and history starts a new line; the older runs
stay for reference.

### The section

A whole match is 30 to 40 minutes of ticks. The run measures a window: `timedemo_start` and
`timedemo_end` (release convars) bound it in ticks. The default is 90 seconds starting at the
middle of the replay, where fights happen; the setup has Short (45 s), Standard (90 s) and Long
(180 s). The whole replay is an option. Ticks are 64 a second.

## How a run drives the game

One **pass** is one game launch that plays the window once and quits.

1. Sigil writes `game/citadel/cfg/sigil_benchmark.cfg`. Every line is release and needs no cheats:

   ```
   // Sigil benchmark run <id>, pass <n>. Sigil removes this file after the run.
   echo "SIGIL_BENCHMARK <id> <n>"
   fps_max
   cl_showfps 4
   timedemo_start <tick>
   timedemo_end <tick>
   timedemoquit replays/<name>
   ```

   `fps_max` with no value prints the frame cap into the log, so the result says when a cap hid
   the difference. `cl_showfps 4` is a second frame source (not an archived convar, so it does not
   stick). `timedemoquit` quits the game when the demo ends.
2. Sigil starts Deadlock through Steam with `-applaunch 1422450 -condebug -novid +exec
   sigil_benchmark`. `-condebug` makes the engine write `console.log` for this launch only, so the
   player's saved Steam launch options are never edited. Where the Steam executable is not found,
   Sigil falls back to a managed `exec sigil_benchmark` block in `autoexec.cfg` (the Launch map
   pattern) and the normal Steam URL; then the run needs `-condebug` in the launch options and says
   so.
3. Sigil follows `console.log` from its end and watches the game process. A pass ends when the
   result line appears and the game is gone, when the game is gone without a result (crash or
   quit: the pass failed, with the last error line), when the player presses Stop (Sigil closes
   the game), or after 15 minutes.
4. Sigil reads the frames (below), stores the pass, restores what the pass changed, and starts the
   next pass after a 5 second rest.

The whole run has a time limit (`benchmark::run_limit`): every launch with its section plus eight
minutes of slack, and ten minutes more. Past it, Sigil closes the game, puts everything back and
records the run as failed with the reason. Each pass has its own 15 minute limit inside that.

### Window

**Window** in the setup: Game settings (the player's own video.txt) or a fixed 720p, 1080p or
1440p window (Grimoire's WIP has the same choice). A fixed window adds `-windowed -noborder -w
<width> -h <height>` to this launch's arguments only, so runs on different days and monitors draw
the same number of pixels. The game writes the size it ran at into video.txt on exit, so Sigil
copies video.txt into the run's folder before the pass (recorded in the marker) and puts it back
after. On the autoexec fallback (no Steam executable) launch arguments cannot be passed, and the
run uses the game's settings.

The first pass of every run is a **warm-up**: the shader cache and the OS file cache fill on the
first playback, so the first pass is slower in a way no mod causes. It is shown greyed and left out
of the result. A run has 3 measured passes by default (1 to 5).

### Variants and interleaving

A pass runs one variant of the game:

- **As it is**: the mods as they are on.
- **Vanilla**: the mods set aside with the existing vanilla stash (`launch::stash_vanilla`), put
  back after the pass (`launch::restore_vanilla`).
- **Without slots**: the given addon slots switched off with the existing rename
  (`addons::set_enabled`), switched back on after the pass.

A comparison interleaves its variants (A B A B A B, after the warm-up). Heat, background tasks and
drivers drift during a run; interleaving spreads the drift over both sides instead of loading it
onto the second.

### Mod cost search

"Which mod costs the most FPS" is a halving search (`benchmark::cost`), the cousin of the crash
bisect in `deadlock_core::bisect`:

1. Measure the base (all on).
2. Split the suspects in two halves and measure each half switched off.
3. The half whose removal gains more FPS holds the cost. If neither gain is bigger than the noise
   (below), stop: no single group costs a measurable amount.
4. Repeat on that half until one mod is left.

Every step is three passes: the base again, then each half switched off, so drift between steps
does not build up. With 32 mods that is 5 halvings, about 15 launches plus the warm-up. The setup shows the estimate in launches and
minutes before Run. Every group measured along the way is kept, so the result also says "these 16
mods cost 4 FPS together".

## Frame capture

No third-party binary, no download, no elevation. Sources, best first:

1. **`timedemo_profile.csv`**, the engine's own per-frame file, when the build writes one (the
   file name ships next to the timedemo strings; where and when it is written is a Windows check).
   Sigil looks for a copy newer than the pass under `game/`, `game/citadel/` and `game/bin/win64/`.
   Frame time is the `frame start delta` column (ms between frame starts).
2. **`prof_*.csv`** from `cl_showfps 4`, per frame (`Frame MS`).
3. **The result line** alone: frames, seconds, average FPS and P5/P50/P95 frame times. A pass
   with only this has no 1% lows and no graph; the result says "summary only".

The source is stored with every pass and shown on the result, so two runs from different sources
are never compared silently.

### Why not PresentMon or ETW

PresentMon reads the DXGI and DxgKrnl present events through ETW. That needs administrator rights
or the Performance Log Users group, so either a UAC prompt for every run or a group change the
player has to make, and Sigil would ship or download a binary (against the rule). A native ETW
consumer in Rust (`windows-sys` already links the ETW API) is possible: a real-time session on the
`Microsoft-Windows-DXGI` provider, `Present_Start` per process. It is roughly a thousand lines that
cannot be tested off Windows, and it still needs elevation.

The trade-off: the engine's own frame timing does not see what the driver and the compositor do
after `Present` (queueing, display latency, frame generation). For the question Sigil answers
(what does this mod, this config, this driver cost in this game) the engine's timing is the right
measurement, and every Deadlock player has it. Precise present capture stays a possible opt-in
later, behind elevation, recorded in a decision when it is built.

## Metrics

Computed by `benchmark::metrics` from one list of frame times in milliseconds. Definitions are
stated on the page and in the export.

| Metric | Definition |
| --- | --- |
| Average FPS | frames / sum of frame times. Not the mean of per-frame FPS, which overweights fast frames. |
| 1% low | 1000 / mean of the slowest 1% of frame times (at least one frame). The "average of the worst 1%" reading CapFrameX and Grimoire's WIP use. |
| 0.1% low | the same over the slowest 0.1%. Shown only with 10,000 frames or more; below that it is one frame. |
| p50 / p95 / p99 / p99.9 | nearest-rank percentiles of frame time. |
| Worst frame | the longest frame. |
| Stutters | frames longer than twice the median of the 30 frames before them and at least 8 ms longer. Counted, and their share of the run time. |
| Variability | standard deviation of frame time. |
| Graph | the run cut into up to 600 bins; each bin has its FPS (frames / time) and its worst frame. |

A measured result is the median of its passes for every metric, with the spread (lowest and
highest pass) next to it.

### Noise and verdicts

Two results differ only when the gap is bigger than the noise. Noise is the larger of the two
spreads of average FPS (highest pass minus lowest), with a floor of 1.5% of the slower result.
`benchmark::compare` says Faster, Slower or Same (within noise) per metric and overall, with the
gap in FPS and percent. One pass a side has no spread; the floor is used and the verdict says
"one pass, low confidence".

## What is captured with a run

- **Machine**: OS and version, CPU name and cores, RAM, GPU name and driver version and date.
  Windows reads the registry (`HARDWARE\DESCRIPTION\System\CentralProcessor\0`, the display adapter
  class key) and `GlobalMemoryStatusEx`. Linux reads `/proc` and `/sys`. Nothing runs a tool.
- **Game**: build (`steam.inf` `ClientVersion`), the replay and window, the frame cap the log
  printed.
- **Settings**: every `setting.*` line of the video.txt the game reads (the same file the
  Performance page finds, `performance::video::locate`), resolution and window mode from it, the
  Performance config that is on (`performance::applied`), the active profile, and the enabled
  mods (slot, name).
- A fingerprint of the settings and of the mod set, so history can say "same settings" or show
  exactly which lines differ.

## Safety

The cfg file, the copied replay, the autoexec fallback block, a vanilla stash and switched-off
slots are game-folder changes. They are app actions: Run is the player's press, the setup says
what will change, and nothing runs from an agent tool.

Before any change, Sigil writes `benchmark-run.json` to the app data directory with every change
it is about to make. Restore (`benchmark::session::restore`) undoes exactly those, in reverse:

- removes `sigil_benchmark.cfg` only when its first line is Sigil's header,
- removes the copied replay only when its size matches the copy,
- removes the autoexec block (`// Sigil benchmark BEGIN` / `END`) and puts back a Launch map
  block the run took out,
- switches back on only slots that are still switched off,
- puts back a vanilla stash the run made.

Restore runs after every pass, on Stop, and at Sigil's start when a marker is left (the game
crashed, Sigil was closed, the PC lost power). It waits while Deadlock runs. When it is done the
marker goes and a notice says what was put back. A run never starts while a marker is open, while
Deadlock runs, or while a vanilla stash from a normal vanilla launch is pending.

## Storage and exports

`<app data>/benchmarks/<run id>/` is the run's folder:

- `run.json`: the spec (with the replay's hash), machine, settings, passes and their summaries,
  the outcome and the notes.
- `pass-NN.txt`: the frame times, one per line. A run of 6 passes at 200 FPS over 90 seconds is
  about 1 MB of text.
- The logs: `video-settings.txt` (video.txt at the start), and per pass `pass-NN.cfg` (the exact
  cfg the game ran), `pass-NN-console.log` (every console line Sigil read, up to 50,000),
  `pass-NN-<frame file>` (the raw `timedemo_profile.csv` or `prof_*.csv` the frames came from),
  and `pass-NN-video-before.txt` on a fixed window run.

**Open logs** on a result opens this folder. History reads `run.json` only.

**Notes**: a finished run takes notes ("new driver", "closed Discord"), each with its time. Notes
are appended (`store::add_note`) and never rewritten; the run's name stays as it was.

- **JSON**: `run.json` as stored, with the metric definitions.
- **CSV**: one row per frame (`run,pass,variant,frame,ms`) and a summary CSV (one row per pass).
- **Result card**: a 1200 x 630 PNG drawn by Sigil (Inter through `swash`, both already linked):
  the headline FPS, lows, the frame graph, the comparison when there is one, machine and build.
  Saved where the player chooses. Nothing is uploaded.

## Screens

Performance > Benchmark (a button in the Performance head) opens a page with four parts:

1. **Setup**: the replay list (from the three sources, each with map, build, length and a warning
   for another build), the section length, passes, the window, the warm-up, the test (Just measure, Mods vs vanilla, One
   mod, Find the costliest mod), and the line of what Run changes and how long it takes.
2. **Running**: the plan as a row of passes (warm-up, A, B, ...), the one in progress with the
   step (writing the cfg, waiting for Deadlock, loading the replay, measuring, restoring), a live
   line from the log, and Stop.
3. **Result**: the headline, the lows and percentiles, the graph, stutters, the comparison card
   when the run had two sides, the cost search table, the machine and settings, and Save card,
   Export JSON, Export CSV.
4. **History**: runs grouped by replay and build, an FPS-over-time line with markers where the
   game build or the driver changed, and Compare: pick any two runs for the comparison card plus a
   diff of their settings.

## Ideas taken, and from where

- Grimoire WIP (MIT): average of the worst 1% for lows, nearest-rank percentiles, 600-bin graph,
  a run ledger with the configuration hashed, pinning a scene to a client build, the honest
  "a completed run proves the workload ran, not that the result is valid" line, a fixed window
  resolution per run, an overall time limit, a per-run log folder with Open logs, a hash check of
  the workload file, and appendable notes on a finished run. Credited in
  `THIRD_PARTY_NOTICES.md`.
- Grimoire WIP, not taken: a local Modlock server, a pinned MangoHud, Proton and a copied game
  folder. Linux only, a second binary, and it needs a server per run.
- DMM `feature/perf-video-tracking` (behaviour only): read video.txt from Steam userdata first;
  Sigil's Performance page already does.

## Not done yet

- Real runs on Windows. Everything below the page is tested on fixtures and parsers fed with the
  exact format strings from the build 6857 string tables.
- Native ETW present capture (opt-in, elevated).
- A recorded bot fight scene; a scene that ships with Sigil (a replay is per build, so it would
  need refreshing every patch).
- A notes editor that changes or removes a note (append only, by design).
- Lifting the frame cap for a run: `fps_max` is archived, so setting it would stick. The run
  reports the cap instead.
- Switching profiles or Performance configs automatically between passes. Today the player runs,
  switches, runs again and compares the two runs.

## Windows checks

1. `steam.exe -applaunch 1422450 -condebug -novid +exec sigil_benchmark` starts the game without a
   Steam prompt and writes `game/citadel/console.log`.
2. `timedemoquit replays/<id>` plays a replay the game downloaded, prints the result line, and
   quits. Note the exact line in `console.log`.
3. `timedemo_start` and `timedemo_end` bound the measurement (and whether playback skips to the
   start or plays through it fast). If they are archived, they stick: note it.
4. Where `timedemo_profile.csv` is written, if anywhere, and its rows.
5. Where `prof_*.csv` from `cl_showfps 4` is written and whether it has one row per frame.
6. What `fps_max` prints with no value.
7. A replay with mods on: whether the engine refuses it (`demo recorded with '%s' addons`).
8. Kill the game mid-pass, then start Sigil: the notice says what was put back and the addons,
   cfg folder and autoexec are as before.
9. A 1080p window run: the game opens windowed at 1920 x 1080, and after the run video.txt holds
   the player's own resolution again.
10. Open logs opens the run's folder with the pass logs, cfg and frame files.
