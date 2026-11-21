# Upstream gaps: the newest Grimoire and Deadlock Mod Manager against Sigil

Status: research snapshot, 2026-11-21; status column rechecked against `rust/crates` after 0.1.22
(2026-11-21). Fresh upstream clones are in `.scratch/upstream/grimoire` and `.scratch/upstream/dmm`
(gitignored). Grimoire (G) is MIT, so its code and copy may be ported. Deadlock Mod Manager (D) is
GPL-3.0: behaviour only, never its code or text. A row marked "has, real game unverified" is built
and tested on fixtures or headless scenes but has never run on a real Deadlock install.

## What was compared

| | Grimoire | Deadlock Mod Manager |
| --- | --- | --- |
| Our vendored copy | `vendor/grimoire` 8941439 (2026-11-16) | `vendor/dmm` 41afe0e (2026-11-15) |
| Upstream `main` compared | fd4f897b (2026-11-19) | 9f09270d (2026-11-19) |
| Latest stable release | v1.35.1 (2026-11-08) | v2.3.0 (2026-11-01) |
| Latest nightly | 1.35.2-nightly.20261120031907.158.gfd4f897 (2026-11-20) | 2.4.0-nightly.20261110.c4e54149 (2026-11-10) |
| Commits on `main` since our vendored copy | 2 | 2 |
| Commits on `main` since the latest stable | 16 | 282 |
| Unreleased notes read | GitHub PRs #571 to #592 | `apps/desktop/CHANGELOG.md` section 2.4.0 (not yet released) and 27 pending `.changeset/*.md` |
| Unmerged branches read | `feat/deadlock-benchmarks-wip`, `feat/custom-performance-presets`, `fix/review-followups-2026-11` | `feature/modding-workbench`, `feature/custom-mod-parts`, `feature/creator-workspace-*`, `feature/vpk-load-order`, `feature/profile-mod-requirements` and others |

The vendored copies are only four commits behind `main`. Both apps' newest *releases* are older
than our vendored commits, so almost everything "new" was already in `vendor/` and Sigil had
ported most of it (the deadlock-api rank rework, leaderboard seasons, crosshair game codes, the
light theme, accent swatches, the multi-file card click, custom mod sources, the party planner,
random list members, contextual tips, Mods only). Most of what this snapshot listed as missing
or partial shipped in 0.1.22. The rows below carry the current status and the file that proves it;
the items in GitHub issues and the unbuilt list in `docs/design/grimoire-unreleased-2026-11-21.md`
stay open.

Left out on purpose (owner decisions, not raised again): DMM's plugins
(`feature/creator-workspace-bridge`), the Le Deadlock and Coding themes, Steam-login match sync
(`b8f0b246` per-account switch, `81a7862a` batching), the Sigil accounts package, translations
(`8a00ccdf` language table, Crowdin), Mod Foundry heavy edits (D `6cf08eff` remove hero sections,
D `feature/modding-workbench` with CSDK 13 and GIF stickers, G's Door Stuck Parry editor in #571),
Divine Skins, DMM's own hosted source (`8774679`, its backend). Already being built, so not listed
as new: D's "Where I play" preset and recent match counts in the server picker (`5d38b7cd`), the
review step for configs bundled in packs (`8774679`), D's viewer material and particle fidelity
(`34fd9caf`), and the 3D extras on the leftovers list.

## The top 10 and where they stand

Ordered by value to players; status checked after 0.1.22.

1. **Mod safety review list with Disallow** (G #592, M). Built: `installed/safety_review.rs` lists
   every flagged mod as Review or Allowed, shows "N mods need attention", runs Check installed
   mods and Allow N script mods, and Disallow turns the mod off before taking the trust back.
   Real game unverified.
2. **Read VPKs whose root folder is an empty name** (G #578, S). Built: `deadlock-core/src/vpk.rs`
   reads an empty first folder as the root and walks the tree a second time when needed
   (`empty_root`, `classic_walk_stays_within`, test `an_empty_root_folder_reads_as_the_root`).
3. **Installs and imports that land on pass the safety scan first** (G #591, S). Partial: Browse,
   source and DeadlockForge installs ask the landing question (`browse/quick.rs`,
   `settings/sources.rs`, `settings/forge.rs`; `installed/safety_gate.rs` decides), but a local
   import still always lands off (`screens/import.rs`).
4. **Custom mod sources inside Browse** (D, M). Partial, and owned by another builder: sources add
   from Settings and show in Browse's header, but D's release history details and deep links are
   not complete. See `design/feature-parity.md`.
5. **Add mods dialog that reads the files** (D `c60a5792`, M). Built:
   `deadlock-core/src/import/parts.rs` reads each loose VPK on a worker, detects category, hero,
   map name and cover, and imports several files as one mod whose parts switch in Manage files.
6. **Installed refuses changes while the game runs** (settings row G32, S). Built:
   `installed/enable.rs`, `notes.rs`, `pack_all.rs`, `safety_review.rs`, `tools.rs` and `checks.rs`
   all call `settings/watch::refuse_while_running`.
7. **GameBanana notices with their own words** (D `7ede98c1`, `cba597f2`, `208df742`, S). Built:
   `browse/details.rs` shows the author's obsolete note and the staff's rules, note, name and date
   on a withheld mod; withheld pages leave the saved catalog and Featured.
8. **Profiles keep the Locker's state** (G #587, S for shuffle, L for colors). Partial: profiles
   store the shuffle switches, the pool and the several-skins switch (`profiles.rs`
   `LockerShuffle`); ability colors wait for real recolor, which is still a brief (`recolor.rs`).
9. **Hidden mods and creators lists that say what they were** (G #580, S). Built:
   `browse/hidden.rs` shows blurred thumbnails, the date hidden, the mod a creator was hidden from
   and a 140 character note, in Browse and Settings > Mods.
10. **Discord: hide party size** (D `97c07548`, S). Built: `settings/templates.rs` drops the count
    from Sigil's own party lines and leaves a line the player wrote as written.

## Gaps by area

Columns: what it is for a player; where it lives upstream; Sigil's status with the file or doc
that shows it; effort (S under a day, M a few days, L a week or more); recommendation.

### New since our vendored commits

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Disallow: take back a mod safety approval; the mod is turned off first and returns to Review | G #592, `electron/main/services/modSafety.ts` (`revokeVpkSafety`), `modSafetyAudit.ts` | has, real game unverified: `desktop/src/screens/installed/safety_review.rs` (Disallow turns the mod off, then removes the trust; refuses while Deadlock runs) | - | Built |
| "Enable mods after download" also enables imports that passed the review | G #591, `electron/main/ipc/mods.ts` (`importCustomModSource`), `docs/mod-inspection.md` | partial: `data.rs`, `browse/quick.rs`, `settings/sources.rs` and `settings/forge.rs` ask `safety_gate` before landing an install on; `desktop/src/screens/import.rs` still always imports switched off | S | Build |
| Stock `gameinfo.gi` template refreshed from the depot (`sv_minrate` 98304 to 133120) | D #1417, `apps/api/src/static/artifacts/deadlock/gameinfo.gi` | not needed: Sigil repairs the player's own file in place (`deadlock-core/src/game_apply.rs`); 98304 appears only in test fixtures | - | Skip: data on D's backend. Refresh `deadlock-core/tests/fixtures/stock-gameinfo.gi` when a test next needs it |
| Translation progress table | D #1415 | on hold (owner) | - | Skip: translation is on hold |

### Mod safety and install

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Mod safety page: Review and Allowed rows, "N mods need attention", Check installed mods, Allow N script mods, "this mod changed since you reviewed it", a startup and launch check that turns off enabled mods nobody approved | G `src/components/ModSafety.tsx`, `src/locales/en/translation.json` `modSafety.*`, `docs/mod-inspection.md` | has, real game unverified: `installed/safety_review.rs` has the list, the notice, Check installed mods, Allow N script mods, "changed since you allowed it" and Disallow. Sigil never turns a mod off at startup without a press, unlike G | - | Built |
| Trust tied to the file's bytes, so any change asks again | G #578 and #592 (`approvalKey`, content hash) | partial: `installed/safety_gate.rs` keys a trust by the mod and its file size; the sha256 exists in `installed/imprint.rs` | S | Build: key the trust on that sha256 |
| Read a VPK whose root folder is stored as an empty name, then walk again so nothing hides behind it | G #578, `electron/main/services/modSafetyScan.ts` | has: `deadlock-core/src/vpk.rs` `empty_root` and `classic_walk_stays_within`, tests `an_empty_root_folder_reads_as_the_root` and `an_empty_root_that_hides_a_second_reading_is_refused` | - | Built |
| Text layouts (`vxml_c` with a DATA block and no LaCo) are read | G #578 | has: the scan searches layout bytes (`docs/design/mod-safety.md`, "Limits") | - | - |
| Enable anyway for a file the scan cannot read | G #578 | has: the gate lists "Not checked: the whole file" with Turn on this once (`installed/safety_gate.rs`) | - | - |
| Name a pack's members from their imprints | G #582, `src/lib/customModImport.ts` | has: each VPK is named from its own `addoninfo.txt` or `modinfo.json` first (`deadlock-core/src/import.rs`, line 538) | - | - |
| Add mods dialog: detects category, cover and map name from the files; several loose VPKs become one mod whose parts switch in Manage files | D `c60a5792`, changesets `vivid-covers-render`, `amber-parts-gather` | has (2026-11-21): a local import review reads each loose VPK on a worker (`deadlock-core/src/import/parts.rs`) and shows what each file holds, a category read from the files (the player can change it), the hero, the map's console name and a cover (a picture beside the VPK, the skin's own hero card, or the hero's render); two or more loose files get "Import as one mod" with a name, and after the copy they land as one variant group whose parts switch in Manage files (`screens/import/parts.rs`, scenes `gaps-add-mods` and `gaps-add-mods-separate`). Add files and Add a folder grow an open review | M | Built |

### Installed

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Refuse toggles and deletes while Deadlock runs | G (Disallow and the startup check rely on it), settings row G32 | has: `settings/watch.rs` `refuse_while_running` is called from Installed's enable, notes, pack all, safety review, tools and checks (`installed/enable.rs`, `notes.rs`, `pack_all.rs`, `safety_review.rs`, `tools.rs`, `checks.rs`) as well as Browse, Overlap, Crosshair and Forge | - | Built |
| Local mods get a rendered 3D preview as their cover | D `e5d22b18`, `apps/desktop/src/lib/mod-thumbnails.ts` | has, real game unverified: Preview in 3D has "Use this view as the mod's cover", which saves the frame as the mod's picture (`viewer/mod.rs` `capture_cover`); not rendered automatically in the background | M | Built (player-triggered) |
| A multi-file card opens the mod page; Choose files in the menu | G #576 | has: `installed/layout.rs` `open_group` with the Settings option | - | - |
| Manage files: unticking every file turns the mod off | D `08ccbaac` | has: one switch per file in the variant picker | - | - |

### Browse and mod details

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Obsolete mods show the author's note; withheld mods show GameBanana staff's reason; withheld mods leave Featured and the catalog | D `7ede98c1`, `cba597f2`, `208df742`, `6bdb2970` | has: `screens/browse/details.rs` `notices()` draws the author's note on an obsolete mod and the staff's rules, note, name and date on a withheld one; a withheld or private page drops the mod from the saved catalog and Featured | - | Built |
| Category trees with breadcrumbs and linked hero filters across Mods, Sounds, Maps and WiPs, third level included | D `cd622cb0`, `apps/desktop/src/components/mod-browsing/catalog-categories.ts`, `category-filter-breadcrumbs.tsx` | has: `browse/tree.rs` reads the third level after the first two, shows breadcrumb chips, matches a pick by its leaf id and follows the section with the hero pick; `browse/hero.rs` styles under the picked hero | - | Built |
| Drag to scroll the landing rails, draggable scrollbars | D `2d8ba447` | has: the Browse by hero rail drags and has a grabbable bar (`screens/browse/rail.rs`); the other landing rails are fixed grids | - | Built |
| Hidden mods and creators: thumbnails, date hidden, where from, a note | G #580, `src/components/settings/` hidden content rows | has: `screens/browse/hidden.rs` (blurred thumbnail, date, the mod a creator came from, a 140 character note; Browse's Hidden items and Settings > Mods) | - | Built |
| YouTube and Vimeo videos on mod pages | D `9fab2e15` | has as a Videos list that opens the browser (`feature-parity.md`, Mod details) | - | Keep: an embedded player needs a web view, which ADR 0006 rules out |

### Locker

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Profiles keep the Locker's ability colors and trippy skins; Grimoire's profiles already kept the shuffle settings | G #587, `electron/main/services/profileLocker.ts` | partial: `deadlock-core/src/profiles.rs` `LockerShuffle` stores the shuffle switches, the pool and the several-skins switch, applied and undone with the profile; ability colors wait for real recolor | S (shuffle), L (colors) | Shuffle built; colors wait for recolor |
| Real ability VFX recolor (one color, rainbow, gradient, trippy) baked into a managed mod | G `docs/ability-vfx-recolor.md`, `electron/main/services/heroColors.ts`, `ipc/trippyEffects.ts` | partial: `deadlock-core/src/recolor.rs` writes a brief only ("records the exact targets ... instead of pretending to edit a game file"); `feature-parity.md` marks Locker Recolor partial | L | Owner decision: is the Locker's particle recolor part of the held Foundry recolor? |
| Hero Skins page: banner with what the hero wears, equipped label, sound rows with play and a switch, page-wide shuffle and 3D switches, explicit pool mode, and a hero with no skins shows the store's most downloaded skins | D `17337665` | partial: hero page, load order, shuffle pools, sounds and the 3D switch exist (`feature-parity.md`, Locker); no suggestions for a hero with nothing installed | S | Open: build the empty-hero suggestions |

### 3D viewer

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| A preview appears only when whole: a loader with build progress covers it until the hero, the room and any staged heroes are ready, 20 s fallback | G #588, `src/components/locker/HeroPoseViewer.tsx`, `src/components/common/PreviewLoader.tsx` | has (2026-11-21): the loader covers the view until the hero, compare side and room are loaded and the first frame is drawn, with a 20 s fallback (`viewer/worker.rs`); Sigil has no staged extra heroes | S | Built |
| Motion previews: dash, heavy melee and parry mods play on a hero against a second hero; an animation mod plays its own clips with a clip strip and pause | G #571, `src/lib/movementGraphClips.ts`, `export-animation-mod-preview` IPC | partial: the clip picker has dash and heavy melee (`3d-viewer.md`, `clipInfo`); no second hero and no per-mod clip strip. Issue superuserkalo/sigil#8 | M | Open: issue #8 |
| Texture-only skins previewed on the full character; partial mods compared with the original | D `e5d22b18` | partial: With mods / Without mods / Split exist; the texture-only case has never run on a real install (`3d-viewer.md` status) | S | Verify on a Windows install before building anything |

### Stats

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Match details show abandoned matches and the teammates who left | D changeset `quiet-hawks-wave` | has: `stats/notes.rs` tags the player's own abandon in the list and the scoreboard (`abandon_tag`, `result_line`), and `stats/board.rs` notes who abandoned | - | Built |
| Street Brawl games tagged in the match list | G #575 | has (owner's call, 2026-11-21): `stats/filters.rs` `is_street_brawl` keeps the rows tagged, `stats/matches.rs` draws the tag, and every aggregate skips them | - | Built |
| deadlock-api rank rework, rank card details, ranked delta, leaderboard seasons, crosshair game codes | G #575 | has: `stats.rs` (`/v1/players/rank/distribution`, `initial_win_streak`, `ranked_delta`), `stats/leaderboard.rs`, `crosshair.rs` (`SHARE_CODE_URL`) | - | - |
| Non-scored matches are not losses | D `b6fbfd67` | has: `stats.rs` `NotScored`, result `None` when unscored | - | - |

### Integrations, sources and performance

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Custom mod sources browsed from the store header, release history, Markdown descriptions, the multi-file picker for a listing's files, deep links that add a source or install one of its mods | D `ef8af43b`, `d9ba608c`, `becf7cda`, `cfa9554d`, `b79a7485`, `c203203b`; `apps/docs/.../mod-sources.mdx` | partial: `desktop/src/screens/settings/sources.rs` adds a source, lists it in a dialog and installs "a listing's first file" (line 431) | M | Build (top 4) |
| Discord Rich Presence: hide party size | D `97c07548` | has: `settings/templates.rs` Hide party size drops the count from Sigil's own party lines and leaves a line the player wrote as written (`settings/options.rs`) | - | Built |
| Detect the running game on Flatpak Steam | D `e02703ce` | unverified: Sigil reads the process table (`deadlock-core/src/launch.rs`); Flatpak is handled for the Discord socket only. The Linux AppImage and `.deb` are built (`docs/runbooks/packaging.md`) but never run on a Flatpak Steam | S | Verify on a Flatpak Steam; issue superuserkalo/sigil#16 |
| Built-in benchmark: launch a scripted run, capture frames, show average FPS, 1% low, p99 frame time, hardware and settings | G branch `feat/deadlock-benchmarks-wip` (unmerged, experimental) | has, real game unverified: `deadlock-core/src/benchmark/`, `screens/performance/benchmark/`, `docs/design/benchmark.md` (replay scene, frame capture from the game's logs, metrics, comparisons, history, exports) | - | Built |
| Import and manage custom performance presets | G branch `feat/custom-performance-presets` (unmerged) | has: Performance import and export of a config, plus presets from a public GitHub repository with a snapshot, hide built-ins and delete (`performance/repos.rs`, `screens/performance/custom.rs`); per-setting switches are stored but not offered in the UI | - | Recheck if Grimoire merges a different model |

## Where Sigil is ahead of both

- An AI agent workbench (BYOK, TinyAgent) that builds, packs and validates mods; neither app has one.
- Hosting a Deadworks server from this PC, Steam relay mode with a join code, opt-in UPnP, invites
  that carry GameBanana ids (ADR 0015).
- Undo for a profile switch, server favorites, recent servers and direct connect.
- A mod safety scan that reads what Panorama actually runs, so skins and HUD mods do not ask for
  nothing (Grimoire flags every HUD mod as "Runs scripts"; DMM does not scan).
- Requirements checked both ways: DMM's missing-on-enable and Grimoire's load-order fix in one place.
- A trash with Undo behind deletes, updates and reorders, and the install journal behind them.
- One native executable with no Electron or CEF, and signed self-update (ADR 0010, ADR 0012).
- The live bridge HUD mod built from the player's own `pak01` instead of a shipped copy.
- Experimental tour; opt-in AI match review (`deadlock-core/src/review.rs`).

## Bookkeeping found on the way

- `docs/design/feature-parity.md`: the Summary table was recounted after 0.1.22; the relay region
  picker and hosting a server now read "has, real game unverified", so 2 rows stay partial
  (Locker Recolor, i18n) and 1 is missing (GameBanana Remote Install).
- The Locker Recolor row is marked partial in `feature-parity.md`, matching `recolor.rs`.

## Counts

Rows in this doc's area tables, including the four new commits:

| Status | Rows |
| --- | ---: |
| Missing | 0 |
| Partial | 8 |
| Has (checked; 4 of them real game unverified) | 24 |
| Unverified (test or owner call) | 1 |
| Not needed or on hold | 2 |
| **Total** | **35** |

By recommendation: 15 built, 5 build, 1 owner decision, 2 verify first, 3 skip or keep, 8 nothing
to do, 1 waiting on real recolor.
