# Core integration notes

`dah-rs` calls `deadlock-core` only through `src/domain.rs`. Nothing else in the
crate names a core type or function. When the core API moves, adjust that file.

## Surface in use

| Area | Items |
| --- | --- |
| `discovery` | `read_game_status(None)`, `GameStatus`, `DeadlockInstall`, `GameInfoStatus` |
| `game_catalog` | `load_game_catalog`, `search_game_catalog`, `discover_game_vpks`, `catalog_index_path`, `GameCatalog`, `CatalogEntry`, `CatalogSource`, `CatalogError` |
| `game_index` | `diff_game_index`, `IndexDiff` |
| `extract` | `extract_from_vpk`, `ExtractOptions`, `ExtractResult` |
| `patch_notes` | `fetch_patch_notes`, `load_patch_note_cache`, `save_patch_note_cache`, `cache_from_notes`, `strip_patch_note_html`, `PatchNote`, `PatchNoteCache` |
| `project` | `create_project`, `read_project`, `write_project`, `write_baseline`, `check_project`, `ModProject`, `ProjectBaseline`, `ProjectCheck`, `ProjectHistoryEntry`, `TargetStatus` |

Options structs are passed by value. `read_game_status(None)` lets the core read
`DAH_GAME_DIR`.

## State owned by the CLI

| Path | Contents |
| --- | --- |
| `<workspace>/.dah/index/` | catalog cache, one JSON per VPK source |
| `<workspace>/.dah/patches.json` | `PatchNoteCache` for the Steam news feed |
| `<workspace>/mods/<slug>/` | project directory created by `dah mod new` |
| `<project>/project.json` | manifest owned by the core |
| `<project>/.dah/baseline.json` | baseline owned by the core |

The patches cache keeps the core `PatchNoteCache` shape at the path the product
specifies. Freshness is a six hour TTL; `--refresh` bypasses it. A failed fetch
falls back to the cached copy with a warning.

## Exit codes

- `0`: success, or a check with no incompatibilities.
- `1`: runtime error, or `dah mod check` and `dah extract` reported failures.
- `2`: clap usage error.

`--json` prints one object on stdout. Errors print `{"ok": false, "error": {...}}`
on stderr.
