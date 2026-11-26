# Performance and size

Status: active. Measured on macOS 26.0 (Apple silicon) only; the Windows exe size is measured from
a cross build, its run time is not.
Updated: 2026-11-26

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

## Launch and mod toggle (2026-11-25)

Launch: every launch step (put a stash back, roll random mods and the Locker shuffle, write Steam's
launch options, set mods aside, mount the extra folders, add the `gameinfo.gi` lines, start Steam,
write the launch marker, read the addon folder again) ran inside `launch_progress::poll` on the UI
thread, so the window froze from the press until Steam had the game. Each step now returns its file,
process and Steam work as a `Work` that runs on a worker and hands back an `Apply`; the overlay holds
the step until it lands, so the order and every check are as before. The missing-files check a
modded press starts runs on its own worker. Still on the UI thread at the press: the `gameinfo.gi`
gate read (it decides whether a dialog opens), the stale map line in `autoexec.cfg` and the stash
marker check, each one small file. Guard:
`launch_progress::tests::launch_steps_run_on_a_worker_and_a_second_press_waits`.

Mod toggle at 300 mods, debug test build, same Mac, load average 3 to 4 for both runs:

| Bench | Before | After |
| --- | --- | --- |
| `measure_real_toggle_at_300_mods` (300 real VPKs, press to the switch drawn flipped, median / p90) | 593 / 610 ms | 112 / 116 ms |
| Same, commits per toggle (press down, press up, scan landing) | 1, 1, 3 | 0, 1, 0 |
| `measure_toggle_cost_at_300_mods` (demo library, no file work, median / p90) | 84 / 86 ms | 68 / 69 ms |
| Same, commits per toggle | 2 | 1 |

What changed: the switch takes no focus on press (that was a commit of its own), the row follows the
rename at once and the folder and overlap read (every enabled VPK's directory) runs only on the scan
worker, whose result commits nothing when it matches the rows shown. Sections are keyed per section
and grid rows by index: keyed by their first mod, a card moving to the other section remounted
whole lists over three more commits. Guard: `jank_tests::one_toggle_at_300_mods_is_one_commit`.

Not reached: the one commit left costs the same as before (about 42 ms in a debug build here). About
a sixth of it is building the mounted cards; the rest is Rune's apply and layout of about 2500
nodes. A per-row `rune::memo` was tried and saved nothing measurable (the reused subtree is still
applied), and Rune's virtual list overscan (two viewports a side) is a Rune constant.

## Memory: pictures, GPU uploads, manifest (2026-11-26)

A memory profile of 0.1.42 on Windows 11 (RTX 5060 Ti, D3D12) measured 284 MB commit and 68 MB
VRAM fresh on Browse and 593 MB and 232 MB after two hours (`.scratch/memory-profile/`, local).
What it found and what changed:

- Decoded pictures stayed on the CPU after upload. Rune's `ImageCache` kept every decoded image
  with its whole CPU mip chain while any `Pixels` handle lived (`State::thumbnails` holds one per
  picture), and each window's image atlas kept a CPU copy of every page. Now an atlas drops a
  picture's CPU pixels once it has copied it for upload (Rune patch 0003); a picture drawn again
  after its atlas page was freed decodes again from its file. Pages live on the GPU only after
  the first upload and new pixels go up as regions.
- Pictures decoded at full size. The card and billboard copies were scaled on the fetch worker
  after a full decode. Now Rune decodes them to the box they are drawn in
  (`ImageCache::file_fit`), reads the header first, and refuses anything over 4096 by 4096
  (`memory::MAX_DECODE_PIXELS`); every other fetched picture is capped at 2560 pixels a side.
- Everything decoded at once. The cache now decodes a picture only when it is drawn
  (`decode_when_drawn`): a request reads its header for the size. A minimized or hidden window
  decodes nothing.
- No eviction. Atlas pages are freed when not drawn for 5 s or, least recently drawn first, while
  over the budget (`memory::THUMB_BUDGET` 32 MB, `IMAGE_BUDGET` 64 MB); textures are destroyed at
  once. Card pictures show their placeholder until ready and fade in.
- D3D12 upload heap blocks (Windows only, unverified here): devices ask for 8 to 16 MB blocks
  (4 to 8 MB upload blocks), large uploads stage in 1 MiB slices, and an idle window polls once
  after an upload burst so the staging chunks are freed (Rune patch 0004). The GPU allocator's
  report is logged at debug level and in the copied memory report.
- Sigil.exe had no application manifest: it now embeds one (Segment Heap, per-monitor v2 DPI,
  UTF-8, long paths, supported OS ids, Common Controls 6) from `build.rs`.

Method (macOS, Apple silicon): the `fastrel` build of `main` and of the change, headless with
`--realtime --size 1280x800 --scale 2`, `SIGIL_DATA_DIR` a copy of one seeded folder (97 cached
pictures), runs alternated before and after. The footprint (`footprint -p`, physical footprint:
dirty, compressed and Metal memory) is sampled 1.5 s before the script ends; the peak is
`/usr/bin/time -l`. Medians of 3 runs, range in brackets, MB.

| Scenario | Before, end | After, end | Before, peak | After, peak |
| --- | --- | --- | --- | --- |
| Browse idle (`browse-dashboard`, 20 s) | 313 (311 to 335) | 290 (280 to 292) | 383 | 349 |
| After visiting every page (11 nav taps, 2.5 s each) | 378 (370 to 389) | 308 (296 to 324) | 527 | 487 |
| The same, then scrolling Browse through the whole grid | 330 (326 to 375) | 278 (267 to 301) | 502 | 481 |
| Locker with art loaded (`locker`, 10 s) | 221 (220 to 235) | 183 (179 to 193) | 288 | 252 |

About 130 MB of every row is Metal's own memory for the headless offscreen targets, unchanged.
Malloc after the page tour went from 226 to 139 MB, and the Locker's GPU allocations
(`IOAccelerator`) from 60 to 11 MB. The fixture has one page of mods, so the scroll passes about
50 cards, not 60. Snapshots of `browse-fixture`, `browse-dashboard`, `browse-details`, `locker`,
`crosshair` and `stats-profile` at 1x and 2x are byte-identical before and after.

Windows, not measured: commit, VRAM, the upload heap blocks and the manifest's effect. The steps
are in the memory profile's section 8; Settings > About > Memory now shows commit first, the
process's GPU memory (DXGI) and the picture figures, and its copied report holds the GPU
allocator report.

## Memory, second batch (2026-11-26)

The route to a lower floor (`.scratch/mem-research/report.md`, local). Each step ships alone.
Nothing here is measured on Windows yet; see "How to measure on Windows" below.

1. **A start the player never sees** (`first_show.rs`, Rune patch 0005). Windows starting Sigil at
   sign-in with Start minimized used to show the window, draw it, make the D3D12 device on the first
   idle turn, then minimize it, and decode pictures meanwhile (257 MB commit, 60 MB VRAM measured on
   0.1.42). Now:
   - with the tray up, Sigil starts with no window (`App::windowless`); the tray's Open makes it,
     CPU first like a launch;
   - without the tray, on Windows, the window opens minimized on the taskbar
     (`App::open_minimized`): Rune paints its first frame on the CPU while the window is hidden
     and copies it when the window is restored;
   - in both, Rune makes a CPU-first window's GPU only once the window is in sight: minimized,
     hidden or covered whole, it stays on the CPU with no device, and a failed try's retry waits
     too (`Life::in_sight`, `cpu_frame::gpu_owed`);
   - pictures wait behind a gate (`first_show::Gate`): until the window has drawn a frame in
     sight, `drain_thumbnails` hands nothing to the image cache, so nothing is decoded;
   - with the transparent sidebar on and no tray (a see-through window needs the GPU from its
     first frame), and on macOS and Linux without a tray, the window shows and minimizes as before;
   - an update counts as started only once a frame was drawn (`updater::started_ok`), and the
     first launch of a new version always opens a window, so a windowless start can neither
     confirm a build that never drew nor be rolled back for not drawing.
   The title bar guard (`win_frame.rs`) skips a minimized window and checks again when a window is
   made or comes back in sight.
2. **Hidden in the tray frees the window and the GPU** (`App::release_gpu_when_closed`, Rune patch
   0006). Minimize to tray and Close to tray drop the window, its device, surface and picture
   atlases. The tray's Open makes a new window, CPU first. Rune now keeps the window's place and size
   (and maximized or full screen) across the release, which it did not before. `on_window`'s hooks
   (shell, Crosshair's pin, glass, the title bar guard) only store the handle or reapply to the new
   window.
3. **Minimized for a while: only the safe half.**
   - Done: the Locker's 3D view, which has its own wgpu device on its render thread, closes when
     Sigil's window has been out of sight (minimized, hidden in the tray, covered whole) for 30 s
     (`viewer::OUT_OF_SIGHT_DROP`), and opens again on the same hero when the window shows. A
     preview dialog and the full screen Hideout stay open: the player opened them.
   - Done: the contract with batch one's freed CPU pictures is settled and tested. An atlas that
     loses its GPU (`ImageAtlas::invalidate`, run on a release or a lost device) frees its
     GPU-only pages with their slots, and a picture whose CPU pixels were dropped decodes again
     from its source when next drawn (Rune patch 0007's test).
   - **Not done: dropping the main window's D3D12 device while minimized.** The plan was Rune
     `drop_gpu()`, the first half of `Host::reattach` (drop the surface, then the device,
     invalidate the atlases), with the restore going through the lost-device path. It frees
     nothing on Windows as things stand: a window that started on the CPU draws through a
     DirectComposition visual (`platform/win32/dcomp_handover.rs`), and wgpu-hal's D3D12 surface
     sets its swap chain as that visual's content on configure (`IDCompositionVisual::SetContent`,
     `vendor/wgpu-hal/src/dx12/mod.rs`) and never clears it on unconfigure or drop. The visual
     keeps the swap chain, the swap chain keeps its command queue, and the queue keeps the device,
     so dropping wgpu's objects leaves the driver's memory in place. What it needs: (a) a
     `Handover::clear()` that calls `SetContent(None)` and commits before the surface is dropped,
     then a check in Task Manager that the process's dedicated GPU memory falls to 0; (b) a
     decision on what the window shows between a restore and the new swap chain's first frame
     (the cleared visual shows the window's GDI surface below it, which holds the launch's CPU
     frame or nothing), either the cloak held until the first frame presents, or a fresh CPU
     frame painted into a new DIB first (`SurfaceMode::Cpu` again, then the launch's hand-over);
     (c) `replace_lost_surface` not counting a deliberate drop against `SURFACE_TRIES`. Both (a)
     and (b) change what DWM composes and can only be judged on a Windows machine, so none of it
     ships here. Hidden in the tray (step 2) already frees everything, because the window itself
     goes.

## Remaining opportunities

- The toggle commit itself (see above): fewer mounted cards needs a configurable overscan in Rune.
- Windows: measure the launch steps and a toggle on a real install with antivirus; the numbers above
  are from a Mac with no Deadlock install.

- Send the traffic-lights fix upstream to Rune, then drop the local patch. Measure real time to
  first presented frame with `RUNE_READY_DIR` (`ready_ms`).
- Memory, next batch: make the GPU only when the window is first shown (started minimized),
  release it when minimized, and the binary diet (duplicate HTTP, TLS and codec stacks).
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
