# Core integration notes

`dah-rs` calls `deadlock-core` only through `src/domain.rs`, with one documented
exception: `src/commands/browse.rs` calls the read-only GameBanana browse API
directly (see "GameBanana browse"). Nothing else in the crate names a core type
or function. When the core API moves, adjust `domain.rs` first.

## Surface in use

| Area | Items |
| --- | --- |
| `discovery` | `read_game_status(None)`, `GameStatus`, `DeadlockInstall`, `GameInfoStatus`, `AddonSlot` |
| `game_catalog` | `load_game_catalog`, `search_game_catalog`, `discover_game_vpks`, `catalog_index_path`, `GameCatalog`, `CatalogEntry`, `CatalogSource`, `CatalogError` |
| `game_index` | `diff_game_index`, `IndexDiff` |
| `extract` | `extract_from_vpk`, `ExtractOptions`, `ExtractResult` |
| `gamebanana` | `fetch_deadlock_mods`, `fetch_deadlock_mods_page`, `parse_subfeed`, `format_browse_mods`, `BrowseMod`, `BrowsePage`, `GAMEBANANA_GAME_ID`, `GAMEBANANA_MAX_PERPAGE` |
| `patch_notes` | `fetch_patch_notes`, `load_patch_note_cache`, `save_patch_note_cache`, `cache_from_notes`, `strip_patch_note_html`, `PatchNote`, `PatchNoteCache` |
| `project` | `create_project`, `read_project`, `write_project`, `read_baseline`, `baseline_path`, `write_baseline`, `check_project`, `ModProject`, `ProjectBaseline`, `ProjectCheck`, `ProjectHistoryEntry`, `TargetStatus` |
| `payload` | `validate_payload`, `has_errors`, `format_payload_issues`, `PayloadIssue`, `PayloadOptions`, `Severity` |
| `vpk_write` | `collect_staging_entries`, `pack_vpk`, `PackOptions`, `PackedVpk` |
| `release` | `latest_release`, `release_dir`, `write_release_manifest`, `ReleaseManifest`, `RELEASE_SCHEMA`, `RELEASE_MANIFEST_FILE` |
| `deploy` | `plan_deploy(game_root, release_dir, manifest, slot)`, `apply_deploy(game_root, release_dir, manifest, plan, approve, backup_root)`, `DeployPlan`, `DeployConflict`, `DeployResult` |
| `migrate` | `plan_migrations(project, catalog) -> Vec<Migration>`, `apply_migrations(project_dir, project, migrations)`, `Migration` |
| `addons` | `list_installed`, `read_conflict_report`, `find_conflicts`, `set_enabled(game_root, slot, enabled, approve)`, `InstalledAddon`, `AddonConflict`, `ConflictReport`, `AddonToggleResult` |

Options structs are passed by value except `PayloadOptions` and `PackOptions`,
which the wrappers build by reference. `read_game_status(None)` lets the core
read `DAH_GAME_DIR`.

`plan_migrations` returns a plain `Vec`, not a `Result`: an empty list means no
basename uniquely matched a live catalog path.

## Packaging wrappers

`domain.rs` holds the only field access for the newer core types:

- `staging_dir`, `staging_payload_options` and `staging_entries` define the
  `<project>/staging` tree and the `staging` prefix that payload paths must not
  repeat.
- `resolve_slot` applies `--slot`, else `project.slot`, else `01`.
- `pack_staging` packs with base name `pakNN` and a 2 GiB chunk cap.
- `build_release_manifest` fills `entries`, `total_bytes`, `vpk_file`,
  `chunk_files` and `baseline_captured_at` from the pack and the baseline.
- `describe_deploy_plan` flattens the plan so commands print conflicts, the
  gameinfo requirement, the replace requirement and core warnings.

## GameBanana browse

`dah browse` reads the public Deadlock subfeed without a key, a login or a
download. The game id is `20948` (`GAMEBANANA_GAME_ID`).

```text
GET https://gamebanana.com/apiv11/Game/20948/Subfeed?_nPage=1&perpage=15&sort=default
User-Agent: dah/0.0.0
```

Paging details that matter:

- The page key is `_nPage`. The endpoint ignores `page`, which silently repeats
  page 1, so the core never sends it.
- A subfeed page is always 15 records. `perpage` is accepted but ignored, so
  `--limit` truncates the mod list locally. `--limit` is capped at 50 and
  `--page` starts at 1.
- `_aMetadata._nRecordCount` is the whole feed count (mods plus Sound, Tutorial
  and Wiki rows). `_aMetadata._bIsComplete` is false while more pages exist.
- Only rows with `_sModelName == "Mod"` become `BrowseMod`. Sound, Tutorial and
  Wiki rows are dropped. A 15 row page can therefore yield 14 mods.
- `thumbnailUrl` is `_aPreviewMedia._aImages[0]._sBaseUrl` joined with
  `_sFile530`, else `_sFile220`. Images without either variant are skipped.
- `updatedAt` is `_tsDateModified` as ISO 8601, falling back to `_tsDateAdded`.
  `version` is `None` when `_sVersion` is absent or blank.

Failures are `Error::Network` with the URL in the message. A non-JSON body
reports the parser error and the first 240 characters of the body, so an HTML
error page is visible instead of a bare parse error. A JSON error object and a
missing `_aRecords` array also report the body snippet.

The CLI surface:

```bash
dah browse                    # page 1, 15 records
dah browse --page 2 --limit 5 # first 5 mods of feed page 2
dah browse --json             # machine-readable
```

`--json` prints one object:

```json
{
  "ok": true,
  "source": "gamebanana:20948",
  "page": 1,
  "perPage": 15,
  "pageCount": 843,
  "total": 12639,
  "complete": false,
  "returned": 3,
  "items": [
    {
      "id": 723495,
      "name": "Suisei as Sinclair",
      "profileUrl": "https://gamebanana.com/mods/723495",
      "model": "Mod",
      "author": "Plariou",
      "category": "Skins",
      "subcategory": "Sinclair",
      "version": "hamburger 20",
      "updatedAt": "2026-11-16T03:34:10.000Z",
      "likes": 15,
      "views": 7052,
      "hasFiles": true,
      "thumbnailUrl": "https://images.gamebanana.com/img/ss/mods/530-90_6af13234b5c13.jpg"
    }
  ]
}
```

`perPage` is the feed stride (15), not the requested `--limit`. `returned` is
the number of items after the cap and after non-mod rows are dropped.

`commands/browse.rs` calls `deadlock_core::gamebanana` directly instead of going
through `domain.rs`. Browse is a stateless read with no adapter fields to
flatten, and the packaging seam in `domain.rs` stays untouched. If the browse
signatures move, that command is the second file to adjust after the core.

## State owned by the CLI

| Path | Contents |
| --- | --- |
| `<workspace>/.dah/index/` | catalog cache, one JSON per VPK source |
| `<workspace>/.dah/patches.json` | `PatchNoteCache` for the Steam news feed |
| `<workspace>/mods/<slug>/` | project directory created by `dah mod new` |
| `<project>/project.json` | manifest owned by the core |
| `<project>/staging/` | compiled payload, laid out as game paths |
| `<project>/.dah/baseline.json` | baseline owned by the core |
| `<project>/builds/<version>/` | packed VPK, chunks and `release.json` |
| `<project>/.dah/backups/` | deploy backups, one folder per approved run |
| `<project>/publish/<version>/` | publish folder from `dah publish prepare` |

`dah mod release` refuses an existing non-empty `builds/<version>` so chunks from
an older pack cannot mix into a new manifest. Bump the version first.

`dah watch` only reads project files. It may refresh the catalog cache under
`<workspace>/.dah/index/`, which is CLI state, not project state.

## Exit codes

- `0`: success, or a check with no incompatibilities.
- `1`: runtime error, an invalid payload, an unsafe deploy, or `dah extract`
  reported failures.
- `2`: clap usage error.

`--json` prints one object on stdout. Errors print `{"ok": false, "error": {...}}`
on stderr. `dah mod validate` exits `1` on payload errors and `0` with warnings
only.

## Write boundaries

- `dah mod pack`, `dah mod release`, `dah mod migrate --apply`,
  `dah publish prepare` and `dah mod deploy --approve` write inside the project
  or its chosen output folder.
- `dah mod deploy` without `--approve` calls `plan_deploy` only and prints the
  plan. No addon file, `gameinfo.gi` or backup is written.
- `dah mod deploy --approve` is the approval gate: it copies the release VPK and
  chunks into `game/citadel/addons`, repairs the addons SearchPaths line when the
  plan asks for it, and backs up replaced files under `.dah/backups`.
- `dah publish prepare` copies files and writes `metadata.json` plus `README.md`.
  It never uploads.
- `dah browse` is read only: one HTTP GET, no file writes, no downloads.
