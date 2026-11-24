//! Packs: a named set of GameBanana mods a player shares so friends run the same mods.
//!
//! A pack travels in the share format Profiles already use: a `mod-profile` document (schema
//! 1.x, the format Grimoire reads and writes) as an `mp1:` code (base64url of gzip JSON), as a
//! `sigil://pack/<code>` link, or as a small `.sigilpack.json` file. A pack writes only what it
//! needs: the format and version, the pack name, an optional note, and per mod the GameBanana
//! mod id, the file id (the variant picked), its place in the load order (`priority`) and
//! whether it is on. Never a path, a URL, a crosshair or autoexec lines. Because it is the same
//! format, Sigil's and Grimoire's profile import read a pack too.
//!
//! A pack is untrusted input. [`read_pack_text`] and [`parse_pack_json`] are strict: the code
//! length, the inflated size, the mod count, the name and note lengths are capped; control and
//! invisible characters are refused; ids must be integers from 1 to `u32::MAX`; wrong types are
//! refused; unknown fields are ignored. Reading a pack never touches the network or the game
//! install. The app looks each mod up on GameBanana (the only host involved) to preview it and
//! installs through the normal download queue and safety gate after the player presses Install.
//!
//! Saved packs (made here or received) live in `<workspace>/.dah/packs/<slug>.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{Error, Result};
use crate::profiles::{
    DEADLOCK_GAMEBANANA_GAME_ID, DEADLOCK_STEAM_APP_ID, MAX_INFLATED_SHARE_CODE_BYTES,
    PORTABLE_FORMAT, SHARE_CODE_PREFIX, decode_share_code, encode_share_code, slug_for,
};
use crate::util::{now_iso, write_atomic};

/// The pack version this build writes, under `extensions.sigil.pack`.
pub const PACK_VERSION: u64 = 1;
/// The `mod-profile` schema a pack is written as.
pub const PACK_SCHEMA_VERSION: &str = "1.1";
/// What a pack link starts with.
pub const LINK_PREFIX: &str = "sigil://pack/";
/// A pack file's extension.
pub const FILE_EXTENSION: &str = ".sigilpack.json";
/// The longest code (or link, or pasted JSON) read, in bytes. 300 mods fit in well under this.
pub const MAX_CODE_BYTES: usize = 64 * 1024;
/// The largest pack file read.
pub const MAX_FILE_BYTES: u64 = MAX_INFLATED_SHARE_CODE_BYTES as u64;
/// Mods in one pack, at most.
pub const MAX_MODS: usize = 300;
/// A pack name's length, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// A note's length, in characters.
pub const MAX_NOTE_CHARS: usize = 300;
/// GameBanana ids fit in 32 bits.
pub const MAX_ID: u64 = u32::MAX as u64;
/// Saved packs kept, at most.
pub const MAX_SAVED: usize = 200;
const PRIORITY_MAX: f64 = 1.0e9;
const EXTENSION_KEY: &str = "sigil";

/// One mod in a pack. Its place in [`Pack::mods`] is its load order: first wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackMod {
    /// GameBanana mod id.
    pub mod_id: u64,
    /// The file (variant) picked, or none for the mod's newest file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u64>,
    /// On after install (when the player asks for that).
    pub enabled: bool,
}

/// A pack: name, optional note, mods in load order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub mods: Vec<PackMod>,
}

/// A pack read from a code, with what was left out on the way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPack {
    pub pack: Pack,
    /// Entries from a source other than GameBanana: nothing to install.
    pub unsupported: usize,
    /// A mod listed twice: the first place counts.
    pub duplicates: usize,
}

// ---------------------------------------------------------------------------
// Text checks
// ---------------------------------------------------------------------------

/// Zero-width and text-direction marks (the same set `mod_sources` refuses), plus the soft
/// hyphen, the word joiner block and tag characters, which can make a line read differently.
pub fn is_invisible(c: char) -> bool {
    crate::mod_sources::is_invisible(c)
        || matches!(c, '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}' | '\u{E0000}'..='\u{E007F}')
}

/// A pack name or note: trimmed, inner runs of spaces kept as typed, no control or invisible
/// character (a line feed only where `lines` allows it), at most `max` characters.
pub fn check_text(value: &str, max: usize, lines: bool, what: &str) -> Result<String> {
    let value = value.trim();
    if value.chars().any(|c| is_invisible(c) || (c.is_control() && !(lines && c == '\n'))) {
        return Err(Error::msg(format!("The {what} has hidden or control characters.")));
    }
    if value.chars().count() > max {
        return Err(Error::msg(format!("The {what} is longer than {max} characters.")));
    }
    Ok(value.to_string())
}

/// A pack name, which must not be empty.
pub fn check_name(name: &str) -> Result<String> {
    let name = check_text(name, MAX_NAME_CHARS, false, "pack name")?;
    if name.is_empty() {
        return Err(Error::msg("A pack needs a name."));
    }
    Ok(name)
}

/// An optional note: empty reads as none.
pub fn check_note(note: &str) -> Result<Option<String>> {
    let note = check_text(&note.replace("\r\n", "\n"), MAX_NOTE_CHARS, true, "note")?;
    Ok((!note.is_empty()).then_some(note))
}

/// A GameBanana id: a JSON integer from 1 to [`MAX_ID`]. Strings, floats, negatives refused.
fn id_of(value: &Value) -> Option<u64> {
    value.as_u64().filter(|id| (1..=MAX_ID).contains(id))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// The pack as a `mod-profile` document holding only the pack's fields.
pub fn to_document(pack: &Pack) -> Value {
    let mods: Vec<Value> = pack
        .mods
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut reference = Map::new();
            reference.insert("submissionId".into(), json!(entry.mod_id));
            // 0 is "no file picked: the newest", so profile readers that require a file id
            // (Sigil's and Grimoire's) still read the entry and fall back to the newest file.
            reference.insert("fileId".into(), json!(entry.file_id.unwrap_or(0)));
            json!({
                "source": "gamebanana",
                "ref": Value::Object(reference),
                "enabled": entry.enabled,
                "priority": index + 1,
            })
        })
        .collect();
    let mut profile = Map::new();
    profile.insert("name".into(), json!(pack.name));
    if let Some(note) = &pack.note {
        profile.insert("description".into(), json!(note));
    }
    json!({
        "format": PORTABLE_FORMAT,
        "schemaVersion": PACK_SCHEMA_VERSION,
        "game": {
            "steamAppId": DEADLOCK_STEAM_APP_ID,
            "gameBananaGameId": DEADLOCK_GAMEBANANA_GAME_ID,
        },
        "profile": Value::Object(profile),
        "mods": mods,
        "extensions": { EXTENSION_KEY: { "pack": PACK_VERSION } },
    })
}

/// The pack's JSON, as written to a pack file.
pub fn to_json(pack: &Pack) -> Result<String> {
    Ok(serde_json::to_string_pretty(&to_document(pack))?)
}

/// The share code: `mp1:` plus base64url of the gzip of the compact JSON.
pub fn to_code(pack: &Pack) -> Result<String> {
    let code = encode_share_code(&serde_json::to_string(&to_document(pack))?)?;
    if code.len() > MAX_CODE_BYTES {
        return Err(Error::msg("This pack is too large for a code. Share it as a file."));
    }
    Ok(code)
}

/// The `sigil://pack/<code>` link.
pub fn to_link(pack: &Pack) -> Result<String> {
    Ok(format!("{LINK_PREFIX}{}", to_code(pack)?))
}

/// One installed mod offered to a new pack, in load order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub mod_id: Option<u64>,
    pub file_id: Option<u64>,
    pub enabled: bool,
}

/// A pack from installed mods, in the order given. Mods without a GameBanana id (local files)
/// cannot be shared: their labels come back as the second value. A mod listed twice keeps its
/// first place. Over [`MAX_MODS`] is refused.
pub fn from_candidates(name: &str, note: &str, candidates: &[Candidate]) -> Result<(Pack, Vec<String>)> {
    let name = check_name(name)?;
    let note = check_note(note)?;
    let mut mods: Vec<PackMod> = Vec::new();
    let mut local = Vec::new();
    for candidate in candidates {
        match candidate.mod_id.filter(|id| (1..=MAX_ID).contains(id)) {
            Some(mod_id) => {
                if !mods.iter().any(|entry| entry.mod_id == mod_id) {
                    mods.push(PackMod {
                        mod_id,
                        file_id: candidate.file_id.filter(|id| (1..=MAX_ID).contains(id)),
                        enabled: candidate.enabled,
                    });
                }
            }
            None => local.push(candidate.label.clone()),
        }
    }
    if mods.is_empty() {
        return Err(Error::msg("None of these mods came from GameBanana, so there is nothing to share."));
    }
    if mods.len() > MAX_MODS {
        return Err(Error::msg(format!("A pack holds at most {MAX_MODS} mods.")));
    }
    Ok((Pack { name, note, mods }, local))
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Read a pack from pasted text: a `sigil://pack/` link, an `mp1:` code or the JSON itself.
/// Never a file path: a file goes through [`read_pack_file`].
pub fn read_pack_text(input: &str) -> Result<ReadPack> {
    let text = input.trim().trim_matches('"').trim();
    if text.is_empty() {
        return Err(Error::msg("Paste a pack code or link first."));
    }
    if text.len() > MAX_CODE_BYTES {
        return Err(Error::msg("This code is too long to be a pack."));
    }
    if text.chars().any(|c| is_invisible(c) || (c.is_control() && !c.is_whitespace())) {
        return Err(Error::msg("This code has hidden characters. Copy it again from the sender."));
    }
    let body = strip_link(text).unwrap_or(text);
    let json = if body.starts_with(SHARE_CODE_PREFIX) {
        decode_share_code(body).map_err(|_| Error::msg("This pack code is damaged. Copy it again."))?
    } else if body.starts_with('{') {
        body.to_string()
    } else {
        return Err(Error::msg("That is not a pack code. A code starts with mp1:."));
    };
    parse_pack_json(&json)
}

/// The code inside a `sigil://pack/` link (any case of the prefix), or none.
pub fn strip_link(text: &str) -> Option<&str> {
    let head = text.get(..LINK_PREFIX.len())?;
    head.eq_ignore_ascii_case(LINK_PREFIX).then(|| &text[LINK_PREFIX.len()..])
}

/// True when pasted text is a pack rather than a profile: a pack link, or a code or JSON that
/// carries Sigil's pack mark (`extensions.sigil.pack`). A profile code without the mark still
/// goes to Profiles > Import. Reads at most [`MAX_CODE_BYTES`].
pub fn is_pack(text: &str) -> bool {
    let text = text.trim();
    if strip_link(text).is_some() {
        return true;
    }
    if text.len() > MAX_CODE_BYTES {
        return false;
    }
    let json = if text.starts_with(SHARE_CODE_PREFIX) {
        match decode_share_code(text) {
            Ok(json) => json,
            Err(_) => return false,
        }
    } else if text.starts_with('{') {
        text.to_string()
    } else {
        return false;
    };
    serde_json::from_str::<Value>(&json)
        .ok()
        .and_then(|value| value.get("extensions")?.get(EXTENSION_KEY)?.get("pack").cloned())
        .is_some()
}

/// Read a pack file, refusing anything over [`MAX_FILE_BYTES`].
pub fn read_pack_file(path: &Path) -> Result<ReadPack> {
    let size = fs::metadata(path)?.len();
    if size > MAX_FILE_BYTES {
        return Err(Error::msg("This file is too large to be a pack."));
    }
    let text = fs::read_to_string(path).map_err(|_| Error::msg("This file is not a pack."))?;
    let text = text.trim_start_matches('\u{FEFF}').trim();
    if text.starts_with('{') {
        if text.len() > MAX_FILE_BYTES as usize {
            return Err(Error::msg("This file is too large to be a pack."));
        }
        if text.chars().any(|c| is_invisible(c) || (c.is_control() && !c.is_whitespace())) {
            return Err(Error::msg("This file has hidden characters."));
        }
        return parse_pack_json(text);
    }
    read_pack_text(text)
}

/// Parse and check a pack document. See the module notes for the rules.
pub fn parse_pack_json(json: &str) -> Result<ReadPack> {
    if json.len() > MAX_INFLATED_SHARE_CODE_BYTES {
        return Err(Error::msg("This pack is too large."));
    }
    let value: Value = serde_json::from_str(json).map_err(|_| Error::msg("This pack is damaged. Copy it again."))?;
    let object = value.as_object().ok_or_else(|| Error::msg("This is not a pack."))?;
    if object.get("format").and_then(Value::as_str) != Some(PORTABLE_FORMAT) {
        return Err(Error::msg("This is not a pack."));
    }
    let version = object
        .get("schemaVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::msg("The pack has no version."))?;
    if version.len() > 16 || version.split('.').next() != Some("1") {
        return Err(Error::msg("This pack was made by a newer version. Update Sigil to read it."));
    }
    if let Some(pack_version) = object
        .get("extensions")
        .and_then(|extensions| extensions.get(EXTENSION_KEY))
        .and_then(|own| own.get("pack"))
        && pack_version.as_u64().is_none_or(|version| version > PACK_VERSION)
    {
        return Err(Error::msg("This pack was made by a newer version. Update Sigil to read it."));
    }
    if let Some(game) = object.get("game") {
        let game = game.as_object().ok_or_else(|| Error::msg("The pack's game is malformed."))?;
        let wrong = |key: &str, want: u64| game.get(key).is_some_and(|id| id.as_u64() != Some(want));
        if wrong("steamAppId", DEADLOCK_STEAM_APP_ID) || wrong("gameBananaGameId", DEADLOCK_GAMEBANANA_GAME_ID) {
            return Err(Error::msg("This pack is for a different game."));
        }
    }
    let meta = object
        .get("profile")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::msg("The pack has no name."))?;
    let name = check_name(
        meta.get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::msg("The pack has no name."))?,
    )?;
    let note = match meta.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => check_note(text)?,
        Some(_) => return Err(Error::msg("The pack's note is malformed.")),
    };
    let entries = object
        .get("mods")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::msg("The pack has no mod list."))?;
    if entries.len() > MAX_MODS {
        return Err(Error::msg(format!("This pack has more than {MAX_MODS} mods.")));
    }

    let mut ordered: Vec<(f64, usize, PackMod)> = Vec::new();
    let mut unsupported = 0;
    for (index, entry) in entries.iter().enumerate() {
        let position = index + 1;
        let bad = || Error::msg(format!("Mod {position} in the pack is malformed."));
        let entry = entry.as_object().ok_or_else(bad)?;
        let source = entry.get("source").and_then(Value::as_str).ok_or_else(bad)?;
        let enabled = entry.get("enabled").and_then(Value::as_bool).ok_or_else(bad)?;
        let priority = entry
            .get("priority")
            .and_then(Value::as_f64)
            .filter(|p| p.is_finite() && (0.0..=PRIORITY_MAX).contains(p))
            .ok_or_else(bad)?;
        let reference = entry.get("ref").and_then(Value::as_object).ok_or_else(bad)?;
        if source != "gamebanana" {
            unsupported += 1;
            continue;
        }
        let mod_id = reference.get("submissionId").and_then(id_of).ok_or_else(bad)?;
        let file_id = match reference.get("fileId") {
            None | Some(Value::Null) => None,
            Some(value) if value.as_u64() == Some(0) => None,
            Some(value) => Some(id_of(value).ok_or_else(bad)?),
        };
        ordered.push((priority, index, PackMod { mod_id, file_id, enabled }));
    }
    ordered.sort_by(|left, right| left.0.total_cmp(&right.0).then(left.1.cmp(&right.1)));
    let mut mods: Vec<PackMod> = Vec::with_capacity(ordered.len());
    let mut duplicates = 0;
    for (_, _, entry) in ordered {
        if mods.iter().any(|known| known.mod_id == entry.mod_id) {
            duplicates += 1;
        } else {
            mods.push(entry);
        }
    }
    if mods.is_empty() {
        return Err(Error::msg("This pack has no GameBanana mods."));
    }
    Ok(ReadPack {
        pack: Pack { name, note, mods },
        unsupported,
        duplicates,
    })
}

// ---------------------------------------------------------------------------
// Preview
// ---------------------------------------------------------------------------

/// What GameBanana says about one pack mod, for the preview row.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub name: String,
    pub nsfw: bool,
    /// A GameBanana image, never any other host.
    pub thumbnail: Option<String>,
    /// The file that would install: the pinned one, else the newest.
    pub file_id: Option<u64>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    /// A GameBanana download link for that file.
    pub download_url: Option<String>,
    /// The pinned file is gone; the newest is used instead.
    pub upgraded: bool,
    /// Why nothing can install, when nothing can.
    pub missing: Option<Missing>,
    /// The category reads as a script mod: the safety gate will ask before it turns on.
    pub script_like: bool,
}

/// Why a pack mod cannot install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// GameBanana does not have it (gone, withheld or private).
    Removed,
    /// It has no file to download.
    NoFiles,
    /// GameBanana could not be asked.
    Unreachable(String),
}

/// A row's state in the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    /// Still being looked up.
    Checking,
    /// Already installed in some slot.
    Installed,
    /// Will download and install.
    WillInstall,
    /// Will install, then wait off until the safety gate's question is answered.
    NeedsReview,
    Missing(Missing),
}

impl RowState {
    /// The row goes to the download queue on Install.
    pub fn installs(&self) -> bool {
        matches!(self, RowState::WillInstall | RowState::NeedsReview)
    }
}

fn gamebanana_download(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !url.chars().any(|c| c.is_control() || c.is_whitespace())
        && crate::install::GAMEBANANA_DOWNLOAD_HOSTS.iter().any(|known| known.eq_ignore_ascii_case(host))
}

/// A GameBanana image URL, or none.
pub fn gamebanana_image(url: &str) -> Option<String> {
    (url.starts_with("https://images.gamebanana.com/") && !url.chars().any(|c| c.is_control() || c.is_whitespace()))
        .then(|| url.to_string())
}

/// The facts of one pack mod from its GameBanana page. `file_id` is the pack's pinned file.
pub fn facts_from_profile(profile: &crate::gamebanana::ModProfile, file_id: Option<u64>) -> Facts {
    let mut facts = Facts {
        name: check_text(&profile.name, 200, false, "name").unwrap_or_default(),
        nsfw: profile.nsfw,
        thumbnail: profile.images.first().and_then(|image| gamebanana_image(&image.url)),
        file_id: None,
        file_name: None,
        size: None,
        download_url: None,
        upgraded: false,
        missing: None,
        script_like: [&profile.category, &profile.super_category]
            .iter()
            .any(|name| name.to_ascii_lowercase().contains("script")),
    };
    if profile.is_hidden() {
        facts.missing = Some(Missing::Removed);
        return facts;
    }
    let live: Vec<&crate::gamebanana::ModFile> = profile
        .files
        .iter()
        .filter(|file| !file.archived && gamebanana_download(&file.download_url))
        .collect();
    let pinned = file_id.and_then(|id| live.iter().find(|file| file.id == id).copied());
    let chosen = pinned.or_else(|| live.iter().max_by_key(|file| (file.added_ts, file.id)).copied());
    match chosen {
        Some(file) => {
            facts.upgraded = file_id.is_some() && pinned.is_none();
            facts.file_id = Some(file.id);
            facts.file_name = Some(check_text(&file.name, 200, false, "file").unwrap_or_default());
            facts.size = (file.size_bytes > 0).then_some(file.size_bytes);
            facts.download_url = Some(file.download_url.clone());
        }
        None => facts.missing = Some(Missing::NoFiles),
    }
    facts
}

/// A failed lookup as a row reason: a 404 means the mod is gone.
pub fn missing_from_error(error: &str) -> Missing {
    if error.contains("HTTP 404") || error.contains("HTTP 410") {
        Missing::Removed
    } else {
        Missing::Unreachable(error.chars().take(160).collect())
    }
}

/// A preview row's state: installed first (no lookup needed to say so), then what the lookup
/// found. A mod in the pack that is installed already is never downloaded again.
pub fn row_state(installed: bool, lookup: Option<&std::result::Result<Facts, String>>) -> RowState {
    if installed {
        return RowState::Installed;
    }
    match lookup {
        None => RowState::Checking,
        Some(Err(error)) => RowState::Missing(missing_from_error(error)),
        Some(Ok(facts)) => match &facts.missing {
            Some(missing) => RowState::Missing(missing.clone()),
            None if facts.script_like => RowState::NeedsReview,
            None => RowState::WillInstall,
        },
    }
}

/// The pack positions that install, in load order (first wins, so it goes in first and takes
/// the lowest free slot).
pub fn install_order(states: &[RowState], skip: &[bool]) -> Vec<usize> {
    states
        .iter()
        .enumerate()
        .filter(|(index, state)| state.installs() && !skip.get(*index).copied().unwrap_or(false))
        .map(|(index, _)| index)
        .collect()
}

// ---------------------------------------------------------------------------
// Saved packs
// ---------------------------------------------------------------------------

/// A pack kept in the Packs list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPack {
    pub slug: String,
    #[serde(flatten)]
    pub pack: Pack,
    /// Made on this machine (true) or received (false).
    pub mine: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn packs_dir(workspace: &Path) -> PathBuf {
    workspace.join(".dah").join("packs")
}

fn pack_path(workspace: &Path, slug: &str) -> Result<PathBuf> {
    if slug.is_empty() || slug.len() > 80 || !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(Error::msg("That pack name cannot be saved."));
    }
    Ok(packs_dir(workspace).join(format!("{slug}.json")))
}

/// Re-check a stored pack with the same rules as a received one: the file is the player's,
/// but it may have been edited by hand.
fn checked(saved: SavedPack) -> Option<SavedPack> {
    let read = parse_pack_json(&serde_json::to_string(&to_document(&saved.pack)).ok()?).ok()?;
    (read.pack == saved.pack).then_some(saved)
}

/// Every saved pack, newest change first. A file that does not read is skipped.
pub fn list_packs(workspace: &Path) -> Vec<SavedPack> {
    let Ok(dir) = fs::read_dir(packs_dir(workspace)) else {
        return Vec::new();
    };
    let mut packs: Vec<SavedPack> = dir
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".json"))
        .filter(|entry| entry.metadata().is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE_BYTES))
        .filter_map(|entry| serde_json::from_str::<SavedPack>(&fs::read_to_string(entry.path()).ok()?).ok())
        .filter_map(checked)
        .take(MAX_SAVED)
        .collect();
    packs.sort_by(|left, right| right.updated_at.cmp(&left.updated_at).then(left.slug.cmp(&right.slug)));
    packs
}

/// Save a new pack under a slug no other pack uses. Workspace only.
pub fn save_new(workspace: &Path, pack: &Pack, mine: bool) -> Result<SavedPack> {
    let base: String = slug_for(&pack.name).chars().take(64).collect::<String>().trim_matches('-').to_string();
    let base = if base.is_empty() { "pack".to_string() } else { base };
    let mut slug = base.clone();
    let mut n = 2;
    while pack_path(workspace, &slug)?.exists() {
        slug = format!("{base}-{n}");
        n += 1;
    }
    let now = now_iso();
    let saved = SavedPack { slug, pack: pack.clone(), mine, created_at: now.clone(), updated_at: now };
    write(workspace, &saved)?;
    Ok(saved)
}

/// Replace a saved pack's contents (name, note or mods), keeping its slug. Workspace only.
pub fn store(workspace: &Path, slug: &str, pack: &Pack) -> Result<SavedPack> {
    let path = pack_path(workspace, slug)?;
    let mut saved: SavedPack = serde_json::from_str(&fs::read_to_string(&path)?)
        .map_err(|_| Error::msg("That pack could not be read."))?;
    check_name(&pack.name)?;
    if pack.mods.len() > MAX_MODS {
        return Err(Error::msg(format!("A pack holds at most {MAX_MODS} mods.")));
    }
    saved.pack = pack.clone();
    saved.updated_at = now_iso();
    write(workspace, &saved)?;
    Ok(saved)
}

fn write(workspace: &Path, saved: &SavedPack) -> Result<()> {
    let path = pack_path(workspace, &saved.slug)?;
    fs::create_dir_all(packs_dir(workspace))?;
    write_atomic(&path, serde_json::to_string_pretty(saved)?.as_bytes())
}

/// Delete a saved pack. Workspace only.
pub fn delete(workspace: &Path, slug: &str) -> Result<()> {
    let path = pack_path(workspace, slug)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// A safe file name for a pack export.
pub fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
        .collect();
    let stem = cleaned.split_whitespace().collect::<Vec<_>>().join("_");
    let stem: String = stem.chars().take(64).collect();
    format!("{}{FILE_EXTENSION}", if stem.is_empty() { "pack" } else { &stem })
}

/// Write a pack file to exactly `path` (the Save dialog asked about replacing), adding the
/// extension when missing. Outside the game install only.
pub fn write_file(path: &Path, pack: &Pack) -> Result<PathBuf> {
    let mut path = path.to_path_buf();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() {
        return Err(Error::msg("The file needs a name."));
    }
    if !name.to_ascii_lowercase().ends_with(".json") {
        path.set_file_name(format!("{name}{FILE_EXTENSION}"));
    }
    write_atomic(&path, to_json(pack)?.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
#[path = "packs_tests.rs"]
mod tests;
