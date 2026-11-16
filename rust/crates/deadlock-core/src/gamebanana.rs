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
    /// Sub category, for example a hero codename. Empty when absent.
    pub subcategory: String,
    pub version: Option<String>,
    /// ISO 8601 timestamp from `_tsDateModified`, falling back to the date added.
    pub updated_at: String,
    pub likes: u64,
    pub views: u64,
    pub has_files: bool,
    pub thumbnail_url: Option<String>,
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
    let page = page.max(1);
    let perpage = perpage.clamp(1, GAMEBANANA_MAX_PERPAGE);
    let url = subfeed_url(page, perpage);

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(REQUEST_TIMEOUT_SECONDS)))
        .build()
        .new_agent();

    let mut response = agent
        .get(&url)
        .header("User-Agent", GAMEBANANA_USER_AGENT)
        .header("Accept", "application/json")
        .call()
        .map_err(|error| Error::Network(format!("GameBanana request to {url} failed: {error}")))?;

    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|error| {
        Error::Network(format!(
            "GameBanana response for page {page} could not be read: {error}"
        ))
    })?;

    if !(200..300).contains(&status) {
        return Err(Error::Network(format!(
            "GameBanana returned HTTP {status} for {url}. {}",
            snippet(&body)
        )));
    }

    parse_subfeed(&body, page, perpage)
}

/// Turn a raw subfeed body into a browse page. Kept separate from the request so
/// a cached body can be parsed without network access.
pub fn parse_subfeed(body: &str, page: usize, perpage: usize) -> Result<BrowsePage> {
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
        .filter(|record| record.is_mod())
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

fn subfeed_url(page: usize, perpage: usize) -> String {
    // GameBanana pages with `_nPage`. The `page` query key is ignored by the
    // Subfeed endpoint, so paging through it silently repeats page 1.
    format!(
        "{GAMEBANANA_API_BASE}/Game/{GAMEBANANA_GAME_ID}/Subfeed?_nPage={page}&perpage={perpage}&sort=default"
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
    #[serde(rename = "_sVersion", default)]
    version: Option<String>,
    #[serde(rename = "_nLikeCount", default)]
    likes: u64,
    #[serde(rename = "_nViewCount", default)]
    views: u64,
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

    fn into_browse_mod(self) -> BrowseMod {
        let updated_seconds = if self.date_modified > 0 {
            self.date_modified
        } else {
            self.date_added
        };

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
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct Named {
    #[serde(rename = "_sName", default)]
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct PreviewMedia {
    #[serde(rename = "_aImages", default)]
    images: Vec<Image>,
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

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}
