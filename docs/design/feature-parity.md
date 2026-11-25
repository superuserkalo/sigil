# Feature parity: Grimoire and Deadlock Mod Manager

Status: historical. A record of the Grimoire and Deadlock Mod Manager parity audit as of 2026-11-21 (Sigil 0.1.22). Not maintained; the code wins where they differ. Original status: audit 2026-11-17, updated the same day; upstream round 2026-11-21 (mod safety review list, sources in Browse, GameBanana notices, hidden lists, Locker shuffle in profiles, Hide party size, Street Brawl in Stats; Summary recounted, Recolor marked partial); statuses rechecked against `rust/crates` after 0.1.22 (2026-11-21); live match view built 2026-11-20; smalls round 2026-11-19 (custom maps page, update progress, package manager note); wave 3 rows updated 2026-11-18; wave 4 (UI B) rows updated 2026-11-18. Browse and mod details are built, including a
bounded local catalog mirror (JSONL, no new crate, no SQLite), the inline sound player and the
multi-VPK picker. The Installed page has its first round. The global shell has its round: drop
anywhere, zoom chords, the shortcuts sheet, What's new after an update, the tray, density, and a
pre-blurred background. What is left is listed below each table. Settings is covered in
`settings-parity.md`.

Sigil must have everything Grimoire (G, `vendor/grimoire`, MIT) and Deadlock Mod Manager (D,
`vendor/dmm`, GPL-3.0) offer a player, organised better. G code and copy may be ported; D is
studied for behaviour only and reimplemented.

How to read the tables:

- **Source**: G, D or G+D.
- **Sigil**: has, partial or missing. "Has" means it works today, not that a stub exists.
- **Effort**: S (under a day), M (a few days), L (a week or more).
- **Priority**: P1 (players notice it is missing), P2 (expected by power users), P3 (nice to have
  or niche). Items that conflict with Sigil's rules are marked "won't" with the reason.

Deferred as one package, "Sigil accounts" (owner 2026-11-20): Grimoire Social sign-in, cloud
backup, DMM's Discover publishing, announcements and feature flags. They come later on Sigil's
own backend (the telemetry Worker is the likely start), not on Grimoire's or DMM's. The community
crosshair library left the package: it is built on the Worker without accounts (ADR 0019).
A community profile gallery (Grimoire Social parity) would need a Sigil-owned HTTPS API on the
Worker with D1: Steam OpenID sign-in, profiles holding the existing share-code payload, likes,
reports and account deletion, the session token in the OS keychain, and an ADR for the new stored
identity before it is built. Cloud backup of profiles needs the same sign-in. Rows marked "won't" for style or rules are closed by the owner and are not raised again.
The relay region picker was decided (owner, 2026-11-20: build it, off by default) and is built.

Two architectural differences drive many rows:

1. Both apps mirror the GameBanana catalog into a local SQLite + FTS5 database. That is how they
   offer an A to Z sort, "added in the last N days", hero filters on sounds and author search.
   Sigil reads `apiv11` live and gets most of the same through the server's own filters (see the
   endpoint notes under Browse). A local mirror stays an option (L, P3).
2. D talks to its own backend (`api.deadlockmods.app`) for announcements, shared profiles, cloud
   backups, community crosshairs, the server list and analytics. Sigil has no backend and no
   telemetry; those rows are marked accordingly.

## Summary

| Area | Has | Partial | Missing | Deferred |
| --- | ---: | ---: | ---: | ---: |
| Browse | 42 | 0 | 0 | 0 |
| Mod details | 22 | 0 | 0 | 0 |
| Installed | 44 | 0 | 0 | 0 |
| Locker | 19 | 1 | 0 | 0 |
| Crosshair | 10 | 0 | 0 | 0 |
| Autoexec | 5 | 0 | 0 | 0 |
| Performance | 8 | 0 | 0 | 0 |
| Stats | 12 | 0 | 0 | 0 |
| Profiles | 10 | 0 | 0 | 1 |
| Servers | 11 | 0 | 0 | 0 |
| Launch | 8 | 0 | 0 | 0 |
| Overlap | 7 | 0 | 0 | 0 |
| Downloads | 11 | 0 | 0 | 0 |
| Global shell | 26 | 1 | 1 | 1 |
| **Total** | 235 | 2 | 1 | 2 |

Counted from the rows below on 2026-11-21 after 0.1.22. Deferred rows wait for Sigil accounts; the
two partial rows are Locker Recolor (briefs only) and i18n; the missing row is GameBanana Remote
Install. The relay region picker and hosting a server read "has, real game unverified": built and
checked in headless scenes, never run on a Windows PC.

## Browse

Endpoints, verified with curl against the live API on 2026-11-17 (Deadlock game id 20948):

- `Game/20948/Subfeed?_nPage=N&_sSort=default&_csvModelInclusions=Mod|Sound|Wip`: the plain
  "popular now" feed, 15 rows a page. It ignores category filters.
- `Mod|Sound|Wip/Index?_nPage=N&_nPerpage=50&_sSort=<alias>` with
  `_aFilters[Generic_Game]=20948`, `[Generic_Category]=<id>` (a root includes its children),
  `[Generic_Name]=contains,<text>`, `[Generic_WasFeatured]=true`, `[Generic_Submitter]=<id>`.
  Accepted sorts: `Generic_Newest`, `Oldest`, `LatestModified`, `LatestUpdated`, `NewAndUpdated`,
  `MostLiked`, `MostViewed`, `MostDownloaded`, `MostCommented`, `LatestComment`,
  `Alphabetically`, `ReverseAlphabetically`. Rejected: `HighestRated`, `MostSubscribed`,
  `Default`, and any date filter (`Generic_DateAdded`, `Generic_Period`: `UNKNOWN_FILTER`).
  Index rows carry no download count even when `_csvProperties` asks for it.
- `Util/Search/Results` (G's search) ignores `Generic_Category`, `_sSort` and `_nPerpage` (always
  15 rows), so Sigil searches through `Index` + `Generic_Name` instead, which honours them all.
- `Mod/Categories?_idGameRow=20948&_sSort=a_to_z&_bShowEmpty=true` and
  `ModCategory/<id>/SubCategories` (ids only in `_sUrl`). Skins has 44 hero children with
  portraits; there is a third level under some heroes. Sounds: `Sound/Categories`.
  `<Model>/Categories?_idCategoryRow=<id>` lists one category's children with `_idRow` and
  `_nCategoryCount`; Sigil reads the third level with it after the first two show.
- `Game/20948/TopSubs`: 3 per period (today, week, month, 3month, 6month, year, alltime).
- `Util/List/Featured?_idGameRow=20948`: featured list (not used; `Generic_WasFeatured` covers it).

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Section switch | G+D | Mods, Sounds, WiPs | has (built) | S | P1 |
| Collections tab | D | GameBanana collections, size buckets, download all | has (built: size buckets, item list, install all through the queue) | L | P3 |
| Maps tab | D | Mods flagged as maps (experimental in D) | has (built: the Maps root category) | M | P3 |
| Combined multi-section feed | G | Mods + sounds in one grid (mirror only) | has (built: the All tab; the plain feed is live, a filtered read uses the saved catalog) | M | P3 |
| Category filter | G+D | Root categories with counts | has (built) | S | P1 |
| Sub category / hero filter | G+D | Hero under Skins with portraits and type-ahead; type under HUD etc. | has (built; 2026-11-21, DMM 2.4.0 `cd622cb`: third level under the picked hero, breadcrumb chips, subtree matching by leaf id, the hero pick follows the section, the hero rail drags, `browse/tree.rs`, `browse/rail.rs`) | S | P1 |
| Hero filter on sounds | G+D | Hero name match on non-skin sections | has (built: hero picker and chip, name match) | S | P2 |
| Include/exclude match mode | D | Invert category and hero filters | has (built: Match in Filters) | M | P3 |
| Server-side search | G+D | Name search against the whole catalog, debounced | has (built, 350 ms) | S | P1 |
| Paste id or link | D | A GameBanana id or URL opens that submission | has (built) | S | P2 |
| Author results in search | D | Matching creators above the results | has (built: creator rail from the saved catalog) | M | P3 |
| Sort: popular, newest, updated, liked, viewed, downloaded | G+D | Order picker | has (built), default from Settings | S | P1 |
| Sort: most discussed, latest comment, A to Z | G (A-Z via mirror) | Extra orders | has (built, server side) | S | P2 |
| Date window (added) | G+D | Any, day, week, month, 3 months | has (built: newest-first read to the cutoff, ordered locally) | M | P1 |
| Date window (updated), custom range | D, G | Second date filter, from/to picker | has (built: Added or Updated, periods and a from/to range) | M | P3 |
| NSFW hide / only / blur | G+D | Session filter over the app setting | has (built) | S | P1 |
| Hide outdated | G+D | Before the Jan 22, 2026 path change, or obsolete | has (built) | S | P2 |
| Installed / not installed | D | Library state filter | has (built) | S | P2 |
| Featured | GameBanana | Only featured submissions | has (built) | S | P2 |
| Quick stats strip | D | Installed, updates, queue, profile on the landing | has (wave 3, `browse/strip.rs`: four shortcuts over the landing rails, counts from the status bar's cache) | S | P3 |
| Top this week strip | GameBanana (D has a dashboard carousel) | Three top picks over the plain feed | has (built) | S | P2 |
| D dashboard: trending rails, browse by hero rail, announcements | D | Landing page | has rails (trending week and month, browse by hero); announcements need D's backend | M | P3 |
| Favorites | D | Star on cards and details, favorites view with count | has (built, local file) | S | P1 |
| Hidden mods and creators | G | Hide with management list | has (built: persisted, management list in Filters; 2026-11-21, G #580: each row shows the mod's picture or a creator's three most liked uploads from the saved catalog, blurred until the row is under the pointer, the date hidden, the mod a creator was hidden from and a note of up to 140 characters, in Browse's Hidden items and Settings > Mods, `browse/hidden.rs`) | S | P2 |
| Artist mode | G+D | Grid scoped to one creator | has (built: "More" in details, chip; 2026-11-21, DMM author profiles: a creator page over the scoped grid with picture, title, join date, subscribers, totals across their Mod and Sound uploads, top categories, most liked upload and their links, `browse/creator_page.rs`, `deadlock_core::creators`) | S | P2 |
| Active filter chips + clear all | D | Removable chips | has (built) | S | P1 |
| State kept when opening a mod and coming back | G+D | Filters, results, scroll | has: details overlay or dock; filters kept for the session | S | P1 |
| Filters kept across restarts | D | Persisted per store | has (built: browse/state.json) | S | P3 |
| Infinite scroll, virtualized | G+D | Pages load near the end | has | - | - |
| Numbered pagination option | D | Page numbers | has (built: View options) | S | P3 |
| Keep old results dimmed while loading | D | No flash to skeleton | has (built) | S | P2 |
| Grid / list view, card size | G | Layout options | has (built: grid or list, three card sizes, persisted) | M | P3 |
| Card: likes, views, author, age, install | G+D | Card facts | has | - | - |
| Card: installed state | G+D | Installed badge | has (built: "Installed" replaces Install) | S | P1 |
| Card: downloads | D | Download count | has (built: the mod page's count is kept in the saved catalog) | M | P3 |
| Sound preview | G+D | Inline player, one at a time, volume | has (built: rodio player, one sample, volume in View options) | M | P2 |
| Hover prefetch of details | D | 250 ms hover prefetch | has (built: profile cached after 250 ms) | S | P3 |
| Card context menu: copy image / link | G | Right click | has (built: open, copy link, copy image link, install, hide mod and creator; wave 4: Grimoire's image menu on cards, the gallery and the full size view: Copy image (the picture as PNG), Copy image address, Open image) | S | P3 |
| Random hero | D | Dice on the hero rail | has (wave 4: Random on Browse by hero opens a random hero's skins, never the last pick) | S | P3 |
| Featured mod per category | D | Weekly pick per category | has (wave 4: Featured this week on the landing, one per category from the saved catalog through `library_facts::featured`, play button for a sound pick) | S | P3 |
| Bulk paste import | G | Many links, a collection or a code | has (wave 4, `browse/paste.rs`: mod links and ids, a lone collection link opens it; preview with installed and duplicates skipped, Select all, Skip NSFW, Queue N, Cancel remaining; one file per mod picked from its list (newest by default); Save N as a profile once they are installed; a profile code or profile JSON goes to Profiles > Import) | M | P3 |
| Rate-limit notice | G | GameBanana 429 | has (wave 4: one toast at most every 10 seconds and the empty grid says GameBanana is rate-limiting Sigil) | S | P3 |
| Catalog sync indicator / local mirror | G+D | SQLite mirror | has (built: bounded JSONL index, background incremental sync, offline browse, author search, date ranges) | L | P3 |
| Custom mod sources in Browse | D | `dmm-source` 1.0 catalogs beside GameBanana in the store header | has (2026-11-21, D `ef8af43b`, `d9ba608c`, `becf7cda`, `cfa9554d`, `b79a7485`: Settings > Mods > Sources adds a source after showing every origin it contacts; each enabled source is a switch beside GameBanana in Browse's header with a search and cards, and its details show every image, tags, the description (Markdown read as plain text), every file of the current release with its own Install (sha256 checked when declared) and the release history when the source declares it. Images load only from the declared media or api origins. Not built: D's components installs (required parts and colour choices) and its `deadlock-mod-manager://sources/...` deep links) | M | P3 |

## Mod details

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Modal or docked panel | G | Overlay or side dock | has | - | - |
| Gallery and lightbox | G+D | Images, arrows, captions | has (wave 4, issue #3: the close button and counter sit on the picture, a press on the backdrop closes; zoom to 400 percent with the wheel, a pinch, a double click or the zoom bar, and pan by dragging while zoomed) | - | - |
| Delete an installed file from details | G | Trash on the file row | has (wave 4: an Installed list above the files, each slot with Delete through Installed's confirm and trash with Undo) | S | P3 |
| Resizable docked details | G | Drag the panel edge | has (wave 4: drag its left edge, 320 to 900 points, kept across launches) | S | P3 |
| About text | G+D | Description | has (plain text, no inline links) | - | - |
| Files list with per-file install | G+D | Name, size, downloads, date, description, archived | has | - | - |
| Changelog | G+D | Updates | has (10) | - | - |
| Comments | G+D | Posts | has (20) | - | - |
| Artist card, hide creator, more from creator | G | Creator row | has | - | - |
| Favorite | D | Star in the header | has (built) | S | P1 |
| Sound sample | G+D | Audio preview | has (built: the inline player) | M | P2 |
| Copy link, view on GameBanana | G | Header actions | has (built: copy icon in the header) | S | P3 |
| Previous / next mod with arrow keys | G | Walk the grid | has (built: left and right in the details) | S | P2 |
| Requirements panel | G+D | Required and recommended mods | has (built: requirements section, install and open) | M | P2 |
| Installed / update / outdated tags, ignore updates | G+D | Status tags | has (tags and ignore in Browse; Installed's update check reads the ignored file or mod, `mod_extras::ignored_update`, `installed/notes.rs` `ignore_update` / `unignore_update`) | - | - |
| Embedded videos | D | Video embeds | has (built: a Videos list; each opens in the browser) | M | P3 |
| Donation / Ko-fi, socials | G+D | Support the creator | has (built: donation links from the creator profile; wave 4: the creator's own links from `Member/<id>/ProfilePage`, a Discord user name copies) | S | P3 |
| 3D model preview | D | three.js viewer | has, real game unverified (Preview in 3D beside "Installed in slot NN" in a mod's details, and in Installed's card menu and file list: the mod's hero with the mod, on or off, against the hero without it; hero picker for several heroes; `docs/design/3d-viewer.md`. Checked on stand-ins only: no Deadlock install on the build Mac) | L | P3 |
| More comments / changelog paging | D | Load more | has (built) | S | P3 |
| Hero detected and override | D | Hero picker | has (built: detected from the page, change in the picker) | M | P3 |
| Withheld / obsolete notices | D | Notices | has (built: notice boxes for withheld, private and obsolete; 2026-11-21, D `7ede98c1`, `cba597f2`, `208df742`, `6bdb2970`: the author's note on an obsolete mod, the staff's rules, note, name and date on a withheld one; a mod page that says withheld or private drops the mod from the saved catalog and Featured, and each Featured pick is checked against its page once a run) | S | P3 |
| Multi-VPK picker after download | G+D | Choose VPKs in an archive | has (built: plan lists the archive's VPKs and installs the chosen one) | M | P2 |

## Installed

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Enabled / disabled groups | G+D | Two sections | has | - | - |
| Enable / disable one | G+D | Toggle | has | - | - |
| Search | G+D | Name, author, slot | has | - | - |
| Import local files | G+D | VPK and archives | has | - | - |
| Import from G / D | G+D | Interchange | has | - | - |
| Reveal in folder, open on GameBanana | G+D | Card menu | has | - | - |
| Status filter (all / enabled / disabled) | G+D | Tabs with counts | has (built) | S | P1 |
| Source filter (GameBanana / local / other managers / unknown) | G | Filter | has (built) | S | P2 |
| Hero / category / tag filters | G+D | Filters | has (built: `mod_extras` stores hero, category, tags, a pin, an ignored update, a picture and a variant group; GameBanana metadata is fetched on the page and backfilled for older entries) | M | P2 |
| Sort (load order, name, hero, recently added, size) | G+D | Sort | has (built; wave 4: sort by hero, the install date on cards from the journal, `library_facts::install_date`) | S | P1 |
| Grid / list, card size | G+D | View options | has (built: Cards, Small cards, List; wave 4: card style options in View, the details line and the slot number each on or off) | S | P3 |
| Multi-select, bulk enable / disable / delete | G+D | Selection mode | has (built: check box on hover, shift-click range, select all shown, Ctrl+A (Cmd+A) selects every shown mod and Escape clears (wave 4); bulk on, off, delete behind a confirm with "Don't ask again") | M | P1 |
| Delete a mod | G+D | Remove the VPK | has (built: card menu and selection; files move to Sigil's trash with restore and Empty trash, `deadlock_core::trash`) | S | P1 |
| Drag-and-drop install | G+D | Drop files on the window | has on Installed (built: drop veil, then the import review) | S | P1 |
| Load order reorder | G+D | Drag to change pak number | has (built: drag a card to reorder, Load first, earlier, later in the card menu, and DMM's load order dialog with keyboard moves, top and bottom buttons and one Apply; all go through two-pass renames with rollback, `deadlock_core::load_order`; wave 4: DMM's search by name or author and Grimoire's type-a-place chip in the dialog; ordering the files inside one multi-file mod is the same list, one row per file) | M | P1 |
| Update checks + update all | G+D | Per-mod badge, batch update with backup | has (built: checked on open at most every 6 hours, Update chip and Update all; same slot, on or off kept, old files backed up; wave 4: Cancel on each Updating or Queued chip and Cancel all beside the progress; a cancel stops the download, `PlanOptions.cancel`; repaired mods keep their check, `baseline_for_repaired`; Update all takes All, Only enabled or Only disabled (DMM); a mod whose installed file was archived offers Replace with each current file; the Update mark follows Settings > Show mod update badges) | M | P1 |
| Sound preview on installed mods | D | Play a sound mod's sample | has (wave 4, `installed/sound.rs`: Play on cards of mods in a Sound, Audio or Music category, the sample from the mod's GameBanana page, through Browse's player at the shared preview volume) | S | P3 |
| Variant groups / mod options | G+D | Several files of one mod as one card | has (built: GameBanana files group by mod id, one card with the playing file, a picker with per-file switch, label rename, split and delete) | M | P2 |
| Group and add variants | G | Group 2+ mods as one card, add files to a group | has (wave 4, `installed/selection.rs`: the selection's More > Group as one mod writes one `variant_group` for the picked files and their card mates, so a group card plus a new file adds it) | S | P3 |
| Merge mods | G | One VPK from several | has (built: lowest free slot lands off, the lower slot wins, strict mode refuses a shared path, a sidecar record, unmerge to the trash) | L | P3 |
| Mod lists / groups | G+D | Named groups, enable per group | has (built: a list from the selection, filter by list, turn on or off per list; wave 4: View > Sections by list shows a section per list with Turn on / Turn off and "Not in a list", an icon per list and Move up / down in the Mod lists dialog, `installed/list_sections.rs`) | M | P3 |
| Tags | G | User tags | has (built: tags per mod, a tag filter, tags in the editor) | M | P3 |
| Solo test | G | Launch with only this mod | has (built: every other mod off, a line above the cards, Restore) | S | P2 |
| Unknown VPK identification | G+D | Match CRCs to GameBanana | has (wave 4: Search GameBanana by name, then, with the "Also compare checksums" opt-in (off by default, Grimoire's experimental switch), the ZIP CRC probe over the hits, `mod_lookup`, confirmed before it is linked; also: embedded `addoninfo.txt` / `modinfo.json`, Sigil's sha256 identity store and the install journal, a pasted GameBanana link; no live archive CRC scan | L | P3 |
| Mod safety scan | G | Scripts and executables in VPKs | has (built: `mod_safety` lists programs, loose scripts, script-mod files and Panorama code that reaches the network, each with its reason; never images, layouts or styles alone. Card button, report and a turn-on gate. Rules: `design/mod-safety.md`) | M | P2 |
| Mod safety gate and view imprint | G | Script mods wait off until reviewed; card menu shows the embedded identity | has (wave 4: turning on a mod with programs, scripts or network calls asks first with Turn on this once and Trust this version, `installed/safety_gate.rs`, skipped by Allow script mods; 2026-11-21, G #592: More > Mod safety lists every flagged mod as Review or Allowed with its reasons and files, "N mods need attention" on Installed with a dismiss that a newly flagged mod undoes, Check installed mods, Allow N script mods, Keep it off for an enabled mod nobody allowed, "changed since you allowed it", and Disallow, which refuses while the game runs, turns the mod off and then takes the trust back, `installed/safety_review.rs`. Unlike Grimoire nothing is switched off at startup without a press; More actions > View imprint shows sha256, size, the root addoninfo.txt and modinfo.json, the GameBanana id, the download link and merge sources, `installed/imprint.rs`) | S | P3 |
| Bulk Locker tag | G | Tag the selection in the Locker | has (wave 4: the selection's More > Add to a Locker category) | S | P3 |
| Copy mod list | G+D | Clipboard export | has (built: slot, name, author, off mark, one a line) | S | P3 |
| Find broken mod (bisect) | D | Halve the enabled set across launches | has (built: `deadlock_core::bisect`, an all-off launch first, still broken / works / not sure, restore all or leave the culprit off) | M | P3 |
| Compatibility review / issues | G+D | Outdated, skeleton, data checks | has (paths the game moved or removed, stale hero models and game data, plus vpkmerge's dry run for data, text, sound bindings, skeletons and cameras against pak01) | L | P2 |
| Compatibility repairs | G+D | Fix mods made for an older build | has (wave 3, `installed/repair.rs`: Repair per mod, Fix all and Revert in the review through `compat_repair::apply_repair` / `revert_repair`, backups and staging in app data, a question first when another mod ships animations, split archives combined and repaired (wave 4, C4), library and notes follow the new size) | L | P2 |
| Issues bulk action | G | Turn off mods with outdated data | has (wave 3: the review's "Turn off N mods with outdated data", a confirm, one Undo) | S | P2 |
| Requirement load-order fix | G | "The base skin is hiding it. Fix it" | has (wave 3, `installed/req_order.rs`: an amber mark on the dependent, its popover names the base and Fix it moves it before the base through the drag path with Undo; a base kept first cannot be passed and says so) | S | P2 |
| Random member per group each launch | D | One list member on at random | has (wave 3: a list's Random switch, a Roll step at each modded launch, map mods (by their files or the GameBanana Maps category, off members read too) and missing files left out, the last pick avoided, one Undo) | S | P3 |
| More than 99 enabled mods | G+D | Overflow folders | has (built: slots 1 to 990 across `sigil-addons2` to `sigil-addons10`, mounted in `gameinfo.gi` after every install, import, merge, reorder or restore and before each modded launch; vanilla launches unmount them; isolated profiles stay at 99) | M | P2 |
| Recap dialogs after bulk actions | D | What changed | has (built: changed, failed with the reason, not tried, why it stopped) | S | P3 |
| Per-mod backups | D | Backups an update replaced | has (wave 4, `installed/backups.rs`: More actions > Backups lists `backups::for_slot` with date and size; Put this version back runs the update undo, `trash::undo_update`; Delete removes the backup) | S | P3 |
| Reinstall | D | Same file again into the same slot | has (wave 4: More actions > Reinstall, the journal's file through the update queue with force, old files backed up; More actions > Delete and reinstall trashes the slot first and puts the old files back on any failure) | S | P3 |
| Analyze addons, HUD check, missing files, read-only | D | Folder health | has (wave 4, `installed/checks.rs`: More > Analyze the addons folder, per folder counts, leftovers, unknown slots with Find which mod this is, doubled slots, read-only files, missing files with Reinstall; Check HUD mods; a modded launch warns when an enabled mod lacks files; Make writable clears the read-only flag through `game_apply::make_writable`) | M | P3 |
| Global (always first) mods | G | Priority folder | has (built: "Always load first" moves the mod to the lowest slot of its group and keeps it there; Locker > General also mounts `citadel/grimoire` for hand-placed VPKs, with a backup and tests) | M | P3 |
| Edit local mod (name, image, NSFW) | G | Metadata editor | has (built: name, a picture copied into Sigil's data folder, a mature flag; GameBanana entries too) | S | P3 |
| Fonts prompt | D | Install shipped fonts | has (built: `deadlock_core::fonts` copies `panorama/fonts` files, adds a `<fontpattern>` block per mod with a config backup, and removes both; offered from the card) | S | P3 |
| Pinned favorites in disabled list | G | Pin | has (built: pinned mods sort to the top of the disabled group) | S | P3 |
| 99 enabled limit warning | G | Toast at the slot cap | has (built: a warning when all 990 slots hold a mod) | S | P2 |

## Locker

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Hero grid and hero page | G+D | Roster with installed skins | has | - | - |
| Recolor | G | Ability and skin colors | partial (`deadlock_core::recolor` writes recolor briefs only: the exact particle and material targets for a hero, no game file is edited. Real recolor (one colour, rainbow, gradient, trippy baked into a managed mod) is on hold with the Mod Foundry heavy edits) | L | P3 |
| General categories | G | Soul containers, urns, hideout, icons | has | - | - |
| Hero favorites, type-ahead, prev/next | G | Navigation | has (built: star on cards, list rows and the hero page, Starred filter, hero search over names and aliases, list view, prev/next walk the filtered list) | S | P3 |
| Skin variants, equip, reorder | G+D | Per-hero load order | has (built: skins in load order, Equip turns the others off, arrows move a skin within the hero's numbers, `deadlock_core::locker`) | M | P2 |
| Several skins per hero, shuffle pool | G+D | Equip adds; a pool for the shuffle | has (wave 4, C14: a Shuffle dialog switch makes Equip add a skin, `locker_parts::equip_wanted`; per-skin pool picks; the shuffle never repeats the skin on now, `shuffle_wanted`) | S | P3 |
| Weapon skins and voice lines per hero | D | Weapon skins, voice lines and gun sounds under their hero | has (wave 4, `locker/parts.rs`: each mod's VPK paths read with the Locker into `locker_parts::hero_parts`, a hero page section Weapons and voice with each mod's switch, `by_hero` sorting) | M | P3 |
| Locker image per mod | G | A picture per mod | has (wave 4, `locker/art.rs`: Locker rows show the picture Installed shows, a Picture button opens the shared editor; one picture per mod, no separate background art) | S | P3 |
| Shuffle on launch | G+D | Random skin per hero | has (per-hero switch, seeded pick, Shuffle now; the launch's Roll step runs it while the Locker page is on, wave 3) | M | P2 |
| Cursor packs | G | Import, apply, restore Deadlock's cursors | has (wave 3, `locker/cursors.rs`: General > Cursors, cards with previews, import a folder, a cursor file's folder or an archive, apply and restore through `cursors::apply` / `restore`, delete with a confirm, Find cursors; wave 4: a banner when Steam put stock files back or the pack was updated, with Apply again; the stock card previews `stock_preview_folder`; a cursor archive Sigil downloaded imports with its GameBanana source and replaces the older pack in place) | M | P3 |
| Browse skins for this hero | G+D | Jump to Browse with the hero filter | has | - | - |
| Ability sounds | G | Per-ability sound mod | has (built: `sounds/abilities` classification by `aN` token, hero page Sounds section, one sound per ability) | M | P3 |
| Custom hero cards | G | Portrait upload and crop | has (built: PNG/JPEG/WebP copy in app data, crop slider, remove) | M | P3 |
| 3D hero viewer | G+D | Posed model | has, real game unverified (Locker hero page: 3D view switch, the hero animated and orbitable with every enabled mod, With mods / Without mods / Split, capture and full screen, a cube button per skin for a one-skin preview; own wgpu renderer with shadows, Grimoire's cloth solver, hero and map particles, the Hideout's baked lighting, probes, fog and map post; Grimoire's Controls panel (grouped clips with friendly names, lighting scenes, backgrounds, switches), turntable, GIF capture with trim and DMM's video recording; `docs/design/3d-viewer.md`. Exports and the cache are tested on fixtures; a real hero export and how it looks are not yet seen) | L | P3 |
| Hideout scene | G | The menu map around the hero | has, real game unverified (Locker rail: Hideout, full screen, the player's mods and the hero at the spawn; checked on the stand-in room only; the Hideout parser follows Grimoire's schema and has not read a live `dl_hideout.vpk`) | L | P3 |
| Locker overrides review | G | Remove applied cards, sounds, colors | has (built: Locker changes dialog for cards, shuffle, sound mods and recolor briefs) | S | P3 |
| Asset catalog previews | G | The game's own textures as thumbnails, by kind, with a lightbox | has (wave 4c, `locker/catalog.rs` over `game_assets::texture_catalog` and `texture_thumbnail`: a Game art section on each hero page, kind chips (cards and portraits, ability icons, skin textures, ability effects, other), search over name and path, 48 tiles a page; previews at most 192 px decoded on one worker, at most 96 held and all released when the section closes; lightbox with path, native size, Copy path and Save preview. A full-size PNG export is not offered: the core caps previews at 256 px) | M | P3 |
| Game sound browser | G+D | The hero's own sound events, played in the app | has (wave 4c, `locker/game_sounds.rs` over `game_assets::sound_catalog` and `clip_audio`: a Game sounds section with ability groups by slot (ULT for the fourth), weapon, movement, melee, voice lines (folded, capped at 200 a group) and other; search; Play through the shared preview player at the status bar's volume; save a clip as MP3 or WAV) | M | P3 |
| Swap a hero's sounds | G+D | Replace a game sound with your audio, as a mod | has (wave 4c: Replace stages an MP3 (or a 16-bit WAV for uncompressed clips) per event, play it back, loop as the game, on or off, volume in 1.5 dB steps, or a volume change of the game's own sound; before building, a check across heroes warns when another hero plays the same clips; each swap is built by `build_sound_swap` into a VPK in app data and lands in the import review, switched off. Not built: Grimoire's waveform trim (the core takes `trim_ms`), DMM's OGG and FLAC input) | M | P3 |
| In-game hero card art | G+D | Your image on the hero's cards in the game, as a mod | has (wave 4c, `locker/card_art.rs` over `hero_card_slots` and `build_hero_card_mod`: In-game card on the hero page, every variant at its shape with the game's art dimmed and its size, PNG (JPEG and WebP are turned into PNG), centre crop to the variant's shape with a soft-image warning, several variants in one mod through the import review, switched off; Game art's Replace this icon does the same for one ability icon) | M | P3 |

## Crosshair

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Editor with live preview over scenes | G+D | Lines, dot, color, game | has | - | - |
| Apply to autoexec with backup | G+D | Managed block | has | - | - |
| Share codes | G | Encode and decode | has | - | - |
| Copy console commands | G | Clipboard | has | - | - |
| Presets: save, apply, delete | G+D | Built-in and saved presets, rename, live mini reticles | has | - | - |
| Import from game config / paste | G+D | Share code, console line, cfg or vcfg paste | has | - | - |
| Pin window always on top | G | Always on top | has (Crosshair "Pin window" calls `shell::set_always_on_top`; shell state, survives a recreated window) | - | - |
| Custom colour and outline colour | G | RGB sliders, outline colour | has (Color group: Red, Green, Blue, outline colour and its RGB) | - | - |
| Remove from autoexec | G | Clear the active crosshair | has (backup, toast with Undo) | - | - |
| Community library | D | Browse, apply and share crosshairs | has (ADR 0019 on Sigil's own Worker, no accounts: Presets and Community switch under the stage, the library as live mini reticle tiles with name, by Sigil and use count, Most used and Newest, Load more, fetched only when Community opens and cached 5 minutes, loading, offline and empty states; a tile only loads into the editor and an unchanged Apply counts one use; Publish on a saved preset with the Worker's name rules checked first and every error code mapped, the delete token kept owner-only for Remove from library; right-click Report hides the tile at once; every row checked against the crosshair ranges before it shows) | - | - |

## Autoexec

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Edit commands, keep unknown lines | G+D | Blocks | has | - | - |
| Command library by category | G | Premade commands | has | - | - |
| Save with backup | G+D | Write the cfg | has | - | - |
| Steam launch options | G | Written to localconfig at launch | has (built: every launch writes the saved launch options through `autoexec::sync_launch_options` in the launch steps; wave 1) | M | P1 |
| Open in external editor | D | Editor handoff | has (Edit in editor, the editor chosen in Settings) | - | - |

## Performance

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Community configs with tiers | G+D | gameinfo.gi presets | has | - | - |
| Version picker, newest from GitHub | G | Versions | has | - | - |
| Author convar switches | G | Visibility and camera opt-ins | has (owner exception) | - | - |
| Restore backup | G+D | Undo | has | - | - |
| Video settings changes (video.txt) | D | Keep, adopt, restore | has (Write adopts, Keep forgets the record, Restore puts yours back) | M | P3 |
| Convar editor and history | D | Search and edit convars | has (search, value edit, per-config history, resets; writes through `apply_to_install`) | M | P3 |
| Import / export a config | D | File exchange | has (export .cfg or clipboard, import paste or file with review, writes through the same edits) | S | P3 |
| GameBanana configs (7 items) | D | Download, file pick, review, license | has (page, file list, archive review with variant picker, license shown) | M | P3 |

## Stats

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Overview: rank, win rate, heroes | G+D | Player summary | has | - | - |
| Matches list | G+D | Recent matches | has (2026-11-21, owner's call, as Grimoire: Street Brawl games are listed with a Street Brawl tag and a With, Without or Only Street Brawl filter; they never count as ranked and stay out of win rate, form, streak, the rank line and the window's averages) | - | - |
| Tracked players, detect Steam users | G | Tracked accounts from `loginusers.vdf`, manual add, a primary account and a switcher | has | S | P2 |
| Match detail scoreboard | G+D | Both teams, final builds, souls over time, damage and healing | has | M | P2 |
| Social: teammates, opponents | G | Best teammates, frequent opponents, win rate with and against, premade flag | has | M | P3 |
| Leaderboard | G | Region and season, the player's place marked | has | S | P3 |
| Live match view | D | Live tab: both teams from the top bar, souls, K/D/A, clock, last match | has, real game unverified (Stats > Live match panel while Deadlock runs: queue, match found, the console's hero, mode, party and match clock; with the opt-in live bridge HUD mod (Settings > Game > Live match, off by default, built from the player's own `pak01` and installed into a free slot) both teams with hero, level, K/D/A, souls and objective damage, team soul and kill totals, the player's souls a minute and a soul lead curve; the last board kept until the next queue with Scoreboard and Statlocker; bridge state (off, outdated with Rebuild, moved top bar, shadowed by another top bar mod); after a game update (the build read every minute while the bridge is on: `steam.inf` ClientVersion plus `pak01_dir.vpk` size and time) the bridge is checked again and, when outdated, rebuilt once Deadlock is closed: by itself with Settings > Game > Live match > Rebuild the bridge after game updates (off by default), else one question per update with Rebuild; opt-in player lookup on deadlock-api for ranks. Checked on fixtures and headless scenes only: no Deadlock install on the build Mac; `docs/design/stats-live.md`) | L | P3 |
| Open on Statlocker | G | Links to statlocker.gg | has (wave 3: the player in the header and tracked list, teammates and opponents, an open match and the scoreboard; a browser link, no app request) | S | P3 |
| Items tab | D | Item win rates and purchase timelines | has (wave 3, `stats/more.rs`: the player's item rows against everyone's, buy time against everyone's, and an open match's purchase timeline) | M | P3 |
| Ranked and date filters | D | Scope Overview, Heroes, Items | has (wave 4: the filter sits on top and scopes every section; with a filter set the overview and Heroes read `fetch_snapshot_filtered` on a worker) | M | P3 |
| Party tab | D | Rate the lineups a party can queue | has (wave 3: pick tracked players and premades, each member's three most played heroes, pair and trio records, estimates for larger parties, low samples greyed; wave 4: DMM's planner, up to six players each offering up to three heroes picked by hand, `party::rate_lineups`) | M | P3 |
| Item meta | D | Everyone's best items | has (wave 4: Stats > Item meta, `items::global_item_stats` since the newest patch (Steam feed cache) or in the filter's window, 30 days when none is set) | S | P3 |

## Profiles

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Save, apply, rename, delete | G+D | Mod setups | has | - | - |
| Profiles keep the Locker shuffle | G | Shuffle switches and pools per profile (G #587) | has (2026-11-21: save and Update store the shuffled heroes, the pool and the several-skins switch under the profile's `sigil` extension; applying a profile puts them in the Locker and Undo puts the earlier ones back; a Locker shuffle edit is stored in the active profile; a profile saved before this leaves the Locker alone. Ability colours wait for real recolor) | S | P3 |
| Include autoexec / crosshair | G | Extra state | has | - | - |
| Share code and file export | G+D | `mp1:` code | has | - | - |
| Import with variant resolution | G+D | Code or file | has | - | - |
| Snapshots (automatic recovery points) | G | Before updates and applies | has (`profiles/snapshots.rs`: a JSON manifest of slots, GameBanana ids and fingerprints in the app data dir; sections with Restore, delete and bulk delete; taken before applies, profile updates and imports) | M | P2 |
| Profile switcher in the titlebar | D | One-click switch | has (`profiles::titlebar_switcher`: the active profile and a menu of saved ones) | S | P3 |
| Separate addons folder per profile | D | Isolation | has (`profiles/isolation.rs`: copies into `citadel/addons/sigil-<slug>`, mounts it through a marked `gameinfo.gi` block with a backup) | M | P3 |
| Cloud backup / Discover publishing | G+D | Needs a backend and sign-in | deferred: Sigil accounts (see below) | - | - |
| Undo a profile switch | - | Sigil extra | has (apply returns the slot changes, the autoexec backup and the previous profile; the toast's Undo puts them back) | S | - |
| File pickers for import and export | - | Sigil extra | has (Rune `file_dialog`; export writes where the player picks) | S | - |

## Servers

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| List with ping, players, password | G+D | Server browser | has | - | - |
| Search | G+D | Name and map | has | - | - |
| Join with content plan and confirm | G+D | Downloads required content | has | - | - |
| Filters: has space, password | G+D | Filters | has | - | - |
| Region and map filters, sortable columns | G+D | Filters | has (region grouped by continent, game mode, map, password; every column sorts ascending, descending, off with missing values last) | S | P2 |
| Removable filter chips | D | Chips | has (one chip per filter, Clear (N)) | S | P3 |
| Valve relay region picker (firewall) | D | Server picker | has, real game unverified (ADR 0016, Servers > Regions, off by default: Valve's relay list from `GetSDRConfig` cached a day, locations grouped by region with ping and health from a bounded, cancellable `ping` sweep, fastest open location, tick to block, region Block all and Allow all, presets saved, loaded and deleted, Apply and Reset as one elevated `netsh -f` batch of `Sigil-Relay-*` rules for `deadlock.exe` only, read back after each change with partial, stale, disabled and firewall-off warnings, a status bar mark on every page (shown even with the bar hidden) and a notice at every launch while any region is blocked; macOS and Linux list and ping but say blocking needs Windows. Round 2: search by city, country or id, sort by name, ping or your matches, shift-click range ticking, D's "Where I play" preset and per-location match counts from deadlock-api's `/v1/players/{id}/region` (only with Player and match stats on and a Stats account set), and a relay health chip in the status bar after a sweep (popover per region with best ping, Measure again, Open Regions; "Relay health in the status bar" keeps it there and measures at startup and every 30 minutes, off by default). Built and checked in headless scenes; not yet run on a Windows PC; the region history endpoint's datacenter codes are mapped as D does and not checked against a live account) | L | P3 |
| Favorites, recent servers, direct connect | - | Sigil extras | has (`.dah/servers.json`; favorites and recent scopes, address dialog that runs the same join check) | M | - |
| Maps view and Launch map | D | Custom map help for custom/Deadworks servers | has (`servers_maps.rs`: GameBanana Maps category, console name from the description or installed VPK, copyable `map <name>`, Launch writes a managed autoexec block with a backup and starts the game) | M | P3 |
| Custom maps page, How to play, Invite friends | D | Experimental custom-maps: Maps page, How to play This Map, Launch Map, Invite Friends (connect code from console.log) | has (2026-11-19, `screens/maps.rs`, `deadlock_core::maps`: off by default; map name from the addon VPK's `maps/<name>.vpk`; Launch map through the gameinfo gate; Host this map hands off to Servers > Host) | M | P3 |
| Host a server from this PC | - | Neither G nor D hosts (G's empty list only says "hosting requires Windows") | has, real game unverified (ADR 0015, Servers > Host: setup checklist for .NET 10, free space, SteamCMD, server files and the pinned Deadworks release; SteamCMD opens in its own window so the Steam sign-in never reaches Sigil; server settings with a map picker that lists stock and installed custom maps, Start and Stop, live console with a command line; plugins on and off; firewall rule behind Windows' admin prompt; LAN join, copyable `connect ip:port`, router forwarding steps, custom-map invite; "older than your game" update banner; no-forwarding hosting (ADR 0015 second amendment): Steam relay mode by default with the join code from the console, Direct mode, an opt-in UPnP switch, Sigil invites that carry GameBanana ids and a Join with invite flow that installs them, and "Play with friends, no setup" (the in-game host, D's Invite friends, with -condebug added for the launch); macOS and Linux show why it needs Windows. Built and checked in headless scenes; not yet run on a Windows PC. No player list beyond typing `status`) | L | P2 |

## Launch

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Launch modded / vanilla | G+D | Two buttons | has | - | - |
| Vanilla stash and restore | G | Move mods aside | has | - | - |
| Confirm dialogs | - | Sigil approval gate | has | - | - |
| Running state, stop game | G+D | Detect and stop | has (`launch.rs` monitors the process table, Stop game behind a confirm, `deadlock_core::launch::stop_game`) | S | P2 |
| Crash advisories from dumps | G+D | Likely culprit mod | has (`launch.rs` advisory: dump error text matched against shipped paths, else the newest install from before the crash; turn the mod off or dismiss) | M | P2 |
| Map mod launch warning | D | Ask to turn map mods off | has (wave 3, `launch_extras.rs`: map mods found from their VPK paths on a worker or the GameBanana Maps category (wave 4), a modded launch asks Turn them off and launch, Launch anyway, Don't ask again; Settings > Mods switch) | S | P3 |
| Launch checklist overlay | D | Steps before launch | has (wave 4: the gameinfo fix here and on the Game page runs `game_apply::apply_gameinfo_repair`, keeping other mounts, with `deploy::repair_gameinfo` as the fallback; `launch.rs`: gameinfo, overlaps and updates before a modded launch, with a fix per row and Launch anyway) | M | P3 |
| Discord rich presence | G+D | Optional presence | has (`settings/discord.rs`; launch and stop set the running state at once, so presence switches without waiting for the 30 s check; 2026-11-21, D `97c07548`: Hide party size leaves the count out of Sigil's own party lines, lines the player wrote show as written) | M | P3 |

## Overlap

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Pairs that touch the same files, winner | G+D | Report | has | - | - |
| Grouped by kind with search | G | HUD, models, sounds | has | - | - |
| Make X win (swap order), disable one | G+D | Resolve | has (`conflicts.rs`: pick a losing slot, then swap the two slot numbers through `load_order` with undo, or turn it off) | S | P1 |
| Ignore pair / file / mod with restore | G+D | Ignore lists | has (`deadlock_core::impact` ignore list in app data, restore list at the bottom of Overlap; wave 4: Ignore all with a confirm and one Undo, and a list per active profile started from the shared one, DMM's ignores per profile) | M | P2 |
| Sidebar badge with count | G | Badge | has (`conflicts::badge_count`, ignores excluded; mounting line in the wave report) | S | P3 |
| Explainer | G+D | How it works | has (`conflicts.rs`, two lines above the table; the resolve dialog repeats it) | S | P3 |
| Overlap badge on cards | G+D | Mark mods that share files | has (`installed/overlap.rs`: a quiet badge on enabled cards and rows from the scan's conflict rows, ignores excluded; its press lists the other mods, files shared and who wins, then Overlap; a hidden model keeps its own mark) | S | P2 |

## Downloads

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Install dialog with plan and approval | - | Sigil's gate | has | - | - |
| Queue with progress | G+D | Several downloads | has (`download_queue.rs` queue on the Downloads page: several archives, byte progress, states); Browse's install dialog still downloads inside its own job | M | P2 |
| Cancel a running download or install | G+D | Stop, Cancel all | has (wave 3: the X on a running row, Cancel all in the list; a busy Browse card cancels its queue row or its plan, and the install dialog's Cancel stops its download and extract through `PlanOptions.cancel`, shown as Cancelled) | S | P2 |
| Pause, resume, cancel, retry | D | Queue controls | has (`download_queue.rs`: pause keeps the part file, resume continues with Range, cancel removes it, retry starts again) | M | P3 |
| Resume after a restart | G+D | Partial file with its validator | has, real network unverified (the queue file follows the bytes every 5 s; a row cut off by a close comes back paused and marked, a one-press install keeps its "install when done" mark; the next start reads the queue file (no network) and a notice offers Resume, which continues each row from its part file with `Range` and `If-Range` (strong ETag or Last-Modified, so a changed file starts over), checks a resumed GameBanana file against its MD5 and hands one-press installs back to Browse so they still install) | M | P2 |
| Choose the file on a one-press install | D | File choice for mods with several files | has (a card's, row's or link's Install by mod id asks GameBanana for the file list first; with several current files the details open with the existing file menu instead of taking the newest; `sigil:` links without a file do the same; the details' own Install keeps taking the newest) | S | P2 |
| Settings files inside a mod | D | Staged review of gameinfo.gi, .cfg, video.txt | has (`mod_settings.rs`: an install, or a refusal of an archive that holds only settings, reads them from the unpacked archive and opens a review with what each would change: convars against your stock gameinfo.gi, video settings against your video.txt, cfg commands as new, replaces yours or already there; Review in Performance caches the gameinfo and its video.txt as "Config from a mod" and opens it, where Apply writes it as a tagged merge; Add to Autoexec puts the commands on the Autoexec page, where Save writes them. Nothing is copied into the game as it ships) | M | P2 |
| File server failover | G+D | Mirrors | has (`download_queue.rs` tries the GameBanana file servers in turn when the chosen host fails; wave 4: the running row and the Browse card name the server, as Grimoire's "Automatic · server", and say "switched from X" for 20 seconds after a failover) | M | P3 |
| Queue position | G | "Queued #2" | has (wave 4: a waiting Browse card says Queued #N, a waiting downloads row #N in line) | S | P3 |
| Slow download warning | G+D | Say why a download crawls | has (`downloads.rs`: the row's warning while it crawls, and one toast per download the first time, with Show downloads) | S | P2 |
| Suspicious archive warning | G | Scan before install | has (`download_queue.rs` scans a ZIP's entry names, unpacks .7z/.rar within limits; the warning comes before the install dialog) | S | P2 |

## Global shell

| Feature | Source | What it does | Sigil | Effort | Priority |
| --- | --- | --- | --- | --- | --- |
| Custom titlebar and window controls | D | Chrome | has | - | - |
| Per-page search in the titlebar | G+D | Search | has | - | - |
| Keyboard: Cmd+number screens, quit, close | D | Shortcuts | has | - | - |
| Update check | G+D | GitHub releases, opt-in at startup | has (Settings) | - | - |
| Zoom shortcuts, settings shortcut, open mods folder | D | Ctrl+=, Ctrl+, Ctrl+M | has (built: Cmd plus, minus and 0 step the zoom, Cmd+, opens Settings, Shift+Cmd+M opens the mods folder) | S | P2 |
| Toasts with undo | G+D | Notifications | has (`notify.rs`: bottom-right stack of 3, hover holds, Undo on slot toggle, crosshair apply, autoexec save, gameinfo fix; wave 4: one title one toast, a status line from the same action folds into its toast, errors stay until closed, reading time for the rest, Send feedback on every error toast and inline error box, issues #4 and #5) | M | P1 |
| Drag-and-drop install anywhere | G+D | Drop overlay, import review | has (built: a drop on any page without its own target opens the import review; Installed keeps its page target) | S | P1 |
| Deep links / 1-click install | G+D | `grimoire:` / `dlmm:` handler for GameBanana 1-Click | has: `sigil:` links open the mod with the file chosen and the Install dialog (see `deep-links.md`); GameBanana listing not requested yet | - | - |
| GameBanana Remote Install | D | Pair and poll a queue | missing: backlogged by the owner (2026-11-18) until GameBanana lists Sigil as a 1-Click manager; the protocol is in `deadlock_core::remote_install`, not wired | M | P3 |
| Status bar | D | Bottom bar with state | has (wave 3, `statusbar.rs`: game found, catalog sync, downloads, an app update, the active profile and the preview volume; counts cached once a second; Appearance switch; the background image can run under it and the volume control can carry art, both off by default) | S | P3 |
| Global preview volume | G | One volume for every preview | has (wave 3: the status bar's volume popover with mute moves the one value Browse's View menu shows; wave 4: a mute and the level before it survive a restart) | S | P3 |
| Per-page feature switches | D | Turn optional pages off | has (wave 3: General > Pages for Locker, Crosshair, Autoexec, Performance, Stats, Servers, and Mods only; a page that is off leaves the sidebar and its route) | S | P3 |
| Onboarding | G+D | Game path, gameinfo fix, autoexec | has (built: first-run setup mounted as an overlay, and Show setup again in Settings; three steps since 2026-11-25: game folder, gameinfo fix, privacy (update check and Help build Sigil). The wave 4 connection step lives in Settings > Network, existing mods in Installed, autoexec creation on the Autoexec page) | M | P2 |
| What's new dialog | D | Per-version notes | has (built: shown once per version after an update, from the release notes the update check already fetched) | S | P3 |
| gameinfo.gi banner with fix | G+D | Reset after updates | has (`banner.rs`: every page, Fix behind a confirm, rechecked on startup, focus, launch, rescan; wave 4: Fix keeps mounts another tool added (`game_apply::apply_gameinfo_repair`, falls back to the plain repair), and Grimoire's Hide until next launch per message) | S | P1 |
| Sidebar badges (installed, overlap counts) | G+D | Counts | has (built: Installed count and the overlap count in amber) | S | P3 |
| i18n | G+D | Languages | partial (General > Language: System, English, Deutsch; every routed UI string has a German entry, checked by tests. G and D ship more languages) | L | P3 |
| Theming | G+D | Themes and accent | has (Settings) | - | - |
| Tray | D | Close to tray | has (built: Windows and Linux tray, default close still quits; macOS rows explain no tray backend) | S | P3 |
| Single instance lock | G | One process | has (Rune pipe or socket; a lock and socket on macOS) | - | - |
| Feature hints / coachmarks | D | Usage hints | has (wave 4, `shell_tips.rs`: Did you know? cards anchored to the control with Show me, Got it, Turn off tips; one a session after the page settles, never over a dialog; two dismissals retire a tip; General > Show feature tips and Reset) | M | P3 |
| Collapsible sidebar | G+D | Icons only | has (wave 4, `shell_w4.rs`: Collapse sidebar at the foot, Cmd or Ctrl+B, tooltips and warning dots while collapsed, kept across launches) | S | P2 |
| App update banner | G | Install or view, hide until next launch; download progress; package manager note | has (wave 4: a banner over every page while an update is out, Install or View update, an X that hides it for the run; 2026-11-19: bytes, percent and a bar while the update downloads, here and in About > Updates; a winget, Scoop, Chocolatey, Homebrew, Nix, Snap, Flatpak or system-package copy says updates come from that manager and the installer is off) | S | P3 |
| "New" marks on pages | D | Until first opened | has (wave 4: Stats, Servers and Performance carry New until opened once, kept in options.json) | S | P3 |
| Experimental tour | (neither) | Reopenable walkthrough | has (wave 4: Settings > Experimental > Take the tour, once on the first visit; each stop has its switch) | S | P3 |
| Easter eggs | G | Rare jokes | has, off by default (wave 4: five knocks on the wordmark open the news computer, 7 percent joke tooltips, 5 percent window title; Appearance > Easter eggs) | S | P3 |
| Seasonal themes | D | Holiday looks by date | has, off by default (wave 4: DMM's five windows and overlap rule, a seasonal mark in the sidebar with a small toy each, Turn off until next time, and Seasonal effects (snow, sparks, lanterns, petals; 28 particles stepped at 15 frames a second, off by default); Sigil's own look, no DMM art) | M | P3 |
| Analytics | D | Consent-gated usage events | has (wave 3, ADR 0014, `settings/telemetry.rs`: a first-start question, Privacy switches for usage events, crash reports and unknown-mod lookup, Send diagnostics with a preview and a reference code; Sigil's own Worker, not Google Analytics) | M | P3 |
| Unknown-mod hash lookup | D | Name a VPK from a hash index | has (wave 3: Identify's Look it up through `telemetry::identify` with lookup consent; GameBanana installs add their hash) | S | P3 |
| Announcements, feature flags | D | Needs D's backend | deferred: Sigil accounts (see below) | - | - |

## Next, by priority

Everything in waves 1 to 4 and the 0.1.22 upstream round shipped. What is left:

1. Browse: GameBanana Remote Install (backlogged until GameBanana lists Sigil as a 1-Click
   manager; the protocol is in `deadlock_core::remote_install`, not wired), custom mod sources
   (another builder), and the unbuilt browse leftovers in `grimoire-unreleased-2026-11-21.md`.
2. Installed: Issues button pulse, attaching local files to a GameBanana card, Add files and Add
   images in the variant picker, floating preview panels, and GameBanana schema tag rules. All are
   listed with issue links and effort in `grimoire-unreleased-2026-11-21.md`.
3. Locker: the skin to card icon link, the cursor cropper, Locker cards' hover 3D and saved angle,
   and the empty-hero suggestions on the Hero Skins page. Locker ability recolor waits on the
   owner's Foundry decision.
4. Viewer: motion previews against a second hero (issue #8), custom animations (issue #9), and the
   Windows checks for texture-only skins, partial mods and the real Hideout.
5. Performance: GitHub presets' per-setting switches in the UI, and keeping an author's gameinfo
   comments and formatting.
6. Imports: local imports following "Enable mods after download" once the scan is clear, and the
   gallery and tags in the import dialog.
7. Distribution: Flatpak (issue #16) and the Linux runtime checks on a distro.
