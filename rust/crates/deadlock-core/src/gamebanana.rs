//! Deadlock mod listings from the public GameBanana API v11.
//!
//! Read only: no API key, no login and no download. The browse feed is the
//! source the mod manager uses before a project exists.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::util::unix_seconds_to_iso;

/// GameBanana game id for Deadlock.
pub const GAMEBANANA_GAME_ID: u64 = 20948;
/// Public API v11 root. Subfeeds need no key.
pub const GAMEBANANA_API_BASE: &str = "https://gamebanana.com/apiv11";
/// GameBanana caps a subfeed page at 50 records.
pub const GAMEBANANA_MAX_PERPAGE: usize = 50;
/// Identifies this client without naming a user or an account.
pub const GAMEBANANA_USER_AGENT: &str = "dah/0.0.0";

const REQUEST_TIMEOUT_SECONDS: u64 = 30;
const BODY_SNIPPET_CHARS: usize = 240;

/// One browse row. Only Mod submissions are represented; Sound, Tutorial and
/// Wiki rows from the same feed are dropped by `fetch_deadlock_mods_page`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowseMod {
    pub id: u64,
    pub name: String,
    pub profile_url: String,
    /// GameBanana model name, always `Mod` for a row this type accepts.
    pub model: String,
    /// Display name of the submitter. Empty when the feed omits it.
    pub author: String,
    /// Root category, for example `Skins`.
    pub category: String,
    /// Sub category, for example a hero codename. Empty when absent. GameBanana names the
    /// deepest category here: a hero's style under Skins > Haze reads as that style.
    pub subcategory: String,
    /// The id of `subcategory`, from its profile URL. Zero when absent. Matched against a
    /// picked category's subtree, since a name alone misses the third level.
    #[serde(default)]
    pub sub_category_id: u64,
    pub version: Option<String>,
    /// ISO 8601 timestamp from `_tsDateModified`, falling back to the date added.
    pub updated_at: String,
    pub likes: u64,
    pub views: u64,
    pub has_files: bool,
    pub thumbnail_url: Option<String>,
    /// Unix seconds the mod was first published. Zero when the feed omits it.
    #[serde(default)]
    pub added_ts: i64,
    /// Unix seconds of the latest change GameBanana reports (`_tsDateModified` or
    /// `_tsDateUpdated`, whichever is later), falling back to the date added.
    #[serde(default)]
    pub updated_ts: i64,
    /// True when GameBanana flags the mod as mature: a content rating, a `warn`
    /// or `hide` initial visibility, or `_bIsNsfw`.
    #[serde(default)]
    pub nsfw: bool,
    /// The sample a sound row offers (`_aPreviewMedia._aMetadata._sAudioUrl`).
    #[serde(default)]
    pub audio_url: Option<String>,
    /// GameBanana featured the submission at some point (`_bWasFeatured`).
    #[serde(default)]
    pub featured: bool,
    /// `_nPostCount`: comments on the submission page.
    #[serde(default)]
    pub comments: u64,
    /// The submitter's member id, for the "more by this creator" filter.
    #[serde(default)]
    pub author_id: u64,
    /// The creator marked the submission obsolete (`_bIsObsolete`).
    #[serde(default)]
    pub obsolete: bool,
}

impl BrowseMod {
    /// `Skins / Sinclair`, or whichever half exists.
    pub fn category_line(&self) -> String {
        match (self.category.is_empty(), self.subcategory.is_empty()) {
            (false, false) => format!("{} / {}", self.category, self.subcategory),
            (false, true) => self.category.clone(),
            (true, false) => self.subcategory.clone(),
            (true, true) => String::new(),
        }
    }
}

/// One fetched page plus the feed metadata count.
///
/// GameBanana serves a fixed 15 records per subfeed page and ignores smaller
/// `perpage` values, so `per_page` is the stride the feed reported (15) and
/// `mods` is capped to the size the caller asked for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub page: usize,
    pub per_page: usize,
    /// `_nRecordCount`: every record in the feed, not only mods.
    pub total: usize,
    /// `_bIsComplete`: false while GameBanana has more pages behind this one.
    pub complete: bool,
    pub mods: Vec<BrowseMod>,
}

impl BrowsePage {
    /// Total pages at this page size, at least one.
    pub fn page_count(&self) -> usize {
        if self.per_page == 0 {
            return 1;
        }
        self.total.div_ceil(self.per_page).max(1)
    }
}

/// Fetch one page and keep only Mod submissions, capped at `perpage` items.
pub fn fetch_deadlock_mods(page: usize, perpage: usize) -> Result<Vec<BrowseMod>> {
    fetch_deadlock_mods_page(page, perpage).map(|page| page.mods)
}

/// Fetch one page of the Deadlock subfeed with its metadata count.
///
/// `perpage` caps the returned mod list. The feed itself serves 15 records per
/// page and ignores smaller sizes, so `BrowsePage::per_page` reports the stride
/// the server used rather than the requested cap.
pub fn fetch_deadlock_mods_page(page: usize, perpage: usize) -> Result<BrowsePage> {
    fetch_deadlock_mods_page_sorted(page, perpage, BrowseSort::Default)
}

/// The subfeed orders GameBanana offers through `_sSort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowseSort {
    /// GameBanana's own ranking of recent, active submissions.
    #[default]
    Default,
    /// Newest submissions first.
    New,
    /// Most recently updated first.
    Updated,
}

impl BrowseSort {
    pub fn query(self) -> &'static str {
        match self {
            BrowseSort::Default => "default",
            BrowseSort::New => "new",
            BrowseSort::Updated => "updated",
        }
    }
}

// ---------------------------------------------------------------------------
// Filtered browse: `<Model>/Index`, the subfeed and the category tree
// ---------------------------------------------------------------------------

/// The GameBanana submission type a browse lists. Deadlock sounds are their own
/// model with their own categories (kill sounds, abilities, music).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum BrowseSection {
    #[default]
    Mods,
    Sounds,
    /// Works in progress: early releases creators share before a mod page.
    Wips,
}

impl BrowseSection {
    pub const ALL: [BrowseSection; 3] =
        [BrowseSection::Mods, BrowseSection::Sounds, BrowseSection::Wips];

    /// The API model name, for example `Mod` in `Mod/Index`.
    pub fn model(self) -> &'static str {
        match self {
            BrowseSection::Mods => "Mod",
            BrowseSection::Sounds => "Sound",
            BrowseSection::Wips => "Wip",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BrowseSection::Mods => "Mods",
            BrowseSection::Sounds => "Sounds",
            BrowseSection::Wips => "WiPs",
        }
    }

    /// The section a row's `_sModelName` belongs to.
    pub fn from_model(model: &str) -> Self {
        if model.eq_ignore_ascii_case("sound") {
            BrowseSection::Sounds
        } else if model.eq_ignore_ascii_case("wip") {
            BrowseSection::Wips
        } else {
            BrowseSection::Mods
        }
    }
}

/// Every order the browse grid offers. `Popular` is GameBanana's own ranking of
/// active submissions, which only the unfiltered subfeed serves; the rest are
/// `_sSort` aliases of `<Model>/Index`, checked against the live API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum BrowseOrder {
    #[default]
    Popular,
    Newest,
    Updated,
    MostLiked,
    MostDownloaded,
    MostViewed,
    MostCommented,
    LatestComment,
    AToZ,
}

impl BrowseOrder {
    pub const ALL: [BrowseOrder; 9] = [
        BrowseOrder::Popular,
        BrowseOrder::Newest,
        BrowseOrder::Updated,
        BrowseOrder::MostLiked,
        BrowseOrder::MostDownloaded,
        BrowseOrder::MostViewed,
        BrowseOrder::MostCommented,
        BrowseOrder::LatestComment,
        BrowseOrder::AToZ,
    ];

    /// A stable id for saved filters, distinct from the server alias.
    pub fn id(self) -> &'static str {
        match self {
            BrowseOrder::Popular => "popular",
            BrowseOrder::Newest => "newest",
            BrowseOrder::Updated => "updated",
            BrowseOrder::MostLiked => "liked",
            BrowseOrder::MostDownloaded => "downloaded",
            BrowseOrder::MostViewed => "viewed",
            BrowseOrder::MostCommented => "discussed",
            BrowseOrder::LatestComment => "latestComment",
            BrowseOrder::AToZ => "aToZ",
        }
    }

    pub fn from_id(id: &str) -> Option<BrowseOrder> {
        BrowseOrder::ALL.into_iter().find(|order| order.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            BrowseOrder::Popular => "Popular now",
            BrowseOrder::Newest => "Newest",
            BrowseOrder::Updated => "Recently updated",
            BrowseOrder::MostLiked => "Most liked",
            BrowseOrder::MostDownloaded => "Most downloaded",
            BrowseOrder::MostViewed => "Most viewed",
            BrowseOrder::MostCommented => "Most discussed",
            BrowseOrder::LatestComment => "Latest comment",
            BrowseOrder::AToZ => "Name, A to Z",
        }
    }

    /// The order a saved subfeed sort stands for.
    pub fn from_sort(sort: BrowseSort) -> Self {
        match sort {
            BrowseSort::Default => BrowseOrder::Popular,
            BrowseSort::New => BrowseOrder::Newest,
            BrowseSort::Updated => BrowseOrder::Updated,
        }
    }

    /// The `_sSort` alias `<Model>/Index` takes. `Popular` has none and reads as
    /// most liked once a filter rules out the subfeed.
    pub fn index_token(self) -> &'static str {
        match self {
            BrowseOrder::Popular | BrowseOrder::MostLiked => "Generic_MostLiked",
            BrowseOrder::Newest => "Generic_Newest",
            BrowseOrder::Updated => "Generic_LatestUpdated",
            BrowseOrder::MostDownloaded => "Generic_MostDownloaded",
            BrowseOrder::MostViewed => "Generic_MostViewed",
            BrowseOrder::MostCommented => "Generic_MostCommented",
            BrowseOrder::LatestComment => "Generic_LatestComment",
            BrowseOrder::AToZ => "Generic_Alphabetically",
        }
    }
}

/// Which stamp a dated browse window reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum DateField {
    /// When the submission was first published (`_tsDateAdded`).
    #[default]
    Added,
    /// When it last changed (`_tsDateModified` or `_tsDateUpdated`).
    Updated,
}

impl DateField {
    pub const ALL: [DateField; 2] = [DateField::Added, DateField::Updated];

    pub fn label(self) -> &'static str {
        match self {
            DateField::Added => "Added",
            DateField::Updated => "Updated",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            DateField::Added => "added",
            DateField::Updated => "updated",
        }
    }

    pub fn from_id(id: &str) -> DateField {
        if id.eq_ignore_ascii_case("updated") {
            DateField::Updated
        } else {
            DateField::Added
        }
    }
}

/// How far back a browse reaches by the date a submission was first added.
/// GameBanana has no date filter, so a bounded query reads newest first and
/// stops at the cutoff; the caller then orders the window itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum BrowsePeriod {
    #[default]
    Any,
    Day,
    Week,
    Month,
    Quarter,
}

impl BrowsePeriod {
    pub const ALL: [BrowsePeriod; 5] = [
        BrowsePeriod::Any,
        BrowsePeriod::Day,
        BrowsePeriod::Week,
        BrowsePeriod::Month,
        BrowsePeriod::Quarter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BrowsePeriod::Any => "Any time",
            BrowsePeriod::Day => "Past day",
            BrowsePeriod::Week => "Past week",
            BrowsePeriod::Month => "Past month",
            BrowsePeriod::Quarter => "Past 3 months",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            BrowsePeriod::Any => "any",
            BrowsePeriod::Day => "day",
            BrowsePeriod::Week => "week",
            BrowsePeriod::Month => "month",
            BrowsePeriod::Quarter => "quarter",
        }
    }

    pub fn from_id(id: &str) -> BrowsePeriod {
        BrowsePeriod::ALL
            .into_iter()
            .find(|period| period.id() == id)
            .unwrap_or(BrowsePeriod::Any)
    }

    /// Seconds the window reaches back. `None` for any time.
    pub fn seconds(self) -> Option<i64> {
        const DAY: i64 = 86_400;
        match self {
            BrowsePeriod::Any => None,
            BrowsePeriod::Day => Some(DAY),
            BrowsePeriod::Week => Some(7 * DAY),
            BrowsePeriod::Month => Some(30 * DAY),
            BrowsePeriod::Quarter => Some(91 * DAY),
        }
    }

    /// The oldest `added_ts` inside the window, given the current time.
    pub fn cutoff(self, now: i64) -> Option<i64> {
        self.seconds().map(|span| now - span)
    }
}

/// Everything that decides which page GameBanana serves. The browse grid keeps
/// one and rebuilds the catalog when it changes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct BrowseQuery {
    pub section: BrowseSection,
    /// A root category or a sub category id. `None` lists the whole section.
    pub category: Option<u64>,
    pub order: BrowseOrder,
    /// Matched against submission names on the server. Empty for no search.
    pub search: String,
    /// Only submissions GameBanana has featured.
    pub featured: bool,
    /// Only one creator's submissions, by member id.
    pub submitter: Option<u64>,
    /// A rolling window on `date_field`, for the period picker.
    pub period: BrowsePeriod,
    /// Which stamp `period`, `date_from` and `date_to` read.
    pub date_field: DateField,
    /// Oldest stamp the grid keeps, unix seconds. Zero for no start.
    pub date_from: i64,
    /// Newest stamp the grid keeps, unix seconds. Zero for no end.
    pub date_to: i64,
    /// The mixed Mod, Sound and WiP feed instead of one section. Only the plain
    /// subfeed serves several models at once; a filtered combined read lists Mods.
    pub all_models: bool,
}

impl BrowseQuery {
    /// True when the query is the plain subfeed: popular order, no filter.
    pub fn uses_subfeed(&self) -> bool {
        self.order == BrowseOrder::Popular
            && self.category.is_none()
            && self.search.trim().is_empty()
            && !self.featured
            && self.submitter.is_none()
            && self.period == BrowsePeriod::Any
            && self.date_from <= 0
            && self.date_to <= 0
    }

    /// The oldest stamp a dated window keeps, unix seconds. Zero for open.
    pub fn window_start(&self, now: i64) -> i64 {
        if self.date_from > 0 {
            return self.date_from;
        }
        self.period.cutoff(now).unwrap_or(0)
    }

    /// True when the grid must read a bounded window and order it here.
    pub fn bounded(&self) -> bool {
        self.period != BrowsePeriod::Any || self.date_from > 0 || self.date_to > 0
    }

    /// The stamp of a row the window reads.
    pub fn stamp(&self, row: &BrowseMod) -> i64 {
        match self.date_field {
            DateField::Added => row.added_ts,
            DateField::Updated => row.updated_ts,
        }
    }

    /// The order the server actually applies. `Popular` reads as most liked
    /// once a filter rules out the subfeed, and a bounded period reads newest
    /// first so the window can stop at its cutoff.
    pub fn server_order(&self) -> BrowseOrder {
        if self.bounded() {
            return match self.date_field {
                DateField::Added => BrowseOrder::Newest,
                DateField::Updated => BrowseOrder::Updated,
            };
        }
        if self.order == BrowseOrder::Popular && !self.uses_subfeed() {
            return BrowseOrder::MostLiked;
        }
        self.order
    }

    /// The order the grid shows: the server order, or the chosen one when the
    /// grid sorts a bounded window itself.
    pub fn shown_order(&self) -> BrowseOrder {
        if self.bounded() {
            if self.order == BrowseOrder::Popular {
                BrowseOrder::MostLiked
            } else {
                self.order
            }
        } else {
            self.server_order()
        }
    }

    /// The request URL for one page.
    pub fn url(&self, page: usize, perpage: usize) -> String {
        let page = page.max(1);
        let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
        let model = if self.all_models {
            "Mod"
        } else {
            self.section.model()
        };
        if self.uses_subfeed() {
            let models = if self.all_models {
                "Mod|Sound|Wip"
            } else {
                model
            };
            return format!(
                "{GAMEBANANA_API_BASE}/Game/{GAMEBANANA_GAME_ID}/Subfeed?_nPage={page}&_sSort=default&_csvModelInclusions={models}"
            );
        }
        let mut url = format!(
            "{GAMEBANANA_API_BASE}/{model}/Index?_nPage={page}&_nPerpage={perpage}&_sSort={}",
            self.server_order().index_token()
        );
        // A category filter already scopes the game; adding both is accepted,
        // but the game alone is needed when no category is set.
        match self.category {
            Some(category) => url.push_str(&format!("&_aFilters%5BGeneric_Category%5D={category}")),
            None => url.push_str(&format!("&_aFilters%5BGeneric_Game%5D={GAMEBANANA_GAME_ID}")),
        }
        let search = self.search.trim();
        if !search.is_empty() {
            url.push_str("&_aFilters%5BGeneric_Name%5D=contains,");
            url.push_str(&percent_encode(search));
        }
        if self.featured {
            url.push_str("&_aFilters%5BGeneric_WasFeatured%5D=true");
        }
        if let Some(submitter) = self.submitter {
            url.push_str(&format!("&_aFilters%5BGeneric_Submitter%5D={submitter}"));
        }
        url
    }
}

/// Percent-encode a query value: unreserved characters stay, everything else
/// (spaces, `&`, `#`, non-ASCII) becomes `%XX` bytes.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// One page of a filtered browse. Index pages serve `perpage` rows; the
/// subfeed serves its fixed 15.
pub fn fetch_browse(query: &BrowseQuery, page: usize, perpage: usize) -> Result<BrowsePage> {
    let page = page.max(1);
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let url = query.url(page, perpage);
    let body = get_body(&url, &format!("page {page}"))?;
    parse_browse(&body, page, perpage)
}

/// A subfeed or index body as a browse page, keeping Mod and Sound rows.
pub fn parse_browse(body: &str, page: usize, perpage: usize) -> Result<BrowsePage> {
    parse_records(body, page, perpage, Record::is_listed)
}

/// One category in the browse tree. A category carries its children: heroes under Skins, a
/// hero's styles under the hero, the NPC kinds under Model Replacement.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowseCategory {
    pub id: u64,
    pub name: String,
    pub icon_url: Option<String>,
    /// Submissions in the category and its children.
    pub count: u64,
    #[serde(default)]
    pub children: Vec<BrowseCategory>,
    /// `_nCategoryCount`: how many children GameBanana says the category has.
    #[serde(default)]
    pub child_count: u64,
}

impl BrowseCategory {
    /// The category's own id and every descendant's, for matching a row's leaf category.
    pub fn subtree_ids(&self) -> Vec<u64> {
        let mut ids = vec![self.id];
        for child in &self.children {
            ids.extend(child.subtree_ids());
        }
        ids
    }

    /// The category with `id` in this subtree.
    pub fn find(&self, id: u64) -> Option<&BrowseCategory> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }
}

/// The path from a root to the category with `id`, root first. Empty when no root holds it.
pub fn category_path(roots: &[BrowseCategory], id: u64) -> Vec<&BrowseCategory> {
    fn walk<'a>(node: &'a BrowseCategory, id: u64, path: &mut Vec<&'a BrowseCategory>) -> bool {
        path.push(node);
        if node.id == id || node.children.iter().any(|child| walk(child, id, path)) {
            return true;
        }
        path.pop();
        false
    }
    let mut path = Vec::new();
    for root in roots {
        if walk(root, id, &mut path) {
            return path;
        }
    }
    Vec::new()
}

/// Category reads in flight at once.
const CATEGORY_READS: usize = 8;

/// The category tree of a section two levels deep: the roots and the children of each root
/// that has any (heroes under Skins, kinds of HUD). [`deepen_categories`] reads the third.
pub fn fetch_categories(section: BrowseSection) -> Result<Vec<BrowseCategory>> {
    let model = section.model();
    let body = get_body(&categories_url(model, None), "the category list")?;
    let mut roots = parse_categories(&body)?;
    fill_children(model, &mut roots, 1);
    Ok(roots)
}

/// The third level of a tree [`fetch_categories`] read: a hero's styles, the NPC kinds under
/// Model Replacement. About one read per hero for Mods, so it runs after the first two levels
/// show. Only categories GameBanana says have children are read, a few at a time.
pub fn deepen_categories(section: BrowseSection, mut roots: Vec<BrowseCategory>) -> Vec<BrowseCategory> {
    fill_children(section.model(), &mut roots, 2);
    roots
}

/// `<Model>/Categories` for the game's roots, or for one category's children. Both carry
/// `_idRow` and `_nCategoryCount`.
pub fn categories_url(model: &str, parent: Option<u64>) -> String {
    match parent {
        None => format!(
            "{GAMEBANANA_API_BASE}/{model}/Categories?_idGameRow={GAMEBANANA_GAME_ID}&_sSort=a_to_z&_bShowEmpty=true"
        ),
        Some(id) => format!(
            "{GAMEBANANA_API_BASE}/{model}/Categories?_idCategoryRow={id}&_sSort=a_to_z&_bShowEmpty=true"
        ),
    }
}

/// Read the children of every node that has some, `depth` levels down. A failed read leaves
/// that node without children; the tree still shows what loaded.
fn fill_children(model: &str, nodes: &mut [BrowseCategory], depth: usize) {
    if depth == 0 {
        return;
    }
    let parents: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.child_count > 0 && node.children.is_empty())
        .map(|(index, _)| index)
        .collect();
    for batch in parents.chunks(CATEGORY_READS) {
        let read: Vec<Vec<BrowseCategory>> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|index| {
                    let id = nodes[*index].id;
                    scope.spawn(move || {
                        get_body(&categories_url(model, Some(id)), &format!("category {id}"))
                            .and_then(|body| parse_categories(&body))
                            .unwrap_or_default()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap_or_default())
                .collect()
        });
        for (index, children) in batch.iter().zip(read) {
            nodes[*index].children = children;
        }
    }
    for node in nodes.iter_mut() {
        fill_children(model, &mut node.children, depth - 1);
    }
}

/// A category's id: `_idRow`, or the number at the end of `_sUrl`, which is
/// the only id the sub category list carries.
fn category_id(value: &Value) -> u64 {
    let id = u64_of(value, "_idRow");
    if id > 0 {
        return id;
    }
    str_of(value, "_sUrl")
        .rsplit('/')
        .next()
        .and_then(|tail| tail.parse().ok())
        .unwrap_or(0)
}

/// A `<Model>/Categories` or `<Model>Category/<id>/SubCategories` body.
pub fn parse_categories(body: &str) -> Result<Vec<BrowseCategory>> {
    let value = json_value(body)?;
    let list = match &value {
        Value::Array(list) => list.clone(),
        _ => value
            .get("_aRecords")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    Ok(list
        .iter()
        .filter(|item| !item.get("_bIsObsolete").and_then(Value::as_bool).unwrap_or(false))
        .map(|item| BrowseCategory {
            id: category_id(item),
            name: decode_entities(&str_of(item, "_sName")),
            icon_url: Some(str_of(item, "_sIconUrl")).filter(|url| !url.is_empty()),
            count: u64_of(item, "_nItemCount"),
            children: Vec::new(),
            child_count: u64_of(item, "_nCategoryCount"),
        })
        .filter(|category| category.id > 0 && !category.name.is_empty())
        .collect())
}

/// The top submissions GameBanana lists per period (`Game/<id>/TopSubs`): a
/// few each for today, this week, this month and so on.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TopSub {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    /// `today`, `week`, `month`, `3month`, `6month`, `year` or `alltime`.
    #[serde(default)]
    pub period: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub likes: u64,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub profile_url: String,
    #[serde(default)]
    pub nsfw: bool,
    /// `_sDescription`: the submission's one-line summary. Empty when it has none.
    #[serde(default)]
    pub description: String,
}

pub fn fetch_top_subs() -> Result<Vec<TopSub>> {
    let url = format!("{GAMEBANANA_API_BASE}/Game/{GAMEBANANA_GAME_ID}/TopSubs");
    let body = get_body(&url, "the top list")?;
    parse_top_subs(&body)
}

pub fn parse_top_subs(body: &str) -> Result<Vec<TopSub>> {
    let value = json_value(body)?;
    let (records, _) = records_and_total(&value);
    Ok(records
        .iter()
        .map(|item| TopSub {
            id: u64_of(item, "_idRow"),
            name: decode_entities(&str_of(item, "_sName")),
            model: str_of(item, "_sModelName"),
            period: str_of(item, "_sPeriod"),
            author: item
                .get("_aSubmitter")
                .map(|member| str_of(member, "_sName"))
                .unwrap_or_default(),
            category: item
                .get("_aRootCategory")
                .map(|category| str_of(category, "_sName"))
                .unwrap_or_default(),
            likes: u64_of(item, "_nLikeCount"),
            image_url: Some(str_of(item, "_sImageUrl")).filter(|url| !url.is_empty()),
            profile_url: str_of(item, "_sProfileUrl"),
            nsfw: is_mature(None, false, Some(str_of(item, "_sInitialVisibility").as_str())),
            description: decode_entities(&str_of(item, "_sDescription")).trim().to_string(),
        })
        .filter(|top| top.id > 0)
        .collect())
}

/// [`fetch_deadlock_mods_page`] in a chosen order.
pub fn fetch_deadlock_mods_page_sorted(
    page: usize,
    perpage: usize,
    sort: BrowseSort,
) -> Result<BrowsePage> {
    let page = page.max(1);
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let url = subfeed_url(page, perpage, sort);
    let body = get_body(&url, &format!("page {page}"))?;
    parse_subfeed(&body, page, perpage)
}

// ---------------------------------------------------------------------------
// Collections: `Collection/Index` for Deadlock and `Collection/<id>/Items`
// ---------------------------------------------------------------------------

/// One GameBanana collection: a named bundle of submissions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub id: u64,
    pub name: String,
    pub profile_url: String,
    pub author: String,
    pub author_id: u64,
    /// `_nItemCount`: how many submissions the collection holds.
    pub item_count: u64,
    pub image_url: Option<String>,
    pub added_ts: i64,
    pub updated_ts: i64,
    pub likes: u64,
    pub comments: u64,
    pub nsfw: bool,
}

/// One page of the game's collections.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPage {
    pub page: usize,
    pub per_page: usize,
    pub total: usize,
    pub complete: bool,
    pub items: Vec<CollectionSummary>,
}

impl CollectionPage {
    pub fn page_count(&self) -> usize {
        if self.per_page == 0 {
            return 1;
        }
        self.total.div_ceil(self.per_page).max(1)
    }
}

pub fn fetch_collections(page: usize, perpage: usize) -> Result<CollectionPage> {
    let page = page.max(1);
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let url = format!(
        "{GAMEBANANA_API_BASE}/Collection/Index?_nPage={page}&_nPerpage={perpage}&_aFilters%5BGeneric_Game%5D={GAMEBANANA_GAME_ID}&_sSort=Generic_Newest"
    );
    let body = get_body(&url, &format!("collections page {page}"))?;
    parse_collections(&body, page, perpage)
}

/// A `Collection/Index` body as one page of collections.
pub fn parse_collections(body: &str, page: usize, perpage: usize) -> Result<CollectionPage> {
    let value = json_value(body)?;
    if value.get("_aRecords").is_none() {
        return Err(Error::Network(format!(
            "GameBanana response had no _aRecords array. {}",
            snippet(body)
        )));
    }
    let (records, total) = records_and_total(&value);
    let complete = value
        .get("_aMetadata")
        .and_then(|meta| meta.get("_bIsComplete"))
        .and_then(Value::as_bool);
    let items: Vec<CollectionSummary> = records
        .iter()
        .filter_map(|item| {
            let id = u64_of(item, "_idRow");
            (id > 0).then(|| CollectionSummary {
                id,
                name: decode_entities(&str_of(item, "_sName")),
                profile_url: str_of(item, "_sProfileUrl"),
                author: item
                    .get("_aSubmitter")
                    .map(|member| str_of(member, "_sName"))
                    .unwrap_or_default(),
                author_id: item
                    .get("_aSubmitter")
                    .map(|member| u64_of(member, "_idRow"))
                    .unwrap_or(0),
                item_count: u64_of(item, "_nItemCount"),
                image_url: item
                    .get("_aPreviewMedia")
                    .and_then(|media| media.get("_aImages"))
                    .and_then(Value::as_array)
                    .and_then(|list| list.first())
                    .and_then(image_of)
                    .map(|image| image.url),
                added_ts: i64_of(item, "_tsDateAdded"),
                updated_ts: latest_change(
                    i64_of(item, "_tsDateAdded"),
                    i64_of(item, "_tsDateModified"),
                    i64_of(item, "_tsDateUpdated"),
                ),
                likes: u64_of(item, "_nLikeCount"),
                comments: u64_of(item, "_nPostCount"),
                nsfw: is_mature(
                    item.get("_bIsNsfw").and_then(Value::as_bool),
                    item.get("_bHasContentRatings")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    Some(str_of(item, "_sInitialVisibility").as_str()),
                ),
            })
        })
        .collect();
    Ok(CollectionPage {
        page,
        per_page: perpage,
        total: total.max(items.len()),
        complete: complete.unwrap_or(items.len() < perpage),
        items,
    })
}

/// One page of a collection's submissions, as browse rows (Mods and Sounds).
pub fn fetch_collection_items(collection_id: u64, page: usize, perpage: usize) -> Result<BrowsePage> {
    let page = page.max(1);
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let url = format!(
        "{GAMEBANANA_API_BASE}/Collection/{collection_id}/Items?_nPage={page}&_nPerpage={perpage}"
    );
    let body = get_body(&url, &format!("collection {collection_id} page {page}"))?;
    parse_browse(&body, page, perpage)
}

/// The provider name of an embedded video link, for its button.
pub fn video_site(url: &str) -> &'static str {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if host.ends_with("youtu.be") || host.ends_with("youtube.com") || host.ends_with("youtube-nocookie.com") {
        "YouTube"
    } else if host.ends_with("vimeo.com") {
        "Vimeo"
    } else {
        "Video"
    }
}


/// One agent for every API read, so requests reuse pooled connections instead of a new TLS
/// handshake each (GameBanana throttles bursts of fresh connections). Built again when the
/// proxy settings change.
fn api_agent() -> ureq::Agent {
    static AGENT: std::sync::Mutex<Option<(u64, ureq::Agent)>> = std::sync::Mutex::new(None);
    let generation = crate::net::proxy_generation();
    let mut slot = AGENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((built_for, agent)) = slot.as_ref()
        && *built_for == generation
    {
        return agent.clone();
    }
    let agent = crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
        .build()
        .new_agent();
    *slot = Some((generation, agent.clone()));
    agent
}

/// A failure worth one more try: HTTP 429 or 5xx, a timeout, or a reset connection. Offline
/// mode's refusal is not.
fn retryable(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => true,
        ureq::Error::Io(io) => io.kind() != std::io::ErrorKind::PermissionDenied,
        _ => false,
    }
}

/// One GET against the public API with the shared timeout and user agent.
/// `what` names the request in an error, for example `page 3`.
pub(crate) fn get_body(url: &str, what: &str) -> Result<String> {
    let agent = api_agent();
    let send = || {
        agent
            .get(url)
            .header("User-Agent", GAMEBANANA_USER_AGENT)
            .header("Accept", "application/json")
            .call()
    };
    // One retry after a short pause for a rate limit, a server hiccup or a dropped connection:
    // a burst of reads (a page, its rails, a hovered mod) can meet a 429 that clears in a second.
    let mut response = match send() {
        Err(error) if retryable(&error) => {
            std::thread::sleep(Duration::from_millis(900));
            send()
        }
        other => other,
    }
    .map_err(|error| Error::Network(format!("GameBanana request to {url} failed: {error}")))?;

    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|error| {
        Error::Network(format!(
            "GameBanana response for {what} could not be read: {error}"
        ))
    })?;

    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "GameBanana returned HTTP {status} for {url}. {}",
            snippet(&body)
        )));
    }

    Ok(body)
}

/// Turn a raw subfeed body into a browse page. Kept separate from the request so
/// a cached body can be parsed without network access.
pub fn parse_subfeed(body: &str, page: usize, perpage: usize) -> Result<BrowsePage> {
    parse_records(body, page, perpage, Record::is_mod)
}

fn parse_records(
    body: &str,
    page: usize,
    perpage: usize,
    keep: fn(&Record) -> bool,
) -> Result<BrowsePage> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        Error::Network(format!(
            "GameBanana response was not valid JSON ({error}). {}",
            snippet(body)
        ))
    })?;

    if let Some(message) = api_error_message(&value) {
        return Err(Error::Network(format!(
            "GameBanana rejected the request: {message}"
        )));
    }

    if value.get("_aRecords").is_none() {
        return Err(Error::Network(format!(
            "GameBanana response had no _aRecords array. {}",
            snippet(body)
        )));
    }

    let feed: Subfeed = serde_json::from_value(value).map_err(|error| {
        Error::Network(format!(
            "GameBanana response did not match the subfeed shape: {error}. {}",
            snippet(body)
        ))
    })?;

    let returned = feed.records.len();
    let stride = if feed.metadata.per_page > 0 {
        feed.metadata.per_page
    } else {
        returned
    };

    let mut mods: Vec<BrowseMod> = feed
        .records
        .into_iter()
        .filter(|record| keep(record))
        .map(Record::into_browse_mod)
        .collect();
    mods.truncate(perpage);

    Ok(BrowsePage {
        page,
        per_page: stride.max(1),
        total: feed.metadata.record_count,
        complete: feed.metadata.is_complete,
        mods,
    })
}

/// Human-readable listing used by `dah browse`.
pub fn format_browse_mods(page: &BrowsePage) -> String {
    if page.mods.is_empty() {
        return format!(
            "GameBanana: no mods on page {} ({} feed records).",
            page.page, page.total
        );
    }

    let mut lines = vec![format!(
        "GameBanana: {} mod(s) on page {} of about {} ({} feed records)",
        page.mods.len(),
        page.page,
        page.page_count(),
        page.total
    )];

    for item in &page.mods {
        lines.push(format!("[{}] {}", item.id, item.name));

        let mut facts = Vec::new();
        if !item.author.is_empty() {
            facts.push(format!("by {}", item.author));
        }
        let category = item.category_line();
        if !category.is_empty() {
            facts.push(category);
        }
        facts.push(format!("{} likes, {} views", item.likes, item.views));
        if let Some(version) = &item.version {
            facts.push(format!("version {version}"));
        }
        if let Some(date) = short_date(&item.updated_at) {
            facts.push(format!("updated {date}"));
        }
        if !item.has_files {
            facts.push("no files".to_string());
        }

        lines.push(format!("  {}", facts.join("  |  ")));
        lines.push(format!("  {}", item.profile_url));
    }

    lines.join("\n")
}

fn subfeed_url(page: usize, perpage: usize, sort: BrowseSort) -> String {
    // GameBanana pages with `_nPage` and orders with `_sSort`. The `page` query
    // key is ignored by the Subfeed endpoint, so paging through it silently
    // repeats page 1.
    format!(
        "{GAMEBANANA_API_BASE}/Game/{GAMEBANANA_GAME_ID}/Subfeed?_nPage={page}&perpage={perpage}&_sSort={}",
        sort.query()
    )
}

fn short_date(timestamp: &str) -> Option<&str> {
    let date = timestamp.get(0..10)?;
    if date.len() == 10 { Some(date) } else { None }
}

/// One line of a non-JSON body, trimmed so the error stays readable.
fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "The response body was empty.".to_string();
    }

    let mut text: String = collapsed.chars().take(BODY_SNIPPET_CHARS).collect();
    if collapsed.chars().count() > BODY_SNIPPET_CHARS {
        text.push_str("...");
    }
    format!("Response starts with: {text}")
}

fn api_error_message(value: &Value) -> Option<String> {
    for key in ["_sErrorCode", "error", "message"] {
        match value.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => {
                return Some(text.trim().to_string());
            }
            Some(other) if other.is_object() || other.is_string() => {
                return Some(other.to_string());
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Default, Deserialize)]
struct Subfeed {
    #[serde(rename = "_aMetadata", default)]
    metadata: SubfeedMetadata,
    #[serde(rename = "_aRecords", default)]
    records: Vec<Record>,
}

#[derive(Debug, Default, Deserialize)]
struct SubfeedMetadata {
    #[serde(rename = "_nRecordCount", default)]
    record_count: usize,
    #[serde(rename = "_nPerpage", default)]
    per_page: usize,
    #[serde(rename = "_bIsComplete", default)]
    is_complete: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Record {
    #[serde(rename = "_idRow", default)]
    id: u64,
    #[serde(rename = "_sName", default)]
    name: String,
    #[serde(rename = "_sProfileUrl", default)]
    profile_url: String,
    #[serde(rename = "_sModelName", default)]
    model: String,
    #[serde(rename = "_tsDateAdded", default)]
    date_added: i64,
    #[serde(rename = "_tsDateModified", default)]
    date_modified: i64,
    #[serde(rename = "_tsDateUpdated", default)]
    date_updated: i64,
    #[serde(rename = "_bHasContentRatings", default)]
    has_content_ratings: bool,
    #[serde(rename = "_sInitialVisibility", default)]
    initial_visibility: Option<String>,
    #[serde(rename = "_bIsNsfw", default)]
    is_nsfw: Option<bool>,
    #[serde(rename = "_sVersion", default)]
    version: Option<String>,
    #[serde(rename = "_nLikeCount", default)]
    likes: u64,
    #[serde(rename = "_nViewCount", default)]
    views: u64,
    #[serde(rename = "_nPostCount", default)]
    posts: u64,
    #[serde(rename = "_bWasFeatured", default)]
    was_featured: bool,
    #[serde(rename = "_bIsObsolete", default)]
    obsolete: bool,
    #[serde(rename = "_bHasFiles", default)]
    has_files: bool,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Option<Named>,
    #[serde(rename = "_aRootCategory", default)]
    root_category: Option<Named>,
    #[serde(rename = "_aSubCategory", default)]
    sub_category: Option<Named>,
    #[serde(rename = "_aPreviewMedia", default)]
    preview_media: Option<PreviewMedia>,
}

impl Record {
    fn is_mod(&self) -> bool {
        self.model.eq_ignore_ascii_case("mod")
    }

    /// A row a filtered browse keeps: a mod, a sound or a WiP, never a
    /// tutorial or a news post.
    fn is_listed(&self) -> bool {
        ["mod", "sound", "wip"]
            .iter()
            .any(|model| self.model.eq_ignore_ascii_case(model))
    }

    fn into_browse_mod(self) -> BrowseMod {
        let author_id = self.submitter.as_ref().map(|value| value.id).unwrap_or(0);
        let audio_url = self
            .preview_media
            .as_ref()
            .and_then(|media| media.metadata.as_ref())
            .map(|meta| meta.audio_url.trim().to_string())
            .filter(|url| !url.is_empty());
        let updated_seconds = latest_change(self.date_added, self.date_modified, self.date_updated);
        let nsfw = is_mature(
            self.is_nsfw,
            self.has_content_ratings,
            self.initial_visibility.as_deref(),
        );

        BrowseMod {
            id: self.id,
            name: self.name,
            profile_url: self.profile_url,
            model: self.model,
            author: self.submitter.map(|value| value.name).unwrap_or_default(),
            category: self
                .root_category
                .map(|value| value.name)
                .unwrap_or_default(),
            sub_category_id: self.sub_category.as_ref().map(Named::row_id).unwrap_or(0),
            subcategory: self
                .sub_category
                .map(|value| value.name)
                .unwrap_or_default(),
            version: normalize_optional(self.version),
            updated_at: if updated_seconds > 0 {
                unix_seconds_to_iso(updated_seconds)
            } else {
                String::new()
            },
            likes: self.likes,
            views: self.views,
            has_files: self.has_files,
            thumbnail_url: self.preview_media.and_then(PreviewMedia::thumbnail),
            added_ts: self.date_added.max(0),
            updated_ts: updated_seconds.max(0),
            nsfw,
            audio_url,
            featured: self.was_featured,
            comments: self.posts,
            author_id,
            obsolete: self.obsolete,
        }
    }
}

/// The later of the modified and updated stamps, else the date added.
fn latest_change(added: i64, modified: i64, updated: i64) -> i64 {
    let latest = modified.max(updated);
    if latest > 0 { latest } else { added }
}

/// GameBanana's list endpoints omit `_bIsNsfw`, so a content rating or a
/// visibility other than `show` counts as mature too, as Grimoire does.
fn is_mature(flag: Option<bool>, has_ratings: bool, visibility: Option<&str>) -> bool {
    flag.unwrap_or(false)
        || has_ratings
        || visibility.is_some_and(|value| {
            let value = value.trim();
            value.eq_ignore_ascii_case("warn") || value.eq_ignore_ascii_case("hide")
        })
}

#[derive(Debug, Default, Deserialize)]
struct Named {
    #[serde(rename = "_idRow", default)]
    id: u64,
    #[serde(rename = "_sName", default)]
    name: String,
    /// Categories on list rows carry no `_idRow`; their profile URL ends in the id.
    #[serde(rename = "_sProfileUrl", default)]
    profile_url: String,
}

impl Named {
    /// `_idRow`, or the number the profile URL ends in.
    fn row_id(&self) -> u64 {
        if self.id > 0 {
            return self.id;
        }
        self.profile_url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .and_then(|tail| tail.parse().ok())
            .unwrap_or(0)
    }
}

#[derive(Debug, Default, Deserialize)]
struct PreviewMedia {
    #[serde(rename = "_aImages", default)]
    images: Vec<Image>,
    #[serde(rename = "_aMetadata", default)]
    metadata: Option<MediaMetadata>,
}

#[derive(Debug, Default, Deserialize)]
struct MediaMetadata {
    #[serde(rename = "_sAudioUrl", default)]
    audio_url: String,
}

impl PreviewMedia {
    /// First image with a 530px or 220px variant. The image and the size suffix
    /// are separate fields, so the URL is joined here.
    fn thumbnail(self) -> Option<String> {
        self.images.into_iter().find_map(Image::thumbnail)
    }
}

#[derive(Debug, Default, Deserialize)]
struct Image {
    #[serde(rename = "_sBaseUrl", default)]
    base_url: String,
    #[serde(rename = "_sFile530", default)]
    file_530: Option<String>,
    #[serde(rename = "_sFile220", default)]
    file_220: Option<String>,
}

impl Image {
    fn thumbnail(self) -> Option<String> {
        // Grid cards are 300 to 380 points wide, over 530 pixels on a 2x
        // screen, so the 220 variant would look soft. Speed comes from the
        // parallel fetch, connection reuse and the disk cache instead.
        let file = self
            .file_530
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| self.file_220.as_deref().filter(|value| !value.is_empty()))?;

        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return None;
        }

        Some(format!("{base}/{file}"))
    }
}

/// Seconds since the Unix epoch on this machine's clock.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// How long ago `ts` was, as a card shows it: `just now`, `5m ago`, `2h ago`,
/// `3d ago`, `4mo ago`, `2y ago`. Empty when the feed gave no date.
pub fn relative_age(ts: i64, now: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let age = (now - ts).max(0);
    if age < MINUTE {
        "just now".to_string()
    } else if age < HOUR {
        format!("{}m ago", age / MINUTE)
    } else if age < DAY {
        format!("{}h ago", age / HOUR)
    } else if age < 30 * DAY {
        format!("{}d ago", age / DAY)
    } else if age < 365 * DAY {
        format!("{}mo ago", age / (30 * DAY))
    } else {
        format!("{}y ago", age / (365 * DAY))
    }
}

/// A calendar date in the local time zone, for example `Nov 16, 2026`. Empty
/// when the feed gave no date.
pub fn calendar_date(ts: i64) -> String {
    use chrono::TimeZone;
    if ts <= 0 {
        return String::new();
    }
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(value) => value.format("%b %-d, %Y").to_string(),
        None => String::new(),
    }
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

// ---------------------------------------------------------------------------
// Mod details: `Mod/<id>/ProfilePage`, `Mod/<id>/Posts` and `Mod/<id>/Updates`
// ---------------------------------------------------------------------------

/// Everything the mod details view shows, read from `Mod/<id>/ProfilePage`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModProfile {
    pub id: u64,
    pub name: String,
    pub profile_url: String,
    /// `_sText` as plain text: paragraphs and line breaks kept, tags dropped.
    pub text: String,
    /// `_sLicense` as plain text, when the author picked one.
    #[serde(default)]
    pub license: Option<String>,
    pub version: Option<String>,
    pub added_ts: i64,
    pub updated_ts: i64,
    pub downloads: u64,
    pub likes: u64,
    pub views: u64,
    /// `_nPostCount`: comments on the mod page.
    pub post_count: u64,
    /// `_nUpdatesCount`: changelog entries.
    pub updates_count: u64,
    pub nsfw: bool,
    /// `_aContentRatings` labels, for example `Skimpy Attire`.
    pub content_ratings: Vec<String>,
    /// Leaf category, for example a hero name.
    pub category: String,
    /// The leaf category's icon, when GameBanana has one (hero portraits do).
    pub category_icon: Option<String>,
    /// Parent category, for example `Skins`. Empty when absent.
    pub super_category: String,
    pub submitter: Option<ModMember>,
    pub files: Vec<ModFile>,
    pub archived_files: Vec<ModFile>,
    pub images: Vec<ModImage>,
    /// A sound page's sample, `_aPreviewMedia._aMetadata._sAudioUrl`.
    #[serde(default)]
    pub audio_url: Option<String>,
    /// Videos the submission embeds (`_aEmbeddedMedia`), as given. Rune has no
    /// video surface, so the details view opens them in the browser.
    #[serde(default)]
    pub videos: Vec<String>,
    /// GameBanana took the submission down (`_bIsWithheld`).
    #[serde(default)]
    pub withheld: bool,
    /// The creator marked it obsolete (`_bIsObsolete`).
    #[serde(default)]
    pub obsolete: bool,
    /// Only the submitter can see it (`_bIsPrivate`).
    #[serde(default)]
    pub private: bool,
    /// The author's note on an obsolete submission (`_sObsolescenceNotice`), as plain text.
    /// `None` when the mod is not obsolete or the author wrote nothing.
    #[serde(default)]
    pub obsolete_note: Option<String>,
    /// Why GameBanana staff withheld the submission (`_aWithholdNotice`). `None` when it is not
    /// withheld, or GameBanana gave no notice it could read.
    #[serde(default)]
    pub withhold: Option<WithholdNotice>,
}

/// GameBanana's notice on a withheld submission: the rules it broke, the staff's note and who
/// withheld it when.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WithholdNotice {
    /// `(code, name)` per rule, the code without its trailing title: `("R.17", "Nonsense")`.
    pub rules: Vec<(String, String)>,
    /// The staff's note (`_sNotes`, HTML on GameBanana) as plain text.
    pub notes: String,
    /// The staff member's name (`_aWithholder._sName`).
    pub by: Option<String>,
    /// When it was withheld, unix seconds (`_tsDateWithheld`).
    pub at: Option<i64>,
}

impl ModProfile {
    /// GameBanana does not show this submission publicly: withheld or private. DMM drops these
    /// from Featured and the saved catalog.
    pub fn is_hidden(&self) -> bool {
        self.withheld || self.private
    }
}

/// Read `_aWithholdNotice`. GameBanana sends an empty array when there is none.
fn withhold_of(value: Option<&Value>) -> Option<WithholdNotice> {
    let value = value.filter(|v| v.is_object())?;
    let rules = value
        .get("_aRulesViolated")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|rule| {
                    let code = str_of(rule, "_sCode");
                    let code = code.split_once(':').map_or(code.as_str(), |(code, _)| code).trim().to_string();
                    let name = str_of(rule, "_sName").trim().to_string();
                    (!code.is_empty() || !name.is_empty()).then_some((code, name))
                })
                .take(12)
                .collect()
        })
        .unwrap_or_default();
    let notes = html_to_text(&str_of(value, "_sNotes")).chars().take(4000).collect::<String>();
    let by = value
        .get("_aWithholder")
        .map(|member| str_of(member, "_sName"))
        .filter(|name| !name.trim().is_empty());
    let at = value.get("_tsDateWithheld").and_then(Value::as_i64).filter(|at| *at > 0);
    Some(WithholdNotice {
        rules,
        notes: notes.trim().to_string(),
        by,
        at,
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModMember {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
    pub profile_url: String,
    /// Donation and support links the member lists (`_aDonationMethods`).
    #[serde(default)]
    pub donations: Vec<ProfileLink>,
    /// The profile bio as plain text, for the links it carries.
    #[serde(default)]
    pub bio: String,
}

/// A named link from a GameBanana profile: a donation method or a social page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLink {
    pub title: String,
    pub url: String,
}

/// One downloadable file on a mod page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModFile {
    pub id: u64,
    pub name: String,
    pub size_bytes: u64,
    pub downloads: u64,
    pub added_ts: i64,
    pub description: String,
    pub download_url: String,
    pub archived: bool,
}

/// One gallery image. `url` is the largest preview GameBanana serves for it
/// (800, then 530, then the original file); `full_url` is always the original.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModImage {
    pub url: String,
    pub full_url: String,
    pub caption: String,
    /// Width over height from any size variant, so a view can reserve the
    /// right box before the image decodes. Zero when unknown.
    pub aspect: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModComment {
    pub id: u64,
    pub poster: ModMember,
    /// Plain text, as `ModProfile::text`.
    pub text: String,
    pub added_ts: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdate {
    pub id: u64,
    pub title: String,
    pub version: Option<String>,
    /// Plain text body. Empty when the update only has labelled changes.
    pub text: String,
    /// `_aChangeLog` lines as `(label, text)`, for example `("Bugfix", "...")`.
    pub changes: Vec<(String, String)>,
    pub added_ts: i64,
}

/// A page of comments plus the total the mod page reports.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModComments {
    pub items: Vec<ModComment>,
    pub total: usize,
}

/// A page of changelog entries plus the total.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdates {
    pub items: Vec<ModUpdate>,
    pub total: usize,
}

/// Read one mod page. Read only, no key.
pub fn fetch_mod_profile(id: u64) -> Result<ModProfile> {
    fetch_profile(BrowseSection::Mods, id)
}

/// Read the newest comments on a mod page.
pub fn fetch_mod_comments(id: u64, perpage: usize) -> Result<ModComments> {
    fetch_comments(BrowseSection::Mods, id, perpage)
}

/// Read the newest changelog entries on a mod page.
pub fn fetch_mod_updates(id: u64, perpage: usize) -> Result<ModUpdates> {
    fetch_updates(BrowseSection::Mods, id, perpage)
}

/// Read one submission page of either model. A sound page has the same shape
/// as a mod page plus an audio sample.
pub fn fetch_profile(section: BrowseSection, id: u64) -> Result<ModProfile> {
    let model = section.model();
    let url = format!("{GAMEBANANA_API_BASE}/{model}/{id}/ProfilePage");
    let body = get_body(&url, &format!("{} {id}", model.to_ascii_lowercase()))?;
    parse_mod_profile_page(&body)
}

pub fn fetch_comments(section: BrowseSection, id: u64, perpage: usize) -> Result<ModComments> {
    fetch_comments_page(section, id, 1, perpage)
}

/// One page of comments. GameBanana serves 15 a page whatever `perpage` asks.
pub fn fetch_comments_page(
    section: BrowseSection,
    id: u64,
    page: usize,
    perpage: usize,
) -> Result<ModComments> {
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let page = page.max(1);
    let model = section.model();
    let url = format!("{GAMEBANANA_API_BASE}/{model}/{id}/Posts?_nPerpage={perpage}&_nPage={page}");
    let body = get_body(&url, &format!("comments on {id}"))?;
    parse_mod_comments(&body)
}

pub fn fetch_updates(section: BrowseSection, id: u64, perpage: usize) -> Result<ModUpdates> {
    fetch_updates_page(section, id, 1, perpage)
}

/// One page of changelog entries.
pub fn fetch_updates_page(
    section: BrowseSection,
    id: u64,
    page: usize,
    perpage: usize,
) -> Result<ModUpdates> {
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let page = page.max(1);
    let model = section.model();
    let url = format!("{GAMEBANANA_API_BASE}/{model}/{id}/Updates?_nPerpage={perpage}&_nPage={page}");
    let body = get_body(&url, &format!("updates on {id}"))?;
    parse_mod_updates(&body)
}

fn json_value(body: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(body).map_err(|error| {
        Error::Network(format!(
            "GameBanana response was not valid JSON ({error}). {}",
            snippet(body)
        ))
    })?;
    if let Some(message) = api_error_message(&value) {
        return Err(Error::Network(format!(
            "GameBanana rejected the request: {message}"
        )));
    }
    Ok(value)
}

fn str_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

fn u64_of(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn i64_of(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn member_of(value: Option<&Value>) -> Option<ModMember> {
    let value = value.filter(|value| value.is_object())?;
    let name = str_of(value, "_sName");
    if name.is_empty() {
        return None;
    }
    let donations = value
        .get("_aDonationMethods")
        .and_then(Value::as_array)
        .map(|methods| {
            methods
                .iter()
                .filter_map(|method| {
                    let title = str_of(method, "_sTitle");
                    let url = str_of(method, "_sValue");
                    (url.starts_with("http") && !title.is_empty()).then(|| ProfileLink {
                        title,
                        url: url.replace("\\/", "/"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ModMember {
        id: u64_of(value, "_idRow"),
        name,
        avatar_url: Some(str_of(value, "_sAvatarUrl")).filter(|url| !url.is_empty()),
        profile_url: str_of(value, "_sProfileUrl"),
        donations,
        bio: html_to_text(&str_of(value, "_aBio")),
    })
}

fn file_of(value: &Value, archived: bool) -> Option<ModFile> {
    let id = u64_of(value, "_idRow");
    let name = str_of(value, "_sFile");
    if id == 0 && name.is_empty() {
        return None;
    }
    Some(ModFile {
        id,
        name: if name.is_empty() { "download".to_string() } else { name },
        size_bytes: u64_of(value, "_nFilesize"),
        downloads: u64_of(value, "_nDownloadCount"),
        added_ts: i64_of(value, "_tsDateAdded"),
        description: str_of(value, "_sDescription"),
        download_url: str_of(value, "_sDownloadUrl"),
        archived: archived
            || value
                .get("_bIsArchived")
                .and_then(Value::as_bool)
                .unwrap_or(false),
    })
}

fn image_of(value: &Value) -> Option<ModImage> {
    let base = str_of(value, "_sBaseUrl");
    let base = base.trim_end_matches('/');
    let file = str_of(value, "_sFile");
    if base.is_empty() || file.is_empty() {
        return None;
    }
    let preview = ["_sFile800", "_sFile530"]
        .iter()
        .map(|key| str_of(value, key))
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| file.clone());
    let aspect = ["800", "530", "220", "100"]
        .iter()
        .find_map(|size| {
            let w = u64_of(value, &format!("_wFile{size}"));
            let h = u64_of(value, &format!("_hFile{size}"));
            (w > 0 && h > 0).then(|| w as f32 / h as f32)
        })
        .unwrap_or(0.0);
    Some(ModImage {
        url: format!("{base}/{preview}"),
        full_url: format!("{base}/{file}"),
        caption: str_of(value, "_sCaption"),
        aspect,
    })
}

/// Turn a `Mod/<id>/ProfilePage` body into the details view's data.
pub fn parse_mod_profile_page(body: &str) -> Result<ModProfile> {
    let value = json_value(body)?;
    let id = u64_of(&value, "_idRow");
    if id == 0 {
        return Err(Error::Network(format!(
            "GameBanana mod page had no _idRow. {}",
            snippet(body)
        )));
    }

    let files = value
        .get("_aFiles")
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(|file| file_of(file, false)).collect())
        .unwrap_or_default();
    let archived_files = value
        .get("_aArchivedFiles")
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(|file| file_of(file, true)).collect())
        .unwrap_or_default();
    let images = value
        .get("_aPreviewMedia")
        .and_then(|media| media.get("_aImages"))
        .and_then(Value::as_array)
        .map(|images| images.iter().filter_map(image_of).collect())
        .unwrap_or_default();
    let content_ratings: Vec<String> = match value.get("_aContentRatings") {
        Some(Value::Object(map)) => map
            .values()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    let visibility = str_of(&value, "_sInitialVisibility");
    let category = value.get("_aCategory");
    let added = i64_of(&value, "_tsDateAdded");

    Ok(ModProfile {
        id,
        name: str_of(&value, "_sName"),
        profile_url: str_of(&value, "_sProfileUrl"),
        text: html_to_text(&str_of(&value, "_sText")),
        license: Some(html_to_text(&str_of(&value, "_sLicense")))
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty()),
        version: Some(str_of(&value, "_sVersion")).filter(|version| !version.is_empty()),
        added_ts: added,
        updated_ts: latest_change(
            added,
            i64_of(&value, "_tsDateModified"),
            i64_of(&value, "_tsDateUpdated"),
        ),
        downloads: u64_of(&value, "_nDownloadCount"),
        likes: u64_of(&value, "_nLikeCount"),
        views: u64_of(&value, "_nViewCount"),
        post_count: u64_of(&value, "_nPostCount"),
        updates_count: u64_of(&value, "_nUpdatesCount"),
        nsfw: is_mature(
            value.get("_bIsNsfw").and_then(Value::as_bool),
            !content_ratings.is_empty(),
            Some(visibility.as_str()),
        ),
        content_ratings,
        category: category
            .map(|category| str_of(category, "_sName"))
            .unwrap_or_default(),
        category_icon: category
            .map(|category| str_of(category, "_sIconUrl"))
            .filter(|url| !url.is_empty()),
        super_category: value
            .get("_aSuperCategory")
            .map(|category| str_of(category, "_sName"))
            .unwrap_or_default(),
        submitter: member_of(value.get("_aSubmitter")),
        files,
        archived_files,
        images,
        audio_url: value
            .get("_aPreviewMedia")
            .and_then(|media| media.get("_aMetadata"))
            .map(|meta| str_of(meta, "_sAudioUrl"))
            .filter(|url| !url.is_empty()),
        videos: value
            .get("_aEmbeddedMedia")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(|url| url.replace("\\/", "/").trim().to_string())
                    .filter(|url| url.starts_with("http"))
                    .take(10)
                    .collect()
            })
            .unwrap_or_default(),
        withheld: value
            .get("_bIsWithheld")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        obsolete: value
            .get("_bIsObsolete")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        private: value
            .get("_bIsPrivate")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        obsolete_note: value
            .get("_bIsObsolete")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| html_to_text(&str_of(&value, "_sObsolescenceNotice")).trim().chars().take(4000).collect::<String>())
            .filter(|note| !note.is_empty()),
        withhold: value
            .get("_bIsWithheld")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| withhold_of(value.get("_aWithholdNotice")).unwrap_or_default()),
    })
}

fn records_and_total(value: &Value) -> (Vec<Value>, usize) {
    let records = match value {
        Value::Array(list) => list.clone(),
        _ => value
            .get("_aRecords")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    };
    let total = value
        .get("_aMetadata")
        .map(|meta| u64_of(meta, "_nRecordCount") as usize)
        .filter(|total| *total > 0)
        .unwrap_or(records.len());
    (records, total)
}

/// Turn a `Mod/<id>/Posts` body into comments.
pub fn parse_mod_comments(body: &str) -> Result<ModComments> {
    let value = json_value(body)?;
    let (records, total) = records_and_total(&value);
    let items = records
        .iter()
        .map(|post| ModComment {
            id: u64_of(post, "_idRow"),
            poster: member_of(post.get("_aPoster")).unwrap_or_else(|| ModMember {
                name: "Unknown".to_string(),
                ..ModMember::default()
            }),
            text: html_to_text(&str_of(post, "_sText")),
            added_ts: i64_of(post, "_tsDateAdded"),
        })
        .filter(|comment| !comment.text.is_empty())
        .collect();
    Ok(ModComments { items, total })
}

/// Turn a `Mod/<id>/Updates` body into changelog entries.
pub fn parse_mod_updates(body: &str) -> Result<ModUpdates> {
    let value = json_value(body)?;
    let (records, total) = records_and_total(&value);
    let items = records
        .iter()
        .enumerate()
        .map(|(index, update)| {
            let title = [str_of(update, "_sTitle"), str_of(update, "_sName")]
                .into_iter()
                .find(|title| !title.is_empty())
                .unwrap_or_default();
            let text = ["_sText", "_sChangeLog", "_sDescription"]
                .iter()
                .map(|key| str_of(update, key))
                .find(|text| !text.is_empty())
                .map(|text| html_to_text(&text))
                .unwrap_or_default();
            let changes = update
                .get("_aChangeLog")
                .and_then(Value::as_array)
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| (str_of(line, "cat"), html_to_text(&str_of(line, "text"))))
                        .filter(|(_, text)| !text.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let added = [
                i64_of(update, "_tsDateAdded"),
                i64_of(update, "_tsDateModified"),
                i64_of(update, "_tsDateUpdated"),
            ]
            .into_iter()
            .find(|ts| *ts > 0)
            .unwrap_or(0);
            let id = u64_of(update, "_idRow");
            ModUpdate {
                id: if id == 0 { index as u64 } else { id },
                title,
                version: Some(str_of(update, "_sVersion")).filter(|version| !version.is_empty()),
                text,
                changes,
                added_ts: added,
            }
        })
        .collect();
    Ok(ModUpdates { items, total })
}

/// GameBanana's rich text as plain text: block tags and `<br>` become line
/// breaks, list items get a dash, other tags are dropped and the common
/// entities are decoded. Runs of blank lines collapse to one.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        match name.as_str() {
            "br" => out.push('\n'),
            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "blockquote"
            | "tr" => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                if closing {
                    out.push('\n');
                }
            }
            "li" if !closing => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                out.push_str("- ");
            }
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);

    let decoded = decode_entities(&out);
    let mut lines: Vec<String> = Vec::new();
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|last| last.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// True for an `https` address on GameBanana or one of its subdomains, and nothing that only
/// looks like one: the host is plain letters, digits, dots and hyphens up to the first `/`, `?`
/// or `#`, so a user name, port, backslash or percent escape in front of a real host fails.
/// Addresses from an API response go through this before Sigil fetches or opens them.
pub fn is_gamebanana_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    if url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let plain = host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    plain
        && !host.contains("..")
        && (host == "gamebanana.com" || host.ends_with(".gamebanana.com"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_real_gamebanana_addresses_pass() {
        for good in [
            "https://gamebanana.com/members/42",
            "https://images.gamebanana.com/img/ss/mods/a.jpg",
            "https://gamebanana.com?x=1",
        ] {
            assert!(is_gamebanana_url(good), "{good}");
        }
        for bad in [
            "http://gamebanana.com/x",
            "https://evil.com/gamebanana.com",
            "https://gamebanana.com.evil.com/x",
            "https://notgamebanana.com/x",
            "https://user@gamebanana.com/x",
            "https://evil.com@gamebanana.com/x",
            "https://gamebanana.com:8080/x",
            "https://evil.com\\.gamebanana.com/x",
            "https://evil.com\\@gamebanana.com/x",
            "https://evil.com%2f.gamebanana.com/x",
            "https://evil.com#.gamebanana.com",
            "https://evil.com?.gamebanana.com",
            "https://a..gamebanana.com/x",
            "https://gamebanana.com/a b",
            "https://gamebanana.com/a\nb",
            "file:///C:/x",
            "javascript:alert(1)",
        ] {
            assert!(!is_gamebanana_url(bad), "{bad}");
        }
    }

    use super::*;

    #[test]
    fn subfeed_rows_carry_dates_and_the_mature_flag() {
        let body = r#"{
            "_aMetadata": {"_nRecordCount": 40, "_nPerpage": 15, "_bIsComplete": false},
            "_aRecords": [
                {"_idRow": 1, "_sModelName": "Mod", "_sName": "Plain",
                 "_tsDateAdded": 100, "_tsDateModified": 300, "_tsDateUpdated": 360,
                 "_bHasContentRatings": false, "_sInitialVisibility": "show"},
                {"_idRow": 2, "_sModelName": "Mod", "_sName": "Rated",
                 "_tsDateAdded": 200, "_bHasContentRatings": true, "_sInitialVisibility": "warn"},
                {"_idRow": 3, "_sModelName": "Mod", "_sName": "Hidden",
                 "_tsDateAdded": 50, "_sInitialVisibility": "hide"},
                {"_idRow": 4, "_sModelName": "Wip", "_sName": "Not a mod"}
            ]
        }"#;
        let page = parse_subfeed(body, 1, 20).unwrap();
        assert_eq!(page.mods.len(), 3);
        assert_eq!(page.mods[0].added_ts, 100);
        assert_eq!(page.mods[0].updated_ts, 360, "the later of modified and updated");
        assert!(!page.mods[0].nsfw);
        assert_eq!(page.mods[1].updated_ts, 200, "falls back to the date added");
        assert!(page.mods[1].nsfw);
        assert!(page.mods[2].nsfw, "a hidden initial visibility is mature");
        assert_eq!(page.page_count(), 3);
        assert!(!page.complete);
    }

    #[test]
    fn profile_page_reads_files_gallery_artist_and_text() {
        let body = r#"{
            "_idRow": 730155,
            "_sName": "Kurumi over Paradox",
            "_sProfileUrl": "https://gamebanana.com/mods/730155",
            "_sText": "Now in Deadlock.<br><br>Features:<br>Voice&nbsp;acting &amp; icons<ul><li>6 variants</li></ul>",
            "_sVersion": "1.2",
            "_tsDateAdded": 1792527295, "_tsDateModified": 1794869541, "_tsDateUpdated": 1794869586,
            "_nDownloadCount": 25901, "_nLikeCount": 9, "_nViewCount": 1000,
            "_nPostCount": 14, "_nUpdatesCount": 3,
            "_sInitialVisibility": "warn",
            "_aContentRatings": {"sa": "Skimpy Attire"},
            "_aCategory": {"_sName": "Paradox", "_sIconUrl": "https://images.gamebanana.com/img/ico/ModCategory/x.png"},
            "_aSuperCategory": {"_sName": "Skins", "_sIconUrl": ""},
            "_aSubmitter": {"_idRow": 6020123, "_sName": "10385",
                "_sAvatarUrl": "https://images.gamebanana.com/static/img/defaults/avatar.gif",
                "_sProfileUrl": "https://gamebanana.com/members/6020123"},
            "_aFiles": [
                {"_idRow": 1892616, "_sFile": "kurumi.zip", "_nFilesize": 22851562,
                 "_tsDateAdded": 1794869486, "_nDownloadCount": 0,
                 "_sDownloadUrl": "https://gamebanana.com/dl/1892616",
                 "_sDescription": "Punishing Gray Raven model", "_bIsArchived": false}
            ],
            "_aArchivedFiles": [
                {"_idRow": 1, "_sFile": "old.zip", "_nFilesize": 10, "_tsDateAdded": 5,
                 "_nDownloadCount": 3, "_sDownloadUrl": "https://gamebanana.com/dl/1"}
            ],
            "_aPreviewMedia": {"_aImages": [
                {"_sBaseUrl": "https://images.gamebanana.com/img/ss/mods", "_sFile": "a.jpg",
                 "_sFile530": "530-90_a.jpg", "_wFile530": 530, "_hFile530": 475, "_sCaption": "PGR model"},
                {"_sBaseUrl": "https://images.gamebanana.com/img/ss/mods/", "_sFile": "b.jpg",
                 "_sFile100": "100-90_b.jpg", "_wFile100": 100, "_hFile100": 50}
            ]}
        }"#;
        let profile = parse_mod_profile_page(body).unwrap();
        assert_eq!(profile.id, 730155);
        assert_eq!(profile.version.as_deref(), Some("1.2"));
        assert_eq!(profile.updated_ts, 1794869586);
        assert_eq!(profile.downloads, 25901);
        assert!(profile.nsfw);
        assert_eq!(profile.content_ratings, vec!["Skimpy Attire".to_string()]);
        assert_eq!(profile.category, "Paradox");
        assert!(profile.category_icon.is_some());
        assert_eq!(profile.super_category, "Skins");
        assert_eq!(profile.submitter.as_ref().unwrap().name, "10385");
        assert_eq!(profile.files.len(), 1);
        assert_eq!(profile.files[0].size_bytes, 22851562);
        assert_eq!(profile.files[0].description, "Punishing Gray Raven model");
        assert_eq!(profile.archived_files.len(), 1);
        assert!(profile.archived_files[0].archived);
        assert_eq!(profile.images.len(), 2);
        assert_eq!(
            profile.images[0].url,
            "https://images.gamebanana.com/img/ss/mods/530-90_a.jpg"
        );
        assert_eq!(
            profile.images[1].url,
            "https://images.gamebanana.com/img/ss/mods/b.jpg",
            "an image with no preview size uses the original"
        );
        assert!((profile.images[1].aspect - 2.0).abs() < 0.001);
        assert_eq!(
            profile.text,
            "Now in Deadlock.\n\nFeatures:\nVoice acting & icons\n- 6 variants"
        );
    }

    #[test]
    fn comments_and_updates_parse_with_totals() {
        let posts = r#"{"_aMetadata": {"_nRecordCount": 14},
            "_aRecords": [{"_idRow": 9, "_tsDateAdded": 10,
              "_sText": "<p>could you use the pgr model?</p>",
              "_aPoster": {"_idRow": 1, "_sName": "HydrogenGT", "_sAvatarUrl": "https://x/y.jpg"}}]}"#;
        let comments = parse_mod_comments(posts).unwrap();
        assert_eq!(comments.total, 14);
        assert_eq!(comments.items[0].poster.name, "HydrogenGT");
        assert_eq!(comments.items[0].text, "could you use the pgr model?");

        let updates = r#"{"_aMetadata": {"_nRecordCount": 3},
            "_aRecords": [{"_idRow": 476791, "_sName": "NEW MODEL", "_tsDateAdded": 20,
              "_sText": "<p>New model</p>", "_aChangeLog": [{"text": "Hair fixed", "cat": "Bugfix"}]}]}"#;
        let updates = parse_mod_updates(updates).unwrap();
        assert_eq!(updates.total, 3);
        assert_eq!(updates.items[0].title, "NEW MODEL");
        assert_eq!(updates.items[0].text, "New model");
        assert_eq!(
            updates.items[0].changes,
            vec![("Bugfix".to_string(), "Hair fixed".to_string())]
        );
    }

    #[test]
    fn browse_query_urls_match_the_live_api() {
        let plain = BrowseQuery::default();
        assert!(plain.uses_subfeed());
        assert_eq!(
            plain.url(2, 50),
            "https://gamebanana.com/apiv11/Game/20948/Subfeed?_nPage=2&_sSort=default&_csvModelInclusions=Mod"
        );

        let filtered = BrowseQuery {
            category: Some(33295),
            search: "long hair & co".to_string(),
            featured: true,
            ..BrowseQuery::default()
        };
        assert!(!filtered.uses_subfeed());
        assert_eq!(filtered.server_order(), BrowseOrder::MostLiked, "popular reads as most liked");
        assert_eq!(
            filtered.url(1, 50),
            "https://gamebanana.com/apiv11/Mod/Index?_nPage=1&_nPerpage=50&_sSort=Generic_MostLiked\
             &_aFilters%5BGeneric_Category%5D=33295\
             &_aFilters%5BGeneric_Name%5D=contains,long%20hair%20%26%20co\
             &_aFilters%5BGeneric_WasFeatured%5D=true"
        );

        let sounds = BrowseQuery {
            section: BrowseSection::Sounds,
            order: BrowseOrder::MostDownloaded,
            submitter: Some(7),
            ..BrowseQuery::default()
        };
        assert_eq!(
            sounds.url(3, 99),
            "https://gamebanana.com/apiv11/Sound/Index?_nPage=3&_nPerpage=50&_sSort=Generic_MostDownloaded\
             &_aFilters%5BGeneric_Game%5D=20948&_aFilters%5BGeneric_Submitter%5D=7"
        );

        let windowed = BrowseQuery {
            order: BrowseOrder::MostViewed,
            period: BrowsePeriod::Week,
            ..BrowseQuery::default()
        };
        assert_eq!(windowed.server_order(), BrowseOrder::Newest, "a window reads newest first");
        assert_eq!(windowed.shown_order(), BrowseOrder::MostViewed);
        assert_eq!(BrowsePeriod::Week.cutoff(1_000_000), Some(1_000_000 - 7 * 86_400));
        assert_eq!(BrowseOrder::from_sort(BrowseSort::Updated), BrowseOrder::Updated);
    }

    #[test]
    fn index_rows_keep_sounds_and_read_audio_featured_and_submitter() {
        let body = r#"{
            "_aMetadata": {"_nRecordCount": 2, "_nPerpage": 50, "_bIsComplete": true},
            "_aRecords": [
                {"_idRow": 83198, "_sModelName": "Sound", "_sName": "Drifter Flow",
                 "_nPostCount": 11, "_bWasFeatured": true, "_bIsObsolete": true,
                 "_aSubmitter": {"_idRow": 4913678, "_sName": "Hopelessone"},
                 "_aPreviewMedia": {"_aMetadata": {"_sAudioUrl": "https://files.gamebanana.com/preview/sounds/a.mp3"}}},
                {"_idRow": 5, "_sModelName": "Tutorial", "_sName": "How to"}
            ]
        }"#;
        let page = parse_browse(body, 1, 50).unwrap();
        assert_eq!(page.mods.len(), 1, "a tutorial is not listed");
        let sound = &page.mods[0];
        assert_eq!(sound.model, "Sound");
        assert_eq!(sound.audio_url.as_deref(), Some("https://files.gamebanana.com/preview/sounds/a.mp3"));
        assert!(sound.featured);
        assert_eq!(sound.comments, 11);
        assert_eq!(sound.author_id, 4913678);
        assert!(sound.obsolete);
        assert!(parse_subfeed(body, 1, 50).unwrap().mods.is_empty(), "the plain feed keeps mods only");
    }

    #[test]
    fn categories_read_ids_from_rows_or_urls() {
        let roots = r#"[
            {"_idRow": 33295, "_sName": "Skins", "_nItemCount": 4257, "_nCategoryCount": 44,
             "_sUrl": "https://gamebanana.com/mods/cats/33295", "_sIconUrl": ""},
            {"_idRow": 1, "_sName": "Old", "_bIsObsolete": true},
            {"_idRow": 31710, "_sName": "Other/Misc", "_nItemCount": 163, "_nCategoryCount": 0}
        ]"#;
        let parsed = parse_categories(roots).unwrap();
        assert_eq!(parsed.len(), 2, "obsolete categories are dropped");
        assert_eq!(parsed[0].id, 33295);
        assert_eq!(parsed[0].icon_url, None);
        assert_eq!(parsed[0].child_count, 44);
        assert_eq!(parsed[1].child_count, 0);

        let subs = r#"[{"_sName": "Mo &amp; Krill", "_sUrl": "https://gamebanana.com/mods/cats/33320",
            "_sIconUrl": "https://images.gamebanana.com/img/ico/ModCategory/x.png", "_nItemCount": 61}]"#;
        let parsed = parse_categories(subs).unwrap();
        assert_eq!(parsed[0].id, 33320, "the id comes from the URL");
        assert_eq!(parsed[0].name, "Mo & Krill");
        assert_eq!(parsed[0].count, 61);
    }

    #[test]
    fn a_row_keeps_its_leaf_category_id() {
        let body = r#"{"_aMetadata": {"_nRecordCount": 1, "_bIsComplete": true, "_nPerpage": 50},
            "_aRecords": [{"_idRow": 7, "_sModelName": "Mod", "_sName": "Minigun Haze",
                "_aRootCategory": {"_sName": "Skins", "_sProfileUrl": "https://gamebanana.com/mods/cats/33295"},
                "_aSubCategory": {"_sName": "Faithful/Vanilla+", "_sProfileUrl": "https://gamebanana.com/mods/cats/50617"}}]}"#;
        let page = parse_browse(body, 1, 50).unwrap();
        assert_eq!(page.mods[0].subcategory, "Faithful/Vanilla+");
        assert_eq!(page.mods[0].sub_category_id, 50617);
    }

    #[test]
    fn category_paths_and_subtrees_reach_the_third_level() {
        let leaf = |id: u64, name: &str| BrowseCategory { id, name: name.into(), ..Default::default() };
        let haze = BrowseCategory { children: vec![leaf(50617, "Faithful/Vanilla+")], child_count: 1, ..leaf(33311, "Haze") };
        let skins = BrowseCategory { children: vec![haze, leaf(33306, "Abrams")], child_count: 2, ..leaf(33295, "Skins") };
        let roots = vec![leaf(31710, "Other/Misc"), skins];
        let path: Vec<u64> = category_path(&roots, 50617).iter().map(|c| c.id).collect();
        assert_eq!(path, vec![33295, 33311, 50617]);
        assert!(category_path(&roots, 9).is_empty());
        assert_eq!(roots[1].subtree_ids(), vec![33295, 33311, 50617, 33306]);
        assert_eq!(roots[1].find(33311).map(|c| c.subtree_ids()), Some(vec![33311, 50617]));
        assert!(categories_url("Sound", Some(6271)).ends_with("Sound/Categories?_idCategoryRow=6271&_sSort=a_to_z&_bShowEmpty=true"));
    }

    #[test]
    fn top_subs_parse_periods() {
        let body = r#"[{"_idRow": 741478, "_sModelName": "Mod", "_sName": "Long Hair Haze",
            "_sPeriod": "today", "_nLikeCount": 9, "_sInitialVisibility": "warn",
            "_sImageUrl": "https://images.gamebanana.com/img/ss/mods/800-90_a.jpg",
            "_sDescription": " Longer hair for Haze &amp; physics ",
            "_aSubmitter": {"_sName": "747shove"}, "_aRootCategory": {"_sName": "Skins"}}]"#;
        let top = parse_top_subs(body).unwrap();
        assert_eq!(top[0].description, "Longer hair for Haze & physics");
        assert_eq!(top[0].period, "today");
        assert_eq!(top[0].author, "747shove");
        assert!(top[0].nsfw);
    }

    #[test]
    fn relative_age_rounds_down_to_the_largest_unit() {
        let now = 1_800_000_000;
        assert_eq!(relative_age(0, now), "");
        assert_eq!(relative_age(now - 20, now), "just now");
        assert_eq!(relative_age(now - 5 * 60, now), "5m ago");
        assert_eq!(relative_age(now - 2 * 3600 - 59, now), "2h ago");
        assert_eq!(relative_age(now - 3 * 86400, now), "3d ago");
        assert_eq!(relative_age(now - 65 * 86400, now), "2mo ago");
        assert_eq!(relative_age(now - 800 * 86400, now), "2y ago");
        assert_eq!(relative_age(now + 100, now), "just now", "a clock skew is not negative");
        assert_eq!(calendar_date(0), "");
        assert!(calendar_date(1_794_869_586).ends_with(", 2026"));
    }

    #[test]
    fn html_to_text_keeps_breaks_and_decodes_entities() {
        assert_eq!(html_to_text("a<br>b<br/><br />c"), "a\nb\n\nc");
        assert_eq!(html_to_text("<p>one</p><p>two</p>"), "one\n\ntwo");
        assert_eq!(html_to_text("&lt;tag&gt; &#39;q&#x27; &unknown; 5 & 6"), "<tag> 'q' &unknown; 5 & 6");
        assert_eq!(html_to_text("<a href=\"x\">link</a>   text"), "link text");
        assert_eq!(html_to_text(""), "");
    }

    #[test]
    fn collections_parse_counts_and_authors() {
        let body = r#"{"_aMetadata": {"_nRecordCount": 1339, "_nPerpage": 15, "_bIsComplete": false},
            "_aRecords": [{
                "_idRow": 164637, "_sModelName": "Collection",
                "_sName": "Tamie's mod suggestions", "_sProfileUrl": "https://gamebanana.com/collections/164637",
                "_tsDateAdded": 1753796776, "_tsDateModified": 1786221493,
                "_nItemCount": 241, "_nLikeCount": 1, "_nPostCount": 3,
                "_aSubmitter": {"_idRow": 4099386, "_sName": "tamie128"},
                "_aPreviewMedia": {"_aImages": [{"_sBaseUrl": "https://images.gamebanana.com/img/ss/collections",
                    "_sFile": "a.jpg", "_sFile530": "530-90_a.jpg"}]}
            }]}"#;
        let page = parse_collections(body, 1, 15).unwrap();
        assert_eq!(page.total, 1339);
        assert_eq!(page.items[0].item_count, 241);
        assert_eq!(page.items[0].author, "tamie128");
        assert_eq!(page.items[0].author_id, 4_099_386);
        assert_eq!(
            page.items[0].image_url.as_deref(),
            Some("https://images.gamebanana.com/img/ss/collections/530-90_a.jpg")
        );
        assert!(!page.complete);
        assert!(parse_collections("{}", 1, 15).is_err());
    }

    #[test]
    fn profile_reads_videos_donations_and_notices() {
        let body = r#"{"_idRow": 741608, "_sName": "Random Respawn Dances",
            "_aEmbeddedMedia": ["https://www.youtube.com/watch?v=PbThLMeH-tg", "https://vimeo.com/1247326746"],
            "_bIsWithheld": true, "_bIsObsolete": true, "_bIsPrivate": false,
            "_aSubmitter": {"_idRow": 5, "_sName": "747shove", "_aBio": "<p>Hi</p>",
                "_aDonationMethods": [{"_sTitle": "Ko-fi Profile", "_sValue": "https:\/\/ko-fi.com\/747shove"}]},
            "_aFiles": [], "_aPreviewMedia": {"_aImages": []}}"#;
        let profile = parse_mod_profile_page(body).unwrap();
        assert_eq!(profile.videos.len(), 2);
        assert!(profile.withheld);
        assert!(profile.obsolete);
        assert!(!profile.private);
        let member = profile.submitter.unwrap();
        assert_eq!(member.donations[0].title, "Ko-fi Profile");
        assert_eq!(member.donations[0].url, "https://ko-fi.com/747shove");
        assert_eq!(member.bio, "Hi");
        assert_eq!(video_site(&profile.videos[0]), "YouTube");
        assert_eq!(video_site(&profile.videos[1]), "Vimeo");
    }

    /// DMM `7ede98c1` and `cba597f2`: the author's note on an obsolete mod and the staff's
    /// notice on a withheld one, in their own words.
    #[test]
    fn profile_reads_the_obsolete_note_and_the_withhold_notice() {
        let body = r#"{"_idRow": 9, "_sName": "Old HUD", "_bIsObsolete": true,
            "_sObsolescenceNotice": "<p>Use <b>New HUD</b> instead.</p>",
            "_bIsWithheld": true,
            "_aWithholdNotice": {"_tsDateWithheld": 1762600000,
                "_aRulesViolated": [{"_sCode": "R.17: Nonsense", "_sName": "Nonsense"}],
                "_sNotes": "<p>Broken download.<br>Fix the archive.</p>",
                "_aWithholder": {"_idRow": 3, "_sName": "Staffer"}},
            "_aFiles": [], "_aPreviewMedia": {"_aImages": []}}"#;
        let profile = parse_mod_profile_page(body).unwrap();
        assert_eq!(profile.obsolete_note.as_deref(), Some("Use New HUD instead."));
        let notice = profile.withhold.unwrap();
        assert_eq!(notice.rules, [("R.17".to_string(), "Nonsense".to_string())]);
        assert_eq!(notice.notes, "Broken download.\nFix the archive.");
        assert_eq!(notice.by.as_deref(), Some("Staffer"));
        assert_eq!(notice.at, Some(1_762_600_000));
        // Withheld without a readable notice still says so; no note when not obsolete.
        let bare = parse_mod_profile_page(r#"{"_idRow": 9, "_bIsWithheld": true, "_aWithholdNotice": [],
            "_sObsolescenceNotice": "stale", "_aFiles": []}"#)
        .unwrap();
        assert_eq!(bare.withhold, Some(WithholdNotice::default()));
        assert_eq!(bare.obsolete_note, None);
        assert!(bare.is_hidden());
    }

    #[test]
    fn dated_queries_name_their_stamp_and_the_mixed_feed() {
        let combined = BrowseQuery {
            all_models: true,
            ..BrowseQuery::default()
        };
        assert!(combined.uses_subfeed());
        assert!(combined.url(1, 50).contains("_csvModelInclusions=Mod|Sound|Wip"));
        let updated = BrowseQuery {
            period: BrowsePeriod::Week,
            date_field: DateField::Updated,
            ..BrowseQuery::default()
        };
        assert_eq!(updated.server_order(), BrowseOrder::Updated);
        assert!(updated.url(1, 50).contains("_sSort=Generic_LatestUpdated"));
        let range = BrowseQuery {
            date_from: 10,
            date_to: 20,
            ..BrowseQuery::default()
        };
        assert!(range.bounded());
        assert_eq!(range.window_start(1_000), 10);
    }
}
