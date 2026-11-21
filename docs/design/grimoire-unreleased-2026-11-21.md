# Grimoire unreleased and planned work against Sigil

Status: research snapshot, 2026-11-21. Read only for Sigil's code; nothing here is built by this
doc. Companion to `upstream-gaps-2026-11-21.md`, which covers what Grimoire and Deadlock Mod
Manager already ship. This doc covers only what Grimoire has not released yet: open PRs,
unmerged branches, closed-but-unmerged PRs, unreleased commits on `main`, open issues, plan docs,
and the open work in its sibling repositories. Grimoire is MIT, so its code and copy may be ported.

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
| Extended hero matching: a setting that also counts hero names in titles and hero categories outside Skins | #593 `1795b537`, `009b0c30` | PR open | partial: hero picker and a name match on non-skin sections only (`docs/design/feature-parity.md`, Browse "Hero filter on sounds") | S | Port the title match from `src/pages/Browse.tsx` and the switch from `ExperimentalSection`/settings in `1795b537`; add it to Settings > Mods |
| Picking a subcategory (Model Replacement > Hideout) filters the saved catalog by that subtree, not the whole root | #520 `fix(browse)` | branch WIP | partial: live reads pass `Generic_Category` (`rust/crates/deadlock-core/src/gamebanana.rs`); `catalog_mirror.rs` keeps no sub category | S | Store `sub_category_id` in mirror rows and match the pick's subtree, as `fix/gb-category-drift` does |
| Mouse back and forward buttons walk back through creator pages and details; refreshed creator controls | #593 `d9044df0` | PR open | missing: Rune exposes `PointerButton::Back/Forward` (`vendor/rune/crates/rune/src/input/canvas_input.rs`), nothing in `rust/crates/desktop/src` handles them | S | Map Back and Forward onto the existing details and artist back stack, app wide |
| "X is no longer on GameBanana": install its replacement and delete the old file, keeping on or off and the load folder | #593 strings `confirmBody` | PR open | has: `rust/crates/desktop/src/screens/browse/replace.rs`, `deadlock-core/src/install.rs` `superseded` | - | - |
| Hidden mods and creators: thumbnails blurred until hover, date hidden, the mod it was hidden from, a 140 character note | #580 | merged-unreleased | in progress: `rust/crates/desktop/src/screens/browse/hidden.rs` (uncommitted) | - | - |
| List layout removed from Browse | #593 `c0137128` | PR open | has: Sigil keeps grid and list (`feature-parity.md`, Browse "Grid / list view") | - | Keep Sigil's list view; nothing to do |

## Installed

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Tag filters cycle: left click goes included, excluded, off; right click clears; a setting turns cycling on | #593 `4583c71a`, `7b9e7b9e` | PR open | missing: one tag filter at a time (`rust/crates/desktop/src/screens/installed/notes.rs`, `tag: Option<String>`) | M | Port `src/components/installed/InstalledTagSelect.tsx` and `src/stores/installedTagsStore.ts`; keep a per-tag state map |
| One tag menu for custom, base (category) and hero tags, with display priorities, move up and down, and reset to default order | #593 `ddeb860f`, `6d4a5dde` | PR open | missing: free text tags per mod (`installed/notes.rs` `Dialog::Tags`) | M | Port `src/components/installed/ModTagMenu.tsx` and `src/lib/installedCardTaxonomy.ts` |
| The Issues button pulses when there are findings the player has not seen | #593 `0cc75cf5` | PR open | missing: the compatibility review has no "new since seen" mark (`installed/repair.rs`, `installed/checks.rs`) | S | Remember the last seen finding keys in options; port the seen logic from `src/components/installed/InstalledIssuesModal.tsx` |
| Attach local VPKs to a GameBanana mod's card; attachments survive updates; one GameBanana submission per group; files must share a load folder | #593 `0a874d24` | PR open | partial: Group as one mod adds a file to a group card (`installed/selection.rs`); the two rules and the update case are not checked | S | Add both rules to `group_name`/`with_groups` and a `mod_updates` test that an update keeps attached files in the group |
| Local mod editing moves into the variant picker: rename, Add VPK files, Add images, Make mod thumbnail | #593 `4d077700`, `787183fe` | PR open | partial: Edit local mod (name, one picture, NSFW) in `installed/notes.rs` `Dialog::EditLocal`; the picker renames labels | S | Add Add files and Add images to the picker; images need a gallery field in `deadlock-core/src/mod_extras.rs` |
| Hero detection for uncategorized and multi-file submissions; heroes inferred from unassigned skin metadata | #593 `9967d3a2` | PR open | partial: `rust/crates/deadlock-core/src/mod_heroes.rs` reads model and material paths only | S | Port the title and metadata fallback from `electron/main/services/modHeroDetection.ts` |
| Category tags follow GameBanana's current schema: every hero skin gets Skins however it is filed (Faithful/Vanilla+ and the misspelled Faithul), hero aliases and possessives, whole-word matches, hero from `sounds/vo/<codename>/` and footsteps | #520 commits 1, 5, 6 | branch WIP | partial: GameBanana category plus model paths (`mod_heroes.rs`, `installed/notes.rs` backfill) | M | Port the tag rules from `fix/gb-category-drift` (`src/lib/categoryPaths.ts`, tag helpers) with its tests |
| Redesigned selection toolbar, selected-card borders, no hover actions while selecting | #593 `a7e43d92` | PR open | has: selection bar with bulk actions (`feature-parity.md`, Installed "Multi-select") | - | Layout only |
| Pinned preview panels: bounded drag, Shift snaps to edges or center, resize corners | #593 `5eeeb5e4` | PR open | missing: Installed has no floating panels; only the docked details resize | M | Only worth it with a floating 3D preview; port `src/components/common/useFloatingPanel.ts` and `src/lib/floatingPanelGeometry.ts` |
| Simpler requirements dialog; the menu item hides when a mod declares no requirements | #593 `1f14a24a`, `bda062f2` | PR open | has: requirements section and fix (`feature-parity.md`, Mod details and Installed) | - | - |
| A multi-file card opens the mod page; Choose files in its menu | #576 | merged-unreleased | has: `installed/layout.rs` `open_group` | - | - |
| Mod safety: Disallow takes an approval back; the mod turns off first | #592 | merged-unreleased | in progress: `installed/safety_review.rs` (uncommitted) | - | - |
| Mod safety reads text layouts and empty root folders; Enable anyway for a file it cannot read | #578 | merged-unreleased | has: `docs/design/mod-safety.md`, `installed/safety_gate.rs`, `deadlock-core/src/vpk.rs` `empty_root` | - | - |
| Imports named from their imprints (pack members, variant labels) | #582 | merged-unreleased | has: `rust/crates/deadlock-core/src/import.rs` | - | - |
| Imprint names cleaned (control, bidi and zero-width characters stripped, 120 character cap); colliding pack labels fall back to file names; pack members keep their own thumbnail | #586 `fix(import)` | branch WIP | missing: no cleaning in `deadlock-core/src/import.rs` | S | Port the cleaning helper from `fix/review-followups-2026-11` into the imprint reader |
| List layout removed from Installed | #593 `c0137128` | PR open | has: Cards, Small cards, List stay (`installed/layout.rs`) | - | Keep; nothing to do |

## Profiles

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A profile keeps the Locker shuffle switches and pools | #587 (and older Grimoire profiles) | merged-unreleased | in progress (profiles keep shuffle) | - | - |
| A profile keeps the Locker's ability colors and trippy skins and rebuilds them on apply; Locker edits save into the active profile | #587 `electron/main/services/profileLocker.ts` | merged-unreleased | owner decision: real ability recolor is undecided (`rust/crates/deadlock-core/src/recolor.rs` writes briefs only) | L | Follows the Locker recolor decision |
| A profile's crosshair is skipped while the crosshair designer is off | #504 | PR open | partial: Crosshair can be turned off as a page (General > Pages), not checked whether profile apply skips its crosshair (`deadlock-core/src/profiles.rs`) | S | Gate the crosshair step of profile apply on the Crosshair page switch |

## Locker

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| New General categories: Patrons (Hidden King, Archmother), Sinners (Sinner's Sacrifice vault), Boxes (crates), with cached model previews for the three props | #593 `23144832`, `src/lib/categoryPaths.ts` | PR open | missing: `rust/crates/deadlock-core/src/locker.rs` `GeneralType` stops at Killstreak Music | S | Port `GENERAL_PROP_MODELS` (`models/npc/patron_amber/`, `models/npc/patron_sapphire/`, `models/props_gameplay/sinners_sacrifice_vault/`), `detectGeneralPropTypes` and the category name aliases |
| Shared mod favorites between Installed and the Locker; saved mods sort first | #593 `fdccada8` | PR open | partial: hero favorites in the Locker and Browse favorites (`rust/crates/desktop/src/screens/locker.rs`); no mod favorite on Installed | S | One favorite flag in `mod_extras`, read by both pages; port the sort from `src/components/locker/ModFavorites` |
| Locker cards: live 3D model only on hover, right drag to rotate, Resume auto-spin, Save preview angle per mod | #593 `src/components/locker/SoulContainerTile.tsx`, `soulModel.ts` | PR open | in progress (per-hero camera); a saved angle per mod is not covered | M | Store a yaw per mod in `mod_extras` and feed it to the card viewer |
| Hero-less sounds (Patron and newscaster VO, urn lines, vaults, shared parry, hit feedback, UI) go to Announcer / SFX; QoL and gameplay tweaks stay out of hero piles | #520 commits 6, 7 | branch WIP | partial: `GeneralType::Announcer` exists (`locker.rs`); these path rules are not checked | S | Port the path list from the two `fix(locker)` commits in `fix/gb-category-drift` |
| Global priority tab and custom Locker categories removed | #593 `a8bf12f9` | PR open | has: Sigil keeps Always load first and Add to a Locker category (`feature-parity.md`, Installed) | - | Keep; nothing to do |
| Create a cursor pack from your own images: a tile per stock cursor, a cropper with a hotspot crosshair, an automatic hotspot guess, Create and apply, export | #423 (closed, never merged) | branch WIP | missing: `rust/crates/deadlock-core/src/cursors.rs` imports and applies packs only | M | Port `src/lib/cursorHotspot.ts`, `src/lib/cursorBmp.ts` (32-bit BI_RGB with alpha) and the `cursor.res` override writer from `feat/cursor-create` |
| Link a skin to its matching card icon mod, so the hero's card follows the skin whenever it is on | #335 (closed, "postponing") | branch WIP | missing: `rust/crates/desktop/src/screens/locker/card_art.rs` has no link | M | Port `src/lib/lockerCardLinks.ts`, `classifyHeroIconOnly` and `reconcileLinkedCards`; run the reconcile after every enable change |
| 3D card snapshots: hero card art baked from the installed skin's pose, the panorama card as backdrop, pose-cycle arrows | branch `feat/locker-3d-card-snapshots` (no PR) | branch WIP | missing: card art comes from the player's image (`locker/card_art.rs`) | L | Render with `rust/crates/sigil-viewer/src/capture.rs` offscreen, then hand the PNG to `build_hero_card_mod` |
| Ability VFX recolor UI: hue, saturation and brightness with a live recolored preview; particle recolor CLI | `docs/ability-vfx-recolor.md` (status in progress) | issue only | owner decision (Locker ability recolor; `deadlock-core/src/recolor.rs` writes briefs) | L | - |
| Hero card apply pipeline | `docs/locker-hero-card-apply.md` (design) | issue only | has: In-game card art (`locker/card_art.rs`, `feature-parity.md` Locker) | - | - |

## 3D viewer

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A preview appears only when whole: a loader with Hideout build progress, staged heroes wait, 400 ms hold, 20 s fallback | #588 | merged-unreleased | partial: loading and failed states (`docs/design/3d-viewer.md`, `viewer-locker-loading`, `viewer-hideout-loading`) | S | Build with the staged pose; port `src/components/common/PreviewLoader.tsx` and the gate in `HeroPoseViewer.tsx` |
| Motion previews: dash, heavy melee and parry mods play on Lash against Bebop; an animation mod plays its own clips with a clip strip, All and pause; body swaps follow the clip | #571 | merged-unreleased | in progress (staged pose, anim-event effects, body groups) | - | - |
| Alt albedo reveal (`g_tAltColor_Reveal` by `g_flAltTextureAmount`), so Violet's brush is black with a red tip | #579 `src/lib/source2AltReveal.ts` | merged-unreleased | missing: no alt reveal blend in `rust/crates/sigil-viewer/src/material.rs` | S | Port the blend from `source2AltReveal.ts` into the material shader |
| Skins whose pak stores an empty root folder name preview | #579 | merged-unreleased | has: `deadlock-core/src/vpk.rs` `empty_root` | - | - |
| Recolor reuses stamp textures; staged actor rigs are disposed after cancel | #586 `fix(viewer)` | branch WIP | in progress (staged pose) | - | - |
| Custom animations (taunts, dances) on any hero skeleton; cloth bones; Source 2 material and lighting parity | `docs/3d-preview-fidelity-plan.md` (no code) | issue only | partial: cloth (`sigil-viewer/src/cloth.rs`) and Grimoire's NPR materials; no animation retarget | L | Phase 3 of Grimoire's plan; later than the in-progress extras |
| Decals and flipbooks: animated pictures on a hero that can follow health or speed | vpkmerge #94 | PR open | excluded: on hold (Foundry heavy edits, painting) | - | - |

## Stats

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| deadlock-api rank rework, rank details, scoreboard, leaderboard seasons, crosshair codes | #575 | merged-unreleased | has: `deadlock-core/src/stats.rs`, `stats/leaderboard.rs`, `crosshair.rs` (`upstream-gaps-2026-11-21.md`) | - | - |
| Street Brawl games tagged in the match list | #575 | merged-unreleased | in progress (Street Brawl tag) | - | - |
| Hardening: Top X% only for a current-season match and `<0.1%` for tiny values; a rank endpoint failure leaves the card at `--`; no fallback to old MMR in the chart ("not enough ranked matches yet"); fast season switches never show an old board; Enter on the Statlocker link opens it | #586 `fix(stats)` | branch WIP | partial: `stats.rs` `top_percent` exists; the "not enough ranked matches" state and the season race guard were not found | S | Port the `fix(stats)` commit of `fix/review-followups-2026-11` behaviour by behaviour |

## Performance and config

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Custom performance presets from public GitHub repositories: inspect a repo at a commit, pick gameinfo and video files (video-only and ConVars snippets too), saved snapshots, optional-setting switches, hide built-ins, delete with confirm, import a repo's VPK addons through the normal import and safety review, import history | #572, issue #568, site #15 | PR open | partial: fixed GitHub catalog (`rust/crates/deadlock-core/src/performance/catalog.rs`) and import of a pasted or file config (`performance/exchange.rs`); no repo source, no hide built-ins | M | Port from `feat/custom-performance-presets`; requests go to GitHub, which Sigil already reads for configs; add the entry to `docs/privacy.md` |
| Keep a gameinfo.gi author's own formatting, sections and comments instead of re-inserting every convar with a marker | issue #590 | issue only | partial: Sigil edits line by line and tags each change `sigil-perf`, the same model (`rust/crates/deadlock-core/src/performance.rs` header) | M | Offer "use the author's file as written" for presets that ship a whole file; keep the BEGIN marker so removal still works |
| Edit convar values and add custom ones in the UI, grouped (Visibility, FOV, QoL) | issue #513 | issue only | has: convar editor and history (`feature-parity.md`, Performance; `performance/classify.rs`) | - | - |
| Crosshair: a crosshair section Sigil wrote is removed from autoexec while the designer is off, since Deadlock now runs `autoexec.cfg` itself; Import from game reads the `base` block of `citadel_hero_settings.lst` | #504 | PR open | partial: Sigil can write `citadel_hero_settings.lst` (`rust/crates/desktop/src/screens/crosshair.rs` `toggle_hero_settings`), but imports from `machine_convars.vcfg` only (`deadlock-core/src/crosshair.rs` `parse_machine_convars`) and has no cleanup when the page is off | S | Port `parseHeroSettings` and `syncCrosshairWithSettings` from `fix/crosshair-autoexec-reset` |
| Teamfight benchmark (see "Benchmark: what Grimoire's WIP does") | #573, vpkmerge #89 | PR open | in progress: `docs/design/benchmark.md`, `deadlock-core/src/benchmark/` (uncommitted) | - | - |
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
| Server picker like ServerPickerX: block relay regions through the firewall | issue #583 | issue only | in progress (region picker extras; ADR 0016) | - | - |
| Hyperline / Modlock integration, the way Deadworks servers are integrated | issue #514 | issue only | missing: Deadworks list and hosting only (ADR 0015, `rust/crates/deadlock-core/src/servers.rs`) | L | Needs an allowlist entry for Hyperline's endpoints in `AGENTS.md` and `docs/privacy.md` first; Modlock is a separate host binary, pinned by version and hash |

## Launch and game health

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Diagnostics include the GPU, driver and hardware acceleration state (black screen reports) | branch `claude/grimoire-black-screen-menu-*` (no PR) | branch WIP | partial: the benchmark reads GPU and driver (`rust/crates/deadlock-core/src/benchmark/system.rs`); Send diagnostics does not (`deadlock-core/src/diagnostics.rs`) | S | Add `system::Machine` to the diagnostics report preview |
| Runs on CPUs without AVX2 | issue #574 | issue only | has: no `target-cpu` override in the Rust build (no `rust/.cargo/config.toml`) | - | Keep it that way |
| Compatibility check skips sound repair for models without game data (soul container, urn, effect meshes), so 13 false "unsupported" findings go away | vpkmerge #95 | PR open | missing: `vpkmerge-core` pinned at df41586 (`rust/crates/deadlock-core/Cargo.toml`) | S | Bump the rev when #95 merges |

## Downloads and installs

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A warning when downloads crawl (Grimoire: under 1 MB/s for 10 s) | #589 | merged-unreleased | has: `rust/crates/desktop/src/screens/downloads.rs` (`Item::is_slow`, one toast per download) | - | - |
| "Enable mods after download" also turns on imports that passed the safety review | #591 | merged-unreleased | partial: `rust/crates/desktop/src/screens/import.rs` always imports off (`upstream-gaps-2026-11-21.md`, top 3) | S | Land on only when the scan is clear or the version is trusted, then follow the setting |
| Local import dialog: variant grouping, editable names, a shared image gallery with a main thumbnail, editable tags, retry of files already imported | #593 `787183fe`, `ImportCustomModsModal.tsx` | PR open | in progress (add-mods dialog with several VPKs as one mod); the gallery and tags at import are not part of it | M | Port the gallery and tag rows of `src/components/ImportCustomModsModal.tsx` after the dialog lands |
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
| vpkmerge MCP server and workbench agent: 19 tools to browse sounds and textures, inspect mods, search modding docs, preview heroes, build swaps and merges into staging | vpkmerge #50 | PR open | partial: Sigil's agent host has game index, paths, pack, validate, install plan, stats and recolor brief tools (`rust/crates/agent-host/src/tools.rs`); no sound or texture catalog tools, no swap builder, no docs search | M | Add read tools over `game_assets::sound_catalog` and `texture_catalog` and a staging-only sound swap tool; painting stays excluded |
| Foundry tab: browse the game's assets and drop in a replacement | `docs/foundry-tab-design.md` (design) | issue only | has: Game art and Game sounds with swap (`locker/catalog.rs`, `locker/game_sounds.rs`); heavy edits excluded | - | - |

## Other

| Feature (player view) | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A Grimoire-hosted mod platform | branch `claude/mod-platform-scoping-*` (no PR) | branch WIP | excluded: Sigil accounts | - | - |
| Apple Silicon macOS build | branches `feat/macos-build`, `claude/mac-native-build-feasibility-*` | branch WIP | has: Sigil is one native executable on macOS (ADR 0006, ADR 0010) | - | - |
| A player user guide in the repo, with a status label on every doc | branch `docs/user-guide-md` | branch WIP | missing: `docs/` holds engineering docs only | S | Optional; Grimoire's real guide is grimoire-site's `src/content/docs/` |
| More languages | `translations/*` (closed PRs) | branch WIP | excluded: translation on hold | - | - |

## Counts

All rows in the area tables above (the benchmark list is one row).

| Sigil status | Rows |
| --- | ---: |
| has | 21 |
| partial | 18 |
| missing | 15 |
| in progress | 10 |
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
all but the excluded Parry editor and four others:
the alt albedo reveal and the whole-preview loader (viewer), imports that follow "Enable mods
after download" (install), and profile ability colors (owner decision).

## Build list

Only missing and partial rows, 33 in all, grouped into rounds that touch separate code so they
can run in parallel. Rounds 1 to 5 can start together; round 6 waits on outside events.

| Round | Code area | Items | Count |
| --- | --- | --- | ---: |
| 1. Installed tags and cards | `desktop/src/screens/installed/` (`notes.rs`, `selection.rs`, `layout.rs`, `repair.rs`), `deadlock-core/src/mod_extras.rs` | tag filter cycling (M), unified tag menu with priorities (M), Issues pulse (S), attach local files to a GameBanana card with its rules (S), Add files and Add images in the variant picker (S), floating preview panels (M, last) | 6 |
| 2. Classification in the core | `deadlock-core/src/mod_heroes.rs`, `locker.rs`, `catalog_mirror.rs`, `gamebanana.rs` | hero detection fallback (S), GameBanana schema tag rules (M), hero-less sounds to Announcer / SFX (S), Patrons, Sinners and Boxes General categories (S), subcategory subtree on the saved catalog (S), extended hero matching setting (S), shared mod favorites (S) | 7 |
| 3. Locker creation tools | `deadlock-core/src/cursors.rs`, `desktop/src/screens/locker/card_art.rs`, `sigil-viewer/src/capture.rs` | cursor pack from your own images (M), skin to card icon link (M), 3D card snapshots (L) | 3 |
| 4. Viewer | `sigil-viewer/src/material.rs`, `desktop/src/viewer/` | alt albedo reveal (S), whole-preview loader (S, with the staged pose), animation retarget and fidelity plan (L, later) | 3 |
| 5. Performance, crosshair and config | `deadlock-core/src/performance*`, `crosshair.rs`, `desktop/src/screens/performance/`, `screens/crosshair.rs`, `profiles.rs` | GitHub repo presets with hide built-ins (M), keep the author's gameinfo formatting (M), crosshair cleanup and `citadel_hero_settings.lst` import (S), profile skips the crosshair when its page is off (S) | 4 |
| 6. Small fixes across areas | `deadlock-core/src/import.rs`, `stats.rs`, `diagnostics.rs`, `desktop/src/app.rs` (mouse buttons), `screens/import.rs`, `agent-host/src/tools.rs`, shell easter eggs | imprint name cleaning (S), stats hardening (S), GPU in diagnostics (S), mouse back and forward (S), imports follow "Enable mods after download" after the scan (S), agent sound and texture tools (M), the joke flower page off by default (S), player user guide (S) | 8 |
| 7. Waiting on outside events | `rust/crates/deadlock-core/Cargo.toml`, `servers.rs` | vpkmerge rev bump for #95 (S, when it merges), Hyperline / Modlock integration (L, needs an allowlist entry first) | 2 |
| **Total** | | | **33** |

Left out because they ride on work in progress: the gallery and tags in the import dialog (after
the add-mods dialog lands) and a saved preview angle per Locker card (after the per-hero camera).

Owner decisions to settle: Locker ability recolor (also decides profile colors from #587), the
global sounds swap, and Grimoire's Door Stuck Parry editor stays on hold with the Foundry.
