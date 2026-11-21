//! Deadlock domain core: game discovery, VPK reading, catalog indexes, project
//! manifests, patch notes and change impact. No UI, no agent runtime.

pub mod util;

pub mod addon_checks;
pub mod addons;
pub mod autoexec;
pub mod benchmark;
pub mod bisect;
pub mod backups;
pub mod crosshair;
pub mod cursors;
pub mod cursors_create;
pub mod demo;
pub mod diagnostics;
pub mod discovery;
pub mod divine_skins;
pub mod dmm_stamp;
pub mod duplicates;
pub mod download_queue;
pub mod error;
pub mod explorer;
pub mod extract;
pub mod folder_repair;
pub mod fonts;
pub mod forge_bridge;
pub mod gamebanana;
pub mod host;
pub mod game_catalog;
pub mod game_assets;
pub mod game_index;
pub mod game_apply;
pub mod game_plans;
pub mod import;
pub mod identify;
pub mod interchange;
pub mod impact;
pub mod install;
pub mod launch;
pub mod library_facts;
pub mod load_order;
pub mod maps;
pub mod locker;
pub mod locker_parts;
pub mod obsolete_commands;
pub mod overflow;
pub mod patch_notes;
pub mod performance;
pub mod payload;
pub mod profile_favorites;
pub mod profiles;
pub mod project;
pub mod project_build;
pub mod recolor;
pub mod relays;
pub mod release;
pub mod remote_install;
pub mod review;
pub mod servers;
pub mod catalog_mirror;
pub mod character;
pub mod cleanup;
pub mod compat_repair;
pub mod deploy;
pub mod migrate;
pub mod mod_updates;
pub mod mod_data;
pub mod mod_extras;
pub mod mod_health;
pub mod mod_heroes;
pub mod mod_kinds;
pub mod mod_lookup;
pub mod mod_merge;
pub mod mod_safety;
pub mod mod_settings;
pub mod game_session;
pub mod live_bridge;
pub mod live_match;
pub mod fast_download;
pub mod mod_sources;
pub mod file_servers;
pub mod requirements;
pub mod net;
pub mod salts;
pub mod steam_users;
pub mod stats;
pub mod steam_launch;
pub mod subscription;
pub mod telemetry;
pub mod toolchain;
pub mod trash;
pub mod viewer;
pub mod vpk;
pub mod vpk_write;

pub use addons::{
    AddonConflict, ConflictReport, InstalledAddon, find_conflicts, list_installed,
    read_conflict_report,
};
pub use autoexec::{
    AUTOEXEC_FILE_NAME, AUTOEXEC_RELATIVE_PATH, AutoexecWriteResult, autoexec_path, list_lines,
    normalize_trailing_newline, read_autoexec, write_autoexec,
};
pub use discovery::{
    AddonSlot, AddonsStatus, DEADLOCK_APP_ID, DeadlockInstall, GameInfoStatus, GameStatus,
    InstallDiscovery, InstallDiscoveryOptions, default_steam_roots, default_steam_roots_for,
    discover_deadlock_install, format_game_status, format_slot_file_name, parse_library_folders,
    read_addons_status, read_game_info_status, read_game_status,
};
pub use error::{Error, Result};
pub use extract::{
    ExtractIssue, ExtractOptions, ExtractResult, ExtractedFile, extract_from_vpk,
    format_extract_result,
};
pub use gamebanana::{
    BrowseMod, BrowsePage, GAMEBANANA_API_BASE, GAMEBANANA_GAME_ID, GAMEBANANA_MAX_PERPAGE,
    GAMEBANANA_USER_AGENT, fetch_deadlock_mods, fetch_deadlock_mods_page, format_browse_mods,
    parse_subfeed,
};
pub use game_catalog::{
    CatalogEntry, CatalogError, CatalogSource, GameCatalog, GameVpkDescriptor,
    LoadGameCatalogOptions, LoadGameCatalogResult, SearchCatalogOptions, SearchCatalogResult,
    SearchGameCatalogOptions, SearchGameCatalogResult, VpkKind, catalog_index_path,
    catalog_sources_by_id, discover_game_vpks, format_catalog_search_results,
    format_catalog_summary, load_game_catalog, search_game_catalog,
};
pub use game_index::{
    ExtensionSummary, GAME_INDEX_SCHEMA, GameIndex, GameIndexSource, HeroSummary, IndexChange,
    IndexDiff, IndexEntry, IndexLoadMeta, IndexMove, LoadGameIndexOptions, LoadGameIndexResult,
    SearchGameIndexOptions, SearchGameIndexResult, SearchOptions, SearchResult,
    default_game_index_path, diff_game_index, format_index_summary, format_index_summary_result,
    format_search_results, game_vpk_path, list_heroes, load_game_index, search_game_index,
    top_extensions,
};
pub use launch::{
    LaunchResult, MODE_MODDED, MODE_VANILLA, STASH_DIR_NAME, STASH_MANIFEST_NAME, STASH_SCHEMA,
    STEAM_RUN_URL, RestoreResult, StashFile, StashManifest, StashResult, StashStatus,
    launch_modded, launch_vanilla, restore_vanilla, stash_manifest_path, stash_status,
    stash_vanilla, vanilla_stash_dir,
};
pub use locker::{
    LOCKER_ASSETS_ENV, LockerAssetPaths, LockerHero, discover_heroes, discover_heroes_with_assets,
    display_name_for, hero_asset_key, read_hero_asset_index, resolve_name_art, resolve_render,
};
pub use overflow::{
    OverflowPlan, OverflowSlot, folder_name as overflow_folder_name, gameinfo_with_overflow,
    overflow_folders_in, plan as plan_overflow,
};
pub use patch_notes::{
    DEADLOCK_NEWS_URL, PATCH_NOTES_CACHE_SCHEMA, PatchNote, PatchNoteCache, cache_from_notes,
    detect_is_patch, fetch_patch_notes, format_patch_note, format_patch_notes,
    load_patch_note_cache, new_since_cache, patch_note_cache_path, save_patch_note_cache,
    strip_patch_note_html,
};
pub use profiles::{
    PROFILE_SCHEMA, Profile, ProfileApplyResult, ProfileSlot, apply_profile, capture_profile,
    find_profile, list_profiles, profile_dir, profile_path, read_profile, write_profile,
};
pub use project::{
    BASELINE_SCHEMA, BaselineEntry, ModProject, PROJECT_SCHEMA, ProjectBaseline, ProjectCheck,
    ProjectHistoryEntry, TargetCheck, TargetStatus, baseline_path, check_project, create_project,
    format_project_check, project_path, read_baseline, read_project, write_baseline, write_project,
};
pub use telemetry::{Consent, ENDPOINT, EVENTS, HashMatch};
pub use vpk::{
    ReadVpkOptions, VPK_DIRECTORY_ARCHIVE, VPK_MAGIC, VpkDirectory, VpkEntry, VpkHeader,
    chunk_path_for, crc32, is_safe_vpk_path, read_vpk_directory, read_vpk_directory_with,
    read_vpk_entry_data, try_chunk_path_for,
};
