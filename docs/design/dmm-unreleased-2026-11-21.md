# Deadlock Mod Manager: unreleased and planned work against Sigil

Status: research snapshot, 2026-11-21. Read only for Sigil's code; nothing here is built. Companion
to `upstream-gaps-2026-11-21.md`, which covers both reference apps; this doc covers only Deadlock
Mod Manager (DMM, GPL-3.0) and every source of work it has not released yet. Behaviour only: no
DMM code or text is copied, only short feature names.

## Sources read

Upstream: `https://github.com/deadlock-mod-manager/deadlock-mod-manager`, fresh clone in
`.scratch/upstream/dmm` (gitignored), `git fetch --all` on 2026-11-21, `origin/main` at 9f09270d
(2026-11-19).

| Source | What was found |
| --- | --- |
| Releases | Latest stable v2.3.0 (2026-11-01). Nightly 2.4.0-nightly.20261110. `apps/desktop/package.json` is 2.4.0 and `CHANGELOG.md` has a finalised 2.4.0 section (commit be8d5fa6, 2026-11-12) with no tag: everything in it is merged and unreleased. 25 minor and about 85 patch entries. |
| Pending changesets | 27 `.changeset/*.md` on `main` (plus `config.json` and `README.md`), all after the 2.4.0 cut. |
| Unmerged remote branches | 273 (`git branch -r --no-merged origin/main`). Matched to PRs by head branch: 201 were squash-merged (their work is on `main`, released or in 2.4.0), 8 have an open PR, 26 had their PR closed unmerged, 38 never had a PR. Feature branches among the last two groups were read commit by commit with `git diff --stat origin/main...<branch>`. |
| Open PRs | 13. Feature or behaviour PRs: #1400, #1399, #1397, #1391, #1386, #1349, #923, #1413. The rest are dependency bumps, a data refresh and the deferred catalog teardown (#715). |
| Issues | 7 open (#1418, #1414, #1385, #1383, #1320, #1311, #477). The `Planned`, `Work In Progress` and `wip` labels hold no issues. Every milestone (v0.9.0 to v2.1) is closed with nothing open. Projects need a token scope we do not have; discussions hold one closed idea (#424, a Rich Presence plugin). |
| Docs site | `apps/docs/content`: no roadmap or "coming soon" page. Pages name four experimental switches (Custom Maps, Mod Foundry, Performance, Profile Management and Sharing); changeset `brisk-ravens-settle` makes profiles, sharing and conflict detection permanent. |

Branches skipped as CI, test harness, packaging metadata or infrastructure (listed so nothing is
hidden): the `feature/tauri-e2e-*` family (8), `feat/desktop-e2e-linux`, `feature/e2e-monkey`,
`ci/nix-e2e`, `ci/nightly-perf-fixes`, `chore/pr-1281-restore-e2e`, `chore/throwaway-unpushed-e2e-*`,
`test/nsfw-author-page-e2e`, the Wayfinder sweep branches (`bug/635` to `bug/646`, `chore/632`,
`chore/633`, `chore/640`, `perf/642`, `research/self-update-constraints`,
`docs/636-package-aware-update-policy`, `bugfix/626-wayfinder-sweep`), `chore/aur-*`, `fix/648-*`,
`fix/649-*`, `fix/aur-package-metadata`, `fix/apt-sigpipe`, `feature/apt-repository`,
`chore/nix-hashes`, `docs/nixpkgs-cef-package`, `chore/release-signing-environment`,
`cursor/feature-branch-releases-*`, `cursor/harden-docker-*`, `chore/gotta-go-fast`,
`chore/deps-bump-oct`, both `dependabot/*`, `chore/knip-*`, `chore/babysit-watcher`,
`fix/ci-bot-prs-merge-queue`, `coderabbitai/autofix/*`, `l10n_crowdin_*`, `chore/language-table`,
`chore/zh-cn-community-update`, `chore/gameinfo`, `chore/perf-catalog-refresh`,
`feature/tauri-cef-runtime-crate`, `feature/cef`, `feat/api-workbench-dashboard`,
`stormix/nuke-api-00` to `09` and `chore/deferred-catalog-teardown` (DMM retiring its own catalog
mirror; Sigil reads GameBanana directly already), `stormix/new-bot`, `feature/quickanswer`,
`bugfix/disable-ai-support*` (DMM's Discord bot), `feat/www-app-native-home`, `feature/www-i18n`,
`feature/agent-discovery`, `feature/add-themes` (website).

Superseded branches (their feature later landed on `main` another way, so nothing new):
`feature/stats`, `feature/stats-02`, `feature/stats-redesign` (#1022), `feature/mod-foundry-01` to
`03`, `feature/multible-modpaths` (became shards), `feature/convar-editor` (shipped as #532),
`feature/hud-compatibility-diagnostics` (#1210, replaced by 2.4.0 `f173624`),
`feature/mod-author-profiles` (#671, replaced by `feature/v2-mod-author-profiles`),
`codex/broken-mod-repair-ux` and its two base PRs (#485 to #487, replaced by the later repair
work), `profile-export-load-order` (#414), `feature/parse-heros`, `fix/local-upload`,
`fix/desktop-multipart-vpk-renaming`, `bugfix/match-sync-reenable-cancel` (#1073).

## Owner scope used

- **excluded** rows name the rule: dropped for good (external plugin system, background image from
  URL, DMM theme names and palettes, Steam-login match sync, Google Analytics, donation and credits
  rows, webview-only options); deferred to the Sigil accounts package (sign-in, friends, cloud
  backup, Discover publishing, community library, announcements, feature flags, author
  verification); on hold (more languages, Mod Foundry heavy edits: recolor, painting, removing hero
  sections or parts, sound editing).
- **in progress** rows are on the owner's current build list: 3D viewer extras, region picker
  "where I play", health dot and search, the install review step for gameinfo and cfg, resumable
  downloads, one-click file choice, live bridge auto rebuild, the add mods dialog with several VPKs
  as one mod (queued), the safety review list, Browse custom sources, refusing toggles while the
  game runs, obsolete and withheld notes, profiles keeping shuffle, hidden lists, Discord party
  size, the Street Brawl tag and a benchmark.

Columns: the feature as a player sees it; where it is upstream; maturity (merged-unreleased, PR
open, branch WIP, issue only; "PR closed" means the branch's PR was closed unmerged and the branch
is still there); Sigil's status with the file or doc that shows it; effort (S under a day, M a few
days, L a week or more); a build note for missing and partial rows.

## Browse

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Custom mod sources browsed in the store next to GameBanana: cards, release history, Markdown descriptions, a file picker | 2.4.0 `ef8af43` | merged-unreleased | in progress (`desktop/src/screens/browse/sources.rs`, `settings/sources.rs`) | M | - |
| Website links that add a mod source or install one of its mods | 2.4.0 `c203203` | merged-unreleased | in progress (with custom sources; `docs/design/deep-links.md`) | S | - |
| DMM's own built-in mod source | 2.4.0 `8774679` | merged-unreleased | excluded: needs DMM's backend (owner left it out in `upstream-gaps-2026-11-21.md`) | - | - |
| Category trees across Mods, Sounds, Maps and WiPs, third level, breadcrumbs, a hero pick that follows the section | 2.4.0 `cd622cb` | merged-unreleased | has: third level read after the first two (`gamebanana::deepen_categories`), styles listed under the picked hero, a breadcrumb in the chip row, a pick matches its subtree by leaf id, the hero pick follows Mods, Sounds, WiPs and All (`desktop/src/screens/browse/tree.rs`) | - | - |
| Drag to scroll the landing rails, draggable scrollbars | 2.4.0 `2d8ba44` | merged-unreleased | has: the Browse by hero rail drags and has a grabbable bar (`desktop/src/screens/browse/rail.rs`); the other landing rails are fixed grids | - | - |
| Obsolete mods: a status row and the author's note | 2.4.0 `304c1dd` | merged-unreleased | in progress (`browse/details.rs` reads `obsolete_note`) | S | - |
| Withheld mods: staff's reason on the page, left out of the store and Featured, no error toasts | 2.4.0 `cba597f`, `208df74`, `6bdb297` | merged-unreleased | in progress (obsolete and withheld notes) | S | - |
| YouTube and Vimeo videos on mod pages | 2.4.0 `9fab2e1` | merged-unreleased | has: a Videos list that opens the browser (`feature-parity.md`, Mod details); an embedded player is a web view, ruled out by ADR 0006 | - | - |
| Map console names read inside inline code in descriptions | branch `bugfix/map-command-inline-code-context` | branch WIP | has: `<code>` and `<pre>` mark a command, prose does not (`deadlock-core/src/servers/map_name.rs`) | - | - |
| Custom Maps tab shows the right map names | changeset `steady-maps-resolve` | merged-unreleased | has: name from the addon VPK's `maps/<name>.vpk` (`deadlock-core/src/maps.rs`) | - | - |

## Installed

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Add several VPKs as one custom mod whose parts switch in Manage files | changeset `amber-parts-gather` | merged-unreleased | in progress (queued: add mods dialog) | M | - |
| Add mods dialog that detects category, cover and map name from the files | changeset `vivid-covers-render` | merged-unreleased | in progress (queued) | M | - |
| Local mods get a rendered 3D preview as their cover in the library and details | 2.4.0 `e5d22b1` | merged-unreleased | missing: covers are GameBanana's picture or one the player picks (`installed` picture editor) | M | Capture one viewer frame per local mod on a worker and store it through `mod_extras` as the picture |
| Unticking every variant in Manage files turns the mod off | 2.4.0 `08ccbaa` | merged-unreleased | has: one switch per file in the variant picker | - | - |
| HUD mods checked against the game's current layouts, with related `console.log` errors | 2.4.0 `f173624` | merged-unreleased | has: `deadlock-core/src/addon_checks.rs` HUD check and `console_layout_lines` | - | - |
| Compatibility reports with copyable diagnostics | 2.4.0 `f173624` | merged-unreleased | has: the Compatibility review's Copy details on each finding and on the whole review (`installed/repair.rs`) | S | - |
| Fixes stay fixed: outdated repairs rebuilt automatically, rebuilt after Deadlock closes if a mod changed while it ran, health rechecked right after a mod updates | 2.4.0 `cae8f28`, `0c411a0`, `6ab16d4`, `89b9669` | merged-unreleased | has: a changed archive or an older repair rule is rebuilt from its file or backup, waits for the game to close, and the open health review is rechecked after an update (`installed/repair.rs`, `compat_repair::REPAIR_RULE`) | S | - |
| Repairs merge game text only from a complete base table; damaged base text stops with a Steam verify hint | PR #1397, #1400 (`kv-parser` localization documents), #1398 | PR open | has: `game_text_problem` reads the game's copy strictly and refuses a damaged table with the file name and the Steam verify hint (`deadlock-core/src/compat_repair.rs`) | M | - |
| Camera repair keeps attachments that already match the current model | PR #1399 | PR open | has: `compat_cameras::keep_current` puts back the author's values on attachments already in the current frame, repair rule 2 (`deadlock-core/src/compat_cameras.rs`) | S | - |
| A full compatibility recheck analyses two mods at once | PR #1386, issue #1385 | PR open | has: the animation pass, then two read-only workers (`installed/repair.rs` `CHECK_WORKERS`) | S | - |
| A saved compatibility review is reset when the mod's archive changes; every archive is stamped | changeset `quiet-owls-review`, 2.4.0 `fdd81c8` | merged-unreleased | has: every review finding and repair record carries the archive's SHA-256 and size, and a changed archive drops both (`installed/repair.rs`) | S | - |
| Analyze asks before re-adding leftover files, offers to remove ones it cannot place, shows files already added, never records files the game ignores as enabled | 2.4.0 `014a0bb`, `42b7203`, `fe2152d`, `ad912c0` | merged-unreleased | has: Analyze's Remove N leftover files asks first and moves them to the trash with Undo (`installed/checks.rs`, `trash::trash_loose`) | S | - |
| A removal names the files that blocked it, and scans never undo the mod order | 2.4.0 `de7727d` | merged-unreleased | has: `trash::blocked` names each file's path and OS error, with the close-the-game hint (`deadlock-core/src/trash.rs`) | S | - |
| Reinstall stops when old files cannot be removed | changeset `stern-gates-hold`, 2.4.0 `fdd81c8` | merged-unreleased | has: Delete and reinstall puts the old files back on any failure (`feature-parity.md`, Reinstall) | - | - |
| An install stops when copying its selected files fails | changeset `tidy-crates-wait` | merged-unreleased | has: `deadlock-core/src/install.rs` stops at the first failure | - | - |
| Unsafe file paths skipped when building compatibility fixes | changeset `quiet-foxes-guard` | merged-unreleased | has: `vpk::is_safe_vpk_path` (`deadlock-core/src/vpk.rs` line 227) | - | - |
| A fix choice and the load-order place survive a reinstall | 2.4.0 `40873f9` | merged-unreleased | has: Reinstall writes the same slot (`feature-parity.md`, Reinstall); repairs rekey with the file (`mod_extras::Extras::rekey`) | - | - |
| Reordering renames only the slots that move | 2.4.0 `29db4ca` | merged-unreleased | has: `deadlock-core/src/load_order.rs` plans renames for moving slots only | - | - |
| Mod compression: pack every enabled mod into 2 to 8 shared VPKs | PR #406 (closed), `feature/mod-compression` | branch WIP (PR closed) | has: Installed > Pack all enabled, 2 to 8 packs of neighbouring slots, each switched on, sources kept off, Unmerge per pack (`mod_merge::pack_all`, `installed/pack_all.rs`, scene `profiles-pack-all`) | - | A "Pack all enabled into N" option over the same merge, with unmerge |
| Mod identity stamped inside each VPK, a ledger reconciled against disk | `feat/vpk-fingerprinting`, `feature/reliable-filemanagement` | branch WIP | has: DMM's `.dmm/fingerprint.dmm` stamp is read on a DMM import (renamed and unclaimed files keep their mod) and on a picked file (`dmm_stamp.rs`, `import::read_dmm`); Sigil writes no stamp, its sidecar model stays | - | Read DMM's stamp when importing from DMM; keep Sigil's sidecar model |
| One skin per hero: a question when an install adds a second skin for the same hero | PR #524 (closed), `feature/duplicate-mod-flag` | branch WIP (PR closed) | has: after an install lands a second skin for a hero, a notice asks and Keep only the new one turns the others off; not asked with several skins per hero on (`desktop/src/screens/one_skin.rs`, scene `profiles-one-skin`) | - | Ask in the install review when the hero already has an enabled skin |
| Favourites per profile, and a random favourite turned on before a modded launch | PR #296 (closed), `feature/293/favourite-mods` | branch WIP (PR closed) | has: each profile keeps its saved mods; applying it brings them back, Undo restores the earlier set (`profile_favorites.rs`); a list's Random switch covers the random pick | - | Store the favourite set in the profile |
| Read-only `gameinfo.gi` and addons explained with a fix | `feat/read-only-error-enhancement` | branch WIP | has: Make writable (`installed/checks.rs`, `game_apply::make_writable`) | - | - |

## Profiles

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Profiles, profile sharing and conflict detection become permanent (no experimental switch) | changeset `brisk-ravens-settle` | merged-unreleased | has: always on (`feature-parity.md`, Profiles and Overlap) | - | - |
| Profile folders that point outside the addons folder are refused | changeset `keen-ledgers-guard` | merged-unreleased | has: isolated folders are built as `citadel/addons/sigil-<slug>` (`deadlock-core/src/profiles/isolation.rs`) | - | - |
| The Default profile's mods never load under another profile | changeset `stray-lanterns-fold` | merged-unreleased | has: an isolated profile moves the plain addons line into its own block (`profiles/isolation.rs` line 328) | - | - |
| An imported or restored profile starts downloading before every mod is looked up | 2.4.0 `aa6878e` | merged-unreleased | has: lookups stream in one by one (`profiles::resolve_import_each`); Import can be pressed while they run and each kept row queues as its lookup returns (`desktop/src/screens/profiles.rs`, scene `profiles-import-early`) | - | - |
| An imported profile keeps its file picks when shared again; restored profiles keep variants | 2.4.0 `fdd81c8`, `6b0dfba` | merged-unreleased | has: import with variant resolution (`feature-parity.md`, Profiles) | - | - |
| Mods an import or backup restore could not install are listed | 2.4.0 `fdd81c8` | merged-unreleased | has: `RowStatus::Failed` and the skipped list (`desktop/src/screens/profiles.rs`) | - | - |
| A backup restore that replaces a profile removes the profile's other mods | 2.4.0 `5180b3f` | merged-unreleased | has: Backups Restore with replace or merge (`settings-parity.md` D row 178) | - | - |
| Game settings backed up before a restore or reset | 2.4.0 `4b641d9` | merged-unreleased | has: `deadlock-core/src/backups.rs` backs up `gameinfo.gi` and `autoexec.cfg` before every write | - | - |

## Locker and skins

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Hero Skins redesign: banners, sound rows, shuffle pool mode, a hero with no skins shows popular ones | 2.4.0 `1733766`, `933a8bc` | merged-unreleased | partial: hero page, sounds, shuffle pools and the 3D switch exist (`feature-parity.md`, Locker); no suggestions for an empty hero | S | Show the most downloaded skins for that hero from the saved catalog |
| Texture-only hero skins recognised and previewed on the full character | 2.4.0 `e5d22b1` | merged-unreleased | in progress (3D viewer extras; `docs/design/3d-viewer.md`) | S | - |
| Partial mods previewed and compared with the original model | 2.4.0 `e5d22b1` | merged-unreleased | in progress (With mods, Without mods and Split exist) | S | - |
| Animated materials, particle effects and heavy cloth rigs in 3D previews | 2.4.0 `34fd9ca`, `274cc1e` | merged-unreleased | in progress (3D viewer extras) | M | - |
| Sound-only hero mods listed under Hero Sounds instead of skins | 2.4.0 `66424d2` | merged-unreleased | has: Weapons and voice and Sounds sections (`locker/parts.rs`, `feature-parity.md` Locker) | - | - |
| Randomising hero skins runs out of memory | issue #1311 | issue only | has: the shuffle is a seeded pick over file lists, no images loaded (`locker_parts::shuffle_wanted`) | - | - |
| Remove hero sections such as hats or capes | 2.4.0 `6cf08ef` | merged-unreleased | excluded: Mod Foundry heavy edits (on hold) | - | - |
| Foundry: per-section painting, retry and revert fixes for painted textures | PR #1056 (closed), changesets `calm-anvils-mend`, `dusky-forges-wake`, `vivid-lanterns-glow` | merged-unreleased / branch WIP | excluded: Mod Foundry heavy edits (on hold) | - | - |
| Prompt to turn on Forge installs from the Foundry | `feat/improve-mod-foundry-01` | branch WIP | has: DeadlockForge installs switch (`desktop/src/screens/settings/forge.rs`) | - | - |

## Stats and match

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Party tab that rates every lineup a party can queue | 2.4.0 `3e8248a` | merged-unreleased | has (`feature-parity.md`, Stats, Party tab) | - | - |
| Filter stats by ranked season, plus an end date | PR #1391, issue #1383 | PR open | partial: `deadlock-core/src/stats/filters.rs` has ranked and a start time only | S | Read deadlock-api's ranked seasons list, add a season picker and an until time (`max_unix_timestamp`) |
| Match details show abandoned matches and the teammates who left | changeset `quiet-hawks-wave` | merged-unreleased | partial: `stats.rs` reads `abandoned_time_s`; no screen shows it | S | A leaver mark on the scoreboard and an Abandoned tag in the list |
| Non-scored matches are not losses; empty hero win rates hidden | changeset `calm-owls-rest`, PR branch `bugfix/1371-non-scored-matches` | merged-unreleased | has: `stats.rs` `NotScored` | - | - |
| Item stats show an error with Retry when they fail | changeset `lofty-meadows-count` | merged-unreleased | missing: no retry in `desktop/src/screens/stats/more.rs` | S | Error box with Try again on the Items and Item meta views |
| The Live tab keeps the last match until the next search | 2.4.0 `03c0d62` | merged-unreleased | has (`feature-parity.md`, Live match view) | - | - |
| Live stats not updating on Linux | issue #1320 | issue only | n/a: Sigil ships no Linux build yet (`docs/runbooks/packaging.md`) | - | - |
| Match sync: per-account switch, batching, skip stored matches, quota, friend history, ConnectCache fix | 2.4.0 `b8f0b24`, `d7cfaf5`, `81a7862`; branches `feat/match-sync-batch-and-reports`, `feature/match-sync-friend-history`, `feature/match-sync-skip-stored-own-matches`; PR #1413 | merged-unreleased / branch WIP / PR open | excluded: Steam-login match sync | - | - |

## Performance and config

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Performance configs written in full, `video.txt` included, with a backup | 2.4.0 `0544a01` | merged-unreleased | has: Performance writes through `apply_to_install` with a backup (`feature-parity.md`, Performance) | - | - |
| `video.txt` settings the game changed are shown; keep a config's `video.txt` | 2.4.0 `e6dd365`, branch `feature/perf-video-tracking` | merged-unreleased | has: Write adopts, Keep forgets, Restore (`feature-parity.md`, Performance) | - | - |
| Malformed game config files are refused instead of rewritten | 2.4.0 `4b641d9` | merged-unreleased | has: every `gameinfo.gi` or `video.txt` write is parsed as KeyValues first and refused with the file and line when it would not load (`game_apply::refuse_malformed_config`, called from `util::write_atomic`); `autoexec.cfg` refuses a command with an unclosed quote (`autoexec::autoexec_syntax_error`) | - | - |
| Review configs bundled in mod packs before they are written | 2.4.0 `8774679` | merged-unreleased | in progress (install review step for gameinfo and cfg) | M | - |
| Game presets page: named option sets with a picker, an editor and an apply preview | `feature/game-presets` (2025, "ui wip") | branch WIP | has: Autoexec command library by category and crosshair presets (`feature-parity.md`, Autoexec, Crosshair) | - | - |
| Performance catalog data refresh | PR #1416 | PR open | has: versions read from GitHub (`feature-parity.md`, Performance) | - | - |

## Servers and maps

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Standalone server picker for Valve relay blocks | 2.4.0 `9068f5f` | merged-unreleased | has, not yet run on Windows (ADR 0016, Servers > Regions) | - | - |
| "Where I play" preset and recent match counts in the picker | 2.4.0 `5d38b7c`, branch `feature/server-picker-match-history` | merged-unreleased | in progress (region picker extras) | S | - |
| Turn off the map mod launch warning from the dialog or Settings | 2.4.0 `0bf0688`, issue #1313 | merged-unreleased | has: Don't ask again and the Settings > Mods switch (`launch_extras.rs`) | - | - |

## Launch and game health

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Toggle fullscreen with Alt+Enter or F11 | issue #1414 | issue only | has: both chords take the window borderless full screen and back through Rune's window API (`shell::is_fullscreen_key`, `Tx::toggle_full_screen`), listed in the shortcuts sheet | - | - |
| A Deadlock process stuck while closing is not treated as running | 2.4.0 `90756e3` | merged-unreleased | has: a Windows process whose exit code is set while it is still listed is left out of every running check (`launch::exiting`, `read_game_process`), and Stop game that cannot close the game says so | - | - |
| An error when cancelling a launch fails, with the progress kept visible | changeset `brief-comets-turn` | merged-unreleased | has: Cancel keeps the overlay up with a Closing Deadlock line; a failed stop comes back with the error, Close and Try again (`desktop/src/screens/launch_progress.rs`) | - | - |
| Custom game search paths kept when launching or disabling mods | 2.4.0 `4b641d9`, branch `bugfix/1257-preserve-search-paths` | merged-unreleased | has: `game_apply::apply_gameinfo_repair` keeps other mounts | - | - |
| Steam found on Windows when its registry path is stale | 2.4.0 `ee0bf0e`, branch `bugfix/windows-steam-path-fallbacks` | merged-unreleased | has: registry, then the usual drives (`deadlock-core/src/discovery.rs` line 173) | - | - |
| Linux runtime fixes: NixOS launch, Flatpak running-game detection, Flatpak audio, host process scan without a fork per process | 2.4.0 `9a070af`, `e02703c`, `ac42377`; PR #1349 | merged-unreleased / PR open | partial: process-table detection exists; no Linux build is shipped or tested (`docs/runbooks/packaging.md`) | S | Verify once a Linux build exists |

## Downloads and installs

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Missing variant downloads streamed with size limits, checksums and server fallback | changeset `sturdy-rivers-flow` | merged-unreleased | has: `install::MAX_DOWNLOAD_BYTES`, the queue's `max_bytes` and MD5 check (`deadlock-core/src/download_queue.rs` line 1549), failover | - | - |
| Pause and resume downloads | `stormix/dmm-37-ability-to-pause-and-resume-downloads` | branch WIP | in progress (resumable downloads; pause and resume exist in `download_queue.rs`) | S | - |
| Download routing used for updates, profile imports and file changes; Default routing falls back to other servers and starts without waiting for server checks | 2.4.0 `7c5922a`, `fdd81c8`, `dc570da` | merged-unreleased | has: every queue user goes through `download_queue.rs` failover (`feature-parity.md`, Downloads) | - | - |
| Remember the chosen file for a 1-click install; restore variant choice from the 3D tab | 2.4.0 `b692e28`, `9c762f0` | merged-unreleased | in progress (one-click file choice) | S | - |
| Zip and 7z imports read the file list instead of unpacking | 2.4.0 `abe0090` | merged-unreleased | has: `list_7z` reads the header with sevenz-rust2, with an unpack fallback for a damaged or encrypted header (`download_queue/scan.rs`) | S | - |
| GameBanana catalog and mod data downloaded compressed | 2.4.0 `4061643` | merged-unreleased | has: `ureq` 3 sends gzip by default (`rust/Cargo.toml`) | - | - |
| Updates found for single-file mods whose file name changes | 2.4.0 `7a7b75b` | merged-unreleased | has: "Match remaining update files" pairs a leftover slot with a leftover upload (`installed/update_match.rs`) | - | - |

## Shell and settings

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| A Features tab to turn extras off, or all at once with a minimal mode | changeset `quiet-lanterns-fold` | merged-unreleased | has: General > Pages and Mods only (`settings-parity.md` D row 71) | - | - |
| Plugins toggle in Optional Features | changeset `hollow-pines-wake` | merged-unreleased | excluded: external plugin system | - | - |
| Contextual tips for new features | 2.4.0 `2a5c1fc` | merged-unreleased | has: `shell_tips.rs` (`feature-parity.md`, Feature hints) | - | - |
| Nightly and rollback explained; older nightlies not offered | 2.4.0 `75e4fe8`, issue #1362 | merged-unreleased | has: Release channel and Going back (`settings-parity.md` G row 94) | - | - |
| Discord Rich Presence: hide party size | 2.4.0 `97c0754` | merged-unreleased | in progress | S | - |
| New themes (Le Deadlock, Gridline, Lovelock, seasonal art) | 2.4.0 `02a1482`; branches `feature/theme-gridline`, `feature/lovelock-theme`, `feature/seasonal-themes` | merged-unreleased / released | excluded: DMM theme names and palettes | - | - |
| Load only the language in use; translation updates | 2.4.0 `65c32aa`, `l10n_crowdin_*` | merged-unreleased | excluded: translation on hold | - | - |
| Linux freezes from backdrop blur removed | changeset `quiet-lanterns-drift` | merged-unreleased | n/a: native renderer, no web view blur (`settings-parity.md` D row 214) | - | - |
| Pointer cursor over menu items and sliders; disabled sliders dimmed | 2.4.0 `3f42916` | merged-unreleased | has: actions set `rune::Cursor::Pointer` (for example `desktop/src/screens/projects.rs`) | - | - |
| Usage analytics with actionable events | `feature/actionable-desktop-analytics` | branch WIP (merged earlier in part) | excluded: Google Analytics; Sigil's own opt-in telemetry is ADR 0014 | - | - |

## Creator and workbench

All rows but the last come from `feature/modding-workbench` (no PR, 10 commits, 721 files, about
77 500 lines, 74 changesets on the branch, last commit 2026-11-09): an addon workbench built on
CSDK 13. Sigil's creator side is the agent workbench with the Game index, Projects and Toolchain
screens (`docs/architecture.md`, Tool catalog).

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Browse any VPK's files with previews and export them, original or editable, into an addon folder | `feature/modding-workbench` | branch WIP | has: Game index Files tab (`desktop/src/screens/explorer.rs`, `deadlock-core/src/explorer/`): any game VPK, addon or project folder as a tree, previews, Export original into `staging/` or editable text into `sources/`; agent tools `files_list` and `files_export` | M | A tree view per VPK (game or installed mod) with the Locker's previewers and Extract to a project |
| Content search inside archive files with matching-line previews, pause and progress | same | branch WIP | has: Search contents on a worker with live hits, Pause and Stop, capped (`explorer/search.rs`); agent tool `files_search` | M | Search decoded text resources on a worker, capped, with a Stop |
| Source 2 data and text previews (KV3, vdata, text DMX) in addons, archives and comparisons | same | branch WIP | has: KV3 data, Panorama and text DMX summaries in the preview pane (`explorer/preview.rs`, `explorer/dmx.rs`); agent tool `files_show` | M | A read-only text pane for decoded resources beside the tree |
| Comparison workspace: drop two sources side by side, detachable window | same | branch WIP | partial: Compare mode with a tree diff and a line diff of one file (`explorer/compare.rs`); no detachable window | M | Two panes over the same previewers; the viewer's Split covers models only |
| Tool setup with download progress, the current hotfix, installed CSDK without Dota downloads or patches | same | branch WIP | partial: `screens/toolchain.rs` checks and runs CSDK on Windows (ADR 0003); the compiler line names CSDK 12 or 13 from the install (`toolchain::csdk_generation`); never downloaded or patched | M | Detect CSDK 13 too; never download or patch it, only point at the player's install |
| Pack and Test: close Deadlock, pack, install into a slot and open the addon folder | same | branch WIP | has: Projects, Pack and Test (`deadlock-core/src/project_test.rs`, `screens/project_tools.rs`): validates, packs a throwaway build, shows the plan, installs after a confirm; refuses while Deadlock runs; not run against a real install yet | S | One app action that runs pack, the deploy plan and the gated install in a row |
| Compile selected files, batch builds, skip unchanged assets with an explicit Rebuild, actionable build errors | same | branch WIP | partial: Projects, Compile changed and Rebuild all (`project_build::run_build`, hash manifest in `.dah/build.json`), per-file errors; no file selection; not verified with a real CSDK | M | Hash inputs per asset and recompile only changed ones |
| Reversible Scan and Repair for source assets | same | branch WIP | has: Projects, Scan and Repair with Undo (`project_build`, undo copy in `.dah/repair-undo`) | M | Rules per asset kind, a preview of changes and an undo copy in the project |
| Bulk rename assets after showing addon and game references | same | branch WIP | has: Projects, Bulk rename lists project and game references before moving (`project_build::{plan_rename, apply_rename}`); references are shown, not rewritten | M | Reference scan through the game index before a rename plan |
| Flag misplaced Panorama images; a code editor in Inspect | same | branch WIP | has: `misplaced_panorama_image` payload warning (core, host and TypeScript validators); a plain text editor for a project's own files in the Files tab | S | Payload rule for images outside `panorama/images`, plus a plain text editor pane |
| Image compiler lists kept in sync when images are added, moved or removed | same | branch WIP | partial: `project_build::sync_image_list` regenerates the list on every build and never ships it; the list format is not verified with a real CSDK | S | Regenerate the compile list from the project tree on each change |
| Source model previews (VMDL with DMX meshes, switchable parts, skeleton layers) | same | branch WIP | partial: the viewer renders compiled models (`docs/design/3d-viewer.md`) | L | Read text DMX in `sigil-viewer` |
| Model painting, image and GIF stickers, particle recolor, material and vertex editing | same | branch WIP | excluded: Mod Foundry heavy edits (on hold) | - | - |
| A mod package format with a packaging wizard (variants, layers, dependencies, authors) and author metadata in `dmm.json` | `feature/dmodpkg` (2025), `stormix/dmm-32-allow-mod-authors-to-include-metadata-that-influence` | branch WIP | partial: `project_release` and `publish_prepare` write a release manifest; `deadlock-core/src/import.rs` reads DMM's `.dmm.json` | M | Read an author's variant and dependency metadata on import; a format of our own stays a decision |

## Other

| Feature | Source | Maturity | Sigil | Effort | Build note |
| --- | --- | --- | --- | --- | --- |
| Linux AppImages next to Flatpaks; Flatpak on Flathub; Flatpak start-up crash | PR #923, issue #477, issue #1418 | PR open / issue only | missing: Sigil packages Windows and macOS zips only (`docs/runbooks/packaging.md`) | M | A Linux build and AppImage in the release script, verified on one distro |
| Accounts: author verification, Discord sign-in, friends, dev sign-in, Better Auth update | `feature/author-verification`, `feat/discord-sign-in`, PRs #151, #175, #180 (closed), PR #1410 | branch WIP / PR open | excluded: Sigil accounts package | - | - |
| Foundry exports and found files kept out of cloud backups | 2.4.0 `6f49788` | merged-unreleased | excluded: cloud backup is Sigil accounts | - | - |
| DMM's own defects and speedups with no new player behaviour (about 45 patch entries: Analyze speed, state saves, profile switch races, extraction hangs, sidebar clipping, crosshair gap preview, interrupted first save, cache watcher, gameinfo tooltip languages, installer size) | 2.4.0 patch section, 11 changesets | merged-unreleased | n/a: DMM's code paths; nothing to port | - | - |

## Counts

Rows in the area tables above (the last "Other" row is a bundle and is counted once).

| Sigil status | Rows |
| --- | ---: |
| has | 60 |
| partial | 10 |
| missing | 10 |
| in progress | 14 |
| excluded | 11 |
| n/a | 3 |
| **Total** | **108** |

| Maturity | Rows |
| --- | ---: |
| merged-unreleased (2.4.0 or a pending changeset) | 70 |
| PR open | 5 |
| branch WIP (no PR, or PR closed) | 24 |
| issue only | 3 |
| mixed (one row, several sources of different maturity) | 6 |
| **Total** | **108** |

Partial and missing rows, by effort: 8 S, 11 M, 1 L.

## Build list

Only missing and partial rows. Grouped so builders in the same round touch different code; each
round's areas are listed with the files they own.

### Round 1 (parallel, 4 lanes, 19 rows)

- **Browse lane** (`desktop/src/screens/browse/`, `deadlock-core/src/gamebanana.rs`,
  `servers_maps.rs`), 3 rows: category trees with breadcrumbs and a hero pick that follows the
  section (M), drag-scroll rails (S), map names inside inline code (S).
- **Stats lane** (`deadlock-core/src/stats/`, `desktop/src/screens/stats/`), 3 rows: ranked season
  and end date filter (S), abandoned matches and leavers (S), item stats Retry (S).
- **Launch and shell lane** (`deadlock-core/src/launch.rs`, `game_apply.rs`,
  `desktop/src/screens/launch_progress.rs`, `shell*.rs`), 4 rows: Alt+Enter and F11 fullscreen (S),
  stuck-closing process (S), cancel-launch failure (S), refuse malformed config files (S).
- **Installed health lane** (`desktop/src/screens/installed/{checks,repair}.rs`,
  `deadlock-core/src/{compat_repair,addon_checks,trash}.rs`), 9 rows: copyable diagnostics (S),
  rebuild repairs on game exit and after an update (S), two-worker recheck (S), review choices keyed
  on the file hash (S), remove leftovers from Analyze (S), name the blocking file on removal (S),
  camera attachment rule (S), strict game text baseline (M), 7z header listing (S, `download_queue.rs`
  is touched by no other lane this round).

### Round 2 (parallel, 3 lanes, 9 rows)

- **Locker and viewer lane** (`desktop/src/screens/locker/`, `desktop/src/viewer/`,
  `mod_extras`), 2 rows: rendered 3D covers for local mods (M), popular skins for an empty hero (S).
  Waits for the 3D viewer extras in progress.
- **Profiles and library lane** (`deadlock-core/src/profiles.rs`, `screens/profiles.rs`,
  `mod_merge.rs`, `import.rs`), 5 rows: early downloads during a profile import (S), per-profile
  favourites (S), Pack all enabled into N (M), read DMM's VPK stamp on import (S), one-skin-per-hero
  question in the install review (S, after the install review step in progress lands).
- **Distribution lane** (`scripts/`, `docs/runbooks/packaging.md`), 2 rows: a Linux build with an
  AppImage (M), then the Linux runtime checks (S).

### Round 3 (creator, one lane or two that split by file, 13 rows)

`desktop/src/screens/{index,projects,toolchain,workbench}.rs`, `deadlock-core/src/{project,
toolchain,game_index}.rs`, `sigil-viewer`. Order inside the round: VPK tree browser with export
(M), text and data previews (M), content search (M), comparison panes (M), Pack and Test as one
gated app action (S), misplaced image rule and text editor pane (S), image compile list sync (S),
incremental compile with Rebuild (M), Scan and Repair (M), bulk rename with references (M), CSDK 13
detection (M), author metadata on import (M), text DMX previews (L). A second builder can take the
viewer row and the toolchain rows while the first owns the browser and previews.

Rounds total: 41 rows (26 S, 14 M, 1 L) across 10 lanes.
