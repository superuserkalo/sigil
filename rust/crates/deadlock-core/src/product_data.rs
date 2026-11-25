//! Opt-in product data, "Help build Sigil" (`docs/decisions/0017-opt-in-product-data.md`).
//!
//! Off by default. Nothing here records, queues or sends anything unless [`allowed`] says yes,
//! and [`allowed`] is the only function that reads the consent pair (`product_asked` and
//! `product` in [`Consent`]). Read it first when reviewing this module.
//!
//! Shape of the data:
//!
//! - [`Record`] is the whole allowlist: one enum variant per record type, each with explicit,
//!   typed fields. There is no free-form payload. Serde rejects unknown fields and unknown types.
//! - Strings exist only as three newtypes, each built by its scrubber and re-checked when read
//!   back from disk: [`Code`] (a short lower-case identifier such as `dark` or `install_failed`),
//!   [`SearchText`] (Browse search text, [`scrub_search`]) and [`ModelName`] (a GPU or CPU model,
//!   [`model_name`]). None can hold a path, an email, a link or a Steam id.
//! - Mods appear only as [`ModId`]: a GameBanana id, 1 to 1e9. A local or private mod has no
//!   GameBanana id, so it cannot be recorded at all.
//!
//! Flow, kept cheap for the UI thread:
//!
//! 1. [`Recorder::record`] checks the in-memory switch and pushes onto an in-memory list. No IO.
//! 2. [`Recorder::persist`] (a worker thread) reads the consent file again, then appends the list
//!    to `<data>/telemetry/product/queue.json`, capped by count, bytes and age.
//! 3. [`Recorder::flush`] (a worker thread) sends the queue in batches to Sigil's own Worker,
//!    `POST /v1/product`, backing off after a failure, and appends what went to the local log
//!    `shared.json` ("What Sigil shared", the last [`LOG_LIMIT`] records).
//! 4. Turning the switch off ([`Recorder::apply_consent`]) clears the list, the queue, the log and
//!    the backoff at once. [`Recorder::delete_my_data`] asks the Worker to delete every raw row for
//!    this install, then clears everything and drops the install id so a new one is made.
//!
//! The install id here is separate from the usage-event id (`telemetry::install_id`), so deleting
//! product data and getting a new id changes nothing else. The Worker stores only a salted
//! SHA-256 of it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::telemetry::{self, Consent};
use crate::util::write_atomic;

// --- the consent gate ----------------------------------------------------------------------

/// The one decision: may a product data record be created, queued or sent?
///
/// True only when the player saw the "Help build Sigil" question (`product_asked`) and said Share
/// or turned the switch on (`product`). Every path below goes through this function, either
/// directly or through the in-memory switch that [`Recorder::apply_consent`] sets from it.
pub fn allowed(consent: &Consent) -> bool {
    consent.product_asked && consent.product
}

// --- limits --------------------------------------------------------------------------------

/// Records held in memory between two persists. A runaway caller cannot grow it further.
pub const PENDING_LIMIT: usize = 256;
/// Records kept in the disk queue. Older ones fall off the front.
pub const QUEUE_LIMIT: usize = 1000;
/// Bytes of the disk queue file.
pub const QUEUE_MAX_BYTES: usize = 512 * 1024;
/// A queued record older than this is dropped unsent.
pub const QUEUE_MAX_AGE_MS: i64 = 14 * 24 * 60 * 60 * 1000;
/// Records kept in "What Sigil shared".
pub const LOG_LIMIT: usize = 200;
/// Records per `/v1/product` request. The Worker rejects more.
pub const BATCH_SIZE: usize = 50;
/// The longest backoff after failed sends.
const MAX_BACKOFF_MS: i64 = 6 * 60 * 60 * 1000;
const FIRST_BACKOFF_MS: i64 = 60 * 1000;
const TIMEOUT: Duration = Duration::from_secs(8);

/// Max mods in one enabled set, filters in one query, heroes in one list.
pub const MAX_SET: usize = 64;
pub const MAX_FILTERS: usize = 8;
pub const MAX_HEROES: usize = 10;
/// Search text is cut to this many characters.
pub const MAX_SEARCH_CHARS: usize = 64;
pub const MAX_MODEL_CHARS: usize = 48;
pub const MAX_CODE_CHARS: usize = 32;
const MAX_GAMEBANANA_ID: u64 = 1_000_000_000;

const DIR_NAME: &str = "product";
const QUEUE_FILE: &str = "queue.json";
const LOG_FILE: &str = "shared.json";
const INSTALL_ID_FILE: &str = "install-id";
const MARKS_FILE: &str = "marks.json";

pub const PRODUCT_PATH: &str = "/v1/product";
pub const DELETE_PATH: &str = "/v1/product/delete";
pub const RELATED_PATH: &str = "/v1/product/related";

// --- scrubbed value types ------------------------------------------------------------------

/// A GameBanana mod (or file) id. Zero, a local mod's missing id and anything as long as a
/// Steam id are refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct ModId(u64);

impl ModId {
    pub fn new(id: u64) -> Option<ModId> {
        (1..=MAX_GAMEBANANA_ID).contains(&id).then_some(ModId(id))
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for ModId {
    type Error = String;
    fn try_from(value: u64) -> std::result::Result<Self, String> {
        ModId::new(value).ok_or_else(|| "not a GameBanana id".to_string())
    }
}

impl From<ModId> for u64 {
    fn from(value: ModId) -> u64 {
        value.0
    }
}

/// A short lower-case identifier the app chose: a theme, a sort, a filter, a reason code.
/// `^[a-z][a-z0-9_]{0,31}$` with at most six digits and no four in a row, so it cannot carry a path, a number long
/// enough to be an account, an email, a link or a sentence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Code(String);

impl Code {
    /// Lower-cases and turns spaces and dashes into underscores, then checks the shape.
    pub fn new(value: &str) -> Option<Code> {
        let value: String = value
            .trim()
            .chars()
            .map(|c| match c {
                ' ' | '-' => '_',
                other => other.to_ascii_lowercase(),
            })
            .collect();
        is_code(&value).then_some(Code(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_CODE_CHARS
        && bytes[0].is_ascii_lowercase()
        && bytes.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
        && bytes.iter().filter(|b| b.is_ascii_digit()).count() <= 6
        && !bytes.windows(4).any(|w| w.iter().all(u8::is_ascii_digit))
}

impl TryFrom<String> for Code {
    type Error = String;
    fn try_from(value: String) -> std::result::Result<Self, String> {
        if is_code(&value) { Ok(Code(value)) } else { Err("not a code".to_string()) }
    }
}

impl From<Code> for String {
    fn from(value: Code) -> String {
        value.0
    }
}

/// Browse search text after [`scrub_search`]. The only free text that can leave the machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SearchText(String);

impl SearchText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SearchText {
    type Error = String;
    fn try_from(value: String) -> std::result::Result<Self, String> {
        // Read back from disk: it must already be its own scrub.
        match scrub_search(&value) {
            Some(clean) if clean.0 == value => Ok(clean),
            _ => Err("not scrubbed search text".to_string()),
        }
    }
}

impl From<SearchText> for String {
    fn from(value: SearchText) -> String {
        value.0
    }
}

/// A GPU or CPU model name after [`model_name`]: letters, digits, spaces, `+` and `-`, at most
/// 48 characters, no run of six digits (a serial number).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelName(String);

impl ModelName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ModelName {
    type Error = String;
    fn try_from(value: String) -> std::result::Result<Self, String> {
        match model_name(&value) {
            Some(clean) if clean.0 == value => Ok(clean),
            _ => Err("not a model name".to_string()),
        }
    }
}

impl From<ModelName> for String {
    fn from(value: ModelName) -> String {
        value.0
    }
}

/// An OS build such as `10.0.22631`: digits and dots only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OsBuild(String);

impl OsBuild {
    /// The first `n.n[.n[.n]]` in a line such as `Windows 10.0.22631`.
    pub fn from_text(text: &str) -> Option<OsBuild> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"\b\d{1,6}(?:\.\d{1,6}){1,3}\b").expect("os build"));
        re.find(text).map(|m| OsBuild(m.as_str().to_string()))
    }
}

impl TryFrom<String> for OsBuild {
    type Error = String;
    fn try_from(value: String) -> std::result::Result<Self, String> {
        match OsBuild::from_text(&value) {
            Some(clean) if clean.0 == value => Ok(clean),
            _ => Err("not an os build".to_string()),
        }
    }
}

impl From<OsBuild> for String {
    fn from(value: OsBuild) -> String {
        value.0
    }
}

// --- scrubbers -----------------------------------------------------------------------------

/// Browse search text, as ADR 0017 sets it: trimmed, lower case, inner whitespace collapsed, at
/// most 64 characters, and dropped whole (None) when it looks like an email, a path, a link or a
/// long number. The Worker applies the same rules again (`services/telemetry/src/product.ts`).
pub fn scrub_search(text: &str) -> Option<SearchText> {
    let lowered: String = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || looks_private(&collapsed) {
        return None;
    }
    let cut: String = collapsed.chars().take(MAX_SEARCH_CHARS).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(SearchText(cut))
}

/// Email, path, link or long number. Checked on the whole text before it is cut, so a cut cannot
/// hide the shape.
fn looks_private(text: &str) -> bool {
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            // Email: any `@` at all.
            r"@",
            // Paths: slashes, a drive letter, a home tilde, a file extension of a known kind.
            r"[/\\]",
            r"(?:^|\s)[a-z]:",
            r"(?:^|\s)~",
            // Links: a scheme, `www.`, or `name.tld`.
            r"://",
            r"\bwww\.",
            r"[a-z0-9-]\.[a-z]{2,}\b",
            // Steam ids in any written form.
            r"\bu:1:",
            r"\bsteam_[0-5]:",
            // A long number: five digits in a row.
            r"\d{5,}",
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("search rule"))
        .collect()
    });
    // Also many digits split by spaces or dashes (a phone number, a spaced Steam id).
    let digits = text.chars().filter(char::is_ascii_digit).count();
    digits > 8 || rules.iter().any(|rule| rule.is_match(text))
}

/// Reduce a GPU or CPU description to its model name: trademark marks, clock speeds, core
/// counts and "Processor" go; letters, digits, spaces, `+` and `-` stay; at most 48 characters.
/// None when nothing useful is left or a six-digit run (a serial) remains.
pub fn model_name(text: &str) -> Option<ModelName> {
    // A GL renderer string such as `GeForce GTX 1050 Ti/PCIe/SSE2` keeps its first part. A
    // backslash or a colon means this is not a model name at all (a path, a key, a device id).
    let text = text.split('/').next().unwrap_or_default();
    if text.contains('\\') || text.contains(':') {
        return None;
    }
    static RULES: OnceLock<Vec<Regex>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            r"(?i)\((?:r|tm|c)\)|[®™©]",
            r"(?i)@.*$",
            r"(?i)\b\d+\.?\d*\s*[gm]hz\b",
            r"(?i)\b\d+-core\b",
            r"(?i)\b(?:cpu|processor)\b",
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("model rule"))
        .collect()
    });
    let mut out = text.to_string();
    for rule in rules {
        out = rule.replace_all(&out, " ").into_owned();
    }
    let kept: String = out
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '+' || c == '-' { c } else { ' ' })
        .collect();
    let mut name = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.len() > MAX_MODEL_CHARS {
        name.truncate(MAX_MODEL_CHARS);
        name = name.trim_end().to_string();
    }
    let mut run = 0;
    for c in name.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 6 {
            return None;
        }
    }
    (name.chars().any(|c| c.is_ascii_alphabetic())).then_some(ModelName(name))
}

/// RAM rounded to the nearest 4 GB, at least 4 and at most 1024.
pub fn round_ram_gb(ram_mb: u64) -> u16 {
    let gb = ram_mb as f64 / 1024.0;
    let rounded = ((gb / 4.0).round() * 4.0).clamp(4.0, 1024.0);
    rounded as u16
}

/// Deadlock's rank badge (`tier * 10 + subrank`, 0 to 116) reduced to the tier, 0 to 11.
pub fn rank_tier(badge: u32) -> Option<u8> {
    let tier = badge / 10;
    (tier <= 11).then_some(tier as u8)
}

/// Seconds from an RFC 3339 time (as Sigil's library records it) until now, for [`KeptBucket`].
pub fn seconds_since(rfc3339: &str) -> Option<u64> {
    let then = chrono::DateTime::parse_from_rfc3339(rfc3339.trim()).ok()?;
    u64::try_from((Utc::now() - then.with_timezone(&Utc)).num_seconds()).ok()
}

// --- buckets -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaytimeBucket {
    #[serde(rename = "under_1h")]
    Under1h,
    #[serde(rename = "h1_10")]
    H1To10,
    #[serde(rename = "h10_50")]
    H10To50,
    #[serde(rename = "h50_200")]
    H50To200,
    #[serde(rename = "over_200h")]
    Over200h,
}

impl PlaytimeBucket {
    pub fn from_secs(secs: u64) -> PlaytimeBucket {
        match secs / 3600 {
            0 => PlaytimeBucket::Under1h,
            1..10 => PlaytimeBucket::H1To10,
            10..50 => PlaytimeBucket::H10To50,
            50..200 => PlaytimeBucket::H50To200,
            _ => PlaytimeBucket::Over200h,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionBucket {
    #[serde(rename = "under_5m")]
    Under5m,
    #[serde(rename = "m5_30")]
    M5To30,
    #[serde(rename = "m30_2h")]
    M30To120,
    #[serde(rename = "over_2h")]
    Over2h,
}

impl SessionBucket {
    pub fn from_secs(secs: u64) -> SessionBucket {
        match secs / 60 {
            0..5 => SessionBucket::Under5m,
            5..30 => SessionBucket::M5To30,
            30..120 => SessionBucket::M30To120,
            _ => SessionBucket::Over2h,
        }
    }
}

/// How long a mod stayed installed, sent when it is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeptBucket {
    #[serde(rename = "under_1d")]
    Under1d,
    #[serde(rename = "d1_7")]
    D1To7,
    #[serde(rename = "d7_30")]
    D7To30,
    #[serde(rename = "d30_90")]
    D30To90,
    #[serde(rename = "over_90d")]
    Over90d,
}

impl KeptBucket {
    pub fn from_secs(secs: u64) -> KeptBucket {
        match secs / 86_400 {
            0 => KeptBucket::Under1d,
            1..7 => KeptBucket::D1To7,
            7..30 => KeptBucket::D7To30,
            30..90 => KeptBucket::D30To90,
            _ => KeptBucket::Over90d,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModAction {
    Installed,
    Removed,
    Enabled,
    Disabled,
    Updated,
    Opened,
    Saved,
    Unsaved,
    NotInterested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    /// Both mods ship the same file; the lower slot wins.
    Overlap,
    /// A finding that the two mods break each other.
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecOutcome {
    Shown,
    Opened,
    Installed,
}

/// One enabled mod and its load order slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotMod {
    pub mod_id: ModId,
    pub slot: u16,
}

/// One hero in the most-played list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeroPlay {
    pub hero_id: u16,
    pub playtime: PlaytimeBucket,
}

// --- the allowlist -------------------------------------------------------------------------

/// Every record type Sigil may send, grouped as ADR 0017 groups them. Adding a variant means
/// adding it to [`KINDS`], to the Worker's `PRODUCT_TYPES` and to `docs/privacy.md`; the tests
/// check the first two.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Record {
    // 1. Mods (GameBanana mods only).
    ModAction { mod_id: ModId, action: ModAction },
    ModKept { mod_id: ModId, kept: KeptBucket },
    EnabledSet { mods: Vec<SlotMod> },
    VariantChosen { mod_id: ModId, file_id: ModId },
    /// A shared pack was installed: the GameBanana mods it queued, in pack order (the count is
    /// the list's length). No pack name or note.
    PackInstall { mods: Vec<ModId> },

    // 2. Mod health.
    InstallResult { mod_id: ModId, ok: bool, reason: Option<Code> },
    ModPair { a: ModId, b: ModId, kind: PairKind },
    CompatFinding { mod_id: ModId, finding: Code, game_build: Option<u32> },
    RepairResult { mod_id: ModId, repair: Code, ok: bool, game_build: Option<u32> },
    OffAfterCrash { mod_id: ModId },
    CrashSuspect { mod_id: ModId },

    // 3. Recommendations and Browse.
    RecShown { row: Code, position: u8, mod_id: ModId, outcome: RecOutcome },
    BrowseQuery { sort: Option<Code>, filters: Vec<Code>, search: Option<SearchText>, results: u32 },

    // 4. Play.
    TopHeroes { heroes: Vec<HeroPlay> },
    RankTier { tier: u8 },

    // 5. Performance.
    PerfConfig { config: Code, version: u32 },
    Benchmark { avg_fps: u16, low_fps: u16, scene: Code, width: u16, height: u16, preset: Option<Code> },
    Hardware {
        gpu: Option<ModelName>,
        cpu: Option<ModelName>,
        ram_gb: u16,
        width: u16,
        height: u16,
        refresh_hz: u16,
    },

    // 6. App use.
    FeatureUse { feature: Code, count: u32 },
    Settings { theme: Code, density: Code, language: Code, mature: bool },
    Launch { modded: bool },
    Session { length: SessionBucket },
}

/// The allowlist of record type names, as they go over the wire.
pub const KINDS: &[&str] = &[
    "mod_action",
    "mod_kept",
    "enabled_set",
    "variant_chosen",
    "pack_install",
    "install_result",
    "mod_pair",
    "compat_finding",
    "repair_result",
    "off_after_crash",
    "crash_suspect",
    "rec_shown",
    "browse_query",
    "top_heroes",
    "rank_tier",
    "perf_config",
    "benchmark",
    "hardware",
    "feature_use",
    "settings",
    "launch",
    "session",
];

impl Record {
    /// The wire name. Exhaustive, so a new variant does not compile without one.
    pub fn kind(&self) -> &'static str {
        match self {
            Record::ModAction { .. } => "mod_action",
            Record::ModKept { .. } => "mod_kept",
            Record::EnabledSet { .. } => "enabled_set",
            Record::VariantChosen { .. } => "variant_chosen",
            Record::PackInstall { .. } => "pack_install",
            Record::InstallResult { .. } => "install_result",
            Record::ModPair { .. } => "mod_pair",
            Record::CompatFinding { .. } => "compat_finding",
            Record::RepairResult { .. } => "repair_result",
            Record::OffAfterCrash { .. } => "off_after_crash",
            Record::CrashSuspect { .. } => "crash_suspect",
            Record::RecShown { .. } => "rec_shown",
            Record::BrowseQuery { .. } => "browse_query",
            Record::TopHeroes { .. } => "top_heroes",
            Record::RankTier { .. } => "rank_tier",
            Record::PerfConfig { .. } => "perf_config",
            Record::Benchmark { .. } => "benchmark",
            Record::Hardware { .. } => "hardware",
            Record::FeatureUse { .. } => "feature_use",
            Record::Settings { .. } => "settings",
            Record::Launch { .. } => "launch",
            Record::Session { .. } => "session",
        }
    }

    /// The record with its bounds applied: lists cut to their maximum, numbers in range. None
    /// when a number is out of range in a way cutting cannot fix.
    pub fn bounded(self) -> Option<Record> {
        Some(match self {
            Record::EnabledSet { mut mods } => {
                mods.retain(|m| m.slot <= 999);
                mods.sort_by_key(|m| m.slot);
                let mut seen = std::collections::HashSet::new();
                mods.retain(|m| seen.insert(m.mod_id));
                mods.truncate(MAX_SET);
                if mods.is_empty() {
                    return None;
                }
                Record::EnabledSet { mods }
            }
            Record::PackInstall { mut mods } => {
                let mut seen = std::collections::HashSet::new();
                mods.retain(|m| seen.insert(*m));
                mods.truncate(MAX_SET);
                if mods.is_empty() {
                    return None;
                }
                Record::PackInstall { mods }
            }
            Record::ModPair { a, b, kind } => {
                if a == b {
                    return None;
                }
                // One order per pair, so the Worker counts it once.
                Record::ModPair { a: a.min(b), b: a.max(b), kind }
            }
            Record::RecShown { position, .. } if position > 100 => return None,
            Record::BrowseQuery { sort, mut filters, search, results } => {
                filters.sort();
                filters.dedup();
                filters.truncate(MAX_FILTERS);
                Record::BrowseQuery { sort, filters, search, results: results.min(1_000_000) }
            }
            Record::TopHeroes { mut heroes } => {
                heroes.retain(|h| (1..=1000).contains(&h.hero_id));
                heroes.truncate(MAX_HEROES);
                if heroes.is_empty() {
                    return None;
                }
                Record::TopHeroes { heroes }
            }
            Record::RankTier { tier } if tier > 11 => return None,
            Record::Benchmark { avg_fps, low_fps, scene, width, height, preset } => {
                if avg_fps == 0 || avg_fps > 2000 || low_fps > avg_fps || !valid_resolution(width, height) {
                    return None;
                }
                Record::Benchmark { avg_fps, low_fps, scene, width, height, preset }
            }
            Record::Hardware { gpu, cpu, ram_gb, width, height, refresh_hz } => {
                if !valid_resolution(width, height) || refresh_hz > 1000 {
                    return None;
                }
                Record::Hardware {
                    gpu,
                    cpu,
                    ram_gb: ((ram_gb.clamp(4, 1024) + 2) / 4) * 4,
                    width,
                    height,
                    refresh_hz,
                }
            }
            Record::FeatureUse { feature, count } => Record::FeatureUse { feature, count: count.clamp(1, 10_000) },
            Record::PerfConfig { config, version } => Record::PerfConfig { config, version: version.min(1_000_000) },
            other => other,
        })
    }
}

fn valid_resolution(width: u16, height: u16) -> bool {
    (320..=16384).contains(&width) && (200..=16384).contains(&height)
}

/// A record with the local time it was made. The time stays on this computer: it orders the
/// local log and drops stale queue entries, and is not sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamped {
    pub at: i64,
    pub record: Record,
}

impl Stamped {
    /// Local time for the "What Sigil shared" list, `2026-11-25 14:03`.
    pub fn when(&self) -> String {
        chrono::DateTime::from_timestamp_millis(self.at)
            .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default()
    }
}

/// What goes with every batch, besides the install id: Sigil's version, the OS and its build,
/// and the game build. Filled by the app at send time.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub version: String,
    pub os_build: Option<OsBuild>,
    pub game_build: Option<u32>,
}

// --- the recorder --------------------------------------------------------------------------

/// The app's recorder. One per data directory; the app makes the global one with [`init`].
pub struct Recorder {
    data_dir: PathBuf,
    enabled: AtomicBool,
    pending: Mutex<Vec<Stamped>>,
    /// Serialises disk work: persist, flush, clear and delete.
    disk: Mutex<()>,
    backoff: Mutex<Backoff>,
}

#[derive(Default, Clone, Copy)]
struct Backoff {
    failures: u32,
    next_at: i64,
}

static GLOBAL: OnceLock<Recorder> = OnceLock::new();

/// Make the global recorder for this data directory, reading the consent file once. Only the
/// desktop app calls this; the CLI and the agent host never do, so [`global`] is None there.
pub fn init(data_dir: &Path) -> &'static Recorder {
    GLOBAL.get_or_init(|| Recorder::open(data_dir))
}

/// The global recorder, once [`init`] ran.
pub fn global() -> Option<&'static Recorder> {
    GLOBAL.get()
}

impl Recorder {
    pub fn open(data_dir: &Path) -> Recorder {
        let consent = telemetry::read_consent(data_dir);
        Recorder {
            data_dir: data_dir.to_path_buf(),
            enabled: AtomicBool::new(allowed(&consent)),
            pending: Mutex::new(Vec::new()),
            disk: Mutex::new(()),
            backoff: Mutex::new(Backoff::default()),
        }
    }

    fn dir(&self) -> PathBuf {
        self.data_dir.join("telemetry").join(DIR_NAME)
    }

    pub fn queue_path(&self) -> PathBuf {
        self.dir().join(QUEUE_FILE)
    }

    pub fn log_path(&self) -> PathBuf {
        self.dir().join(LOG_FILE)
    }

    fn install_id_path(&self) -> PathBuf {
        self.dir().join(INSTALL_ID_FILE)
    }

    fn marks_path(&self) -> PathBuf {
        self.dir().join(MARKS_FILE)
    }

    /// The in-memory switch, as [`allowed`] last decided it.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The player changed the answer. Off clears the in-memory list, the queue, the log and the
    /// marks at once. Does not write the consent file; `telemetry::write_consent` does.
    pub fn apply_consent(&self, consent: &Consent) {
        let on = allowed(consent);
        self.enabled.store(on, Ordering::Relaxed);
        if !on {
            self.clear();
        }
    }

    /// Record one event. A push onto an in-memory list, no IO, safe on the UI thread. False when
    /// the switch is off or the record falls outside its bounds.
    pub fn record(&self, record: Record) -> bool {
        if !self.enabled() {
            return false;
        }
        let Some(record) = record.bounded() else {
            return false;
        };
        let mut pending = lock(&self.pending);
        // Feature counts merge: one record per feature between two persists.
        if let Record::FeatureUse { feature, count } = &record
            && let Some(Stamped { record: Record::FeatureUse { count: total, .. }, .. }) = pending
                .iter_mut()
                .find(|s| matches!(&s.record, Record::FeatureUse { feature: f, .. } if f == feature))
        {
            *total = total.saturating_add(*count).min(10_000);
            return true;
        }
        if pending.len() >= PENDING_LIMIT {
            pending.remove(0);
        }
        pending.push(Stamped { at: now_ms(), record });
        true
    }

    /// Records waiting in memory.
    pub fn pending_len(&self) -> usize {
        lock(&self.pending).len()
    }

    /// Move the in-memory list to the disk queue. Reads the consent file again first: without
    /// consent nothing is written and everything already there is removed. Run on a worker.
    pub fn persist(&self) -> Result<usize> {
        let _disk = lock(&self.disk);
        let consent = telemetry::read_consent(&self.data_dir);
        if !allowed(&consent) {
            self.enabled.store(false, Ordering::Relaxed);
            lock(&self.pending).clear();
            self.remove_files();
            return Ok(0);
        }
        let fresh: Vec<Stamped> = std::mem::take(&mut *lock(&self.pending));
        let count = fresh.len();
        let mut queue = self.read_queue();
        if count == 0 && queue.is_empty() {
            return Ok(0);
        }
        queue.extend(fresh);
        self.write_queue(&mut queue)?;
        Ok(count)
    }

    /// The disk queue, with every limit applied and every entry checked again.
    pub fn read_queue(&self) -> Vec<Stamped> {
        let mut queue = read_list(&self.queue_path());
        cap_queue(&mut queue, now_ms());
        queue
    }

    fn write_queue(&self, queue: &mut Vec<Stamped>) -> Result<()> {
        cap_queue(queue, now_ms());
        loop {
            let bytes = serde_json::to_vec(queue)?;
            if bytes.len() <= QUEUE_MAX_BYTES || queue.is_empty() {
                return write_atomic(&self.queue_path(), &bytes);
            }
            let drop = (queue.len() / 10).max(1);
            queue.drain(0..drop);
        }
    }

    /// "What Sigil shared": the last records the Worker accepted, newest last.
    pub fn shared_log(&self) -> Vec<Stamped> {
        let mut log = read_list(&self.log_path());
        let extra = log.len().saturating_sub(LOG_LIMIT);
        log.drain(0..extra);
        log
    }

    /// Persist, then send the queue in batches of [`BATCH_SIZE`]. Returns how many records went.
    /// After a failure the next try waits 1, 2, 4 ... minutes, at most 6 hours.
    pub fn flush(&self, context: &Context) -> Result<usize> {
        self.flush_at(telemetry::ENDPOINT, context)
    }

    fn flush_at(&self, base: &str, context: &Context) -> Result<usize> {
        self.persist()?;
        if !self.enabled() {
            return Ok(0);
        }
        let now = now_ms();
        if lock(&self.backoff).next_at > now {
            return Ok(0);
        }
        let _disk = lock(&self.disk);
        // The switch may have gone off while persist ran.
        if !allowed(&telemetry::read_consent(&self.data_dir)) {
            return Ok(0);
        }
        let mut queue = self.read_queue();
        let version = telemetry::app_version(&context.version);
        let install_id = self.install_id();
        let mut sent = 0;
        while !queue.is_empty() {
            let take = queue.len().min(BATCH_SIZE);
            let batch: Vec<Stamped> = queue[..take].to_vec();
            let body = envelope(&install_id, &version, context, &batch);
            match telemetry::post(&telemetry::endpoint_at(base, PRODUCT_PATH), &version, &body, TIMEOUT) {
                Ok(_) => {
                    *lock(&self.backoff) = Backoff::default();
                    queue.drain(0..take);
                    sent += take;
                    self.write_queue(&mut queue)?;
                    self.append_log(batch)?;
                }
                Err(error) => {
                    let mut backoff = lock(&self.backoff);
                    backoff.failures = backoff.failures.saturating_add(1);
                    let wait = FIRST_BACKOFF_MS
                        .saturating_mul(1_i64 << backoff.failures.saturating_sub(1).min(16))
                        .min(MAX_BACKOFF_MS);
                    backoff.next_at = now_ms() + wait;
                    telemetry::note(&format!("product data was not sent: {error}"));
                    break;
                }
            }
        }
        Ok(sent)
    }

    fn append_log(&self, batch: Vec<Stamped>) -> Result<()> {
        let mut log = read_list(&self.log_path());
        let now = now_ms();
        log.extend(batch.into_iter().map(|s| Stamped { at: now, record: s.record }));
        let extra = log.len().saturating_sub(LOG_LIMIT);
        log.drain(0..extra);
        write_atomic(&self.log_path(), &serde_json::to_vec(&log)?)
    }

    /// The random id for product data, separate from the usage-event id. Made on first use.
    pub fn install_id(&self) -> String {
        let path = self.install_id_path();
        if let Ok(text) = fs::read_to_string(&path) {
            let id = text.trim().to_ascii_lowercase();
            if telemetry::is_uuid(&id) {
                return id;
            }
        }
        let id = telemetry::uuid_v4();
        let _ = write_atomic(&path, id.as_bytes());
        id
    }

    /// True once per Sigil version for `key` (hardware class, settings): the caller records only
    /// when it answers true. Marks are cleared with the queue.
    pub fn once_per_version(&self, key: &str, version: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        let _disk = lock(&self.disk);
        let path = self.marks_path();
        let mut marks: serde_json::Map<String, Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if marks.get(key).and_then(Value::as_str) == Some(version) {
            return false;
        }
        marks.insert(key.to_string(), Value::String(version.to_string()));
        let _ = serde_json::to_vec(&marks).map(|bytes| write_atomic(&path, &bytes));
        true
    }

    /// Drop the in-memory list, the queue, the log and the marks. The install id stays.
    pub fn clear(&self) {
        lock(&self.pending).clear();
        let _disk = lock(&self.disk);
        self.remove_files();
        *lock(&self.backoff) = Backoff::default();
    }

    fn remove_files(&self) {
        for path in [self.queue_path(), self.log_path(), self.marks_path()] {
            let _ = fs::remove_file(path);
        }
    }

    /// Settings > Privacy > Delete my data. Asks the Worker to delete every raw row for this
    /// install id, then clears the queue and the log and drops the id, so the next record gets a
    /// new one. The player's press is the consent; this works with the switch off too. On a
    /// failed request nothing local changes, so the press can be repeated with the same id.
    pub fn delete_my_data(&self, version: &str) -> Result<u64> {
        self.delete_at(telemetry::ENDPOINT, version)
    }

    fn delete_at(&self, base: &str, version: &str) -> Result<u64> {
        let version = telemetry::app_version(version);
        let body = json!({ "installId": self.install_id() });
        let text = telemetry::post(&telemetry::endpoint_at(base, DELETE_PATH), &version, &body, TIMEOUT)?;
        let deleted = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("deleted").and_then(Value::as_u64))
            .ok_or_else(|| Error::Message("The Sigil server did not confirm the delete.".to_string()))?;
        self.clear();
        let _disk = lock(&self.disk);
        let _ = fs::remove_file(self.install_id_path());
        Ok(deleted)
    }
}

/// Mods other installs use together with `mod_id`, from `GET /v1/product/related`. The Worker
/// answers only for pairs at least 10 installs share. Sends only the mod id, no install id.
pub fn related(mod_id: ModId, version: &str) -> Result<Vec<ModId>> {
    related_at(telemetry::ENDPOINT, mod_id, version)
}

fn related_at(base: &str, mod_id: ModId, version: &str) -> Result<Vec<ModId>> {
    let version = telemetry::app_version(version);
    let url = format!("{}?mod={}", telemetry::endpoint_at(base, RELATED_PATH), mod_id.get());
    let text = telemetry::get(&url, &version, TIMEOUT)?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| Error::Message(format!("The Sigil server answered something unreadable: {error}")))?;
    Ok(value
        .get("mods")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| item.get("modId").and_then(Value::as_u64).and_then(ModId::new))
                .collect()
        })
        .unwrap_or_default())
}

// --- what sharing gives back ---------------------------------------------------------------
//
// Every reader below sends only what its query names (mod ids, or a GPU and CPU model name and a
// screen height), never the install id, and the Worker answers only with counts that met the
// floor. The app calls them only while `allowed` says yes; that gate lives with the caller, which
// also caches the answers.

pub const HEALTH_PATH: &str = "/v1/product/health";
pub const FPS_PATH: &str = "/v1/product/fps";
pub const RISING_PATH: &str = "/v1/product/rising";
/// Mods one health request may ask about (the Worker's own limit).
pub const MAX_HEALTH_MODS: usize = 40;

/// Install failures of one mod: failed installs whose reason points at the mod, and the attempts
/// from the same epochs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallHealth {
    pub failed: u32,
    pub attempts: u32,
    /// Unix seconds when the newest period behind the numbers ended (0 when unknown).
    #[serde(default)]
    pub until: i64,
}

/// Reports that a mod did not work on one game build, and how many installs used it there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildHealth {
    pub build: u32,
    pub reports: u32,
    pub users: u32,
    #[serde(default)]
    pub until: i64,
}

/// Reports that a mod overlaps or conflicts with another, and how many installs had both on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictHealth {
    pub mod_id: u64,
    pub reports: u32,
    pub together: u32,
    #[serde(default)]
    pub until: i64,
}

/// What players who share reported about one mod, from `GET /v1/product/health`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModHealth {
    pub mod_id: u64,
    #[serde(default)]
    pub install: Option<InstallHealth>,
    #[serde(default)]
    pub broken: Vec<BuildHealth>,
    #[serde(default)]
    pub conflicts: Vec<ConflictHealth>,
}

/// The fewest reports a health chip rests on: the Worker's floor (10 installs), checked again
/// here so a chip can never stand on less.
pub const HEALTH_MIN_REPORTS: u32 = 10;
/// "Often fails to install": at least this share of attempts failed, in percent.
pub const FAIL_RATE_PCT: u32 = 25;
/// "Reported broken on the current game build": at least this share of the installs that used
/// the mod on that build reported it, in percent. Without a known base the reports must reach
/// twice the floor instead.
pub const BROKEN_RATE_PCT: u32 = 20;
/// "Often conflicts with": at least this share of installs that had both on reported it.
pub const CONFLICT_RATE_PCT: u32 = 30;

/// One chip a mod card may carry. Wording is about reports, never about safety.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthChip {
    /// `failed` of `attempts` reported installs failed.
    FailsToInstall { failed: u32, attempts: u32 },
    /// `reports` installs reported it broken on the current build, of `users` that used it there.
    BrokenOnBuild { reports: u32, users: u32 },
    /// `reports` installs that had both on reported an overlap or a conflict with `other`.
    ConflictsWith { other: u64, reports: u32 },
}

fn rate_at_least(part: u32, whole: u32, pct: u32) -> bool {
    whole > 0 && u64::from(part) * 100 >= u64::from(whole) * u64::from(pct)
}

impl ModHealth {
    /// The chips this mod earns, strongest first. `build` is the game build the player has now:
    /// a broken report counts only for that exact build, so a game update clears it. `have` is
    /// the player's installed GameBanana mods: a conflict counts only with one of those.
    /// `updated` is when the creator last updated the mod (Unix seconds, 0 when unknown):
    /// reports from periods that all ended before it are about an older version and are dropped.
    pub fn chips(&self, build: Option<u32>, have: &std::collections::HashSet<u64>, updated: i64) -> Vec<HealthChip> {
        let current = |until: i64| updated <= 0 || until <= 0 || until > updated;
        let mut out = Vec::new();
        if let Some(b) = build.filter(|b| *b > 0)
            && let Some(found) = self.broken.iter().find(|f| f.build == b && current(f.until))
        {
            let strong = if found.users >= found.reports {
                rate_at_least(found.reports, found.users, BROKEN_RATE_PCT)
            } else {
                found.reports >= 2 * HEALTH_MIN_REPORTS
            };
            if found.reports >= HEALTH_MIN_REPORTS && strong {
                out.push(HealthChip::BrokenOnBuild { reports: found.reports, users: found.users });
            }
        }
        if let Some(install) = self.install.as_ref().filter(|i| current(i.until)) {
            let attempts = install.attempts.max(install.failed);
            if install.failed >= HEALTH_MIN_REPORTS && rate_at_least(install.failed, attempts, FAIL_RATE_PCT) {
                out.push(HealthChip::FailsToInstall { failed: install.failed, attempts });
            }
        }
        if let Some(c) = self
            .conflicts
            .iter()
            .filter(|c| c.mod_id != self.mod_id && have.contains(&c.mod_id) && current(c.until))
            .filter(|c| c.reports >= HEALTH_MIN_REPORTS && rate_at_least(c.reports, c.together.max(c.reports), CONFLICT_RATE_PCT))
            .max_by_key(|c| (c.reports, std::cmp::Reverse(c.mod_id)))
        {
            out.push(HealthChip::ConflictsWith { other: c.mod_id, reports: c.reports });
        }
        out
    }
}

/// Mod health for up to [`MAX_HEALTH_MODS`] mods from `GET /v1/product/health`. Sends only the
/// mod ids. Mods with nothing published are absent from the answer.
pub fn health(mods: &[ModId], version: &str) -> Result<Vec<ModHealth>> {
    health_at(telemetry::ENDPOINT, mods, version)
}

fn health_at(base: &str, mods: &[ModId], version: &str) -> Result<Vec<ModHealth>> {
    if mods.is_empty() {
        return Ok(Vec::new());
    }
    let version = telemetry::app_version(version);
    let ids: Vec<String> = mods.iter().take(MAX_HEALTH_MODS).map(|m| m.get().to_string()).collect();
    let url = format!("{}?mods={}", telemetry::endpoint_at(base, HEALTH_PATH), ids.join(","));
    let value = read_json(&telemetry::get(&url, &version, TIMEOUT)?)?;
    Ok(value
        .get("mods")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| serde_json::from_value::<ModHealth>(item.clone()).ok())
                .filter(|h| ModId::new(h.mod_id).is_some())
                .collect()
        })
        .unwrap_or_default())
}

/// Frame rates other players measured with one performance config on PCs like this one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FpsExpectation {
    /// The config id as a [`Code`] spells it (`Code::new(config id)`).
    pub config: String,
    pub version: u32,
    /// Median average frame rate.
    pub fps: u32,
    /// Median 1% low.
    pub low: u32,
    /// Installs it rests on (at least the floor).
    pub installs: u32,
}

/// The version number Sigil records for a config version text: its first run of digits.
pub fn config_version(text: &str) -> u32 {
    text.split(|c: char| !c.is_ascii_digit()).find(|p| !p.is_empty()).and_then(|p| p.parse().ok()).unwrap_or(0)
}

/// Frame rates per config on this hardware class, from `GET /v1/product/fps`. Sends the reduced
/// GPU and CPU model names and the screen height, nothing else.
pub fn fps(gpu: &ModelName, cpu: Option<&ModelName>, height: u16, version: &str) -> Result<Vec<FpsExpectation>> {
    fps_at(telemetry::ENDPOINT, gpu, cpu, height, version)
}

fn fps_at(base: &str, gpu: &ModelName, cpu: Option<&ModelName>, height: u16, version: &str) -> Result<Vec<FpsExpectation>> {
    let version = telemetry::app_version(version);
    let mut url = format!("{}?gpu={}&height={height}", telemetry::endpoint_at(base, FPS_PATH), query_encode(gpu.as_str()));
    if let Some(cpu) = cpu {
        url.push_str(&format!("&cpu={}", query_encode(cpu.as_str())));
    }
    let value = read_json(&telemetry::get(&url, &version, TIMEOUT)?)?;
    Ok(value
        .get("configs")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| serde_json::from_value::<FpsExpectation>(item.clone()).ok())
                .filter(|e| e.fps > 0 && e.installs >= HEALTH_MIN_REPORTS && Code::new(&e.config).is_some())
                .collect()
        })
        .unwrap_or_default())
}

/// Mods whose installs grew most in the newest closed period among players who share, best
/// first, from `GET /v1/product/rising`. Sends nothing but the request.
pub fn rising(version: &str) -> Result<Vec<ModId>> {
    rising_at(telemetry::ENDPOINT, version)
}

fn rising_at(base: &str, version: &str) -> Result<Vec<ModId>> {
    let version = telemetry::app_version(version);
    let value = read_json(&telemetry::get(&telemetry::endpoint_at(base, RISING_PATH), &version, TIMEOUT)?)?;
    Ok(value
        .get("mods")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|item| item.get("modId").and_then(Value::as_u64).and_then(ModId::new)).take(50).collect())
        .unwrap_or_default())
}

fn read_json(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|error| Error::Message(format!("The Sigil server answered something unreadable: {error}")))
}

/// Percent-encodes everything but unreserved characters. Model names are already reduced to
/// letters, digits, spaces, `+` and `-`.
fn query_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The `/v1/product` body. Records go without their local time.
pub fn envelope(install_id: &str, version: &str, context: &Context, batch: &[Stamped]) -> Value {
    let records: Vec<&Record> = batch.iter().map(|s| &s.record).collect();
    let mut body = json!({
        "installId": install_id,
        "version": version,
        "os": os_name(),
        "records": records,
    });
    if let Some(build) = &context.os_build {
        body["osBuild"] = json!(build);
    }
    if let Some(build) = context.game_build.filter(|b| *b > 0 && *b < 100_000_000) {
        body["gameBuild"] = json!(build);
    }
    body
}

/// The OS names the Worker accepts.
fn os_name() -> &'static str {
    match telemetry::os_name() {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        _ => "other",
    }
}

fn read_list(path: &Path) -> Vec<Stamped> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    // Entry by entry, so one edited or unknown entry is dropped and the rest survive.
    let Ok(values) = serde_json::from_str::<Vec<Value>>(&text) else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Stamped>(value).ok())
        .filter_map(|s| s.record.bounded().map(|record| Stamped { at: s.at, record }))
        .collect()
}

fn cap_queue(queue: &mut Vec<Stamped>, now: i64) {
    queue.retain(|s| now - s.at <= QUEUE_MAX_AGE_MS && s.at <= now + 60_000);
    let extra = queue.len().saturating_sub(QUEUE_LIMIT);
    queue.drain(0..extra);
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "product_data_tests.rs"]
mod tests;
