//! A local copy of the GameBanana Deadlock catalog.
//!
//! The public API serves 50 rows a page and no date filter, so the browse grid reads it live.
//! This module keeps an index on disk beside the app's own files: a background sync walks the
//! newest pages of Mods, Sounds and WiPs a few at a time, so the grid can fall back to it when
//! the network is down, search creators by name, and answer date ranges without reading the
//! whole feed. It is bounded in entries and bytes and prunes the oldest rows first.
//!
//! Nothing here writes to the game install. Paths come from the desktop data folder; the CLI
//! never needs the mirror.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::gamebanana::{
    self, BrowseMod, BrowseOrder, BrowsePeriod, BrowseQuery, BrowseSection,
};

/// Format of `meta.json` and of each `entries.jsonl` line.
pub const MIRROR_SCHEMA: u32 = 1;
/// Rows kept before the oldest are pruned.
pub const MAX_ENTRIES: usize = 40_000;
/// Bytes the two files may take. A full mirror stays near 18 MB.
pub const MAX_BYTES: u64 = 24 * 1024 * 1024;
/// Rows a sync page asks for.
pub const SYNC_PAGE_SIZE: usize = 50;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorEntry {
    pub id: u64,
    /// `Mod`, `Sound` or `Wip`.
    pub model: String,
    pub name: String,
    pub author: String,
    pub author_id: u64,
    pub category: String,
    pub subcategory: String,
    /// The id of `subcategory`, the row's deepest category. Zero on rows saved before it was
    /// kept; those match a pick by name.
    #[serde(default)]
    pub sub_category_id: u64,
    pub profile_url: String,
    #[serde(default)]
    pub thumbnail_url: Option<String>,
    #[serde(default)]
    pub audio_url: Option<String>,
    pub added_ts: i64,
    pub updated_ts: i64,
    pub likes: u64,
    pub views: u64,
    pub comments: u64,
    /// Read from the mod page the first time its details open. List rows carry none.
    #[serde(default)]
    pub downloads: Option<u64>,
    pub nsfw: bool,
    pub obsolete: bool,
    pub featured: bool,
}

impl MirrorEntry {
    pub fn from_browse(row: &BrowseMod) -> Self {
        MirrorEntry {
            id: row.id,
            model: row.model.clone(),
            name: row.name.clone(),
            author: row.author.clone(),
            author_id: row.author_id,
            category: row.category.clone(),
            subcategory: row.subcategory.clone(),
            sub_category_id: row.sub_category_id,
            profile_url: row.profile_url.clone(),
            thumbnail_url: row.thumbnail_url.clone(),
            audio_url: row.audio_url.clone(),
            added_ts: row.added_ts,
            updated_ts: row.updated_ts,
            likes: row.likes,
            views: row.views,
            comments: row.comments,
            downloads: None,
            nsfw: row.nsfw,
            obsolete: row.obsolete,
            featured: row.featured,
        }
    }

    pub fn section(&self) -> BrowseSection {
        BrowseSection::from_model(&self.model)
    }

    /// `Skins / Haze`, or whichever half exists.
    pub fn category_line(&self) -> String {
        match (self.category.is_empty(), self.subcategory.is_empty()) {
            (false, false) => format!("{} / {}", self.category, self.subcategory),
            (false, true) => self.category.clone(),
            (true, false) => self.subcategory.clone(),
            (true, true) => String::new(),
        }
    }

    pub fn age_stamp(&self) -> i64 {
        self.updated_ts.max(self.added_ts)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SectionProgress {
    /// The next page the sync reads, oldest first after the newest pages.
    pub next_page: usize,
    /// Unix seconds of the newest row the section has.
    pub newest: i64,
    /// Every page of the section has been read at least once.
    pub swept: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorMeta {
    pub schema: u32,
    /// Unix seconds of the last finished sync step.
    pub synced_at: i64,
    /// Steps finished, so the UI can say how far the copy goes.
    pub steps: u64,
    /// By section model name.
    pub sections: BTreeMap<String, SectionProgress>,
    /// Rows a mod page showed GameBanana no longer serves publicly, as `Model:id`. A later
    /// index row for one is not kept, so a withheld mod cannot come back through a stale page.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub withheld: std::collections::BTreeSet<String>,
}

impl Default for MirrorMeta {
    fn default() -> Self {
        MirrorMeta {
            schema: MIRROR_SCHEMA,
            synced_at: 0,
            steps: 0,
            sections: BTreeMap::new(),
            withheld: Default::default(),
        }
    }
}

/// The filter a mirror page answers. Names fold case and punctuation, as the hero picker does.
#[derive(Debug, Clone, Default)]
pub struct MirrorFilter {
    pub section: BrowseSection,
    /// Keep every model instead of one section (the combined feed).
    pub all_models: bool,
    /// Matches the name, category, sub category and author.
    pub query: String,
    /// Folded root category name, kept or excluded.
    pub category: Option<String>,
    pub subcategory: Option<String>,
    /// The picked sub category and every category under it (Model Replacement > NPCs holds
    /// Troopers). A row that carries its leaf id matches by it; `subcategory`'s name is the
    /// fallback for rows saved without one.
    pub subtree: Vec<u64>,
    /// Children of `category` when only the root is known: keep the root's own name too.
    pub exclude_category: bool,
    /// Extended hero matching: a row whose leaf category is one of `hero_ids` (the hero's
    /// branches anywhere in the section's tree) or whose title names the hero.
    pub hero: Option<HeroMatch>,
    pub author_id: Option<u64>,
    /// Unix seconds, zero for open.
    pub added_from: i64,
    pub added_to: i64,
    pub updated_from: i64,
    pub updated_to: i64,
    /// Drop mature rows, or keep only them.
    pub hide_mature: bool,
    pub only_mature: bool,
    /// Drop rows older than the January 22, 2026 path change.
    pub hide_outdated: bool,
}

impl MirrorFilter {
    pub fn section(section: BrowseSection) -> Self {
        MirrorFilter {
            section,
            ..MirrorFilter::default()
        }
    }

    pub fn matches(&self, entry: &MirrorEntry) -> bool {
        if !self.all_models && entry.section() != self.section {
            return false;
        }
        if self.hide_mature && entry.nsfw {
            return false;
        }
        if self.only_mature && !entry.nsfw {
            return false;
        }
        if self.hide_outdated && outdated(entry) {
            return false;
        }
        if let Some(author) = self.author_id
            && entry.author_id != author
        {
            return false;
        }
        if let Some(category) = self.category.as_deref() {
            let hit = fold(&entry.category) == fold(category);
            if hit == self.exclude_category {
                return false;
            }
        }
        if let Some(sub) = self.subcategory.as_deref() {
            let hit = if !self.subtree.is_empty() && entry.sub_category_id > 0 {
                self.subtree.contains(&entry.sub_category_id)
            } else {
                let wanted = fold(sub);
                fold(&entry.subcategory) == wanted || fold(&entry.name).contains(&wanted)
            };
            if hit == self.exclude_category {
                return false;
            }
        }
        if let Some(hero) = &self.hero
            && (hero.matches(entry.sub_category_id, &entry.name)
                || fold(&entry.subcategory) == fold(&hero.name))
                == self.exclude_category
        {
            return false;
        }
        if self.added_from > 0 && entry.added_ts < self.added_from {
            return false;
        }
        if self.added_to > 0 && entry.added_ts > self.added_to {
            return false;
        }
        if self.updated_from > 0 && entry.updated_ts < self.updated_from {
            return false;
        }
        if self.updated_to > 0 && entry.updated_ts > self.updated_to {
            return false;
        }
        let query = fold(&self.query);
        if query.is_empty() {
            return true;
        }
        fold(&entry.name).contains(&query)
            || fold(&entry.category).contains(&query)
            || fold(&entry.subcategory).contains(&query)
            || fold(&entry.author).contains(&query)
    }
}

/// A hero picked with extended matching on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeroMatch {
    pub name: String,
    /// Every category of the section named for the hero, with its subtree.
    pub ids: Vec<u64>,
}

impl HeroMatch {
    pub fn matches(&self, leaf: u64, title: &str) -> bool {
        (leaf > 0 && self.ids.contains(&leaf)) || title_names_hero(title, &self.name)
    }
}

/// Other names players write for a hero in titles. Whole words only, so "harrowing" is not
/// Nurse Harrow.
fn hero_aliases(hero: &str) -> Vec<String> {
    let name = words(hero);
    let mut aliases = vec![name.clone(), name.replace(' ', "")];
    match name.as_str() {
        "deadman danny" => aliases.extend(["deadman".into(), "danny".into()]),
        "nurse harrow" => aliases.push("harrow".into()),
        "mo and krill" => aliases.extend(["mo krill".into(), "mokrill".into()]),
        "lady geist" => aliases.push("geist".into()),
        "grey talon" => aliases.extend(["gray talon".into(), "talon".into()]),
        _ => {}
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

/// Lower case words split on anything that is not a letter or digit, `&` read as "and", and a
/// possessive `'s` dropped, joined by single spaces.
fn words(text: &str) -> String {
    let text = text.to_lowercase().replace('&', " and ").replace("'s ", " ").replace("\u{2019}s ", " ");
    let text = text.strip_suffix("'s").unwrap_or(&text).to_string();
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `title` names `hero` as whole words: "Haze_Bikini", "Abrams' Coat" and
/// "mo&krill skin" count; "Hazel" and "harrowing" do not.
pub fn title_names_hero(title: &str, hero: &str) -> bool {
    let title = format!(" {} ", words(title));
    hero_aliases(hero)
        .iter()
        .filter(|alias| !alias.is_empty())
        .any(|alias| title.contains(&format!(" {alias} ")))
}

#[derive(Debug, Clone, Default)]
pub struct MirrorPage {
    pub entries: Vec<MirrorEntry>,
    pub total: usize,
    pub complete: bool,
}

/// One creator the mirror knows, for the author results over the grid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreatorHit {
    pub id: u64,
    pub name: String,
    pub count: usize,
    /// Unix seconds of the creator's newest row.
    pub newest: i64,
}

/// The on-disk index. `entries` is in memory; a sync step appends to it and saves atomically.
#[derive(Debug, Clone, Default)]
pub struct Mirror {
    root: PathBuf,
    pub entries: Vec<MirrorEntry>,
    pub meta: MirrorMeta,
}

fn withheld_key(model: &str, id: u64) -> String {
    let model = model.to_ascii_lowercase();
    let mut chars = model.chars();
    let model: String = chars.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(chars).collect();
    format!("{model}:{id}")
}

/// The folder the mirror lives in: `<data>/browse/mirror`.
pub fn mirror_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("browse").join("mirror")
}

pub fn entries_path(data_dir: &Path) -> PathBuf {
    mirror_dir(data_dir).join("entries.jsonl")
}

pub fn meta_path(data_dir: &Path) -> PathBuf {
    mirror_dir(data_dir).join("meta.json")
}

impl Mirror {
    /// Read the index from disk. A missing or half-written file is an empty mirror.
    pub fn load(data_dir: &Path) -> Mirror {
        let root = mirror_dir(data_dir);
        let meta = fs::read_to_string(meta_path(data_dir))
            .ok()
            .and_then(|text| serde_json::from_str::<MirrorMeta>(&text).ok())
            .filter(|meta| meta.schema == MIRROR_SCHEMA)
            .unwrap_or_default();
        let entries = fs::read_to_string(entries_path(data_dir))
            .map(|text| {
                text.lines()
                    .filter_map(|line| serde_json::from_str::<MirrorEntry>(line).ok())
                    .filter(|entry| entry.id > 0 && !entry.name.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Mirror {
            root,
            entries,
            meta,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Add or replace one row. True when the stored copy changed.
    pub fn upsert(&mut self, mut entry: MirrorEntry) -> bool {
        if entry.id == 0 || self.meta.withheld.contains(&withheld_key(&entry.model, entry.id)) {
            return false;
        }
        match self
            .entries
            .iter_mut()
            .find(|existing| existing.id == entry.id && existing.model == entry.model)
        {
            Some(existing) => {
                // A profile read known downloads; a later index row must not drop them.
                if entry.downloads.is_none() {
                    entry.downloads = existing.downloads;
                }
                if *existing == entry {
                    return false;
                }
                *existing = entry;
                true
            }
            None => {
                self.entries.push(entry);
                true
            }
        }
    }

    /// Drop one row: GameBanana no longer shows it publicly (withheld or private, DMM
    /// `208df742`). The next sync adds it back only if the index lists it again. True when a
    /// row was dropped.
    pub fn remove(&mut self, model: &str, id: u64) -> bool {
        let fresh = self.meta.withheld.insert(withheld_key(model, id));
        let before = self.entries.len();
        self.entries
            .retain(|entry| !(entry.id == id && entry.model.eq_ignore_ascii_case(model)));
        self.entries.len() != before || fresh
    }

    /// GameBanana shows the submission again (staff lifted the withhold): its next index row
    /// is kept. True when it was withheld here.
    pub fn restore(&mut self, model: &str, id: u64) -> bool {
        self.meta.withheld.remove(&withheld_key(model, id))
    }

    /// Record the download count a mod page reported. True when it was new.
    pub fn upsert_downloads(&mut self, model: &str, id: u64, downloads: u64) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id && entry.model.eq_ignore_ascii_case(model))
        else {
            return false;
        };
        if entry.downloads == Some(downloads) {
            return false;
        }
        entry.downloads = Some(downloads);
        true
    }

    pub fn downloads(&self, model: &str, id: u64) -> Option<u64> {
        self.entries
            .iter()
            .find(|entry| entry.id == id && entry.model.eq_ignore_ascii_case(model))
            .and_then(|entry| entry.downloads)
    }

    /// Creators whose name matches `text`, most submissions first.
    pub fn creators(&self, text: &str, limit: usize) -> Vec<CreatorHit> {
        let wanted = fold(text);
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut hits: BTreeMap<u64, CreatorHit> = BTreeMap::new();
        for entry in &self.entries {
            if entry.author_id == 0 || entry.author.is_empty() {
                continue;
            }
            if !fold(&entry.author).contains(&wanted) {
                continue;
            }
            let hit = hits.entry(entry.author_id).or_insert_with(|| CreatorHit {
                id: entry.author_id,
                name: entry.author.clone(),
                count: 0,
                newest: 0,
            });
            hit.count += 1;
            hit.newest = hit.newest.max(entry.age_stamp());
        }
        let mut hits: Vec<CreatorHit> = hits.into_values().collect();
        hits.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| b.newest.cmp(&a.newest))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        hits.truncate(limit.min(12));
        hits
    }

    /// A filtered, ordered page, one-based as the grid paginates.
    pub fn page(&self, filter: &MirrorFilter, order: BrowseOrder, page: usize, perpage: usize) -> MirrorPage {
        let perpage = perpage.max(1);
        let mut rows: Vec<&MirrorEntry> = self.entries.iter().filter(|row| filter.matches(row)).collect();
        sort_entries(&mut rows, order);
        let total = rows.len();
        let start = (page.max(1) - 1).saturating_mul(perpage);
        let entries: Vec<MirrorEntry> = rows
            .into_iter()
            .skip(start)
            .take(perpage)
            .cloned()
            .collect();
        MirrorPage {
            complete: start + entries.len() >= total,
            entries,
            total,
        }
    }

    /// True when a date range can be answered here. The grid falls back to the live feed
    /// when the mirror has not reached far enough back.
    pub fn covers(&self, filter: &MirrorFilter) -> bool {
        !self.entries.is_empty()
            && self
                .entries
                .iter()
                .any(|entry| filter.matches(entry))
    }

    pub fn summary(&self) -> String {
        let sections = if self.meta.sections.is_empty() {
            "not synced yet".to_string()
        } else {
            let swept = self
                .meta
                .sections
                .values()
                .filter(|section| section.swept)
                .count();
            format!("{swept} of {} sections read in full", self.meta.sections.len())
        };
        format!("{} mods saved, {sections}", self.entries.len())
    }

    /// Write the index. Bounded in rows and bytes; the oldest rows go first.
    pub fn save(&mut self) -> Result<()> {
        self.prune();
        fs::create_dir_all(&self.root)?;
        let mut body = String::with_capacity(self.entries.len() * 176);
        for entry in &self.entries {
            if let Ok(line) = serde_json::to_string(entry) {
                body.push_str(&line);
                body.push('\n');
            }
        }
        write_atomic(&self.root.join("entries.jsonl"), body.as_bytes())?;
        let meta = serde_json::to_vec_pretty(&self.meta)?;
        write_atomic(&self.root.join("meta.json"), &meta)?;
        Ok(())
    }

    /// Drop the oldest rows until the mirror is inside both bounds.
    pub fn prune(&mut self) {
        self.entries
            .retain(|entry| entry.id > 0 && !entry.name.is_empty());
        if self.entries.len() > MAX_ENTRIES {
            let mut order: Vec<usize> = (0..self.entries.len()).collect();
            order.sort_by_key(|index| std::cmp::Reverse(self.entries[*index].age_stamp()));
            let keep: std::collections::HashSet<usize> =
                order.into_iter().take(MAX_ENTRIES).collect();
            let mut kept = Vec::with_capacity(MAX_ENTRIES);
            for (index, entry) in self.entries.drain(..).enumerate() {
                if keep.contains(&index) {
                    kept.push(entry);
                }
            }
            self.entries = kept;
        }
        // 176 bytes is a middle estimate per row; the exact size never needs to fit.
        let cap = (MAX_BYTES / 176) as usize;
        if self.entries.len() > cap {
            self.entries.sort_by_key(|entry| std::cmp::Reverse(entry.age_stamp()));
            self.entries.truncate(cap);
        }
    }

    /// Advance the sync one step: the next pages of one section, then save.
    ///
    /// Pages are read newest first and the walk wraps to page one after the feed ends, so
    /// the copy is fresh at the top and complete further down over time. A network failure
    /// leaves the step's progress where it was.
    pub fn sync_step(&mut self) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        for section in BrowseSection::ALL {
            let model = section.model().to_string();
            let progress = self.meta.sections.get(&model).cloned().unwrap_or_default();
            let mut next_page = progress.next_page.max(1);
            let mut newest = progress.newest;
            let mut swept = progress.swept;
            let query = BrowseQuery {
                section,
                order: BrowseOrder::Newest,
                ..BrowseQuery::default()
            };
            // Page one always, so new uploads land on every step, then one page further back.
            let pages = if next_page > 1 {
                vec![1, next_page]
            } else {
                vec![1, 2]
            };
            for page in pages {
                let read = gamebanana::fetch_browse(&query, page, SYNC_PAGE_SIZE)?;
                for row in &read.mods {
                    if self.upsert(MirrorEntry::from_browse(row)) {
                        report.added += 1;
                    }
                }
                report.pages += 1;
                newest = newest.max(
                    read.mods
                        .iter()
                        .map(|row| row.added_ts)
                        .max()
                        .unwrap_or(0),
                );
                if page == 1 && next_page <= 1 {
                    next_page = 2;
                    continue;
                }
                if read.complete || read.mods.is_empty() {
                    swept = true;
                    next_page = 1;
                    break;
                }
                next_page = page + 1;
            }
            let progress = self.meta.sections.entry(model).or_default();
            progress.newest = newest;
            progress.swept |= swept;
            progress.next_page = next_page;
        }
        self.meta.steps += 1;
        self.meta.synced_at = gamebanana::now_unix();
        self.save()?;
        report.saved = self.entries.len();
        Ok(report)
    }

    /// A page of rows for the grid when the network is out or offline mode is on.
    pub fn offline_page(
        &self,
        filter: &MirrorFilter,
        order: BrowseOrder,
        page: usize,
        perpage: usize,
    ) -> MirrorPage {
        self.page(filter, order, page, perpage)
    }

    /// The date window as a mirror filter, so the grid and the sync agree on one shape.
    pub fn range_filter(
        section: BrowseSection,
        period: BrowsePeriod,
        field: gamebanana::DateField,
        from: i64,
        to: i64,
        now: i64,
    ) -> MirrorFilter {
        let start = if from > 0 {
            from
        } else {
            period.cutoff(now).unwrap_or(0)
        };
        let mut filter = MirrorFilter::section(section);
        match field {
            gamebanana::DateField::Added => {
                filter.added_from = start;
                filter.added_to = to;
            }
            gamebanana::DateField::Updated => {
                filter.updated_from = start;
                filter.updated_to = to;
            }
        }
        filter
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub pages: usize,
    pub added: usize,
    pub saved: usize,
}

fn sort_entries(rows: &mut [&MirrorEntry], order: BrowseOrder) {
    match order {
        BrowseOrder::Popular | BrowseOrder::MostLiked => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.likes))
        }
        BrowseOrder::Newest => rows.sort_by_key(|row| std::cmp::Reverse(row.added_ts)),
        BrowseOrder::Updated | BrowseOrder::LatestComment => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.updated_ts))
        }
        BrowseOrder::MostDownloaded => {
            rows.sort_by_key(|row| std::cmp::Reverse(row.downloads.unwrap_or(0)))
        }
        BrowseOrder::MostViewed => rows.sort_by_key(|row| std::cmp::Reverse(row.views)),
        BrowseOrder::MostCommented => rows.sort_by_key(|row| std::cmp::Reverse(row.comments)),
        BrowseOrder::AToZ => rows.sort_by_key(|row| fold(&row.name)),
    }
}

/// Lower-case letters and digits only, so "Mo & Krill" and "Mo and Krill" agree.
pub fn fold(name: &str) -> String {
    name.replace('&', "and")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Rows the January 22, 2026 path change left behind.
pub fn outdated(entry: &MirrorEntry) -> bool {
    const OUTDATED_BEFORE: i64 = 1_769_040_000;
    entry.obsolete || (entry.updated_ts > 0 && entry.updated_ts < OUTDATED_BEFORE)
}

fn write_atomic(path: &Path, body: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, name: &str, author: &str, added: i64, updated: i64) -> MirrorEntry {
        MirrorEntry {
            id,
            model: "Mod".into(),
            name: name.into(),
            author: author.into(),
            author_id: 7,
            category: "Skins".into(),
            subcategory: "Haze".into(),
            added_ts: added,
            updated_ts: updated,
            likes: id,
            ..MirrorEntry::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sigil-mirror-{name}-{}-{}",
            std::process::id(),
            gamebanana::now_unix()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_index_round_trips_through_disk() {
        let dir = temp_dir("roundtrip");
        let mut mirror = Mirror::load(&dir);
        assert!(mirror.is_empty());
        assert!(mirror.upsert(entry(1, "Long Hair Haze", "747shove", 10, 20)));
        assert!(!mirror.upsert(entry(1, "Long Hair Haze", "747shove", 10, 20)));
        mirror.upsert_downloads("Mod", 1, 55);
        mirror.meta.steps = 2;
        mirror.save().unwrap();

        let loaded = Mirror::load(&dir);
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.downloads("Mod", 1), Some(55));
        assert_eq!(loaded.meta.steps, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_profile_download_count_survives_a_later_index_row() {
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "A", "x", 1, 1));
        mirror.upsert_downloads("Mod", 1, 9);
        mirror.upsert(entry(1, "A renamed", "x", 1, 2));
        assert_eq!(mirror.downloads("Mod", 1), Some(9));
        assert_eq!(mirror.entries[0].name, "A renamed");
    }

    /// DMM `208df742`: a withheld mod leaves the saved catalog and a stale index row does not
    /// bring it back; lifting the withhold lets it in again.
    #[test]
    fn a_withheld_mod_leaves_the_catalog_and_stays_out() {
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "A", "x", 1, 1));
        mirror.upsert(entry(2, "B", "x", 1, 1));
        assert!(mirror.remove("mod", 1));
        assert_eq!(mirror.entries.len(), 1);
        assert!(!mirror.upsert(entry(1, "A", "x", 1, 2)), "a later index row is not kept");
        assert!(!mirror.remove("Mod", 1), "nothing new to drop");
        assert!(mirror.restore("Mod", 1));
        assert!(mirror.upsert(entry(1, "A", "x", 1, 2)));
        assert_eq!(mirror.entries.len(), 2);
    }

    #[test]
    fn creators_are_ranked_and_deduplicated() {
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "Haze hair", "Hazel", 10, 10));
        mirror.upsert(entry(2, "Haze coat", "hazel", 20, 20));
        mirror.upsert(entry(3, "Other", "Someone", 30, 30));
        let hits = mirror.creators("haze", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].count, 2);
        assert!(mirror.creators("", 5).is_empty());
    }

    #[test]
    fn filters_answer_sections_dates_and_maturity() {
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "Long Hair Haze", "747shove", 100, 200));
        mirror.upsert(entry(2, "Old Skin", "Someone", 10, 20));
        let mut filter = MirrorFilter::section(BrowseSection::Mods);
        filter.added_from = 50;
        let page = mirror.page(&filter, BrowseOrder::Newest, 1, 10);
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].id, 1);
        assert!(page.complete);
        filter.hide_mature = true;
        mirror.entries[0].nsfw = true;
        assert_eq!(mirror.page(&filter, BrowseOrder::Newest, 1, 10).total, 0);
    }

    #[test]
    fn filters_can_exclude_and_read_the_updated_stamp() {
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "Long Hair Haze", "747shove", 100, 200));
        mirror.upsert(entry(2, "Haze jacket", "someone", 50, 900));
        let mut filter = MirrorFilter::section(BrowseSection::Mods);
        filter.subcategory = Some("Haze".into());
        filter.exclude_category = true;
        assert_eq!(mirror.page(&filter, BrowseOrder::Newest, 1, 10).total, 0);
        let mut updated = MirrorFilter::section(BrowseSection::Mods);
        updated.updated_from = 500;
        assert_eq!(mirror.page(&updated, BrowseOrder::Updated, 1, 10).total, 1);
    }

    /// Grimoire #520: Model Replacement > Hideout keeps Hideout rows, not the whole root, and a
    /// pick holds the categories under it (NPCs holds Troopers).
    #[test]
    fn a_sub_category_pick_keeps_its_subtree_only() {
        let row = |id: u64, root: &str, sub: &str, leaf: u64| MirrorEntry {
            id,
            model: "Mod".into(),
            name: format!("Mod {id}"),
            category: root.into(),
            subcategory: sub.into(),
            sub_category_id: leaf,
            ..MirrorEntry::default()
        };
        let mut mirror = Mirror::default();
        mirror.upsert(row(1, "Model Replacement", "Hideout", 44206));
        mirror.upsert(row(2, "Model Replacement", "Troopers", 44687));
        mirror.upsert(row(3, "Model Replacement", "Soul Container", 41868));
        mirror.upsert(row(4, "HUD", "Character HUD", 44745));
        mirror.upsert(row(5, "Model Replacement", "Hideout", 0));
        let ids = |filter: &MirrorFilter| -> Vec<u64> {
            let mut ids: Vec<u64> = mirror.entries.iter().filter(|e| filter.matches(e)).map(|e| e.id).collect();
            ids.sort();
            ids
        };
        let mut filter = MirrorFilter::section(BrowseSection::Mods);
        filter.category = Some("Model Replacement".into());
        assert_eq!(ids(&filter), vec![1, 2, 3, 5]);
        filter.subcategory = Some("Hideout".into());
        filter.subtree = vec![44206];
        assert_eq!(ids(&filter), vec![1, 5], "an old row without an id matches by name");
        filter.subcategory = Some("NPCs".into());
        filter.subtree = vec![44691, 44687, 44688];
        assert_eq!(ids(&filter), vec![2]);
        filter.exclude_category = true;
        filter.category = None;
        assert_eq!(ids(&filter), vec![1, 3, 4, 5]);
    }

    /// Grimoire #593: extended hero matching takes the hero's branches and whole-word titles.
    #[test]
    fn extended_hero_matching_reads_branches_and_titles() {
        let hero = HeroMatch { name: "Haze".into(), ids: vec![33311, 50617] };
        assert!(hero.matches(50617, "Anything"));
        assert!(hero.matches(0, "Haze_Bikini"));
        assert!(hero.matches(0, "Haze's coat"));
        assert!(!hero.matches(0, "Hazel eyes"));
        assert!(title_names_hero("Nurse harrow mask", "Nurse Harrow"));
        assert!(title_names_hero("Harrow hat", "Nurse Harrow"));
        assert!(!title_names_hero("A harrowing tale", "Nurse Harrow"));
        assert!(title_names_hero("RatKing Flag", "Rat King"));
        assert!(title_names_hero("mo&krill drill", "Mo & Krill"));
        assert!(title_names_hero("Danny's revolver", "Deadman Danny"));
        let mut mirror = Mirror::default();
        mirror.upsert(entry(1, "Haze HUD", "x", 1, 1));
        mirror.entries[0].category = "HUD".into();
        mirror.entries[0].subcategory = "Character HUD".into();
        mirror.upsert(entry(2, "Bikini", "x", 1, 1));
        mirror.entries[1].sub_category_id = 33311;
        mirror.upsert(entry(3, "Abrams coat", "x", 1, 1));
        mirror.entries[2].sub_category_id = 33306;
        mirror.entries[2].subcategory = "Abrams".into();
        let mut filter = MirrorFilter::section(BrowseSection::Mods);
        filter.hero = Some(hero);
        let mut ids: Vec<u64> = mirror.entries.iter().filter(|e| filter.matches(e)).map(|e| e.id).collect();
        ids.sort();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn the_mirror_is_bounded() {
        let mut mirror = Mirror::default();
        for id in 1..=(MAX_ENTRIES + 50) as u64 {
            mirror.entries.push(entry(id, &format!("mod {id}"), "x", id as i64, id as i64));
        }
        mirror.prune();
        assert_eq!(mirror.entries.len(), MAX_ENTRIES);
        // The oldest rows went first.
        assert!(mirror.entries.iter().all(|entry| entry.id > 50));
    }

    #[test]
    fn folders_follow_the_data_dir() {
        let dir = Path::new("/tmp/sigil");
        assert!(entries_path(dir).ends_with("browse/mirror/entries.jsonl"));
        assert_eq!(mirror_dir(dir), dir.join("browse").join("mirror"));
    }
}
