# Grimoire unreleased and planned work against Sigil

Status: historical. A snapshot of unreleased Grimoire work on 2026-11-21. Not maintained; the code wins where they differ. Original status: research snapshot, 2026-11-21; status column rechecked against `rust/crates` after 0.1.22
(2026-11-21). Companion to `upstream-gaps-2026-11-21.md`, which covers what Grimoire and Deadlock
Mod Manager already ship. This doc covers only what Grimoire has not released yet: open PRs,
unmerged branches, closed-but-unmerged PRs, unreleased commits on `main`, open issues, plan docs,
and the open work in its sibling repositories. Grimoire is MIT, so its code and copy may be ported.
A row marked "has, real game unverified" is built and tested on fixtures or headless scenes but
has never run on a real Deadlock install.

## What was read

| Source | What | Notes |
| --- | --- | --- |
| `Slush97/grimoire` `main` | fd4f897b (2026-11-20) | 16 commits since `v1.35.1`; nightly `1.35.2-nightly.20261120031907.158.gfd4f897` lists only #591 and #592 as new |
| Remote branches not merged into `main` | 131 | 112 are branches of PRs that were squash-merged (already on `main`, so released or in the 16 commits); the rest are below |
| Open PRs | #594, #593, #573, #572, #504 | #593 (2026-11-20, +8696 -7535, 175 files) is the big one |
| Closed, never merged | #586, #520, #423, #420, #342, #335, #263, #237, #4, translations | #420, #263 and #4 landed later in another form; #342 was replaced by #584 |
| Branches with no PR | `feat/locker-3d-card-snapshots`, `feat/macos-build`, `claude/mac-native-build-feasibility-*`, `claude/grimoire-black-screen-menu-*`, `claude/mod-platform-scoping-*`, `claude/clever-noether-*`, `docs/user-guide-md`, `chore/t1-only-built-deps` | |
| Open issues | #590, #583, #574, #568, #547, #514, #513 | No labels, no milestones, discussions off; project boards not readable with this token |
| Plan docs on `main` | `docs/ability-vfx-recolor.md` (in progress), `docs/3d-preview-fidelity-plan.md` (no code), `docs/foundry-tab-design.md` (design), `docs/locker-hero-card-apply.md` (design), `docs/performance-config-integration.md` (planned preset UI) | `CHANGELOG.md` stops at 1.10.0; release notes live in PR bodies |
| `Slush97/vpkmerge` | open PRs #95, #94, #89, #50; issue #44 | Sigil links `vpkmerge-core` at df41586 = vpkmerge `main` (ADR 0013) |
| `Slush97/grimoire-social` | open PR #11 (benchmark aggregation), #7 (nix) | |
| `Slush97/grimoire-site` | open PRs #15, #16, #17 | docs companions to #572, #575 and the 2026-11-11 merges; no new features |

Skipped as CI or infra (one line, as asked): `ci/azure-artifact-signing`, `ci/nightly-*`,
`ci/macos-release-matrix` (#354, closed), `chore/release-*`, `chore/*vpkmerge*`,
`chore/dependabot-alerts`, `chore/deps-minor-bumps`, `chore/t1-only-built-deps`,
`docs/azure-signing`, `docs/agents-md`, `refactor/*`, `test/*`, `claude/clever-noether-*` (a
scheduled Discord triage agent for Grimoire's own issues), `feat/nix-stable-hint` and issue #547
(Nix packaging), grimoire-social #7.

Column meanings:

- **Maturity**: merged-unreleased (on `main`, not in a stable release), PR open, branch WIP
  (unmerged branch or a closed PR that never merged), issue only (an issue or a plan doc).
- **Sigil**: has, partial, missing, in progress (being built now), excluded (owner rule), owner
  decision. The file or doc after it is the proof.
- **Effort**: S under a day, M a few days, L a week or more.
- Paths in the build notes are Grimoire paths unless they start with `rust/` or `docs/`.

## Browse

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Extended hero matching: a setting that also counts hero names in titles and hero categories outside Skins | #593 `1795b537`, `009b0c30` | PR open | has, always on: a hero pick also takes the hero's categories anywhere in the section's tree and whole-word title mentions with aliases (`catalog_mirror::HeroMatch`, `browse/tree.rs`); no switch | - | - |
| Picking a subcategory (Model Replacement > Hideout) filters the saved catalog by that subtree, not the whole root | #520 `fix(browse)` | branch WIP | has: rows keep `sub_category_id`; a pick matches its subtree, older rows by name (`catalog_mirror.rs`) | - | - |
| Mouse back and forward buttons walk back through creator pages and details; refreshed creator controls | #593 `d9044df0` | PR open | has: the window root takes `PointerButton::Back/Forward`; Back closes Settings, a mod's details or a creator's page first, then walks the recorded page history, Forward returns (`shell::back`, `shell::forward`, `State::go`) | - | - |
| "X is no longer on GameBanana": install its replacement and delete the old file, keeping on or off and the load folder | #593 strings `confirmBody` | PR open | has: `rust/crates/desktop/src/screens/browse/replace.rs`, `deadlock-core/src/install.rs` `superseded` | - | - |
| Hidden mods and creators: thumbnails blurred until hover, date hidden, the mod it was hidden from, a 140 character note | #580 | merged-unreleased | has: `rust/crates/desktop/src/screens/browse/hidden.rs` (Browse's Hidden items and Settings > Mods) | - | - |
| List layout removed from Browse | #593 `c0137128` | PR open | has: Sigil keeps grid and list (`feature-parity.md`, Browse "Grid / list view") | - | Keep Sigil's list view; nothing to do |

## Installed

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Tag filters cycle: left click goes included, excluded, off; right click clears; a setting turns cycling on | #593 `4583c71a`, `7b9e7b9e` | PR open | has: `installed/notes.rs` `CYCLE_TAG_FILTERS`, `tag_state` (right click excludes), Settings > Cycle tag filters | - | - |
| One tag menu for custom, base (category) and hero tags, with display priorities, move up and down, and reset to default order | #593 `ddeb860f`, `6d4a5dde` | PR open | has: `installed/notes.rs` (one tag menu, `Move up` / `Move down`, `reset_tag_order`, priorities saved) | - | - |
| The Issues button pulses when there are findings the player has not seen | #593 `0cc75cf5` | PR open | missing: the compatibility review has no "new since seen" mark (`installed/repair.rs`, `installed/checks.rs`) | S | Open: remember the last seen finding keys in options |
| Attach local VPKs to a GameBanana mod's card; attachments survive updates; one GameBanana submission per group; files must share a load folder | #593 `0a874d24` | PR open | partial: Group as one mod adds a file to a group card (`installed/selection.rs`); the two rules and the update case are not checked | S | Open: add both rules to `group_name`/`with_groups` and a `mod_updates` test |
| Local mod editing moves into the variant picker: rename, Add VPK files, Add images, Make mod thumbnail | #593 `4d077700`, `787183fe` | PR open | partial: Edit local mod (name, one picture, NSFW) in `installed/notes.rs` `Dialog::EditLocal`; the picker renames labels; `mod_extras` has a `gallery` field but no UI reads it | S | Open: Add files and Add images in the picker |
| Hero detection for uncategorized and multi-file submissions; heroes inferred from unassigned skin metadata | #593 `9967d3a2` | PR open | has: `deadlock-core/src/mod_heroes.rs` reads model and material paths, `asset_heroes` reads sounds, footsteps and portraits, `heroes_in_text` matches titles with aliases, and `guess_heroes` stops at the first level that names a hero; `HeroCache` keeps the answer per file | - | - |
| Category tags follow GameBanana's current schema: every hero skin gets Skins however it is filed (Faithful/Vanilla+ and the misspelled Faithul), hero aliases and possessives, whole-word matches, hero from `sounds/vo/<codename>/` and footsteps | #520 commits 1, 5, 6 | branch WIP | partial: GameBanana category plus model paths (`mod_heroes.rs`, `installed/notes.rs` backfill) | M | Port the tag rules from `fix/gb-category-drift` (`src/lib/categoryPaths.ts`, tag helpers) with its tests |
| Redesigned selection toolbar, selected-card borders, no hover actions while selecting | #593 `a7e43d92` | PR open | has: selection bar with bulk actions (`feature-parity.md`, Installed "Multi-select") | - | Layout only |
| Pinned preview panels: bounded drag, Shift snaps to edges or center, resize corners | #593 `5eeeb5e4` | PR open | missing: Installed has no floating panels; only the docked details resize | M | Only worth it with a floating 3D preview; port `src/components/common/useFloatingPanel.ts` and `src/lib/floatingPanelGeometry.ts` |
| Simpler requirements dialog; the menu item hides when a mod declares no requirements | #593 `1f14a24a`, `bda062f2` | PR open | has: requirements section and fix (`feature-parity.md`, Mod details and Installed) | - | - |
| A multi-file card opens the mod page; Choose files in its menu | #576 | merged-unreleased | has: `installed/layout.rs` `open_group` | - | - |
| Mod safety: Disallow takes an approval back; the mod turns off first | #592 | merged-unreleased | has, real game unverified: `installed/safety_review.rs` (Disallow refuses while the game runs, turns the mod off, then takes the trust back) | - | - |
| Mod safety reads text layouts and empty root folders; Enable anyway for a file it cannot read | #578 | merged-unreleased | has: `docs/design/mod-safety.md`, `installed/safety_gate.rs`, `deadlock-core/src/vpk.rs` `empty_root` | - | - |
| Imports named from their imprints (pack members, variant labels) | #582 | merged-unreleased | has: `rust/crates/deadlock-core/src/import.rs` | - | - |
| Imprint names cleaned (control, bidi and zero-width characters stripped, 120 character cap); colliding pack labels fall back to file names; pack members keep their own thumbnail | #586 `fix(import)` | branch WIP | has: `import::clean_imprint_text` strips control, bidi and zero-width characters and caps at 120 for every title and author read for an import; file and stamp names also lose `(1)`, `- Copy` and version noise (`import::tidy_name`); pack members keep their own cover through the add-mods dialog (`import/parts.rs`) | - | - |
| List layout removed from Installed | #593 `c0137128` | PR open | has: Cards, Small cards, List stay (`installed/layout.rs`) | - | Keep; nothing to do |

## Profiles

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A profile keeps the Locker shuffle switches and pools | #587 (and older Grimoire profiles) | merged-unreleased | has: `deadlock-core/src/profiles.rs` `LockerShuffle` under the profile's `sigil` extension, saved and applied with the profile; Undo restores the earlier Locker state | - | - |
| A profile keeps the Locker's ability colors and trippy skins and rebuilds them on apply; Locker edits save into the active profile | #587 `electron/main/services/profileLocker.ts` | merged-unreleased | owner decision: real ability recolor is undecided (`rust/crates/deadlock-core/src/recolor.rs` writes briefs only); shuffle is kept | L | Follows the Locker recolor decision |
| A profile's crosshair is skipped while the crosshair designer is off | #504 | PR open | has: `profiles.rs` `ApplyChoices { skip_crosshair }`, set from `prefs.page_on(Screen::Crosshair)` in `screens/profiles.rs`, with `CROSSHAIR_SKIPPED` in the result | - | - |

## Locker

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| New General categories: Patrons (Hidden King, Archmother), Sinners (Sinner's Sacrifice vault), Boxes (crates), with cached model previews for the three props | #593 `23144832`, `src/lib/categoryPaths.ts` | PR open | has: `locker.rs` `GeneralType::{Patrons, Sinners, Boxes}` with path rules and rail names | - | - |
| Shared mod favorites between Installed and the Locker; saved mods sort first | #593 `fdccada8` | PR open | has: `mod_extras` `favorite`, Installed's card menu and `toggle_favorite` (`installed/notes.rs`), favorites saved per profile (`profile_favorites.rs`), a favourite sorts first (`installed.rs` line 459) | - | - |
| Locker cards: live 3D model only on hover, right drag to rotate, Resume auto-spin, Save preview angle per mod | #593 `src/components/locker/SoulContainerTile.tsx`, `soulModel.ts` | PR open | in progress (per-hero camera); a saved angle per mod is not covered | M | Open: store a yaw per mod and feed it to the card viewer |
| Hero-less sounds (Patron and newscaster VO, urn lines, vaults, shared parry, hit feedback, UI) go to Announcer / SFX; QoL and gameplay tweaks stay out of hero piles | #520 commits 6, 7 | branch WIP | has: `locker.rs` classifies sounds before anything else, including patrons, the Sinner's vault, urns, announcer and UI paths (`GeneralType::Announcer`) | - | - |
| Global priority tab and custom Locker categories removed | #593 `a8bf12f9` | PR open | has: Sigil keeps Always load first and Add to a Locker category (`feature-parity.md`, Installed) | - | Keep; nothing to do |
| Create a cursor pack from your own images: a tile per stock cursor, a cropper with a hotspot crosshair, an automatic hotspot guess, Create and apply, export | #423 (closed, never merged) | branch WIP | partial: `screens/locker/cursor_create.rs` and `deadlock_core::cursors_create` build a pack from an image per stock cursor, guess the hotspot and nudge it with arrows, and the pack applies like any other; no free cropper and no export | M | Open: the cropper |
| Link a skin to its matching card icon mod, so the hero's card follows the skin whenever it is on | #335 (closed, "postponing") | branch WIP | missing: `rust/crates/desktop/src/screens/locker/card_art.rs` has no link | M | Open: port `lockerCardLinks.ts` and reconcile after every enable change |
| 3D card snapshots: hero card art baked from the installed skin's pose, the panorama card as backdrop, pose-cycle arrows | branch `feat/locker-3d-card-snapshots` (no PR) | branch WIP | has, real game unverified: `locker/card_art.rs` "Use the 3D view" takes the hero page's posed 3D view as the card art, centre-cropped per variant (`viewer/snapshot`) | - | - |
| Ability VFX recolor UI: hue, saturation and brightness with a live recolored preview; particle recolor CLI | `docs/ability-vfx-recolor.md` (status in progress) | issue only | owner decision (Locker ability recolor; `deadlock-core/src/recolor.rs` writes briefs) | L | - |
| Hero card apply pipeline | `docs/locker-hero-card-apply.md` (design) | issue only | has: In-game card art (`locker/card_art.rs`, `feature-parity.md` Locker) | - | - |

## 3D viewer

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A preview appears only when whole: a loader with Hideout build progress, staged heroes wait, 400 ms hold, 20 s fallback | #588 | merged-unreleased | has: `viewer/worker.rs` holds the view until the hero, the compare side and the room are loaded and the first frame is drawn, with `HOLD_AT_MOST` (20 s) and a fallback; Sigil has no staged extra heroes | - | - |
| Motion previews: dash, heavy melee and parry mods play on Lash against Bebop; an animation mod plays its own clips with a clip strip, All and pause; body swaps follow the clip | #571 | merged-unreleased | partial: the clip picker has dash and heavy melee and body swaps follow the clip; no second hero and no per-mod clip strip. Issue superuserkalo/sigil#8 | M | Open: issue #8 |
| Alt albedo reveal (`g_tAltColor_Reveal` by `g_flAltTextureAmount`), so Violet's brush is black with a red tip | #579 `src/lib/source2AltReveal.ts` | merged-unreleased | has: `sigil-viewer/src/material/alt_reveal.rs`, wired in `material.rs` | - | - |
| Skins whose pak stores an empty root folder name preview | #579 | merged-unreleased | has: `deadlock-core/src/vpk.rs` `empty_root` | - | - |
| Recolor reuses stamp textures; staged actor rigs are disposed after cancel | #586 `fix(viewer)` | branch WIP | in progress (staged pose) | - | - |
| Custom animations (taunts, dances) on any hero skeleton; cloth bones; Source 2 material and lighting parity | `docs/3d-preview-fidelity-plan.md` (no code) | issue only | partial: cloth (`sigil-viewer/src/cloth.rs`) and Grimoire's NPR materials; no animation retarget. Issue superuserkalo/sigil#9 | L | Open: issue #9 |
| Decals and flipbooks: animated pictures on a hero that can follow health or speed | vpkmerge #94 | PR open | excluded: on hold (Foundry heavy edits, painting) | - | - |

## Stats

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| deadlock-api rank rework, rank details, scoreboard, leaderboard seasons, crosshair codes | #575 | merged-unreleased | has: `deadlock-core/src/stats.rs`, `stats/leaderboard.rs`, `crosshair.rs` (`upstream-gaps-2026-11-21.md`) | - | - |
| Street Brawl games tagged in the match list | #575 | merged-unreleased | has (owner's call, 2026-11-21): `stats/filters.rs` `is_street_brawl`, the tag in `stats/matches.rs`, and every aggregate skips them | - | - |
| Hardening: Top X% only for a current-season match and `<0.1%` for tiny values; a rank endpoint failure leaves the card at `--`; no fallback to old MMR in the chart ("not enough ranked matches yet"); fast season switches never show an old board; Enter on the Statlocker link opens it | #586 `fix(stats)` | branch WIP | has: `stats.rs` `rank_in_current_season` gates the share to a badge earned in the running season (from the ranked-season list) and `RankBenchmark::top_percent_text` writes `<0.1%` under a tenth; a failed rank request leaves `PlayerRank::unavailable` and the brief reads `--`; the chart draws ranked matches only and says "Not enough ranked matches to chart yet."; one leaderboard load at a time (`Job`), so an earlier board can never land over a newer one and the caption names the board's own season and region; the Statlocker link is its own button, so no key event bubbles from a match row | - | - |

## Performance and config

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Custom performance presets from public GitHub repositories: inspect a repo at a commit, pick gameinfo and video files (video-only and ConVars snippets too), saved snapshots, optional-setting switches, hide built-ins, delete with confirm, import a repo's VPK addons through the normal import and safety review, import history | #572, issue #568, site #15 | PR open | partial: `performance/repos.rs` reads a public repository at a commit, lists gameinfo and video files, keeps a snapshot, offers hide built-ins, delete and Apply as written, imports the repo's VPKs through the normal import, and keeps import history (`screens/performance/custom.rs`). The core stores per-setting switches but the UI does not offer them | M | Open: per-setting switches |
| Keep a gameinfo.gi author's own formatting, sections and comments instead of re-inserting every convar with a marker | issue #590 | issue only | partial: Apply as written takes the author's ConVars values in their order (`performance/as_written.rs`) but comments are dropped, so formatting is not kept | M | Open: keep comments and sections |
| Edit convar values and add custom ones in the UI, grouped (Visibility, FOV, QoL) | issue #513 | issue only | has: convar editor and history (`feature-parity.md`, Performance; `performance/classify.rs`) | - | - |
| Crosshair: a crosshair section Sigil wrote is removed from autoexec while the designer is off, since Deadlock now runs `autoexec.cfg` itself; Import from game reads the `base` block of `citadel_hero_settings.lst` | #504 | PR open | has: `crosshair.rs` `parse_hero_settings` for Import from game, `screens/crosshair_game.rs` for the page-off cleanup, and profile apply skips the crosshair while the page is off (`profiles.rs` `skip_crosshair`) | - | - |
| Teamfight benchmark (see "Benchmark: what Grimoire's WIP does") | #573, vpkmerge #89 | PR open | has, real game unverified: `docs/design/benchmark.md`, `deadlock-core/src/benchmark/`, `screens/performance/benchmark/`; never run against a real Deadlock | - | - |
| Community benchmark aggregates | grimoire-social #11 | PR open | excluded: Sigil accounts (social backend) | - | - |

### Benchmark: what Grimoire's WIP does

For Sigil's builder, from `feat/deadlock-benchmarks-wip` (`docs/benchmark.md`,
`electron/main/services/benchmark*.ts`, `resources/benchmark/`, `src/components/benchmark/`):

- Off by default behind Settings > Experimental > Benchmark launcher; a Launch Benchmark entry in
  the sidebar. Asks the player to close a normal Deadlock session first.
- Runs an isolated writable game copy (links to the installed VPKs, about 1 GB) in its own
  Proton prefix, so retail files are never changed. Linux only today (Python 3, rsync, Modlock
  CLI and host 0.1.17, Proton, Steam).
- Workload: a bundled replay-derived teamfight (match 132204644, Midtown, 12 actor tracks), run
  by a Modlock scenario as a spectator with troopers, camps, bots and hibernation off. Pinned to
  client build 6860; another build blocks the launch. A `manifest.json` holds sha256 for every
  fixture file.
- One warm-up and three 30 second measured windows, each with a 5 second lead-in; about three
  minutes after load.
- Supervisor: Stop cancels, quitting the app stops, an 8 minute timeout cleans up, a recovered
  session record offers Stop before the next launch. Only the session's own processes are killed.
- Display: Use game settings (copies `cfg/video.txt` on each launch) or fixed 720p, 1080p, 1440p
  windowed; other settings kept.
- Per-session folder with Open logs: `server-launch.log`, `client.log`, `video-settings.txt`,
  `display.json`, the exact scenario used, `result.json` with `score_eligible: false`.
- Capture: MangoHud 0.8.4 on Linux (raw frame times, sensors, normalised CSVs); Windows and Metal
  are stubs; a missing recorder says "disabled" with a reason instead of a zero score.
- Results dialog opens after the run and reopens later (View latest results): average FPS, 1%
  low, p99 and longest frame time, duration, frame interval count, FPS over time and a frame time
  graph whose bins keep the worst frame; hardware and display under a details fold; explicit
  missing, disabled and failed states.
- Ledger: `ledger.sqlite3` with configuration hashes, UTC dates, suite and run labels, pass
  markers, outcomes and artifact hashes; notes append without rewriting; a CLI with `list`,
  `note`, `diff`, `export`, `verify`, `import-session`.
- Planned, not built: history browser, run detail and compare views, cross-platform capture,
  opt-in community aggregation (grimoire-social #11: allowlisted fields, a cell needs 5
  contributors and 10 launches).

Already in Sigil's design (`docs/design/benchmark.md`): warm-up pass, 1 to 5 measured passes,
600-bin graph with the worst frame per bin, pinning to a game build, machine and settings
capture (`benchmark/system.rs` has GPU and driver), history, comparisons and exports, restore at
start after a crash. Not yet in it: a fixed window resolution per run, an overall timeout, a
per-run log folder with Open logs, a fixture hash check, and appendable notes on a finished run
(`benchmark/store.rs` has one `note` string).

## Servers and maps

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Server picker like ServerPickerX: block relay regions through the firewall | issue #583 | issue only | has, real game unverified: `screens/servers_regions/` (Valve's relay list cached a day, ping, tick and block, presets, Apply and Reset as one `netsh` batch, Where I play); never run on a Windows PC | - | - |
| Hyperline / Modlock integration, the way Deadworks servers are integrated | issue #514 | issue only | missing: Deadworks list and hosting only (ADR 0015, `rust/crates/deadlock-core/src/servers.rs`) | L | Needs an allowlist entry for Hyperline's endpoints in `AGENTS.md` and `docs/privacy.md` first; Modlock is a separate host binary, pinned by version and hash |

## Launch and game health

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Diagnostics include the GPU, driver and hardware acceleration state (black screen reports) | branch `claude/grimoire-black-screen-menu-*` (no PR) | branch WIP | has: `diagnostics::machine_line` reads the benchmark's `Machine` and every report carries it (Send diagnostics preview and summary, Copy report, the troubleshooting section); a Mac without a reportable adapter says so instead of claiming software rendering | - | - |
| Runs on CPUs without AVX2 | issue #574 | issue only | has: no `target-cpu` override in the Rust build (no `rust/.cargo/config.toml`) | - | Keep it that way |
| Compatibility check skips sound repair for models without game data (soul container, urn, effect meshes), so 13 false "unsupported" findings go away | vpkmerge #95 | PR open | has: `compat_repair::check` drops sound findings when the game's own model carries an empty `m_modelInfo.m_keyValueText` (the rule of vpkmerge #95), while present but unreadable metadata still reports why | - | Bump `vpkmerge-core` and drop the local guard when #95 merges |

## Downloads and installs

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A warning when downloads crawl (Grimoire: under 1 MB/s for 10 s) | #589 | merged-unreleased | has: `rust/crates/desktop/src/screens/downloads.rs` (`Item::is_slow`, one toast per download) | - | - |
| "Enable mods after download" also turns on imports that passed the safety review | #591 | merged-unreleased | partial: Browse, source and Forge installs ask the landing question before they land on (`browse/quick.rs`, `settings/sources.rs`, `settings/forge.rs`); `screens/import.rs` still always imports off | S | Open |
| Local import dialog: variant grouping, editable names, a shared image gallery with a main thumbnail, editable tags, retry of files already imported | #593 `787183fe`, `ImportCustomModsModal.tsx` | PR open | partial: the add-mods dialog groups several VPKs as one mod with editable names (`screens/import/parts.rs`); a gallery and tags at import are not part of it | M | Open after the dialog |
| Compact download queue states and transitions | #593 `fda8d276` | PR open | has: queue rows with states (`feature-parity.md`, Downloads) | - | Layout only |

## Shell and settings

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Light theme | #584 (replaced the closed #342) | merged-unreleased | has: `rust/crates/desktop/src/ui.rs` `Theme::Light` | - | - |
| Custom accent and glow colors kept as swatches | #581 | merged-unreleased | has (`upstream-gaps-2026-11-21.md`, ported list) | - | - |
| Three-digit hex accepted; light theme contrast fixes | #586 `fix(settings)` | branch WIP | has: `rust/crates/desktop/src/prefs.rs` `parse_hex` takes `#rgb` | - | - |
| "The Sex Update": a joke page with a flower field and a bee cursor that leaves pollen, behind an Experimental switch | #594 | PR open | missing: Sigil's easter eggs have no such page (`feature-parity.md`, Global shell "Easter eggs") | S | Optional look: build it off by default; port `src/components/sex-mode/` (`FlowerField.tsx`, `BeeEffects.tsx`) and `public/bee-cursor.svg` |

## Foundry and creator

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Door Stuck Mechanics: Parry editor (Shield, Catch, Stun parts, linked colors, emoji or image symbols, parry sounds, undo and redo) | #571, vpkmerge #88 | merged-unreleased | excluded: on hold (Foundry heavy edits) | - | - |
| Decals and flipbooks on hero models | vpkmerge #94 | PR open | excluded: on hold (Foundry heavy edits, painting) | - | - |
| Global sounds swap: UI clicks, music, ambience, NPC lines, item sounds | vpkmerge issue #44 | issue only | owner decision: the swap exists for hero events (`rust/crates/desktop/src/screens/locker/game_sounds.rs`); is a global catalog part of the held sound editing? | M | If yes to build: enumerate `soundevents/` beyond `vo/` and `hero/` by tree prefix, as the issue describes |
| vpkmerge MCP server and workbench agent: 19 tools to browse sounds and textures, inspect mods, search modding docs, preview heroes, build swaps and merges into staging | vpkmerge #50 | PR open | partial: the agent host has `game_textures`, `game_sounds`, `files_list`, `files_show`, `files_search`, `files_compare` and `files_export` beside index, paths, pack, validate, install plan, stats and recolor brief tools (`rust/crates/agent-host/src/tools.rs`); no docs search and no sound swap builder | M | Open |
| Foundry tab: browse the game's assets and drop in a replacement | `docs/foundry-tab-design.md` (design) | issue only | has: Game art and Game sounds with swap (`locker/catalog.rs`, `locker/game_sounds.rs`); heavy edits excluded | - | - |

## Other

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A Grimoire-hosted mod platform | branch `claude/mod-platform-scoping-*` (no PR) | branch WIP | excluded: Sigil accounts | - | - |
| Apple Silicon macOS build | branches `feat/macos-build`, `claude/mac-native-build-feasibility-*` | branch WIP | has: Sigil is one native executable on macOS (ADR 0006, ADR 0010) | - | - |
| A player user guide in the repo, with a status label on every doc | branch `docs/user-guide-md` | branch WIP | has: `docs/user-guide.md` (install, Browse, Installed, load order, profiles, Locker, 3D, crosshair, autoexec, Performance and benchmark, Stats and live, Servers, Launch, updates, privacy, feedback) | - | - |
| More languages | `translations/*` (closed PRs) | branch WIP | excluded: translation on hold | - | - |

## Counts

All rows in the area tables above (the benchmark list is one row).

| Sigil status | Rows |
| --- | ---: |
| has (4 of them real game unverified) | 44 |
| partial | 12 |
| missing | 6 |
| in progress | 2 |
| excluded | 6 |
| owner decision | 3 |
| **Total** | **73** |

| Maturity | Rows |
| --- | ---: |
| merged-unreleased | 18 |
| PR open | 30 |
| branch WIP | 15 |
| issue only | 10 |
| **Total** | **73** |

Of the 18 merged-unreleased rows (the 16 commits on `main` since v1.35.1), Sigil has or is building
all but the excluded Parry editor, imports that follow "Enable mods after download" (partial) and
profile ability colors (owner decision). The alt albedo reveal, the whole-preview loader and the
benchmark all shipped in 0.1.22.

## Build list

The open rows after 0.1.22, grouped so builders in the same round touch different code. The two
rows owned by other builders (stats hardening, sound repair) are left out.

| Round | Code area | Items | Count |
| --- | --- | --- | ---: |
| 1. Installed cards | `desktop/src/screens/installed/` (`notes.rs`, `selection.rs`, `layout.rs`, `repair.rs`), `deadlock-core/src/mod_extras.rs` | Issues pulse (S), attach local files to a GameBanana card with its rules (S), Add files and Add images in the variant picker (S), floating preview panels (M, last), GameBanana schema tag rules (M) | 5 |
| 2. Locker creation tools | `deadlock-core/src/cursors.rs`, `desktop/src/screens/locker/card_art.rs`, `screens/locker/cards.rs` | cursor cropper (M), skin to card icon link (M), Locker cards' hover 3D and saved angle (M, after the per-hero camera) | 3 |
| 3. Viewer | `sigil-viewer/src/`, `desktop/src/viewer/` | motion previews against a second hero (M, issue #8), animation retarget and fidelity plan (L, issue #9) | 2 |
| 4. Performance and config | `deadlock-core/src/performance*`, `desktop/src/screens/performance/` | GitHub preset per-setting switches in the UI (S), keep the author's gameinfo comments and formatting (M) | 2 |
| 5. Imports, classification and agent | `desktop/src/screens/import.rs`, `deadlock-core/src/import.rs`, `agent-host/src/tools.rs` | local imports follow "Enable mods after download" after the scan (S), gallery and tags in the import dialog (M), sound swap builder and modding docs search for the agent (M) | 3 |
| 6. Waiting on outside events | `rust/crates/deadlock-core/Cargo.toml`, `servers.rs` | vpkmerge rev bump for #95 (S, when it merges), Hyperline / Modlock integration (L, needs an allowlist entry first) | 2 |
| 7. Optional look | shell easter eggs | the joke flower page, off by default (S) | 1 |

Owner decisions to settle: Locker ability recolor (also decides profile colors from #587), the
global sounds swap, and Grimoire's Door Stuck Parry editor stays on hold with the Foundry.
