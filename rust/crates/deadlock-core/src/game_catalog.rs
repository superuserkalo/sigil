//! The game ships more than one VPK worth knowing about: the main citadel pak,
//! shared engine files in core, and one directory file per map. The catalog
//! indexes them together and remembers which VPK owns each path.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::game_index::{
    GameIndex, IndexEntry, IndexLoadMeta, LoadGameIndexOptions, LoadGameIndexResult,
    SearchGameIndexOptions, load_game_index,
};
use crate::util::{escape_regex, now_iso};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VpkKind {
    Citadel,
    Core,
    Map,
}

#[derive(Debug, Clone)]
pub struct GameVpkDescriptor {
    pub id: String,
    pub path: PathBuf,
    pub kind: VpkKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSource {
    pub id: String,
    pub path: String,
    pub kind: VpkKind,
    pub entries: usize,
    pub size: u64,
    pub mtime_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub path: String,
    pub crc32: u32,
    pub preload_bytes: u16,
    pub archive_index: u16,
    pub offset: u32,
    pub length: u32,
    /// Source VPK id: `citadel`, `core`, or `maps/<name>`.
    pub vpk: String,
}

impl CatalogEntry {
    pub fn from_index_entry(entry: &IndexEntry, vpk: &str) -> Self {
        Self {
            path: entry.path.clone(),
            crc32: entry.crc32,
            preload_bytes: entry.preload_bytes,
            archive_index: entry.archive_index,
            offset: entry.offset,
            length: entry.length,
            vpk: vpk.to_string(),
        }
    }

    pub fn total_length(&self) -> u32 {
        u32::from(self.preload_bytes).saturating_add(self.length)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameCatalog {
    pub schema: u32,
    pub built_at: String,
    pub sources: Vec<CatalogSource>,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Debug, Clone)]
pub struct LoadGameCatalogOptions {
    pub game_root: PathBuf,
    pub cache_dir: PathBuf,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogError {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LoadGameCatalogResult {
    pub catalog: GameCatalog,
    pub indexes: BTreeMap<String, GameIndex>,
    pub previous: BTreeMap<String, GameIndex>,
    /// Per-source cache state, keyed by VPK id.
    pub meta: BTreeMap<String, IndexLoadMeta>,
    pub errors: Vec<CatalogError>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchGameCatalogOptions {
    pub query: Option<String>,
    pub hero: Option<String>,
    pub extension: Option<String>,
    pub folder: Option<String>,
    /// Restrict to one source, for example `core` or `maps/city`.
    pub vpk: Option<String>,
    pub limit: Option<usize>,
}

/// Stable alias for callers that use the catalog-specific name.
pub type SearchCatalogOptions = SearchGameCatalogOptions;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchGameCatalogResult {
    pub total: usize,
    pub entries: Vec<CatalogEntry>,
    pub truncated: bool,
}

/// Stable alias for callers that use the catalog-specific name.
pub type SearchCatalogResult = SearchGameCatalogResult;

impl From<SearchGameIndexOptions> for SearchGameCatalogOptions {
    fn from(value: SearchGameIndexOptions) -> Self {
        Self {
            query: value.query,
            hero: value.hero,
            extension: value.extension,
            folder: value.folder,
            vpk: None,
            limit: value.limit,
        }
    }
}

pub fn discover_game_vpks(game_root: &Path) -> Result<Vec<GameVpkDescriptor>> {
    let mut descriptors = Vec::new();

    let candidates = [
        (
            "citadel",
            game_root.join("game").join("citadel").join("pak01_dir.vpk"),
            VpkKind::Citadel,
        ),
        (
            "core",
            game_root.join("game").join("core").join("pak01_dir.vpk"),
            VpkKind::Core,
        ),
    ];

    for (id, path, kind) in candidates {
        if is_file(&path) {
            descriptors.push(GameVpkDescriptor {
                id: id.to_string(),
                path,
                kind,
            });
        }
    }

    let maps_dir = game_root.join("game").join("citadel").join("maps");
    let mut map_files = Vec::new();

    if let Ok(read_dir) = fs::read_dir(&maps_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !is_file(&path) {
                continue;
            }

            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(map_name) = strip_directory_suffix(&name) else {
                continue;
            };

            map_files.push((map_name, name, path));
        }
    }

    map_files.sort_by(|left, right| left.1.cmp(&right.1));

    for (map_name, _, path) in map_files {
        descriptors.push(GameVpkDescriptor {
            id: format!("maps/{map_name}"),
            path,
            kind: VpkKind::Map,
        });
    }

    Ok(descriptors)
}

pub fn load_game_catalog(options: LoadGameCatalogOptions) -> Result<LoadGameCatalogResult> {
    let descriptors = discover_game_vpks(&options.game_root)?;

    if descriptors.is_empty() {
        return Err(Error::msg(format!(
            "No VPK files found under {}/game.",
            options.game_root.display()
        )));
    }

    let mut indexes = BTreeMap::new();
    let mut previous = BTreeMap::new();
    let mut meta = BTreeMap::new();
    let mut sources = Vec::new();
    let mut errors = Vec::new();

    for descriptor in descriptors {
        let loaded: LoadGameIndexResult = match load_game_index(LoadGameIndexOptions {
            dir_vpk_path: descriptor.path.clone(),
            cache_path: Some(catalog_index_path(&options.cache_dir, &descriptor.id)),
            force: options.force,
        }) {
            Ok(loaded) => loaded,
            Err(error) if descriptor.kind == VpkKind::Citadel => return Err(error),
            Err(error) => {
                errors.push(CatalogError {
                    id: descriptor.id,
                    message: error.to_string(),
                });
                continue;
            }
        };

        sources.push(CatalogSource {
            id: descriptor.id.clone(),
            path: descriptor.path.to_string_lossy().into_owned(),
            kind: descriptor.kind,
            entries: loaded.index.entries.len(),
            size: loaded.index.source.size,
            mtime_ms: loaded.index.source.mtime_ms,
        });

        meta.insert(
            descriptor.id.clone(),
            IndexLoadMeta::from_result(&loaded),
        );

        if let Some(previous_index) = loaded.previous {
            previous.insert(descriptor.id.clone(), previous_index);
        }

        indexes.insert(descriptor.id, loaded.index);
    }

    let mut entries = Vec::new();

    for (vpk, index) in &indexes {
        for entry in &index.entries {
            entries.push(CatalogEntry::from_index_entry(entry, vpk));
        }
    }

    Ok(LoadGameCatalogResult {
        catalog: GameCatalog {
            schema: 1,
            built_at: now_iso(),
            sources,
            entries,
        },
        indexes,
        previous,
        meta,
        errors,
    })
}

pub fn search_game_catalog(
    catalog: &GameCatalog,
    options: SearchCatalogOptions,
) -> SearchCatalogResult {
    let query = options
        .query
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_lowercase();
    let tokens: Vec<&str> = if query.is_empty() {
        Vec::new()
    } else {
        query.split_whitespace().collect()
    };
    let hero = options
        .hero
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);
    let extension = normalize_extension(options.extension.as_deref());
    let folder = options
        .folder
        .as_deref()
        .map(|value| value.trim().trim_matches('/').to_lowercase());
    let vpk = options
        .vpk
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let limit = clamp_limit(options.limit, 1000);

    let hero_pattern = hero.as_ref().map(|value| {
        Regex::new(&format!("(^|/)heroes[^/]*/{}(/|$)", escape_regex(value)))
            .expect("escaped hero pattern is valid")
    });

    let mut matches: Vec<&CatalogEntry> = Vec::new();

    for entry in &catalog.entries {
        if let Some(vpk) = vpk {
            if entry.vpk != vpk {
                continue;
            }
        }

        let path = entry.path.to_lowercase();

        if tokens.iter().any(|token| !path.contains(token)) {
            continue;
        }

        if let Some(pattern) = &hero_pattern {
            if !pattern.is_match(&path) {
                continue;
            }
        }

        if let Some(extension) = &extension {
            if !path.ends_with(&format!(".{extension}")) {
                continue;
            }
        }

        if let Some(folder) = &folder {
            if !folder.is_empty() && !path.starts_with(&format!("{folder}/")) {
                continue;
            }
        }

        matches.push(entry);
    }

    matches.sort_by(|left, right| compare_matches(left, right, &query));

    let total = matches.len();
    let entries = matches.into_iter().take(limit).cloned().collect();

    SearchGameCatalogResult {
        total,
        entries,
        truncated: total > limit,
    }
}

pub fn format_catalog_summary(catalog: &GameCatalog, errors: &[CatalogError]) -> String {
    let total: usize = catalog.sources.iter().map(|source| source.entries).sum();
    let mut lines = vec![format!(
        "Game catalog: {} VPK(s), {} entries",
        catalog.sources.len(),
        total
    )];

    for source in &catalog.sources {
        lines.push(format!(
            "  {}: {} entries ({})",
            source.id, source.entries, source.path
        ));
    }

    for error in errors {
        lines.push(format!("  {}: skipped ({})", error.id, error.message));
    }

    lines.join("\n")
}

pub fn format_catalog_search_results(
    result: &SearchGameCatalogResult,
    options: &SearchGameCatalogOptions,
) -> String {
    if result.total == 0 {
        return "No matching paths in the game catalog.".to_string();
    }

    let mut filters = Vec::new();

    if let Some(query) = options.query.as_deref().filter(|value| !value.is_empty()) {
        filters.push(format!("query \"{query}\""));
    }
    if let Some(hero) = &options.hero {
        filters.push(format!("hero {hero}"));
    }
    if let Some(extension) = &options.extension {
        filters.push(format!("type {extension}"));
    }
    if let Some(folder) = &options.folder {
        filters.push(format!("folder {folder}"));
    }
    if let Some(vpk) = &options.vpk {
        filters.push(format!("vpk {vpk}"));
    }

    let mut lines = vec![if filters.is_empty() {
        format!("Showing {} of {} paths", result.entries.len(), result.total)
    } else {
        format!(
            "Showing {} of {} paths matching {}",
            result.entries.len(),
            result.total,
            filters.join(", ")
        )
    }];

    for entry in &result.entries {
        lines.push(format!(
            "{}: {} ({} bytes)",
            entry.vpk,
            entry.path,
            entry.total_length()
        ));
    }

    if result.truncated {
        lines.push("Output truncated. Narrow the query or raise the limit.".to_string());
    }

    lines.join("\n")
}

pub fn catalog_index_path(cache_dir: &Path, vpk: &str) -> PathBuf {
    if vpk == "citadel" {
        return cache_dir.join("citadel-pak01.json");
    }

    let mut sanitized = String::new();
    let mut last_was_separator = false;

    for character in vpk.chars() {
        if character.is_ascii_alphanumeric() {
            sanitized.push(character.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('-');
            last_was_separator = true;
        }
    }

    cache_dir.join(format!("{sanitized}.json"))
}

fn strip_directory_suffix(name: &str) -> Option<String> {
    let lowered = name.to_ascii_lowercase();
    if !lowered.ends_with("_dir.vpk") {
        return None;
    }

    let stem = &name[..name.len() - "_dir.vpk".len()];
    (!stem.is_empty()).then(|| stem.to_string())
}

fn is_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn normalize_extension(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    let normalized = value.strip_prefix('.').unwrap_or(&value);
    (!normalized.is_empty()).then(|| normalized.to_string())
}

fn clamp_limit(value: Option<usize>, maximum: usize) -> usize {
    value.unwrap_or(50).clamp(1, maximum)
}

fn compare_matches(left: &CatalogEntry, right: &CatalogEntry, query: &str) -> std::cmp::Ordering {
    if !query.is_empty() {
        let score = score_path(&left.path, query).cmp(&score_path(&right.path, query));
        if score != std::cmp::Ordering::Equal {
            return score;
        }
    }

    left.path
        .len()
        .cmp(&right.path.len())
        .then_with(|| left.path.to_lowercase().cmp(&right.path.to_lowercase()))
}

fn score_path(path: &str, query: &str) -> u8 {
    let lowered = path.to_lowercase();
    let name = lowered.rsplit('/').next().unwrap_or(&lowered);

    if name == query {
        0
    } else if lowered.ends_with(&format!("/{query}")) {
        1
    } else if name.starts_with(query) {
        2
    } else {
        3
    }
}

/// Group catalog entries by source. Useful for a project baseline or UI table.
pub fn catalog_sources_by_id(catalog: &GameCatalog) -> HashMap<&str, &CatalogSource> {
    catalog
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source))
        .collect()
}
