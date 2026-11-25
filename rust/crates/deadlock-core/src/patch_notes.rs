//! Official Deadlock news from the Steam news feed for app 1422450.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::{now_iso, unix_seconds_to_iso, write_atomic};

pub const DEADLOCK_NEWS_URL: &str = "https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/";
pub const PATCH_NOTES_CACHE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchNote {
    pub id: String,
    pub title: String,
    pub url: String,
    pub author: String,
    /// ISO 8601 timestamp.
    pub published_at: String,
    /// Raw feed contents. Use `strip_patch_note_html` for display.
    pub contents: String,
    pub tags: Vec<String>,
    pub is_patch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PatchNoteCache {
    pub schema: u32,
    pub app_id: String,
    pub fetched_at: String,
    pub notes: Vec<PatchNote>,
}

impl Default for PatchNoteCache {
    fn default() -> Self {
        Self {
            schema: PATCH_NOTES_CACHE_SCHEMA,
            app_id: crate::discovery::DEADLOCK_APP_ID.to_string(),
            fetched_at: String::new(),
            notes: Vec::new(),
        }
    }
}

pub fn fetch_patch_notes(count: usize) -> Result<Vec<PatchNote>> {
    let url = format!(
        "{DEADLOCK_NEWS_URL}?appid={}&count={count}&maxlength=0",
        crate::discovery::DEADLOCK_APP_ID
    );

    crate::net::check().map_err(Error::Network)?;
    let mut response = crate::net::get(&url)
        .header("User-Agent", concat!("Sigil/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| Error::Network(format!("Steam news request failed: {error}")))?;

    let feed: NewsFeed = response.body_mut().read_json().map_err(|error| {
        Error::Network(format!("Steam news response was not valid JSON: {error}"))
    })?;

    Ok(feed
        .appnews
        .newsitems
        .into_iter()
        .map(NewsItem::into_patch_note)
        .collect())
}

pub fn patch_note_cache_path(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join("patches.json")
}

pub fn load_patch_note_cache(cache_path: &Path) -> Result<PatchNoteCache> {
    let text = match fs::read_to_string(cache_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PatchNoteCache::default());
        }
        Err(error) => return Err(error.into()),
    };

    let cache: PatchNoteCache = serde_json::from_str(&text)?;

    if cache.schema != PATCH_NOTES_CACHE_SCHEMA {
        return Err(Error::msg(format!(
            "Patch note cache {} uses unsupported schema {}.",
            cache_path.display(),
            cache.schema
        )));
    }

    Ok(cache)
}

pub fn save_patch_note_cache(cache_path: &Path, cache: &PatchNoteCache) -> Result<()> {
    let mut text = serde_json::to_string(cache)?;
    text.push('\n');
    write_atomic(cache_path, text.as_bytes())
}

/// Notes in `notes` whose ids are absent from the cache.
pub fn new_since_cache(cache: &PatchNoteCache, notes: &[PatchNote]) -> Vec<PatchNote> {
    let known: std::collections::HashSet<&str> =
        cache.notes.iter().map(|note| note.id.as_str()).collect();

    notes
        .iter()
        .filter(|note| !note.id.is_empty() && !known.contains(note.id.as_str()))
        .cloned()
        .collect()
}

pub fn detect_is_patch(title: &str, contents: &str, tags: &[String]) -> bool {
    let tag_match = tags.iter().any(|tag| tag.to_lowercase().contains("patch"));

    if tag_match {
        return true;
    }

    let title = title.to_lowercase();
    let contents = strip_patch_note_html(contents).to_lowercase();

    title.contains("patch notes")
        || title.contains("update")
        || title.contains("hotfix")
        || title.contains("patch")
        || contents.contains("patch notes")
        || contents.contains("hotfix")
}

pub fn strip_patch_note_html(contents: &str) -> String {
    let breaks = Regex::new(r"(?i)<\s*br\s*/?\s*>|</\s*p\s*>").expect("break pattern is valid");
    let tags = Regex::new(r"<[^>]*>").expect("tag pattern is valid");
    let bbcode = Regex::new(r"\[[^\[\]]*\]").expect("bbcode pattern is valid");

    let mut text = breaks.replace_all(contents, "\n").into_owned();
    text = tags.replace_all(&text, "").into_owned();
    text = bbcode.replace_all(&text, "").into_owned();
    text = decode_entities(&text);

    let mut lines = Vec::new();
    for line in text.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !collapsed.is_empty() {
            lines.push(collapsed);
        }
    }

    lines.join("\n").trim().to_string()
}

pub fn format_patch_note(note: &PatchNote) -> String {
    let mut lines = vec![format!("{} ({})", note.title, note.published_at)];

    if !note.url.is_empty() {
        lines.push(note.url.clone());
    }

    if !note.author.is_empty() {
        lines.push(format!("Author: {}", note.author));
    }

    let body = strip_patch_note_html(&note.contents);

    if !body.is_empty() {
        let body = truncate_text(&body, 600);
        lines.push(body);
    }

    lines.join("\n")
}

pub fn format_patch_notes(notes: &[PatchNote]) -> String {
    if notes.is_empty() {
        return "No patch notes.".to_string();
    }

    notes
        .iter()
        .map(format_patch_note)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }

    let mut output: String = value.chars().take(limit).collect();
    output.push_str("...");
    output
}

fn decode_entities(value: &str) -> String {
    let named = [
        ("&nbsp;", " "),
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#39;", "'"),
        ("&#x27;", "'"),
    ];

    let mut text = value.to_string();

    for (entity, replacement) in named {
        if text.contains(entity) {
            text = text.replace(entity, replacement);
        }
    }

    let numeric = Regex::new(r"&#(x?)([0-9A-Fa-f]+);").expect("entity pattern is valid");

    numeric
        .replace_all(&text, |captures: &regex::Captures<'_>| {
            let radix = if captures.get(1).map(|value| value.as_str()) == Some("x") {
                16
            } else {
                10
            };
            let Some(digits) = captures.get(2) else {
                return String::new();
            };
            u32::from_str_radix(digits.as_str(), radix)
                .ok()
                .and_then(char::from_u32)
                .map(|character| character.to_string())
                .unwrap_or_default()
        })
        .into_owned()
}

#[derive(Debug, Deserialize)]
struct NewsFeed {
    appnews: AppNews,
}

#[derive(Debug, Deserialize)]
struct AppNews {
    #[serde(default)]
    newsitems: Vec<NewsItem>,
}

#[derive(Debug, Deserialize)]
struct NewsItem {
    #[serde(default)]
    gid: serde_json::Value,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    contents: String,
    #[serde(default)]
    date: i64,
    #[serde(default)]
    tags: Vec<String>,
}

impl NewsItem {
    fn into_patch_note(self) -> PatchNote {
        let id = match self.gid {
            serde_json::Value::String(value) => value,
            serde_json::Value::Number(value) => value.to_string(),
            _ => String::new(),
        };

        let is_patch = detect_is_patch(&self.title, &self.contents, &self.tags);

        PatchNote {
            id,
            title: self.title,
            url: self.url,
            author: self.author,
            published_at: unix_seconds_to_iso(self.date),
            contents: self.contents,
            tags: self.tags,
            is_patch,
        }
    }
}

/// Called by a caller that wants to stamp a freshly fetched cache.
pub fn cache_from_notes(notes: Vec<PatchNote>) -> PatchNoteCache {
    PatchNoteCache {
        schema: PATCH_NOTES_CACHE_SCHEMA,
        app_id: crate::discovery::DEADLOCK_APP_ID.to_string(),
        fetched_at: now_iso(),
        notes,
    }
}
