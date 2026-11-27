# Performance and size

Status: active. Measured on macOS 26.0 (Apple silicon) only; the Windows exe size is measured from
a cross build, its run time is not.
Updated: 2026-11-27

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
4. **Heap diet** (the macOS heap report's owners, `.scratch/mem-research/heap-report.md`, local):
   - Catalog search index (`catalog_index/matcher.rs`): every token's text once in one string with
     a sorted id list for lookup (it was a `HashMap<Box<str>, u32>` key and a `Vec<Box<str>>`),
     postings in one flat list with per-token offsets (one `Vec` per token before; a token's df
     is its run's length now), and the lower-case raw title kept only for titles with non-Latin
     letters, the only ones a non-Latin typed word can be found in. Measured with a counting
     allocator on the 2,606-mod index in `.scratch/search-review`: 4.49 MB to 2.28 MB of
     allocated bytes, far fewer allocations. Every result is identical: the full search output
     (ids, score and coverage bits, closest and rated flags) for the 316 eval queries, each of
     their prefixes and each with "nsfw" added, 6,310 searches over the plain index and the one
     with likes and ratings, compared byte for byte before and after; `search_eval` gives the
     same hit@5 and recall@10.
   - Performance page: its parsed configs (about 3.1 MB in the heap report) go when the page is
     left (`State::go`, `performance::release`) and come back from the disk cache on return. A
     few strings per row stay (`RowSummary`), so the list draws at once from them while the cache
     is read again on a worker, configs in parallel (2026-11-27).
   - Segmented selectors measured their labels outside a view (Browse's toolbar tier from
     handlers and polls), which made Rune build a second text system for the UI thread. They now
     keep widths measured while a view runs, and outside one answer from those or an estimate
     (Rune patch 0008 adds `text::in_view`).
   - Translations: only the language read builds its table (0.14 MB of German in a release
     build, which reads English only).
   - Windows: `icu.dll` and its word dictionaries load the first time word boundaries are asked
     for, not after every launch's first frame (Rune patch 0008, about 800 KB and a thread).
5. **Executable diet, the safe cuts.** The agent host (`dah-agent`, TinyAgent, reqwest, h2,
   hyper, tokio, jsonschema and native-tls) is linked only with the desktop crate's `agent`
   feature, which release builds leave off; without it `Sigil agent-host` says so and exits with
   code 2 (`helpers.rs`). The benchmark card draws with Rune's own Inter bytes
   (`rune::text::embedded_fonts`, Rune patch 0009) instead of embedding the three faces again
   (1,248,684 B). `cargo tree` confirms none of the agent stack is in the default graph.
   Measured on the Windows exe (`cargo xwin build --profile fastrel`, no LTO, same sources): with
   the `agent` feature 104,840,192 B, without it 98,588,672 B, so the agent stack was 6.25 MB;
   with Inter, about 7.5 MB in all. These are file-backed pages: they shrink the download and the
   "Working set (memory)" column by what of them was touched, not the private memory columns.
   `panic`, `opt-level`, the TLS and HTTP stacks, the compression features and Rune's `tooling`
   feature are unchanged.
6. **Hardware acceleration** (Settings > Appearance, Windows only; `hardware.rs`, Rune patch
   0010). On by default (`hardware::DEFAULT`, one constant to flip once CPU frame times are
   measured on Windows). Off, from the next launch (the row offers Restart), Rune draws every
   frame on the CPU (`App::hardware_acceleration(false)`): no D3D12 device, swap chain, upload
   heap or GPU atlas copy exists. What only the GPU drew degrades in the CPU painter
   (`Scene::strict`) rather than starting one: a blurred layer draws nothing (so nothing meant to
   be obscured shows sharp), a turned layer draws unturned, a backdrop blur inside a layer and
   native surfaces are left out. Sigil replaces what it used those for: the transparent sidebar
   is off, a hidden mature picture is decoded once more, blurred on Rune's image worker
   (`ImageCache::file_fit_blurred`, with the same decode limits), the Personalize background
   shows unblurred, and a mature pack's art shows its icon. The 3D viewer keeps its own GPU
   device while a view is open. Rune also stays on the CPU, strict, when DXGI lists no hardware
   adapter (VMs, Microsoft's Basic Render Driver: before, WARP drew the same frames and kept
   every GPU resource in process memory) and after the GPU failed its three tries (frames that
   needed it used to stop there). Not measured: frame times of a long Browse scroll on the CPU,
   which is why the default stays on.

Each of these is written down as unmeasured on Windows. The time to first frame is unchanged by
construction (the first frame was already painted on the CPU), but that too wants a `ready_ms`
check.

### How to measure on Windows

Use a release build (`scripts/package-windows.sh`) on the PC the 0.1.42 profile came from, or any
Windows 10 or 11 PC with a discrete GPU, at 1080p. In Task Manager > Details, right-click the
column header > Select columns, and add "Commit size", "Memory (active private working set)",
"Working set (memory)" and "Dedicated GPU memory". Read the numbers for `Sigil.exe` after the wait
each row gives. `.scratch/memory-profile/` (local) has `sampler.py` and `vmmap.py`, which log the
same counters every second; they are optional.

Time to first frame: set `RUNE_READY_DIR=%TEMP%\sigil-ready` before starting Sigil; the file
`Sigil.exe.<pid>` in it holds `ready_ms`. Take the median of 5 cold starts before and after a
change; a move of more than 5% fails.

| # | What to do | Pass |
| --- | --- | --- |
| 1a | Settings > General: Start with your computer and Start minimized on, Minimize to tray on. Sign out and in (or run `Sigil.exe --minimized`). Wait 60 s. | No Sigil window; tray icon present; commit at most 40 MB; Dedicated GPU memory 0; no `d3d12.dll` in Resource Monitor's modules for Sigil |
| 1b | The same with Minimize to tray and Close to tray off. | A Sigil button on the taskbar, nothing on screen; same numbers. Click the button: the window shows at once with its content (no white or black flash), pictures fade in, and Dedicated GPU memory rises after a moment |
| 1c | First launch after an update, started minimized with the tray on. | A minimized window rather than none; the next launch keeps the update (no rollback) |
| 2 | Visit every page, then hide to the tray (Close with Close to tray on). Wait 40 s. Then open from the tray. | Hidden: Dedicated GPU memory 0, commit at most 60 MB. Reopened: same place, size and maximized state as before; opens as fast as a cold start |
| 3 | Locker > a hero > 3D view on. Minimize Sigil for 40 s, then restore. | Dedicated GPU memory falls by the viewer's share while minimized; the view loads the same hero again on restore |
| 4 | Browse idle 90 s after a fresh start, before and after this batch. | Commit lower than before by a few MB (the search index alone was 2.2 MB smaller on macOS) |
| 5 | Settings > Appearance > Hardware acceleration off, Restart. Browse idle 90 s. Then scroll Browse 150 wheel notches. | Idle: private working set at most 75 MB, commit at most 90 MB, Dedicated GPU memory 0. After scrolling: private working set at most 95 MB. Scrolling looks as smooth as with acceleration on (for a number: a build with `--features rune/frame-profile` and `RUNE_FRAME_LOG=frames.jsonl`, 95th percentile frame under 16 ms at 1080p and 1440p on a 6-core CPU). Mature cards show blurred, never sharp; the transparent sidebar row says it needs hardware acceleration |
| 6 | `Sigil.exe agent-host` in a terminal. File size of `Sigil.exe`. | Prints that this build has no agent host and exits; the exe is several MB smaller than 0.1.44's |

A failure in any row is a bug to report with the row number and the four Task Manager numbers.

## Smoother and sooner (2026-11-27)

The owner's report from Windows: animations such as the sidebar should be smoother, things should
load faster and pictures should get sharp faster. Every number here is from a Mac (Apple silicon,
macOS 26) shared with other build jobs (load average 10 to 60 during the runs), not from Windows.
Paired runs come from one `fastrel` build with a temporary switch that restored the old behaviour
(removed before commit), alternated, so both sides saw the same machine.

### Animations

What moves, all on the motion tokens in `motion.rs` (critically damped springs, ease out): hover
fills land at once and fade out in 100 ms; selection thumbs, tab underlines and toasts closing
140 ms; menus, dialogs and drawers in 180 ms with a 4 to 24 point travel, out in 80 to 110 ms;
page bodies fade in over 120 ms with the old page gone the same frame; card pictures fade in over
140 ms; the billboard's pager bar fills linearly (a countdown, on purpose); grid rows mount without
a fade. These are paint only: no commit, no layout. Nothing was long or linear that should not be,
so no duration changed.

What was wrong:

- **The sidebar jumped.** Its width changed in one frame (216 to 48 points and back) while only
  the logo faded, and every page re-laid out at the new width in that frame. Now the sidebar's
  width glides on `base` (140 ms) and clips what it holds, and the page beside it is laid out once
  at the width it lands at and slides there as one (`motion::sidebar_glide`, `motion::page_glide`,
  Rune's `hold_landing_width`): no text is shaped and nothing is laid out while it moves.
- **Biggest cost: a second full Browse rebuild after every sidebar toggle, visit and resize.**
  Browse's scroll area stored its laid-out width from a layout handler with a normal commit, so
  the frame after the real commit rebuilt the whole page again, although the views already read
  that width in the first commit (`toolbar::page_width`). The handler now stores it live only
  (`browse/page.rs`, `browse/sources.rs`).

CPU per frame, `fastrel`, headless (`--scene <scene> --script <taps> --fps 60 --scale 2 --size
1360x860`, `RUNE_FRAME_LOG`), three toggles after the first, ms:

| | Before | After |
| --- | --- | --- |
| Browse landing, sidebar toggle | two commits in a row: 11.8 to 14.7, then 6.9 to 9.8 (18.7 to 23.8 together) | one commit: 14.3 to 18.4 |
| Browse landing, frames while it moves | 3.0 to 3.7 mean, 4.7 worst (the logo fading) | 4.1 to 5.2 mean, 6.8 worst (the glide) |
| Installed, sidebar toggle | one commit, 3.2 to 7.3 | one commit, 6.8 to 7.6 |
| Installed, frames while it moves | 0.4 to 1.3 mean, 2.3 worst | 1.8 to 2.7 mean, 5.2 worst |
| Sidebar press to Browse | a commit and a second one of 1.5 to 4.7 | one commit, no second |

The `fastrel` rows were taken before the glide's spring was kept to the moving edges (the page's
own contents then sprang too), so its glide frames are an upper bound. The final code, debug test
runtime (`smooth_tests::probe_shell_transitions`, 1280 by 800 at 1x, a calm machine, two paired
runs), wall ms of the commit frames and of the frames after:

| | Before | After |
| --- | --- | --- |
| Browse landing, collapse | 23.1 to 24.0, then a second commit of 8.6 to 9.4 | one commit, 24.0 to 29.3 |
| Browse landing, frames after | 5.2 to 5.8 | 5.2 to 6.7, and 29 to 34 on three frames |
| Installed, collapse frames after | 3.0 to 5.0 | 3.0 to 3.9, and 21 to 24 on three frames |
| Sidebar press to Browse | 11.5 to 13.5, then 4.2 to 4.4 | one commit, 10.4 to 10.7 |

The three heavier glide frames in a debug build are the page's text drawn at new fractional
offsets; the optimized build showed no glide frame over 6.8 ms of CPU. Judge the glide in a
release build.

### First content

Browse's first cards come from one GameBanana read on a worker (or the saved catalog, or the
local search index); nothing parses on the UI thread for long. Live window, `fastrel`, fresh data
folder each time (cold) or the second launch on it (warm), `SIGIL_LOAD_TRACE`: the first cards
were in at 0.35 to 0.7 s cold and 0.16 to 0.27 s warm, before and after alike, usually in the
first frame's poll. Nothing to fix here on this Mac; on Windows the read is the same one request.

The second memory batch's two suspects, checked:

- The picture gate (`first_show::Gate`) starts open for a window that opens shown
  (`Gate::at_start(Start::Shown)`, test `a_normal_launch_shows_and_holds_nothing`): it costs a
  normal launch nothing.
- The fetch crews went from 6 to 4 workers. They start only while pictures wait and leave when the
  queue is empty, so the idle cost was already zero; they are back to 6 (a browser's connections
  per host) while pictures load.

### Pictures getting sharp

The path of a card picture: the grid's visible rows ask (`track_visible`, the rows on screen and
one more, urgent), the crew reads the disk cache or GameBanana's 530 pixel thumbnail, the poll
hands the file to Rune, which reads its header for the size, and the pixels decode (scaled to the
card at the screen's scale) the first time the card draws it, then fade in. The billboard shows
the 530 pixel thumbnail and swaps to the original upload (or the top list's 800 pixel art) once
that is decoded, scaled to the banner. There is no per-frame upload budget: every decoded picture
drawn in a frame is copied to the atlas in that frame.

What was slow:

- **Biggest: the billboard went blank while its original decoded.** It switched to the original
  as soon as Rune knew its size from the header, before any pixel was decoded, and a picture
  without pixels draws nothing: the banner showed its empty fill from the header read until the
  multi-megabyte original was decoded, instead of the thumbnail. Now it draws the original only
  once its pixels are ready and asks for both decodes, the thumbnail's first
  (`top::billboard_pixels`). In the debug runtime's landing load, frames with an empty billboard
  went from 22 to 29 of about 25 to 0.
- The next pick's original decoded only when it was shown, so every step showed the thumbnail
  first. It now decodes 1.5 s before the step (`top::prepare_next`, woken by the interval, no
  commit).
- Header reads and decodes shared one first-in, first-out queue on Rune's three decoder threads,
  so the decode of a card on screen waited behind the header reads of every picture asked for
  before it. Decodes now go first (Rune patch 0021, `image/decoders.rs`). A header read that
  finds the size already known is skipped, and a header read takes the first 256 KB of a file
  instead of the whole file (an original is 1 to 5 MB), reading the rest only when the header
  lies past it.
- Cards scrolled into view waited behind margin rows queued earlier: an urgent request now moves
  pictures that were already waiting to the front, in the order asked (`data::enqueue`).

Live window, `fastrel`, ms after the first frame, 3 alternated runs each:

| | Before | After |
| --- | --- | --- |
| Warm: billboard sharp | 721, 747, 800 | 369, 411, 439 |
| Cold: billboard sharp | 1012, 1142, 1727 | 1111, 1263, 1266 |
| Cold: every picture then asked for decoded | 1949, 2109, not within 12 s | 314, 1211, 1347 |

Warm, the billboard is sharp in about half the time; cold, the original's download dominates
and the time did not change measurably. The "every picture" milestone depends on what the
landing has chosen to show at that moment, so read it as a trend. Snapshots of the landing are
unchanged except for the billboard during a load.

Memory cost: up to two more fetch threads while pictures download (none when idle); one decoded
banner (about 10 MB with its mips at 2x) held on the CPU for up to 1.5 s before a billboard step,
or until the next step when a pause stops it; header reads hold at most 256 KB instead of a whole
original. The atlas budgets, the decode pixel limit (`MAX_DECODE_PIXELS`) and the 2560 pixel cap
are unchanged, and animated pictures decode as before.

How to see it: `SIGIL_LOAD_TRACE=<file>` makes Sigil append its milestones (`first-frame`,
`browse-cards`, `pictures-shown`, `billboard-sharp`, and `page browse visit N pictures-shown`
for each later visit) in milliseconds since start. Ignored tests `probe_shell_transitions` and
`measure_browse_pictures` (`SMOOTH_COLD=1` for a cold cache) repeat the in-process measurements.
Guards: `the_sidebar_glides_in_one_commit_and_lays_nothing_out_while_it_moves`,
`the_sidebar_lands_at_once_under_reduced_motion`, `opening_browse_is_one_commit`,
`the_billboard_draws_the_upload_only_once_it_is_decoded`, the crew's `queue_order_tests`, and in
Rune `a_decode_goes_ahead_of_every_header_read_queued_before_it`.

### Pictures that flickered

The owner's second report: a picture in a mod's details was there, went grey, came back; or it
was low resolution, then sharp, in the details and in the Locker. One cause under all of them:
since the second memory pass, pictures decode when drawn and their size is known from the
header long before their pixels, but several views still took a known size to mean decoded. A
picture without pixels draws nothing, so each switch to one showed the empty box (grey) for the
whole decode.

- **Details, there, grey, back.** The details open on the card's own picture (its 530 pixel copy,
  already decoded by the grid) and, when the page lands, switch the box to the gallery's 800 pixel
  copy, which was then only queued. The box went to its empty fill until that copy was fetched and
  decoded. Now the card's copy stays until the gallery copy can draw, then the gallery copy takes
  over in the same element (`details/media.rs`, `data::first_ready`). The full size view does the
  same with the original: the gallery copy stays until the original is decoded. Guard:
  `a_shown_details_picture_never_goes_back_to_its_empty_box` (fails on the old rule: "frame 0
  after the page landed: the box went empty").
- **Cards, there, grey, back.** When cards grow (a sidebar toggle, a wider window, closing docked
  details, a zoom), Sigil fetches a larger copy of every held card picture, and that copy replaced
  the one on screen at once: every card went back to its empty box until its new copy decoded. Now
  a copy that is drawn stays until the larger one can draw (`data::land_picture`,
  `promote_upgrades`). Guard: `a_larger_copy_keeps_the_shown_pixels_until_it_is_ready` (fails on
  the old rule).
- **Locker, low resolution then sharp, with a blank between.** A hero page shows the hero's card
  art and swaps to the full render once it is "ready", which was "its size is known": the render
  drew nothing for its decode. It now swaps once the render's pixels draw and asks for the decode
  at once (`locker.rs ready_pixels`); the card pick tiles likewise (`card_art.rs`). The card art
  standing in is the only low resolution step left, and only while the render decodes: no second
  fitted decode of the same art exists (the Locker decodes its art files whole).
- The billboard had the same cause (above).

A picture on screen is not evicted: atlas pages are freed only when nothing drew them for 5 s,
or over the budget only among pages not drawn in the last 3 frames, and both run only on drawn
frames. Not changed: a picture drawn small and later large (a landing tile, then the details box)
draws its smaller level from the atlas for the length of a decode when its CPU copy was freed
after upload (the first memory pass); it is soft for that moment, never empty.

### On Windows

1. Collapse and expand the sidebar (the button by the logo, or Ctrl+B) on Browse and on Installed:
   the sidebar should slide in about a seventh of a second and the page slide with its edge, with
   no hitch at the start and no text re-wrapping during the slide.
2. Press Browse in the sidebar from another page: the page fades in with no stutter after its
   first frame.
3. Open a mod's details from Browse, several times, on a cold and a warm start: the big picture
   never goes grey; it shows the card's picture, then the sharper one in place. Close docked
   details or collapse the sidebar over the grid: no card goes grey. In the Locker, open a hero:
   the card art, then the full render, with no empty frame between.
4. Start Sigil with `SIGIL_LOAD_TRACE=%TEMP%\sigil-load.txt` twice (the second start is warm) and
   read the file: `billboard-sharp` minus `first-frame` was about 0.4 s warm here. On Browse the
   billboard should never flash empty while it loads; it shows the small picture, then the sharp
   one. Each billboard step should show the next picture sharp at once.
5. Scroll the Browse grid quickly and stop: the cards in view should fill before the ones beyond.

## Remaining opportunities

- Locker visits commit several times while its art lands (12 to 24 ms each in `fastrel` here)
  during the page fade; batching those commits would smooth that page switch.
- Every frame of any motion repaints the whole window (3 to 5 ms on the Browse landing here).
- The billboard original takes 150 to 300 ms to decode and scale; a decoder that scales while it
  decodes (JPEG DCT scaling) would cut that.

- The toggle commit itself (see above): fewer mounted cards needs a configurable overscan in Rune.
- Windows: measure the launch steps and a toggle on a real install with antivirus; the numbers above
  are from a Mac with no Deadlock install.

- Send the traffic-lights fix upstream to Rune, then drop the local patch. Measure real time to
  first presented frame with `RUNE_READY_DIR` (`ready_ms`).
- Memory: dropping the main window's D3D12 device while it is minimized (step 3 above says what
  it needs and why it waits for a Windows machine). The duplicate codec stacks, `opt-level = "s"`
  for dependencies and `panic = "abort"` remain owner calls.
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
