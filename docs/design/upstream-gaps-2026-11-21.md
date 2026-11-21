# Upstream gaps: the newest Grimoire and Deadlock Mod Manager against Sigil

Status: research snapshot, 2026-11-21. Read only for Sigil's code; nothing here is built. Fresh
upstream clones are in `.scratch/upstream/grimoire` and `.scratch/upstream/dmm` (gitignored).
Grimoire (G) is MIT, so its code and copy may be ported. Deadlock Mod Manager (D) is GPL-3.0:
behaviour only, never its code or text.

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
random list members, contextual tips, Mods only). This doc lists what is still missing or
partial, plus the four new commits.

Left out on purpose (owner decisions, not raised again): DMM's plugins
(`feature/creator-workspace-bridge`), the Le Deadlock and Coding themes, Steam-login match sync
(`b8f0b246` per-account switch, `81a7862a` batching), the Sigil accounts package, translations
(`8a00ccdf` language table, Crowdin), Mod Foundry heavy edits (D `6cf08eff` remove hero sections,
D `feature/modding-workbench` with CSDK 13 and GIF stickers, G's Door Stuck Parry editor in #571),
Divine Skins, DMM's own hosted source (`8774679`, its backend). Already being built, so not listed
as new: D's "Where I play" preset and recent match counts in the server picker (`5d38b7cd`), the
review step for configs bundled in packs (`8774679`), D's viewer material and particle fidelity
(`34fd9caf`), and the 3D extras on the leftovers list.

## Top 10 worth building next

Ordered by value to players.

1. **Mod safety review list with Disallow** (G #592, M). One place that lists every flagged mod
   as Review or Allowed, a "N mods need attention" notice, Check installed mods, Allow N script
   mods, and Disallow to take a trust back. Today Sigil only asks at the moment a mod is turned
   on and a trust can never be withdrawn.
2. **Read VPKs whose root folder is an empty name** (G #578, S). Sigil's directory reader stops at
   an empty folder name, so a mod like the "Puppy Ivy" skin misreads in the index, Overlap and the
   safety scan. Port Grimoire's rule (empty first folder means the root) and its second walk.
3. **Installs and imports that land on pass the safety scan first** (G #591, S). With "New mods
   start turned off" off, a Browse install lands on without the gate; local imports always land
   off. Make both follow the setting, and land on only when the scan is clear or that version is
   trusted.
4. **Custom mod sources inside Browse** (D, M). Sigil reads `dmm-source` 1.0 sources, but only in a
   Settings dialog that installs a listing's first file. D puts the source picker in the store
   header, with cards, release history, Markdown descriptions, a file picker and deep links.
5. **Add mods dialog that reads the files** (D `c60a5792`, M). Detect category, cover and map name
   while importing, and import several loose VPKs as one mod with parts.
6. **Installed refuses changes while the game runs** (settings row G32, S). The hook is pending in
   Installed toggles and deletes; Grimoire's Disallow and the startup check depend on the same
   guard.
7. **GameBanana notices with their own words** (D `7ede98c1`, `cba597f2`, `208df742`, S). Show the
   author's note on obsolete mods and the staff reason on withheld ones, and leave withheld mods
   out of Featured and the saved catalog.
8. **Profiles keep the Locker's state** (G #587, S for shuffle, L for colors). A profile should
   carry the shuffle switches and pools (Grimoire's profiles already did) and, once real recolor
   exists, ability colors.
9. **Hidden mods and creators lists that say what they were** (G #580, S). Thumbnails (blurred
   until hover), the date hidden, the mod it was hidden from, and a short note.
10. **Discord: hide party size** (D `97c07548`, S). A switch that drops "(3 of 6)", "Party of 3"
    and "In Queue 3" from presence.

## Gaps by area

Columns: what it is for a player; where it lives upstream; Sigil's status with the file or doc
that shows it; effort (S under a day, M a few days, L a week or more); recommendation.

### New since our vendored commits

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Disallow: take back a mod safety approval; the mod is turned off first and returns to Review | G #592, `electron/main/services/modSafety.ts` (`revokeVpkSafety`), `modSafetyAudit.ts` | missing: `desktop/src/screens/installed/safety_gate.rs` `trust()` only adds to `safety-trusted.json`; nothing removes an entry | S | Build, as part of the review list (top 1) |
| "Enable mods after download" also enables imports that passed the review | G #591, `electron/main/ipc/mods.ts` (`importCustomModSource`), `docs/mod-inspection.md` | partial: `desktop/src/screens/import.rs` always imports switched off; `prefs.land_disabled` applies to Browse, sources and Forge installs, which then land on without the gate (`installed/safety_gate.rs` runs only from `enable::switch`) | S | Build (top 3) |
| Stock `gameinfo.gi` template refreshed from the depot (`sv_minrate` 98304 to 133120) | D #1417, `apps/api/src/static/artifacts/deadlock/gameinfo.gi` | not needed: Sigil repairs the player's own file in place (`deadlock-core/src/game_apply.rs`); 98304 appears only in test fixtures | - | Skip: data on D's backend. Refresh `deadlock-core/tests/fixtures/stock-gameinfo.gi` when a test next needs it |
| Translation progress table | D #1415 | on hold (owner) | - | Skip: translation is on hold |

### Mod safety and install

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Mod safety page: Review and Allowed rows, "N mods need attention", Check installed mods, Allow N script mods, "this mod changed since you reviewed it", a startup and launch check that turns off enabled mods nobody approved | G `src/components/ModSafety.tsx`, `src/locales/en/translation.json` `modSafety.*`, `docs/mod-inspection.md` | partial: the scan, the per-card report and the turn-on gate exist (`docs/design/mod-safety.md`); no list, no notice, no check of mods that were already on | M | Build (top 1) |
| Trust tied to the file's bytes, so any change asks again | G #578 and #592 (`approvalKey`, content hash) | partial: `installed/safety_gate.rs` keys a trust "by the mod and its file size" | S | Build: key on the sha256 Sigil already computes for View imprint |
| Read a VPK whose root folder is stored as an empty name, then walk again so nothing hides behind it | G #578, `electron/main/services/modSafetyScan.ts` | missing: `deadlock-core/src/vpk.rs` breaks the folder loop on an empty name (lines 269 to 272), so the rest of the tree misreads | S | Build (top 2); port Grimoire's rule and its test |
| Text layouts (`vxml_c` with a DATA block and no LaCo) are read | G #578 | has: the scan searches layout bytes (`docs/design/mod-safety.md`, "Limits") | - | - |
| Enable anyway for a file the scan cannot read | G #578 | has: the gate lists "Not checked: the whole file" with Turn on this once (`installed/safety_gate.rs`) | - | - |
| Name a pack's members from their imprints | G #582, `src/lib/customModImport.ts` | has: each VPK is named from its own `addoninfo.txt` or `modinfo.json` first (`deadlock-core/src/import.rs`, line 538) | - | - |
| Add mods dialog: detects category, cover and map name from the files; several loose VPKs become one mod whose parts switch in Manage files | D `c60a5792`, changesets `vivid-covers-render`, `amber-parts-gather` | partial: the import review lists names and previews (`screens/import.rs`); grouping is a later step (Installed > Group as one mod); no category, cover or map detection there | M | Build (top 5) |

### Installed

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Refuse toggles and deletes while Deadlock runs | G (Disallow and the startup check rely on it), settings row G32 | partial: `settings/watch.rs` `refuse_while_running` guards Browse, Overlap, Crosshair, tools and Forge; Installed toggles and deletes do not call it yet (`docs/design/settings-parity.md` row 32, "hook pending") | S | Build (top 6) |
| Local mods get a rendered 3D preview as their cover | D `e5d22b18`, `apps/desktop/src/lib/mod-thumbnails.ts` | missing: covers are the GameBanana picture or one the player picks (`installed` picture editor) | M | Build once the viewer has run on a real install |
| A multi-file card opens the mod page; Choose files in the menu | G #576 | has: `installed/layout.rs` `open_group` with the Settings option | - | - |
| Manage files: unticking every file turns the mod off | D `08ccbaac` | has: one switch per file in the variant picker | - | - |

### Browse and mod details

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Obsolete mods show the author's note; withheld mods show GameBanana staff's reason; withheld mods leave Featured and the catalog | D `7ede98c1`, `cba597f2`, `208df742`, `6bdb2970` | partial: `screens/browse/details.rs` `notices()` shows fixed sentences; the reason text is not read | S | Build (top 7) |
| Category trees with breadcrumbs and linked hero filters across Mods, Sounds, Maps and WiPs, third level included | D `cd622cb0`, `apps/desktop/src/components/mod-browsing/catalog-categories.ts`, `category-filter-breadcrumbs.tsx` | partial: root and sub category, hero picker and a name match on sounds (`docs/design/feature-parity.md`, Browse); no breadcrumb or third level found | M | Build, P3 |
| Drag to scroll the landing rails, draggable scrollbars | D `2d8ba447` | missing (no rail drag in `screens/browse/`) | S | Build if Rune's scroll view allows it; P3 |
| Hidden mods and creators: thumbnails, date hidden, where from, a note | G #580, `src/components/settings/` hidden content rows | missing: Settings shows counts and names (`screens/settings.rs` line 1765) | S | Build (top 9) |
| YouTube and Vimeo videos on mod pages | D `9fab2e15` | has as a Videos list that opens the browser (`feature-parity.md`, Mod details) | - | Keep: an embedded player needs a web view, which ADR 0006 rules out |

### Locker

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Profiles keep the Locker's ability colors and trippy skins; Grimoire's profiles already kept the shuffle settings | G #587, `electron/main/services/profileLocker.ts` | missing: `deadlock-core/src/profiles.rs` has no Locker, shuffle or recolor field | S (shuffle), L (colors) | Build the shuffle part now (top 8); colors wait for recolor |
| Real ability VFX recolor (one color, rainbow, gradient, trippy) baked into a managed mod | G `docs/ability-vfx-recolor.md`, `electron/main/services/heroColors.ts`, `ipc/trippyEffects.ts` | partial: `deadlock-core/src/recolor.rs` writes a brief only ("records the exact targets ... instead of pretending to edit a game file"), yet `feature-parity.md` marks Locker Recolor "has" | L | Owner decision: is the Locker's particle recolor part of the held Foundry recolor? Correct the parity row either way |
| Hero Skins page: banner with what the hero wears, equipped label, sound rows with play and a switch, page-wide shuffle and 3D switches, explicit pool mode, and a hero with no skins shows the store's most downloaded skins | D `17337665` | partial: hero page, load order, shuffle pools, sounds and the 3D switch exist (`feature-parity.md`, Locker); no suggestions for a hero with nothing installed | S | Build the empty-hero suggestions; the rest is layout |

### 3D viewer

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| A preview appears only when whole: a loader with build progress covers it until the hero, the room and any staged heroes are ready, 20 s fallback | G #588, `src/components/locker/HeroPoseViewer.tsx`, `src/components/common/PreviewLoader.tsx` | partial: loading and failed states exist (`docs/design/3d-viewer.md`, `viewer-locker-loading`, `viewer-hideout-loading`); no gate for staged heroes or Hideout progress | S | Build with the staged pose work |
| Motion previews: dash, heavy melee and parry mods play on a hero against a second hero; an animation mod plays its own clips with a clip strip and pause | G #571, `src/lib/movementGraphClips.ts`, `export-animation-mod-preview` IPC | partial: the clip picker has dash and heavy melee (`3d-viewer.md`, `clipInfo`); no second hero and no per-mod clip strip | M | Build after the staged pose; G's Parry editor stays on hold (Foundry) |
| Texture-only skins previewed on the full character; partial mods compared with the original | D `e5d22b18` | partial: With mods / Without mods / Split exist; the texture-only case is unverified on a real install (`3d-viewer.md` status) | S | Verify on a Windows install before building anything |

### Stats

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Match details show abandoned matches and the teammates who left | D changeset `quiet-hawks-wave` | partial: `deadlock-core/src/stats.rs` reads `abandoned_time_s`; no screen under `desktop/src/screens/stats/` shows it | S | Build |
| Street Brawl games tagged in the match list | G #575 | different: Sigil's statistics ask for `game_mode=normal` only (`deadlock-core/src/stats/filters.rs`) | S | Owner decision: list Street Brawl with a tag, or keep it out |
| deadlock-api rank rework, rank card details, ranked delta, leaderboard seasons, crosshair game codes | G #575 | has: `stats.rs` (`/v1/players/rank/distribution`, `initial_win_streak`, `ranked_delta`), `stats/leaderboard.rs`, `crosshair.rs` (`SHARE_CODE_URL`) | - | - |
| Non-scored matches are not losses | D `b6fbfd67` | has: `stats.rs` `NotScored`, result `None` when unscored | - | - |

### Integrations, sources and performance

| Feature | Upstream | Sigil | Effort | Recommendation |
| --- | --- | --- | --- | --- |
| Custom mod sources browsed from the store header, release history, Markdown descriptions, the multi-file picker for a listing's files, deep links that add a source or install one of its mods | D `ef8af43b`, `d9ba608c`, `becf7cda`, `cfa9554d`, `b79a7485`, `c203203b`; `apps/docs/.../mod-sources.mdx` | partial: `desktop/src/screens/settings/sources.rs` adds a source, lists it in a dialog and installs "a listing's first file" (line 431) | M | Build (top 4) |
| Discord Rich Presence: hide party size | D `97c07548` | missing: `settings/templates.rs` has a `{party}` variable but no switch | S | Build (top 10) |
| Detect the running game on Flatpak Steam | D `e02703ce` | unverified: Sigil reads the process table (`deadlock-core/src/launch.rs`); Flatpak is handled for the Discord socket only | S | Test on a Flatpak Steam before building |
| Built-in benchmark: launch a scripted run, capture frames, show average FPS, 1% low, p99 frame time, hardware and settings | G branch `feat/deadlock-benchmarks-wip` (unmerged, experimental) | missing | L | Owner decision: wait until Grimoire merges it; it needs a frame capture backend per OS |
| Import and manage custom performance presets | G branch `feat/custom-performance-presets` (unmerged) | has: Performance import and export of a config (`feature-parity.md`, Performance) | - | Recheck if Grimoire merges a different model |

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

- `docs/design/feature-parity.md`: the Summary table still counts 6 missing and 5 partial; the rows
  now show 1 missing (GameBanana Remote Install) and 4 partial.
- The Locker Recolor row says "has" while `recolor.rs` only writes briefs (see Locker above).

## Counts

Rows in this doc's area tables, including the four new commits:

| Status | Rows |
| --- | ---: |
| Missing | 8 |
| Partial | 14 |
| Has (checked) | 9 |
| Unverified or different (test or owner call) | 2 |
| Not needed or on hold | 2 |
| **Total** | **35** |

By recommendation: 19 build, 3 owner decision, 2 verify first, 4 skip or keep, 7 nothing to do.
