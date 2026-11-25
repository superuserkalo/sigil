//! The public catalog search index (ADR 0018): one string table and, per GameBanana mod, the
//! title and packed field ids, built by Sigil's Worker from GameBanana's own public text.
//!
//! The app downloads it by default: the request carries no id and nothing about the player
//! (only the headers every Worker call sends), and the answer is the same for everyone. It is
//! not behind a consent switch, but it goes through the offline gate and the proxy like every
//! other download. Search runs on the PC over this file ([`matcher`]); typed text never leaves.
//!
//! The download is untrusted input: the body is capped (8 MB on the wire by its length header,
//! 40 MB decoded), every list and string is capped while it parses, and a body that fails any
//! check is dropped while the previous file stays. A corrupt file on disk reads as "no index",
//! so Browse falls back to GameBanana's title search instead of breaking.
//!
//! Files, under `<data>/catalog-index/`: `index.json` (the body as served, after it validated),
//! `meta.json` (the ETag and when the server was last asked) and `cards.json` (GameBanana card
//! rows fetched by id for search results, so a repeat search shows at once and offline).

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Read;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::gamebanana::BrowseMod;
use crate::util::write_atomic;

pub mod matcher;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod eval;

pub use matcher::{Found, Hit, Matcher, normalize};

/// The index route on Sigil's Worker.
pub const PATH: &str = "/v1/catalog/tags";
/// The index format this app reads (`PUBLISH_VERSION` in the Worker's `publish.ts`).
pub const SUPPORTED_VERSION: u32 = 3;
/// Field ids are packed as `stringId * PACK + class`.
pub const PACK: u32 = 8;
/// The server is asked at most this often (its `Cache-Control: max-age`).
pub const REFRESH_EVERY_MS: u64 = 3 * 60 * 60 * 1000;

/// Body caps. The real index (about 2,500 mods) is under 1 MB decoded, 300 KB on the wire.
pub const MAX_COMPRESSED_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_BODY_BYTES: u64 = 40 * 1024 * 1024;
pub const MAX_MODS: usize = 100_000;
pub const MAX_STRINGS: usize = 1_000_000;
pub const MAX_FIELDS_PER_MOD: usize = 512;
/// The longest title or string kept. GameBanana titles are far shorter.
pub const MAX_STRING_BYTES: usize = 512;
/// Card rows kept in `cards.json`, newest first.
pub const MAX_CARDS: usize = 6_000;

/// One mod in the index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexMod {
    pub id: u64,
    pub title: String,
    /// When the Worker last changed this mod's text, unix ms.
    pub updated: u64,
    /// `stringId * PACK + class`, only ids inside the string table.
    pub fields: Vec<u32>,
    /// GameBanana likes (`l`), when the server sends them.
    pub likes: Option<u64>,
    /// The page has content ratings (`n`: 1). None when the server sends no rating key for
    /// this mod; `Some(false)` is not sent today but reads the same as unrated.
    pub rated: Option<bool>,
    /// When the mod was added to GameBanana (`a`), unix seconds.
    pub added: Option<u64>,
}

/// The parsed, validated index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogIndex {
    pub version: u32,
    /// When the Worker built it, unix ms.
    pub built: u64,
    pub strings: Vec<String>,
    pub mods: Vec<IndexMod>,
}

impl CatalogIndex {
    /// The normalised strings of one class a mod carries, for example its categories.
    pub fn strings_of(&self, m: &IndexMod, class: u8) -> impl Iterator<Item = &str> {
        m.fields
            .iter()
            .filter(move |packed| (*packed % PACK) as u8 == class)
            .filter_map(|packed| self.strings.get((packed / PACK) as usize).map(String::as_str))
    }
}

// --- parsing -----------------------------------------------------------------------

/// A JSON array that refuses to grow past `N` items while it parses, so a hostile body cannot
/// allocate more than the caps allow.
struct Capped<T, const N: usize>(Vec<T>);

impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Capped<T, N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Seq<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Seq<T, N> {
            type Value = Capped<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "an array of at most {N} items")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error> {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1024));
                while let Some(item) = seq.next_element()? {
                    if out.len() >= N {
                        return Err(de::Error::custom(format!("more than {N} items")));
                    }
                    out.push(item);
                }
                Ok(Capped(out))
            }
        }
        deserializer.deserialize_seq(Seq::<T, N>(PhantomData))
    }
}

#[derive(Deserialize)]
struct RawIndex {
    v: u32,
    built: u64,
    strings: Capped<String, MAX_STRINGS>,
    mods: Capped<RawMod, MAX_MODS>,
}

#[derive(Deserialize)]
struct RawMod {
    i: u64,
    ti: String,
    #[serde(default)]
    u: u64,
    x: Capped<u64, MAX_FIELDS_PER_MOD>,
    /// Optional keys added with the version unchanged: likes, content rating, added time.
    #[serde(default)]
    l: Option<u64>,
    #[serde(default)]
    n: Option<u8>,
    #[serde(default)]
    a: Option<u64>,
}

/// Parse and check an index body. Any failure rejects the whole body: wrong types, a version
/// this app does not read, a list or string over its cap, a zero mod id. Field ids that point
/// outside the string table are dropped (the reference matcher skips them too).
pub fn parse(body: &[u8]) -> Result<CatalogIndex> {
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err(Error::msg("the search index is larger than the cap"));
    }
    let raw: RawIndex = serde_json::from_slice(body)?;
    if raw.v != SUPPORTED_VERSION {
        return Err(Error::msg(format!("search index version {} is not {SUPPORTED_VERSION}", raw.v)));
    }
    let strings = raw.strings.0;
    if let Some(long) = strings.iter().find(|s| s.len() > MAX_STRING_BYTES) {
        return Err(Error::msg(format!("a search index string is {} bytes", long.len())));
    }
    let count = strings.len() as u64;
    let mut seen = std::collections::HashSet::new();
    let mut mods = Vec::with_capacity(raw.mods.0.len());
    for m in raw.mods.0 {
        if m.i == 0 {
            return Err(Error::msg("a search index mod has id 0"));
        }
        if m.ti.len() > MAX_STRING_BYTES {
            return Err(Error::msg(format!("a search index title is {} bytes", m.ti.len())));
        }
        if !seen.insert(m.i) {
            continue;
        }
        let fields = m
            .x
            .0
            .into_iter()
            .filter(|packed| packed / u64::from(PACK) < count)
            .filter_map(|packed| u32::try_from(packed).ok())
            .collect();
        mods.push(IndexMod {
            id: m.i,
            title: m.ti,
            updated: m.u,
            fields,
            likes: m.l,
            rated: m.n.map(|flag| flag == 1),
            added: m.a,
        });
    }
    Ok(CatalogIndex {
        version: raw.v,
        built: raw.built,
        strings,
        mods,
    })
}

// --- storage -----------------------------------------------------------------------

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("catalog-index")
}

pub fn index_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("index.json")
}

fn meta_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("meta.json")
}

fn cards_path(data_dir: &Path) -> PathBuf {
    dir(data_dir).join("cards.json")
}

/// The ETag of the saved index and when the server was last asked.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub checked_ms: u64,
}

pub fn read_meta(data_dir: &Path) -> Meta {
    fs::read(meta_path(data_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_meta(data_dir: &Path, meta: &Meta) -> Result<()> {
    write_atomic(&meta_path(data_dir), &serde_json::to_vec(meta)?)
}

/// The saved index, or None when there is none or it does not validate.
pub fn load(data_dir: &Path) -> Option<CatalogIndex> {
    let path = index_path(data_dir);
    let size = fs::metadata(&path).ok()?.len();
    if size > MAX_BODY_BYTES {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    match parse(&bytes) {
        Ok(index) => Some(index),
        Err(error) => {
            eprintln!("[sigil catalog] the saved search index was ignored: {error}");
            None
        }
    }
}

/// True when the server should be asked again: never asked, the last ask is older than
/// [`REFRESH_EVERY_MS`], or no valid index is saved.
pub fn refresh_due(data_dir: &Path, now_ms: u64, have_index: bool) -> bool {
    let meta = read_meta(data_dir);
    !have_index || meta.checked_ms == 0 || now_ms.saturating_sub(meta.checked_ms) >= REFRESH_EVERY_MS
}

// --- download ----------------------------------------------------------------------

/// What a refresh found.
#[derive(Debug)]
pub enum Refresh {
    /// The server answered 304: the saved index is current.
    Unchanged,
    /// A new index was saved.
    Updated(CatalogIndex),
}

/// Ask the Worker for the index with the saved ETag, save a new one when it validates, and
/// record the time of the answer. `version` is the app version for `x-sigil-version`. Any
/// failure leaves the saved index as it was. A body that fails validation still counts as an
/// answer, so a server serving garbage is asked again after the usual pause, not every start;
/// a network failure is not recorded, and the caller retries after its own pause.
pub fn refresh(data_dir: &Path, version: &str, now_ms: u64) -> Result<Refresh> {
    refresh_from(data_dir, crate::telemetry::ENDPOINT, version, now_ms)
}

pub(crate) fn refresh_from(data_dir: &Path, base: &str, version: &str, now_ms: u64) -> Result<Refresh> {
    let mut meta = read_meta(data_dir);
    let have_index = load(data_dir).is_some();
    let etag = if have_index { meta.etag.clone() } else { None };
    let url = crate::telemetry::endpoint_at(base, PATH);
    let fetched = fetch(&url, etag.as_deref(), version)?;
    meta.checked_ms = now_ms;
    match fetched {
        Fetched::NotModified => {
            write_meta(data_dir, &meta)?;
            Ok(Refresh::Unchanged)
        }
        Fetched::Body { bytes, etag } => {
            let index = match parse(&bytes) {
                Ok(index) => index,
                Err(error) => {
                    write_meta(data_dir, &meta)?;
                    return Err(error);
                }
            };
            write_atomic(&index_path(data_dir), &bytes)?;
            meta.etag = etag;
            write_meta(data_dir, &meta)?;
            Ok(Refresh::Updated(index))
        }
    }
}

enum Fetched {
    NotModified,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

fn agent() -> ureq::Agent {
    crate::net::with_proxy(crate::net::builder())
        .middleware(crate::net::gate)
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn fetch(url: &str, etag: Option<&str>, version: &str) -> Result<Fetched> {
    let mut request = agent()
        .get(url)
        .header("Accept", "application/json")
        .header("User-Agent", "Sigil")
        .header("x-sigil-client", crate::telemetry::CLIENT_HEADER)
        .header("x-sigil-version", crate::telemetry::app_version(version));
    if let Some(etag) = etag.filter(|tag| !tag.is_empty() && tag.len() < 200 && tag.is_ascii()) {
        request = request.header("If-None-Match", etag);
    }
    let mut response = request.call().map_err(|error| Error::Network(error.to_string()))?;
    let status = response.status().as_u16();
    if status == 304 {
        return Ok(Fetched::NotModified);
    }
    if !(200..300).contains(&status) {
        return Err(Error::Network(format!("the search index answered HTTP {status}")));
    }
    let wire = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    if wire.is_some_and(|length| length > MAX_COMPRESSED_BYTES) {
        return Err(Error::Network("the search index is larger than the cap".to_string()));
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    // The decoded body is read one byte past the cap so an oversized one is refused, not cut.
    let mut bytes = Vec::new();
    response
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES + 1)
        .reader()
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Error::Network(format!("the search index could not be read: {error}")))?;
    if bytes.len() as u64 > MAX_BODY_BYTES {
        return Err(Error::Network("the search index is larger than the cap".to_string()));
    }
    Ok(Fetched::Body { bytes, etag })
}

// --- card rows for results ---------------------------------------------------------------

/// A GameBanana row fetched by id for a search result, with when it was fetched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CachedCard {
    pub fetched: i64,
    pub row: BrowseMod,
}

/// The saved card rows by id. A missing or corrupt file reads as empty.
pub fn read_cards(data_dir: &Path) -> HashMap<u64, CachedCard> {
    let path = cards_path(data_dir);
    if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > MAX_BODY_BYTES {
        return HashMap::new();
    }
    fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<CachedCard>>(&bytes).ok())
        .map(|list| list.into_iter().filter(|card| card.row.id > 0).map(|card| (card.row.id, card)).collect())
        .unwrap_or_default()
}

/// Save the card rows, newest first, at most [`MAX_CARDS`].
pub fn write_cards(data_dir: &Path, cards: &HashMap<u64, CachedCard>) -> Result<()> {
    let mut list: Vec<&CachedCard> = cards.values().collect();
    list.sort_by(|a, b| b.fetched.cmp(&a.fetched).then(a.row.id.cmp(&b.row.id)));
    list.truncate(MAX_CARDS);
    write_atomic(&cards_path(data_dir), &serde_json::to_vec(&list)?)
}
