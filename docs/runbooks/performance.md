# Performance and size

Status: active. Measured on macOS 26.0 (Apple silicon) only; the Windows exe size is measured from
a cross build, its run time is not.
Updated: 2026-11-17

How Sigil is measured, what changed in the 2026-11-17 pass and what is left. The method follows
the Rust Performance Book as bundled in the `rust-performance` skill: one falsifiable experiment at
a time, the same workload before and after, keep a change only when it beats noise. Citations name
the chapter and section.

## Measurement loop

All harness scripts live in `.scratch/perf/` (local, not tracked); the commands below rebuild them.

| Metric | Workload | Command |
| --- | --- | --- |
| Startup (proxy) | headless run of the default startup screen (Browse), one frame, 30 runs after 2 warm-ups | `python3 -I startup_headless.py <bin> 30`: wraps `<bin> --tree /dev/null --frames 1` in `/usr/bin/time -l` and parses Rune's `gpu init` and `first frame` line |
| Frame CPU | headless scene under a 3 s wheel script at 60 fps, 5 runs per scene | `<bin> --scene <scene> --script wheel.json --fps 60 --scale 2 --size 1360x860 --tree /dev/null`; Rune prints `first frame`, mean, p50, p95, max |
| Locker load stall | `apply_locker` on the UI thread with a live `ImageCache`, 7 runs | a temporary `DAH_PERF_LOCKER` hook in `app::setup` (not kept) |
| Binary size | `cargo build --release -p dah-desktop`; Windows via `cargo xwin build --release --target x86_64-pc-windows-msvc` | `ls -l` |
| Memory | peak RSS of the runs above | `/usr/bin/time -l` |

Scenes: `browse-fixture` (offline GameBanana page), `stats-profile`, `locker` (45 hero renders from
the Grimoire checkout), `installed`. Toolchain 1.99.1, `aarch64-apple-darwin`.

The live window used to abort on launch on macOS 26 inside Rune's
`platform::macos::traffic_lights::layout`: Rune moved AppKit's titlebar container to a per-instance
subclass while AppKit was already observing it with KVO, which hid KVO's own subclass, so AppKit's
observer got no value and failed a Swift cast to `NSTitlebarContainerView`. Fixed locally on
2026-11-17 by hooking `-layout` on the view's own class instead
(`scripts/patches/rune/0001-macos-traffic-lights-kvo.patch`, applied by `scripts/vendor-rune.sh`).
Real first-presented-frame timing (`RUNE_READY_DIR`, `ready_ms`) can now be taken; the numbers
above still come from the headless proxy.

## Results

Values are medians; ranges are min to max over the runs, or p10 to p90 where marked. Binary sizes
are bytes on disk.

### Baseline (before this pass, default release profile)

| Metric | Value |
| --- | --- |
| macOS `dah-desktop` | 43,291,824 B |
| Windows `Sigil.exe` | about 35.6 MB (reported before the pass, older sources) |
| Startup proxy, wall | 46.8 ms (p10 43.3, p90 48.4, stdev 2.1), GPU init 10.6 ms, first frame 5.0 ms, peak RSS 30.9 MB |
| Frame CPU mean / p95 / max, ms | browse-fixture 5.7 / 10.0 / 12.6; stats-profile 6.3 / 14.4 / 31.3; locker 9.7 / 16.9 / 43.9 (first frame 21 to 24 ms); installed 2.4 / 5.1 / 5.3 |
| Locker load on the UI thread (`apply_locker`) | 239 ms (152 to 263), worker read 37 ms (28 to 61) |
| Peak RSS with Locker art loaded, 2 s / 10 s | 527 MB (499 to 639) / 0.90 to 1.0 GB |

### Experiments

| # | Hypothesis | Change | Result | Kept |
| --- | --- | --- | --- | --- |
| 1 | One codegen unit and fat LTO shrink the binary with no frame-time cost (Build Configuration, "Codegen Units" and "Link-time Optimization") | `[profile.release] codegen-units = 1, lto = "fat"` in `rust/Cargo.toml` | macOS 43.29 MB to 34.21 MB (-21%); peak RSS 30.9 to 29.0 MB; startup and frame times within run-to-run noise (startup 47.9 vs 47.1 ms on the same day); release build 3m48s to about 7m | yes |
| 2 | Stripping symbols removes the Mach-O symbol table (Build Configuration, "Strip Symbols") | `strip` on a copy of the LTO binary | -3.63 MB on macOS (34.21 to 30.58 MB). No effect on Windows, where symbols live in the PDB that is not shipped. It costs symbol names in macOS crash reports, which is how the launch crash below was located | no: the shipped Windows exe gains nothing |
| 3 | `opt-level = "s"` trades a little speed for size (Build Configuration, "Optimization Level") | `opt-level = "s"` on top of 1 | -3.76 MB (34.21 to 30.45 MB) but frame CPU regressed past noise: stats-profile mean 6.3 to 7.5 ms, p95 14.5 to 18.7, max 31 to 48; browse p95 10.0 to 13.8; locker mean 9.0 to 10.4. `"z"` inlines less still, so it was not built | no |
| 4 | `panic = "abort"` shrinks the binary (Build Configuration, "Abort on `panic!`") | not built | Rejected on behaviour: Rune's image decoder catches a decoder panic (`image.rs`, `decode_caught`) so one bad GameBanana thumbnail cannot break later images, and `jobs.rs` and the Browse details fetch report a panicked worker as an error. With abort, a malformed web image would close Sigil | no |
| 5 | `apply_locker` copies and hashes about 100 MB of hero PNG bytes on the UI thread and keeps them in `State` (Heap Allocations, "`clone`") | `HeroCard` keeps the art path; the worker reads only a 32 byte header for the size; `apply_locker` asks `ImageCache::file`, which reads and decodes on Rune's image worker | UI stall 239 ms (152 to 263) to 0.29 ms (0.22 to 0.39); worker read 37 ms to 4 ms; peak RSS at 2 s 527 MB to 246 MB, at 10 s (decodes done) 0.90 to 1.0 GB down to 0.67 to 0.71 GB. Also removes a per-frame clone of the hero's render bytes in the Recolor pane. Locker, stats-profile and installed snapshots are byte-identical before and after | yes |
| 6 | Folding `dah` and `dah-agent` into Sigil (owner request, decision 0010) costs the window path nothing | `cli` and `agent-host` subcommands checked first in `main` | see "Folding" below: startup paired difference +0.08 ms median (stdev 1.9 ms, n = 40), bundle -10.1 MB | yes |
| 7 | Browse became the start screen and decoded every thumbnail of each page it read (50 a page, plus 21 trending images when the rails draw 6), keeping up to 320 (about 0.85 MB each with mips) | Page reads no longer queue thumbnails; the grid's `on_visible_rows` queues the visible rows plus one, the cap is 48 with two margin rows, and the trending rails queue only the six tiles they draw | macOS release, window open on Browse, 20 s: RSS 218 MB to 162 MB, phys footprint 212 MB to 156 MB. 54 MB of what is left is the three Retina window drawables (`CAMetalLayer`), about 14 MB at 1x | yes |

Suspects from the brief that measurement cleared:

- Crosshair scene art: `ImageCache::load` decodes on Rune's worker; the UI thread only hashes 284 KB
  of WebP once. Not a stall.
- `installed::poll` rereads `library.json` only when the addon slot signature changes, not per poll.
- `stats_hero_icons` reads its files on the Stats worker (`fetch_stats`), not in a view.
- `LockerAssetPaths::discover` runs on workers and in the Toolchain report, not per frame.
- Browse already virtualizes its grid (`virtual_grid`), so only visible cards are built.
- No GameBanana page cache exists: the first Browse cards always wait for the network. Only
  thumbnails are cached on disk. Showing the last page from disk would be a behaviour change.

### Folding the CLI and agent host into Sigil

Same sources, fat LTO, macOS:

| Build | Size |
| --- | --- |
| Sigil without the fold | 35,423,200 B |
| `dah` standalone | 8,690,032 B |
| `dah-agent` standalone | 7,718,880 B |
| Bundle before (three files) | 51,832,112 B |
| Sigil with the fold (one file) | 41,720,224 B (+6.30 MB on Sigil, -10.11 MB, -19.5% on the bundle) |

- Startup, paired and alternated 40 times: unfolded 45.8 ms, folded 47.0 ms, paired difference
  +0.08 ms median (mean +0.44, stdev 1.91): no measurable cost. Peak RSS +0.3 MB.
- No static initializers before or after (`otool -l` shows no `__mod_init_func`), so the window
  path runs none of the folded code.
- `Sigil cli doctor --json` takes 9.5 ms against 7.3 ms for the standalone `dah` (+2.1 ms paired,
  stdev 0.6): the larger image loads more pages.
- End to end on macOS: `dah-desktop cli doctor` prints the doctor report; `dah-desktop agent-host`
  with a mock OpenAI-compatible server answered `state`, ran a `game_status` tool call through
  `dah-desktop cli doctor --json` and returned its JSON to the model, and logged
  `dah-rs=<exe> cli`.

### Windows (cross build, `x86_64-pc-windows-msvc`, `+crt-static`)

Same sources, measured from `cargo xwin build --release`:

| Build | Size |
| --- | --- |
| Sigil.exe folded, default profile (no LTO, 16 codegen units) | 51,119,104 B |
| Sigil.exe folded, fat LTO and one codegen unit | 47,438,848 B (-3.68 MB, -7.2%) |
| Sigil.exe without the fold, fat LTO | 38,876,672 B |
| `dah.exe` / `dah-agent.exe`, fat LTO | 11,584,512 B / 9,053,696 B |
| Bundle before (three exes, fat LTO) | 59,514,880 B |
| Bundle after (Sigil.exe only) | 47,438,848 B (-12.08 MB, -20.3%) |

`scripts/package-windows.sh` then produced a 22,766,592 B zip holding one 47,615,488 B Sigil.exe
(later sources than the rows above) and `scripts/verify-windows.sh` passed. Sigil.exe keeps the GUI
subsystem (PE subsystem 2); the standalone `dah.exe` and `dah-agent.exe`
are console programs (3). The 35.6 MB figure from before the pass was an older, smaller source tree
without the fold or LTO; the app has grown since (Stats, Servers, Performance, Browse filters), so
compare the same-source rows above, not that figure. Run time on Windows is not measured.

Locker with the art Sigil actually ships (`scripts/pack-locker-art.sh`, 4.5 MB of WebP instead of
the 100 MB of Grimoire PNGs used above), 7 paired runs: `apply_locker` 7.8 ms (5.3 to 14.8) before,
0.24 ms (0.22 to 1.85) after; peak RSS at 2 s 187 MB to 171 MB. The stall is smaller with shipped
art but still longer than a 120 Hz frame before the change.

### After (all kept changes)

| Metric | Before | After |
| --- | --- | --- |
| macOS Sigil, one file | 43.29 MB plus 16.4 MB of helpers | 41.72 MB, helpers included |
| Windows bundle (same sources) | 59.51 MB in three exes (fat LTO), 51.12 MB for Sigil.exe alone without LTO | 47.44 MB, one exe |
| Startup proxy | 46.8 ms wall, 30.9 MB peak RSS | 47.0 to 48.7 ms wall, 29.9 MB (the Browse screen grew a filter bar in parallel work during the pass, so the first frame does more) |
| Locker load stall | 239 ms | 0.29 ms |
| Peak RSS, Locker art loaded | 0.90 to 1.0 GB | 0.67 to 0.71 GB |
| Frame CPU, locker mean / p95 / max | 9.7 / 16.9 / 43.9 ms | 8.5 / 15.2 / 31.5 ms (not attributed: the headless Locker scene preloads art before its first frame and does not take the changed path) |

Other screens' frame times stayed within noise. Sources changed in parallel during the pass
(Browse, Settings, deadlock-core), so only paired or same-source comparisons above are attributed
to a change.

## Remaining opportunities

- Send the traffic-lights fix upstream to Rune, then drop the local patch. Measure real time to
  first presented frame with `RUNE_READY_DIR` (`ready_ms`).
- Thumbnail memory: `State::thumbnails` never evicts, so scrolling Browse pages grows RSS without
  bound. A bounded cache keyed by visible range would cap it; it needs a re-decode policy.
- Size: `rust/crates/tinyagent` pulls reqwest with default features (native-tls, HTTP/2) next to
  ureq's rustls; moving reqwest onto rustls and HTTP/1 would drop one TLS stack and h2. It changes
  the provider transport, so it needs its own decision and a provider test.
- Size: the `tooling` Rune feature (headless `--snap`, `--tree`, `--script`) is on in release
  builds; Rune documents about 3.4 MB for it. Sigil's own measurement loop uses it, so turning it off
  for the shipped exe only (a desktop cargo feature that packaging disables) is a product call.
- Stats-profile max frame (about 30 ms) and Locker first frame (about 21 ms) come from first-frame
  text shaping and GPU uploads; a frame profile (`--features rune/frame-profile`,
  `RUNE_FRAME_LOG`) would split them by phase.
- Startup does `init_state` work (game discovery, release refresh, subscriptions) before the
  window on the UI thread. It is about 30 ms of the 47 ms proxy including process start and exit;
  profile it on Windows, where Steam library discovery is real.
- PGO (Build Configuration, "Profile-guided Optimization") is untested.
