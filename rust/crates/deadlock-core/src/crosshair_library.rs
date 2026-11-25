//! The community crosshair library (ADR 0019): the client for Sigil's Worker routes, the checks
//! a library row passes before the app shows it, and the local file that keeps delete tokens.
//!
//! Every request goes through [`crate::net::builder`] with the proxy from Settings and the
//! offline gate, and carries `x-sigil-client: 1` and `x-sigil-version`, like the app's other
//! Worker calls. Browsing and applying send nothing about the player; publishing sends the name
//! and the 19 look settings the player chose to publish.
//!
//! The list is untrusted input. A row is shown only when its id has the documented shape, its
//! name passes the same rules the Worker applies, and its config has exactly the documented keys
//! with every value in range and [`Crosshair::validate`] passing. A bad row is skipped; a body
//! that is too large, not JSON or the wrong shape fails as a whole so the caller keeps its cache.
//!
//! A delete token is a secret: [`DeleteToken`] never prints its value in `Debug` or `Display`,
//! the token file is written owner-only on Unix, and no log line carries it.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::crosshair::Crosshair;
use crate::error::{Error, Result};

/// The library routes live on the same Worker as telemetry and the search index.
pub const ENDPOINT: &str = crate::telemetry::ENDPOINT;
const LIST_PATH: &str = "/v1/crosshairs";

/// The local file in the app data directory: published ids with their delete tokens, and the
/// ids this player reported.
pub const STORE_FILE: &str = "crosshair-library.json";

/// A list response larger than this is refused. A full page of 60 rows is about 9 KiB.
pub const MAX_LIST_BYTES: u64 = 256 * 1024;
/// The Worker sends at most 60 rows a page; anything past that is ignored.
pub const MAX_PAGE_ROWS: usize = 60;
/// Small answers (publish, delete, use, report) are read up to this many bytes.
const MAX_SMALL_BYTES: u64 = 4 * 1024;
/// A cursor longer than this, or not base64url, is dropped and the walk ends.
const MAX_CURSOR: usize = 512;
/// Reported ids kept locally; the oldest go first.
const MAX_REPORTED: usize = 1000;
/// Published entries kept locally.
const MAX_PUBLISHED: usize = 500;
const STORE_FILE_MAX: u64 = 512 * 1024;

pub const NAME_MIN: usize = 2;
pub const NAME_MAX: usize = 32;

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const SMALL_TIMEOUT: Duration = Duration::from_secs(10);

/// The order of a list walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sort {
    /// Most uses first.
    #[default]
    Used,
    /// Newest first.
    New,
}

impl Sort {
    pub fn query(self) -> &'static str {
        match self {
            Sort::Used => "used",
            Sort::New => "new",
        }
    }
}

/// One crosshair from the library, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryCrosshair {
    pub id: String,
    pub name: String,
    /// The look. `disable_hero_specific_crosshairs` is always false here: it is the player's
    /// own preference and the app keeps the player's value when loading.
    pub settings: Crosshair,
    pub uses: u64,
    pub official: bool,
    pub created_day: String,
}

/// One page of the list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub rows: Vec<LibraryCrosshair>,
    pub next_cursor: Option<String>,
    /// Rows the response held that failed a check and were left out.
    pub skipped: usize,
}

/// Why a library call did not work. Each maps to one short sentence in the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryError {
    /// Offline mode is on, or the Worker could not be reached.
    Offline,
    /// The same look is already in the library, under this id.
    Duplicate(String),
    /// This network published 3 today.
    PublishLimit,
    /// The library took its 300 for today.
    DailyLimit,
    /// The name is on the blocklist.
    BlockedName,
    /// The name broke the character or length rules.
    BadName,
    /// A setting was refused; the field the Worker named.
    BadConfig(String),
    /// Too many requests in a minute.
    RateLimited,
    /// The library holds its maximum.
    LibraryFull,
    /// The crosshair is gone (deleted, removed or hidden).
    NotFound,
    /// The delete token did not match.
    WrongToken,
    /// This network reported 5 today.
    ReportLimit,
    /// The Worker is not set up for this route.
    NotConfigured,
    /// Anything else: an unexpected status or a body the app could not read.
    Failed(String),
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl LibraryError {
    /// The plain sentence the app shows.
    pub fn message(&self) -> String {
        match self {
            LibraryError::Offline => "The crosshair library needs a connection.".to_string(),
            LibraryError::Duplicate(_) => "This crosshair is already in the library.".to_string(),
            LibraryError::PublishLimit => {
                "You can publish 3 crosshairs a day. Try again tomorrow.".to_string()
            }
            LibraryError::DailyLimit => {
                "The library took all it can for today. Try again tomorrow.".to_string()
            }
            LibraryError::BlockedName => "That name is not allowed. Pick another one.".to_string(),
            LibraryError::BadName => name_rule().to_string(),
            LibraryError::BadConfig(_) => {
                "The library refused one of this crosshair's settings.".to_string()
            }
            LibraryError::RateLimited => "Too many requests. Wait a minute and try again.".to_string(),
            LibraryError::LibraryFull => "The library is full right now.".to_string(),
            LibraryError::NotFound => "That crosshair is no longer in the library.".to_string(),
            LibraryError::WrongToken => {
                "The library did not accept the removal key for this crosshair.".to_string()
            }
            LibraryError::ReportLimit => "You reported 5 crosshairs today. Try again tomorrow.".to_string(),
            LibraryError::NotConfigured | LibraryError::Failed(_) => {
                "The crosshair library is not answering. Try again later.".to_string()
            }
        }
    }

    /// Map a Worker error answer. `body` is the JSON the Worker sent.
    fn from_answer(status: u16, body: &str) -> LibraryError {
        let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let code = value.get("error").and_then(Value::as_str).unwrap_or_default();
        match code {
            "duplicate" => {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| is_library_id(id))
                    .unwrap_or_default();
                LibraryError::Duplicate(id.to_string())
            }
            "publish_limit" => LibraryError::PublishLimit,
            "daily_limit" => LibraryError::DailyLimit,
            "blocked_name" => LibraryError::BlockedName,
            "bad_name" => LibraryError::BadName,
            "bad_config" => {
                let field = value
                    .get("field")
                    .and_then(Value::as_str)
                    .filter(|field| field.len() <= 40 && field.bytes().all(|b| b.is_ascii_alphanumeric()))
                    .unwrap_or("config");
                LibraryError::BadConfig(field.to_string())
            }
            "rate_limited" => LibraryError::RateLimited,
            "library_full" => LibraryError::LibraryFull,
            "not_found" => LibraryError::NotFound,
            "wrong_token" => LibraryError::WrongToken,
            "report_limit" => LibraryError::ReportLimit,
            "not_configured" => LibraryError::NotConfigured,
            _ => LibraryError::Failed(format!("HTTP {status} {}", truncate(code, 40))),
        }
    }
}

/// The name rule in one sentence, shown under the publish field.
pub fn name_rule() -> &'static str {
    "Use 2 to 32 letters, digits, spaces or . - _ ' ! +"
}

// --- ids and names -------------------------------------------------------------------------

/// A community id (16 lower case hex) or an official one (`sigil-` and 1 to 32 of a-z, 0-9, -).
pub fn is_library_id(id: &str) -> bool {
    if id.len() == 16 {
        return id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    }
    match id.strip_prefix("sigil-") {
        Some(rest) => {
            (1..=32).contains(&rest.len())
                && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        None => false,
    }
}

/// A letter the Worker allows: ASCII, or Latin-1 and Latin Extended-A without `×` and `÷`.
fn is_name_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || (('\u{C0}'..='\u{17F}').contains(&c) && c != '\u{D7}' && c != '\u{F7}')
}

/// The name as the Worker would store it, or why it would refuse it (without the blocklist,
/// which only the Worker knows). Trimmed, whitespace runs collapsed, 2 to 32 characters, only
/// the allowed characters, no dot directly before a letter or digit, at least one letter or
/// digit. The Worker also NFC-normalises; a name typed in the app is already composed, and a
/// decomposed accent is refused here rather than sent.
pub fn clean_name(raw: &str) -> std::result::Result<String, LibraryError> {
    if raw.len() > 256 {
        return Err(LibraryError::BadName);
    }
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let length = name.chars().count();
    if !(NAME_MIN..=NAME_MAX).contains(&length) {
        return Err(LibraryError::BadName);
    }
    let allowed = |c: char| is_name_letter(c) || c.is_ascii_digit() || matches!(c, ' ' | '.' | '-' | '_' | '\'' | '!' | '+');
    if !name.chars().all(allowed) {
        return Err(LibraryError::BadName);
    }
    if !name.chars().any(|c| is_name_letter(c) || c.is_ascii_digit()) {
        return Err(LibraryError::BadName);
    }
    let chars: Vec<char> = name.chars().collect();
    if chars
        .windows(2)
        .any(|pair| pair[0] == '.' && (is_name_letter(pair[1]) || pair[1].is_ascii_digit()))
    {
        return Err(LibraryError::BadName);
    }
    Ok(name)
}

// --- the config object ---------------------------------------------------------------------

/// The 19 keys in the Worker's canonical order with their integer ranges (None for opacity and
/// booleans).
const INT_KEYS: [(&str, i64, i64); 14] = [
    ("pipGap", 0, 64),
    ("pipHeight", 0, 64),
    ("pipWidth", 0, 64),
    ("pipOutlineBorder", 0, 16),
    ("pipOutlineGap", 0, 16),
    ("dotSize", 0, 64),
    ("dotOutlineBorder", 0, 16),
    ("dotOutlineGap", 0, 16),
    ("colorR", 0, 255),
    ("colorG", 0, 255),
    ("colorB", 0, 255),
    ("outlineColorR", 0, 255),
    ("outlineColorG", 0, 255),
    ("outlineColorB", 0, 255),
];
const OPACITY_KEYS: [&str; 4] = ["pipOpacity", "pipOutlineOpacity", "dotOpacity", "dotOutlineOpacity"];
const BOOL_KEYS: [&str; 1] = ["pipGapStatic"];
const OPTIONAL_KEY: &str = "disableHeroSpecificCrosshairs";

fn round_opacity(value: f64) -> f64 {
    (value * 100.0).round() / 100.0 + 0.0
}

/// The config object the Worker takes: the 19 look keys in canonical order, opacities rounded
/// to hundredths. The hero-specific toggle is the player's preference and is left out.
pub fn config_json(c: &Crosshair) -> Value {
    let mut map = Map::new();
    let mut put = |key: &str, value: Value| {
        map.insert(key.to_string(), value);
    };
    put("pipGap", c.pip_gap.into());
    put("pipGapStatic", c.pip_gap_static.into());
    put("pipHeight", c.pip_height.into());
    put("pipWidth", c.pip_width.into());
    put("pipOpacity", round_opacity(c.pip_opacity).into());
    put("pipOutlineBorder", c.pip_outline_border.into());
    put("pipOutlineGap", c.pip_outline_gap.into());
    put("pipOutlineOpacity", round_opacity(c.pip_outline_opacity).into());
    put("dotOpacity", round_opacity(c.dot_opacity).into());
    put("dotSize", c.dot_size.into());
    put("dotOutlineBorder", c.dot_outline_border.into());
    put("dotOutlineGap", c.dot_outline_gap.into());
    put("dotOutlineOpacity", round_opacity(c.dot_outline_opacity).into());
    put("colorR", c.color_r.into());
    put("colorG", c.color_g.into());
    put("colorB", c.color_b.into());
    put("outlineColorR", c.outline_color_r.into());
    put("outlineColorG", c.outline_color_g.into());
    put("outlineColorB", c.outline_color_b.into());
    Value::Object(map)
}

/// True when two crosshairs look the same in game: the 19 look settings match with opacities
/// rounded to hundredths. The hero-specific toggle does not count.
pub fn same_look(a: &Crosshair, b: &Crosshair) -> bool {
    config_json(a) == config_json(b)
}

/// A whole number: a JSON integer, or a float with no fraction (`5.0`). Never a string.
fn whole(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        return Some(n);
    }
    let f = value.as_f64()?;
    (f.is_finite() && f.fract() == 0.0 && f.abs() < 1e9).then_some(f as i64)
}

/// Parse a config object with the Worker's rules: exactly the 19 keys (plus an optional
/// boolean `disableHeroSpecificCrosshairs`, dropped), integers whole and in range, opacities
/// 0 to 1, booleans real booleans. The result also passes [`Crosshair::validate`].
pub fn parse_config(value: &Value) -> std::result::Result<Crosshair, String> {
    let object = value.as_object().ok_or("config")?;
    for (key, value) in object {
        let known = INT_KEYS.iter().any(|(k, _, _)| k == key)
            || OPACITY_KEYS.contains(&key.as_str())
            || BOOL_KEYS.contains(&key.as_str());
        if known {
            continue;
        }
        if key == OPTIONAL_KEY && value.is_boolean() {
            continue;
        }
        return Err(truncate(key, 40));
    }
    let int = |key: &str, low: i64, high: i64| -> std::result::Result<i32, String> {
        let n = object.get(key).and_then(whole).ok_or(key.to_string())?;
        if (low..=high).contains(&n) { Ok(n as i32) } else { Err(key.to_string()) }
    };
    let opacity = |key: &str| -> std::result::Result<f64, String> {
        let n = object
            .get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && (0.0..=1.0).contains(n))
            .ok_or(key.to_string())?;
        Ok(round_opacity(n))
    };
    let flag = |key: &str| object.get(key).and_then(Value::as_bool).ok_or(key.to_string());
    let range = |key: &str| {
        let (_, low, high) = INT_KEYS.iter().find(|(k, _, _)| *k == key).copied().unwrap_or((key, 0, 0));
        int(key, low, high)
    };
    let crosshair = Crosshair {
        pip_gap: range("pipGap")?,
        pip_gap_static: flag("pipGapStatic")?,
        pip_height: range("pipHeight")?,
        pip_width: range("pipWidth")?,
        pip_opacity: opacity("pipOpacity")?,
        pip_outline_border: range("pipOutlineBorder")?,
        pip_outline_gap: range("pipOutlineGap")?,
        pip_outline_opacity: opacity("pipOutlineOpacity")?,
        dot_opacity: opacity("dotOpacity")?,
        dot_size: range("dotSize")?,
        dot_outline_border: range("dotOutlineBorder")?,
        dot_outline_gap: range("dotOutlineGap")?,
        dot_outline_opacity: opacity("dotOutlineOpacity")?,
        color_r: range("colorR")?,
        color_g: range("colorG")?,
        color_b: range("colorB")?,
        outline_color_r: range("outlineColorR")?,
        outline_color_g: range("outlineColorG")?,
        outline_color_b: range("outlineColorB")?,
        disable_hero_specific_crosshairs: false,
    };
    crosshair.validate().map_err(|_| "config".to_string())?;
    Ok(crosshair)
}

// --- the list ------------------------------------------------------------------------------

fn parse_row(value: &Value) -> Option<LibraryCrosshair> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?;
    if !is_library_id(id) {
        return None;
    }
    let name = clean_name(object.get("name")?.as_str()?).ok()?;
    let settings = parse_config(object.get("config")?).ok()?;
    let uses = object.get("uses").and_then(Value::as_u64).unwrap_or(0).min(1_000_000_000);
    let official = object.get("official").and_then(Value::as_bool).unwrap_or(false);
    // An official row must have an official id, and a community row a community one.
    if official != id.starts_with("sigil-") {
        return None;
    }
    let created_day = object
        .get("created_day")
        .and_then(Value::as_str)
        .filter(|day| day.len() == 10 && day.bytes().all(|b| b.is_ascii_digit() || b == b'-'))
        .unwrap_or_default()
        .to_string();
    Some(LibraryCrosshair {
        id: id.to_string(),
        name,
        settings,
        uses,
        official,
        created_day,
    })
}

fn clean_cursor(value: Option<&Value>) -> Option<String> {
    let cursor = value?.as_str()?;
    let ok = !cursor.is_empty()
        && cursor.len() <= MAX_CURSOR
        && cursor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'='));
    ok.then(|| cursor.to_string())
}

/// Parse a list response. A body over [`MAX_LIST_BYTES`], not JSON, or without a
/// `crosshairs` array is an error; a row that fails any check is skipped; at most
/// [`MAX_PAGE_ROWS`] rows are read; a row repeating an earlier id is skipped.
pub fn parse_page(body: &[u8]) -> Result<Page> {
    if body.len() as u64 > MAX_LIST_BYTES {
        return Err(Error::msg("The crosshair list is larger than the cap."));
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| Error::msg("The crosshair list is not valid JSON."))?;
    let rows = value
        .get("crosshairs")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The crosshair list has an unknown shape."))?;
    let mut page = Page {
        next_cursor: clean_cursor(value.get("next_cursor")),
        ..Page::default()
    };
    for row in rows.iter() {
        if page.rows.len() >= MAX_PAGE_ROWS {
            page.skipped += 1;
            continue;
        }
        match parse_row(row) {
            Some(row) if !page.rows.iter().any(|kept| kept.id == row.id) => page.rows.push(row),
            _ => page.skipped += 1,
        }
    }
    Ok(page)
}

// --- the client ----------------------------------------------------------------------------

/// The Worker client: the base address and the app version for `x-sigil-version`.
#[derive(Clone, Debug)]
pub struct Client {
    base: String,
    version: String,
}

/// What a publish returns: the new id and its delete token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Published {
    pub id: String,
    pub delete_token: DeleteToken,
}

/// A delete token: 43 base64url characters. Its value never shows in `Debug` or `Display`;
/// read it only with [`DeleteToken::expose`] to send it back to the Worker.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeleteToken(String);

impl DeleteToken {
    pub fn parse(value: &str) -> Option<DeleteToken> {
        (value.len() == 43
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .then(|| DeleteToken(value.to_string()))
    }

    /// The token itself, only for the delete request body.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeleteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeleteToken(hidden)")
    }
}

impl fmt::Display for DeleteToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("hidden")
    }
}

struct Answer {
    status: u16,
    body: String,
}

impl Client {
    /// The live Worker. `version` is the app version.
    pub fn new(version: &str) -> Client {
        Client::at(ENDPOINT, version)
    }

    /// A client for another base address (tests point it at a local server).
    pub fn at(base: &str, version: &str) -> Client {
        Client {
            base: base.trim_end_matches('/').to_string(),
            version: crate::telemetry::app_version(version),
        }
    }

    fn agent(timeout: Duration) -> ureq::Agent {
        crate::net::with_proxy(crate::net::builder())
            .middleware(crate::net::gate)
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .new_agent()
    }

    fn send(
        &self,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
        limit: u64,
    ) -> std::result::Result<Answer, LibraryError> {
        if crate::net::is_offline() {
            return Err(LibraryError::Offline);
        }
        let url = format!("{}{}", self.base, path);
        let agent = Client::agent(timeout);
        let result = match body {
            Some(body) => {
                let payload = serde_json::to_vec(body).map_err(|e| LibraryError::Failed(e.to_string()))?;
                agent
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .header("Accept", "application/json")
                    .header("User-Agent", "Sigil")
                    .header("x-sigil-client", crate::telemetry::CLIENT_HEADER)
                    .header("x-sigil-version", &self.version)
                    .send(payload.as_slice())
            }
            None => agent
                .get(&url)
                .header("Accept", "application/json")
                .header("User-Agent", "Sigil")
                .header("x-sigil-client", crate::telemetry::CLIENT_HEADER)
                .header("x-sigil-version", &self.version)
                .call(),
        };
        // A transport failure (no route, DNS, TLS, the offline gate) reads as no connection.
        let mut response = result.map_err(|_| LibraryError::Offline)?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        response
            .body_mut()
            .with_config()
            .limit(limit + 1)
            .reader()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| LibraryError::Failed("the answer could not be read".to_string()))?;
        if bytes.len() as u64 > limit {
            return Err(LibraryError::Failed("the answer is larger than the cap".to_string()));
        }
        Ok(Answer {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        })
    }

    /// One page of the list in `sort` order, after `cursor` when given.
    pub fn list(&self, sort: Sort, cursor: Option<&str>) -> std::result::Result<Page, LibraryError> {
        let mut path = format!("{LIST_PATH}?sort={}", sort.query());
        if let Some(cursor) = cursor.and_then(|c| clean_cursor(Some(&Value::String(c.to_string())))) {
            path.push_str("&cursor=");
            path.push_str(&cursor);
        }
        let answer = self.send(&path, None, LIST_TIMEOUT, MAX_LIST_BYTES)?;
        if answer.status != 200 {
            return Err(LibraryError::from_answer(answer.status, &answer.body));
        }
        parse_page(answer.body.as_bytes()).map_err(|e| LibraryError::Failed(e.to_string()))
    }

    /// Publish `settings` as `name`. The name is checked here first, so a name the Worker
    /// would refuse never leaves the PC.
    pub fn publish(&self, name: &str, settings: &Crosshair) -> std::result::Result<Published, LibraryError> {
        let name = clean_name(name)?;
        settings
            .validate()
            .map_err(|_| LibraryError::BadConfig("config".to_string()))?;
        let body = serde_json::json!({ "name": name, "config": config_json(settings) });
        let answer = self.send(LIST_PATH, Some(&body), SMALL_TIMEOUT, MAX_SMALL_BYTES)?;
        if answer.status != 201 && answer.status != 200 {
            return Err(LibraryError::from_answer(answer.status, &answer.body));
        }
        let value: Value = serde_json::from_str(&answer.body)
            .map_err(|_| LibraryError::Failed("the publish answer is not JSON".to_string()))?;
        let id = value.get("id").and_then(Value::as_str).filter(|id| is_library_id(id));
        let token = value.get("delete_token").and_then(Value::as_str).and_then(DeleteToken::parse);
        match (id, token) {
            (Some(id), Some(delete_token)) => Ok(Published {
                id: id.to_string(),
                delete_token,
            }),
            _ => Err(LibraryError::Failed("the publish answer has an unknown shape".to_string())),
        }
    }

    /// Remove a published crosshair with its token. A crosshair already gone counts as removed.
    pub fn delete(&self, published: &Published) -> std::result::Result<(), LibraryError> {
        if !is_library_id(&published.id) {
            return Err(LibraryError::NotFound);
        }
        let body = serde_json::json!({ "delete_token": published.delete_token.expose() });
        let path = format!("{LIST_PATH}/{}/delete", published.id);
        let answer = self.send(&path, Some(&body), SMALL_TIMEOUT, MAX_SMALL_BYTES)?;
        match answer.status {
            200 => Ok(()),
            404 => Ok(()),
            status => Err(LibraryError::from_answer(status, &answer.body)),
        }
    }

    /// Count one apply. `Ok(true)` when the Worker counted it.
    pub fn send_use(&self, id: &str) -> std::result::Result<bool, LibraryError> {
        self.counted(id, "use")
    }

    /// Report a crosshair. `Ok(true)` when the Worker counted it.
    pub fn report(&self, id: &str) -> std::result::Result<bool, LibraryError> {
        self.counted(id, "report")
    }

    fn counted(&self, id: &str, action: &str) -> std::result::Result<bool, LibraryError> {
        if !is_library_id(id) {
            return Err(LibraryError::NotFound);
        }
        let path = format!("{LIST_PATH}/{id}/{action}");
        let answer = self.send(&path, Some(&serde_json::json!({})), SMALL_TIMEOUT, MAX_SMALL_BYTES)?;
        if answer.status != 200 {
            return Err(LibraryError::from_answer(answer.status, &answer.body));
        }
        let value: Value = serde_json::from_str(&answer.body).unwrap_or(Value::Null);
        Ok(value.get("counted").and_then(Value::as_bool).unwrap_or(false))
    }
}

// --- the local store -----------------------------------------------------------------------

/// What the app keeps about the library: the player's published crosshairs by preset id, and
/// the ids the player reported (hidden from the player's own list).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub version: u32,
    /// Preset id to the published crosshair and its delete token.
    #[serde(default)]
    pub published: BTreeMap<String, Published>,
    #[serde(default)]
    pub reported: Vec<String>,
}

impl Store {
    pub fn is_reported(&self, id: &str) -> bool {
        self.reported.iter().any(|kept| kept == id)
    }

    pub fn add_reported(&mut self, id: &str) {
        if !self.is_reported(id) && is_library_id(id) {
            self.reported.push(id.to_string());
        }
        let over = self.reported.len().saturating_sub(MAX_REPORTED);
        self.reported.drain(0..over);
    }
}

/// Read the store. A missing, oversized or corrupt file reads as empty; entries with a bad id
/// or token are dropped.
pub fn load_store(path: &Path) -> Store {
    let Ok(meta) = std::fs::metadata(path) else {
        return Store::default();
    };
    if meta.len() > STORE_FILE_MAX {
        return Store::default();
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Store::default();
    };
    let Ok(mut store) = serde_json::from_str::<Store>(&text) else {
        return Store::default();
    };
    store
        .published
        .retain(|key, published| !key.is_empty() && key.len() <= 200 && is_library_id(&published.id));
    while store.published.len() > MAX_PUBLISHED {
        let Some(first) = store.published.keys().next().cloned() else { break };
        store.published.remove(&first);
    }
    store.reported.retain(|id| is_library_id(id));
    store.reported.dedup();
    store
}

/// Write the store through a temporary file, owner-only on Unix from the first byte.
pub fn save_store(path: &Path, store: &Store) -> Result<()> {
    let mut store = store.clone();
    store.version = 1;
    let mut text = serde_json::to_vec_pretty(&store)?;
    text.push(b'\n');
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(&text)?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&temp, path).map_err(Error::from)
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
#[path = "crosshair_library_tests.rs"]
mod tests;
