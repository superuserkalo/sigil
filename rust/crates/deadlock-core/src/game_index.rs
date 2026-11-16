//! A cached, searchable list of the paths inside one game VPK.
//!
//! The cache stores metadata only. Extraction re-reads the VPK tree so a cache
//! edited on disk is never trusted for bytes.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util::{basename, escape_regex, now_iso, write_atomic};
use crate::vpk::{ReadVpkOptions, VpkHeader, read_vpk_directory_with};

pub const GAME_INDEX_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    pub path: String,
    pub crc32: u32,
    pub preload_bytes: u16,
    pub archive_index: u16,
    pub offset: u32,
    pub length: u32,
}

impl IndexEntry {
    pub fn total_length(&self) -> u32 {
        u32::from(self.preload_bytes).saturating_add(self.length)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameIndexSource {
    pub path: String,
    pub size: u64,
    pub mtime_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameIndex {
    pub schema: u32,
    pub source: GameIndexSource,
    pub built_at: String,
    pub header: VpkHeader,
    pub entries: Vec<IndexEntry>,
    pub skipped: u64,
    pub duplicates: u64,
}

#[derive(Debug, Clone)]
pub struct LoadGameIndexOptions {
    pub dir_vpk_path: PathBuf,
    /// Omit to index without a cache.
    pub cache_path: Option<PathBuf>,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct LoadGameIndexResult {
    pub index: GameIndex,
    pub from_cache: bool,
    pub cache_path: Option<PathBuf>,
    pub cache_error: Option<String>,
    /// The previous cache contents when the cache was rebuilt. Used for diffs.
    pub previous: Option<GameIndex>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchGameIndexOptions {
    pub query: Option<String>,
    /// Hero codename, for example `billy` or `punkgoat`.
    pub hero: Option<String>,
    /// Compiled extension, with or without a leading dot, for example `vmdl_c`.
    pub extension: Option<String>,
    /// Path prefix, for example `models/heroes_wip`.
    pub folder: Option<String>,
    pub limit: Option<usize>,
}

/// Stable alias for callers that use the shorter name.
pub type SearchOptions = SearchGameIndexOptions;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchGameIndexResult {
    pub total: usize,
    pub entries: Vec<IndexEntry>,
    pub truncated: bool,
}

/// Stable alias for callers that use the shorter name.
pub type SearchResult = SearchGameIndexResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HeroSummary {
    pub codename: String,
    pub entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSummary {
    pub extension: String,
    pub entries: usize,
}

#[derive(Debug, Clone, Default)]
pub struct IndexLoadMeta {
    pub from_cache: bool,
    pub cache_path: Option<PathBuf>,
    pub cache_error: Option<String>,
}

impl IndexLoadMeta {
    pub fn from_result(result: &LoadGameIndexResult) -> Self {
        Self {
            from_cache: result.from_cache,
            cache_path: result.cache_path.clone(),
            cache_error: result.cache_error.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexChange {
    pub path: String,
    pub previous: IndexEntry,
    pub current: IndexEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexMove {
    pub from: String,
    pub to: String,
    pub entry: IndexEntry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDiff {
    pub added: Vec<IndexEntry>,
    pub removed: Vec<IndexEntry>,
    pub changed: Vec<IndexChange>,
    pub moved: Vec<IndexMove>,
}

impl IndexDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.moved.is_empty()
    }
}

pub fn default_game_index_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("citadel-pak01.json")
}

pub fn game_vpk_path(game_root: &Path) -> PathBuf {
    game_root.join("game").join("citadel").join("pak01_dir.vpk")
}

pub fn load_game_index(options: LoadGameIndexOptions) -> Result<LoadGameIndexResult> {
    let metadata = fs::metadata(&options.dir_vpk_path)?;
    let source = GameIndexSource {
        path: options.dir_vpk_path.to_string_lossy().into_owned(),
        size: metadata.len(),
        mtime_ms: modified_ms(&metadata),
    };

    let cached = match &options.cache_path {
        Some(cache_path) => read_cache_any(cache_path),
        None => None,
    };

    if !options.force {
        if let (Some(cached), Some(cache_path)) = (&cached, &options.cache_path) {
            if cache_matches(cached, &source) {
                return Ok(LoadGameIndexResult {
                    index: cached.clone(),
                    from_cache: true,
                    cache_path: Some(cache_path.clone()),
                    cache_error: None,
                    previous: None,
                });
            }
        }
    }

    let directory = read_vpk_directory_with(
        &options.dir_vpk_path,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;

    let entries = directory
        .by_path
        .values()
        .map(|entry| IndexEntry {
            path: entry.path.clone(),
            crc32: entry.crc32,
            preload_bytes: entry.preload_bytes,
            archive_index: entry.archive_index,
            offset: entry.offset,
            length: entry.length,
        })
        .collect();

    let index = GameIndex {
        schema: GAME_INDEX_SCHEMA,
        source,
        built_at: now_iso(),
        header: directory.header,
        entries,
        skipped: directory.skipped,
        duplicates: directory.duplicates,
    };

    let mut cache_error = None;

    if let Some(cache_path) = &options.cache_path {
        if let Err(error) = write_cache(cache_path, &index) {
            cache_error = Some(error.to_string());
        }
    }

    Ok(LoadGameIndexResult {
        index,
        from_cache: false,
        cache_path: options.cache_path,
        cache_error,
        previous: cached,
    })
}

pub fn search_game_index(index: &GameIndex, options: SearchOptions) -> SearchResult {
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
    let limit = clamp_limit(options.limit, 500);

    let hero_pattern = hero.as_ref().map(|value| {
        Regex::new(&format!("(^|/)heroes[^/]*/{}(/|$)", escape_regex(value)))
            .expect("escaped hero pattern is valid")
    });

    let mut matches: Vec<&IndexEntry> = Vec::new();

    for entry in &index.entries {
        let path = entry.path.to_lowercase();

        if tokens.iter().any(|token| !path.contains(token)) {
            continue;
        }

        if let (Some(pattern), Some(_)) = (&hero_pattern, &hero) {
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

    SearchResult {
        total,
        entries,
        truncated: total > limit,
    }
}

pub fn list_heroes(index: &GameIndex) -> Vec<HeroSummary> {
    let pattern = Regex::new(r"(?i)(^|/)heroes[^/]*/([^/]+)/").expect("hero pattern is valid");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for entry in &index.entries {
        let Some(captures) = pattern.captures(&entry.path) else {
            continue;
        };
        let Some(codename) = captures.get(2) else {
            continue;
        };
        *counts.entry(codename.as_str().to_lowercase()).or_default() += 1;
    }

    let mut summary: Vec<HeroSummary> = counts
        .into_iter()
        .map(|(codename, entries)| HeroSummary { codename, entries })
        .collect();

    summary.sort_by(|left, right| {
        right
            .entries
            .cmp(&left.entries)
            .then_with(|| left.codename.cmp(&right.codename))
    });

    summary
}

pub fn top_extensions(index: &GameIndex, limit: usize) -> Vec<ExtensionSummary> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for entry in &index.entries {
        let extension = match entry.path.rfind('.') {
            Some(position) => entry.path[position + 1..].to_lowercase(),
            None => "(none)".to_string(),
        };
        *counts.entry(extension).or_default() += 1;
    }

    let mut summary: Vec<ExtensionSummary> = counts
        .into_iter()
        .map(|(extension, entries)| ExtensionSummary { extension, entries })
        .collect();

    summary.sort_by(|left, right| {
        right
            .entries
            .cmp(&left.entries)
            .then_with(|| left.extension.cmp(&right.extension))
    });
    summary.truncate(limit);
    summary
}

pub fn format_index_summary(index: &GameIndex, loaded: &IndexLoadMeta) -> String {
    let mut lines = vec![
        format!("Game index: {}", index.source.path),
        format!("Entries: {}", index.entries.len()),
        format!("VPK version: {}", index.header.version),
    ];

    if index.skipped > 0 {
        lines.push(format!(
            "Skipped entries: {} (unsafe or empty paths)",
            index.skipped
        ));
    }

    if index.duplicates > 0 {
        lines.push(format!(
            "Duplicate paths: {} (last entry wins)",
            index.duplicates
        ));
    }

    let heroes = list_heroes(index);
    if !heroes.is_empty() {
        let shown: Vec<String> = heroes
            .iter()
            .take(24)
            .map(|hero| format!("{} ({})", hero.codename, hero.entries))
            .collect();
        let suffix = if heroes.len() > shown.len() {
            ", ..."
        } else {
            ""
        };
        lines.push(format!("Heroes: {}{suffix}", shown.join(", ")));
    }

    let extensions = top_extensions(index, 8);
    if !extensions.is_empty() {
        let shown: Vec<String> = extensions
            .iter()
            .map(|item| format!("{} ({})", item.extension, item.entries))
            .collect();
        lines.push(format!("Top extensions: {}", shown.join(", ")));
    }

    if loaded.from_cache {
        lines.push(format!(
            "Cache: {}",
            loaded
                .cache_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "memory".to_string())
        ));
    } else {
        match &loaded.cache_path {
            Some(path) => lines.push(format!("Cache: rebuilt at {}", path.display())),
            None => lines.push("Cache: rebuilt (not written)".to_string()),
        }
    }

    if let Some(error) = &loaded.cache_error {
        lines.push(format!("Cache warning: {error}"));
    }

    lines.join("\n")
}

pub fn format_index_summary_result(result: &LoadGameIndexResult) -> String {
    format_index_summary(&result.index, &IndexLoadMeta::from_result(result))
}

pub fn format_search_results(
    result: &SearchGameIndexResult,
    options: &SearchGameIndexOptions,
) -> String {
    if result.total == 0 {
        return "No matching paths in the game index.".to_string();
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
        let archive = if entry.archive_index == crate::vpk::VPK_DIRECTORY_ARCHIVE {
            "dir".to_string()
        } else {
            entry.archive_index.to_string()
        };
        lines.push(format!(
            "{} ({} bytes, archive {})",
            entry.path,
            entry.total_length(),
            archive
        ));
    }

    if result.truncated {
        lines.push("Output truncated. Narrow the query or raise the limit.".to_string());
    }

    lines.join("\n")
}

/// Compare two indexes. Paths present in both with different content become
/// `changed`. Remaining paths are paired as `moved` when their basename is
/// unique on both sides.
pub fn diff_game_index(previous: &GameIndex, current: &GameIndex) -> IndexDiff {
    let old_by_path: HashMap<String, &IndexEntry> = previous
        .entries
        .iter()
        .map(|entry| (entry.path.to_lowercase(), entry))
        .collect();
    let new_by_path: HashMap<String, &IndexEntry> = current
        .entries
        .iter()
        .map(|entry| (entry.path.to_lowercase(), entry))
        .collect();

    let mut changed = Vec::new();
    let mut removed = Vec::new();
    let mut added = Vec::new();

    for (key, old) in &old_by_path {
        match new_by_path.get(key) {
            Some(new) => {
                if old.crc32 != new.crc32
                    || old.length != new.length
                    || old.preload_bytes != new.preload_bytes
                {
                    changed.push(IndexChange {
                        path: new.path.clone(),
                        previous: (*old).clone(),
                        current: (*new).clone(),
                    });
                }
            }
            None => removed.push((*old).clone()),
        }
    }

    for (key, new) in &new_by_path {
        if !old_by_path.contains_key(key) {
            added.push((*new).clone());
        }
    }

    removed.sort_by(|left, right| left.path.cmp(&right.path));
    added.sort_by(|left, right| left.path.cmp(&right.path));
    changed.sort_by(|left, right| left.path.cmp(&right.path));

    let removed_names = unique_basenames(&removed);
    let added_names = unique_basenames(&added);
    let mut moved = Vec::new();
    let mut moved_old: HashSet<String> = HashSet::new();
    let mut moved_new: HashSet<String> = HashSet::new();

    for old in &removed {
        let name = basename(&old.path).to_lowercase();
        let Some(new_path) = added_names.get(&name) else {
            continue;
        };
        if !removed_names.contains_key(&name) {
            continue;
        }
        if moved_old.contains(&old.path) || moved_new.contains(new_path) {
            continue;
        }

        let Some(new) = added.iter().find(|entry| &entry.path == new_path) else {
            continue;
        };

        moved.push(IndexMove {
            from: old.path.clone(),
            to: new.path.clone(),
            entry: new.clone(),
        });
        moved_old.insert(old.path.clone());
        moved_new.insert(new.path.clone());
    }

    removed.retain(|entry| !moved_old.contains(&entry.path));
    added.retain(|entry| !moved_new.contains(&entry.path));
    moved.sort_by(|left, right| left.from.cmp(&right.from));

    IndexDiff {
        added,
        removed,
        changed,
        moved,
    }
}

/// Basename to path, only for basenames that appear exactly once.
fn unique_basenames(entries: &[IndexEntry]) -> HashMap<String, String> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for entry in entries {
        *counts
            .entry(basename(&entry.path).to_lowercase())
            .or_default() += 1;
    }

    let mut unique = HashMap::new();

    for entry in entries {
        let name = basename(&entry.path).to_lowercase();
        if counts.get(&name) == Some(&1) {
            unique.insert(name, entry.path.clone());
        }
    }

    unique
}

fn modified_ms(metadata: &fs::Metadata) -> f64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn cache_matches(cached: &GameIndex, source: &GameIndexSource) -> bool {
    cached.schema == GAME_INDEX_SCHEMA
        && cached.source.path == source.path
        && cached.source.size == source.size
        && cached.source.mtime_ms == source.mtime_ms
}

fn read_cache_any(cache_path: &Path) -> Option<GameIndex> {
    let text = fs::read_to_string(cache_path).ok()?;
    let parsed: GameIndex = serde_json::from_str(&text).ok()?;
    (parsed.schema == GAME_INDEX_SCHEMA).then_some(parsed)
}

fn write_cache(cache_path: &Path, index: &GameIndex) -> Result<()> {
    let mut text = serde_json::to_string(index)?;
    text.push('\n');
    write_atomic(cache_path, text.as_bytes())
}

fn normalize_extension(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    let normalized = value.strip_prefix('.').unwrap_or(&value);
    (!normalized.is_empty()).then(|| normalized.to_string())
}

fn clamp_limit(value: Option<usize>, maximum: usize) -> usize {
    value.unwrap_or(50).clamp(1, maximum)
}

fn compare_matches(left: &IndexEntry, right: &IndexEntry, query: &str) -> std::cmp::Ordering {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpk::VpkHeader;

    fn entry(path: &str, crc32: u32) -> IndexEntry {
        IndexEntry {
            path: path.to_string(),
            crc32,
            preload_bytes: 0,
            archive_index: 0x7fff,
            offset: 0,
            length: 1,
        }
    }

    #[test]
    fn diff_pairs_a_unique_basename_as_a_move() {
        let previous = GameIndex {
            schema: GAME_INDEX_SCHEMA,
            source: GameIndexSource {
                path: "old".to_string(),
                size: 1,
                mtime_ms: 1.0,
            },
            built_at: String::new(),
            header: VpkHeader {
                version: 2,
                header_size: 28,
                tree_size: 0,
                file_data_section_size: 0,
                archive_md5_section_size: 0,
                other_md5_section_size: 0,
                signature_section_size: 0,
            },
            entries: vec![
                entry("models/heroes_wip/billy/billy.vmdl_c", 1),
                entry("models/heroes_wip/billy/old.vmdl_c", 2),
            ],
            skipped: 0,
            duplicates: 0,
        };
        let current = GameIndex {
            source: GameIndexSource {
                path: "new".to_string(),
                size: 1,
                mtime_ms: 1.0,
            },
            entries: vec![
                entry("models/heroes/billy/billy.vmdl_c", 1),
                entry("models/heroes_wip/billy/old.vmdl_c", 2),
                entry("models/heroes_wip/billy/new.vmdl_c", 3),
            ],
            ..previous.clone()
        };

        let diff = diff_game_index(&previous, &current);
        assert_eq!(diff.moved.len(), 1);
        assert_eq!(diff.moved[0].to, "models/heroes/billy/billy.vmdl_c");
        assert_eq!(diff.added.len(), 1);
        assert!(diff.removed.is_empty());
    }
}
