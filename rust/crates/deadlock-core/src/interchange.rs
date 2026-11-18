//! The Deadlock Mod Interchange: the manager-neutral document Grimoire and Deadlock Mod Manager
//! both read and write (`deadlock-mod-interchange`, version 1, `mod-interchange.json`). Ported
//! from Grimoire's `src/lib/modInterchange.ts` and `electron/main/services/modInterchange/
//! bundle.ts` (MIT).
//!
//! A bundle is a folder holding `mod-interchange.json` plus the VPKs it lists, with paths
//! relative to the manifest. A document can also travel alone (no files): its mods then name
//! GameBanana submissions to download. Three sections: mods, profiles (which mods are on, the
//! autoexec lines, a crosshair key) and crosshairs (the game's own `citadel_crosshair_*`
//! convars, so no app's model leaks into the format).
//!
//! How each app shares: Grimoire exports and imports this bundle and reads DMM's own files;
//! Deadlock Mod Manager exports and imports the same bundle and also shares a profile as a link
//! on its own server (not on Sigil's network allowlist, so Sigil does not read those links).
//! Sigil reads and writes the bundle, keeps its `mp1:` profile codes
//! ([`crate::profiles`]) and crosshair share codes ([`crate::crosshair`]).
//!
//! Reading is lenient like Grimoire's: a broken entry costs that entry, as a warning, never the
//! document; a file path that leaves the bundle folder is dropped. Export writes only into the
//! folder the caller picks. Import produces [`crate::import::ForeignMod`]s for the existing
//! import plan (installs land switched off), profiles and crosshair presets; nothing here writes
//! the game install.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::crosshair::Crosshair;
use crate::error::{Error, Result};
use crate::import::ForeignMod;
use crate::profiles::{PROFILE_SCHEMA, Profile, ProfileSlot};
use crate::util::now_iso;

pub const FORMAT: &str = "deadlock-mod-interchange";
pub const VERSION: u32 = 1;
pub const MANIFEST: &str = "mod-interchange.json";
/// How Sigil names itself in `source.manager`.
pub const MANAGER: &str = "sigil";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub manager: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeFile {
    /// Plain file name, `*.vpk`.
    pub name: String,
    /// Relative to the manifest in a bundle; absolute after [`read_bundle`].
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// False for an alternative variant that is kept but not loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum Origin {
    #[serde(rename_all = "camelCase")]
    Gamebanana {
        /// `mod` or `sound`.
        submission_type: String,
        submission_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_name: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Local {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_id: Option<String>,
    },
}

impl Origin {
    /// The GameBanana submission id, for a GameBanana origin.
    pub fn gamebanana_id(&self) -> Option<u64> {
        match self {
            Origin::Gamebanana { submission_id, .. } => submission_id.parse().ok(),
            Origin::Local { .. } => None,
        }
    }

    pub fn file_id(&self) -> Option<u64> {
        match self {
            Origin::Gamebanana { file_id, .. } => *file_id,
            Origin::Local { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeMod {
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub order: f64,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nsfw: Option<bool>,
    pub files: Vec<InterchangeFile>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extensions: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProfileMod {
    pub mod_key: String,
    pub enabled: bool,
    pub order: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeProfile {
    pub key: String,
    pub name: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub mods: Vec<InterchangeProfileMod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crosshair_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoexec: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterchangeCrosshair {
    pub key: String,
    pub name: String,
    pub active: bool,
    pub convars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub format: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub source: Source,
    /// Sections present: `mods`, `profiles`, `crosshairs`.
    pub contents: Vec<String>,
    pub mods: Vec<InterchangeMod>,
    pub profiles: Vec<InterchangeProfile>,
    pub crosshairs: Vec<InterchangeCrosshair>,
    pub warnings: Vec<String>,
}

impl Document {
    /// An empty document from Sigil.
    pub fn new() -> Document {
        Document {
            format: FORMAT.to_string(),
            version: VERSION,
            created_at: Some(now_iso()),
            source: Source {
                manager: MANAGER.to_string(),
                manager_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                profile_name: None,
            },
            contents: vec!["mods".to_string()],
            mods: Vec::new(),
            profiles: Vec::new(),
            crosshairs: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn include(&mut self, section: &str) {
        if !self.contents.iter().any(|known| known == section) {
            self.contents.push(section.to_string());
        }
    }

    /// Drop profile entries that name a mod the document does not carry, with a warning.
    pub fn drop_dangling_profile_entries(&mut self) {
        let keys: Vec<String> = self.mods.iter().map(|m| m.key.clone()).collect();
        for profile in &mut self.profiles {
            let before = profile.mods.len();
            profile.mods.retain(|entry| keys.contains(&entry.mod_key));
            let dropped = before - profile.mods.len();
            if dropped > 0 {
                self.warnings.push(format!(
                    "Profile {}: {dropped} entry(s) point at mods that are not available",
                    profile.name
                ));
            }
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Document::new()
    }
}

/// `gamebanana:mod:<id>` or `gamebanana:sound:<id>`, the key both apps use.
pub fn gamebanana_key(sound: bool, id: u64) -> String {
    format!("gamebanana:{}:{id}", if sound { "sound" } else { "mod" })
}

/// One plain `*.vpk` file name.
pub fn is_plain_vpk_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\'])
        && name != "."
        && name != ".."
        && name.to_ascii_lowercase().ends_with(".vpk")
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

fn number(value: &Value, key: &str, fallback: f64) -> f64 {
    value.get(key).and_then(Value::as_f64).filter(|n| n.is_finite()).unwrap_or(fallback)
}

fn parse_origin(raw: &Value) -> Option<Origin> {
    match raw.get("provider").and_then(Value::as_str)? {
        "gamebanana" => {
            let id = match raw.get("submissionId")? {
                Value::Number(n) => n.as_u64()?.to_string(),
                Value::String(s) => s.trim().to_string(),
                _ => return None,
            };
            if id.is_empty() || id.starts_with('0') || !id.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let submission_type =
                if raw.get("submissionType").and_then(Value::as_str) == Some("sound") { "sound" } else { "mod" };
            Some(Origin::Gamebanana {
                submission_type: submission_type.to_string(),
                submission_id: id,
                file_id: raw.get("fileId").and_then(Value::as_u64).filter(|id| *id > 0),
                file_name: text(raw, "fileName"),
            })
        }
        "local" => Some(Origin::Local { local_id: text(raw, "localId") }),
        _ => None,
    }
}

fn parse_file(raw: &Value) -> Option<InterchangeFile> {
    Some(InterchangeFile {
        name: text(raw, "name")?,
        path: text(raw, "path")?,
        sha256: text(raw, "sha256"),
        size: raw.get("size").and_then(Value::as_u64),
        selected: raw.get("selected").and_then(Value::as_bool),
    })
}

fn parse_mod(raw: &Value) -> std::result::Result<InterchangeMod, String> {
    if !raw.is_object() {
        return Err("not an object".to_string());
    }
    let key = text(raw, "key").ok_or("it has no key")?;
    let name = text(raw, "name").ok_or_else(|| format!("{key} has no name"))?;
    let origin = parse_origin(raw.get("origin").unwrap_or(&Value::Null))
        .ok_or_else(|| format!("{name} has an unsupported origin"))?;
    let files = raw
        .get("files")
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(parse_file).collect())
        .unwrap_or_default();
    Ok(InterchangeMod {
        key,
        name,
        enabled: raw.get("enabled").and_then(Value::as_bool) == Some(true),
        order: number(raw, "order", 0.0),
        origin,
        author: text(raw, "author"),
        description: text(raw, "description"),
        category: text(raw, "category"),
        hero: text(raw, "hero"),
        thumbnail_url: text(raw, "thumbnailUrl"),
        link: text(raw, "link"),
        nsfw: raw.get("nsfw").and_then(Value::as_bool),
        files,
        extensions: raw.get("extensions").and_then(Value::as_object).cloned().unwrap_or_default(),
    })
}

fn parse_profile(raw: &Value) -> std::result::Result<InterchangeProfile, String> {
    if !raw.is_object() {
        return Err("not an object".to_string());
    }
    let key = text(raw, "key").ok_or("it has no key")?;
    let name = text(raw, "name").ok_or_else(|| format!("{key} has no name"))?;
    let mods = raw
        .get("mods")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    Some(InterchangeProfileMod {
                        mod_key: text(entry, "modKey")?,
                        enabled: entry.get("enabled").and_then(Value::as_bool) == Some(true),
                        order: number(entry, "order", index as f64),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let autoexec: Option<Vec<String>> = raw.get("autoexec").and_then(Value::as_array).map(|lines| {
        lines.iter().filter_map(Value::as_str).map(str::to_string).collect()
    });
    Ok(InterchangeProfile {
        key,
        name,
        active: raw.get("active").and_then(Value::as_bool) == Some(true),
        description: text(raw, "description"),
        mods,
        crosshair_key: text(raw, "crosshairKey"),
        autoexec: autoexec.filter(|lines| !lines.is_empty()),
    })
}

fn parse_crosshair(raw: &Value) -> std::result::Result<InterchangeCrosshair, String> {
    if !raw.is_object() {
        return Err("not an object".to_string());
    }
    let key = text(raw, "key").ok_or("it has no key")?;
    let mut convars = BTreeMap::new();
    if let Some(map) = raw.get("convars").and_then(Value::as_object) {
        for (name, value) in map {
            let value = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            convars.insert(name.clone(), value);
        }
    }
    Ok(InterchangeCrosshair {
        name: text(raw, "name").unwrap_or_else(|| key.clone()),
        key,
        active: raw.get("active").and_then(Value::as_bool) == Some(true),
        convars,
    })
}

fn parse_section<T>(
    raw: Option<&Value>,
    label: &str,
    parse: fn(&Value) -> std::result::Result<T, String>,
    key_of: fn(&T) -> &str,
    warnings: &mut Vec<String>,
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for (index, entry) in raw.and_then(Value::as_array).into_iter().flatten().enumerate() {
        match parse(entry) {
            Err(reason) => warnings.push(format!("Skipped {label} #{index}: {reason}")),
            Ok(parsed) if out.iter().any(|known| key_of(known) == key_of(&parsed)) => {
                warnings.push(format!("Skipped duplicate {label} key {}", key_of(&parsed)));
            }
            Ok(parsed) => out.push(parsed),
        }
    }
    out
}

/// Parse a document leniently: the envelope must be right, a broken entry is only a warning.
pub fn parse_document(text_in: &str) -> Result<Document> {
    let raw: Value = serde_json::from_str(text_in)
        .map_err(|error| Error::msg(format!("{MANIFEST} is not valid JSON: {error}")))?;
    if !raw.is_object() {
        return Err(Error::msg(format!("{MANIFEST} must be a JSON object")));
    }
    let format = raw.get("format").and_then(Value::as_str).unwrap_or("");
    if format != FORMAT {
        return Err(Error::msg(format!(
            "Not a mod interchange document (format is \"{format}\", expected \"{FORMAT}\")"
        )));
    }
    let version = raw.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version < 1 || version > u64::from(VERSION) {
        return Err(Error::msg(format!(
            "Unsupported interchange version {version} (this build understands {VERSION})"
        )));
    }
    let mut warnings: Vec<String> = raw
        .get("warnings")
        .and_then(Value::as_array)
        .map(|w| w.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mods = parse_section(raw.get("mods"), "mod", parse_mod, |m| &m.key, &mut warnings);
    let profiles = parse_section(raw.get("profiles"), "profile", parse_profile, |p| &p.key, &mut warnings);
    let crosshairs =
        parse_section(raw.get("crosshairs"), "crosshair", parse_crosshair, |c| &c.key, &mut warnings);
    let source = raw.get("source").cloned().unwrap_or(Value::Null);
    let mut document = Document {
        format: FORMAT.to_string(),
        version: version as u32,
        created_at: text(&raw, "createdAt"),
        source: Source {
            manager: text(&source, "manager").unwrap_or_else(|| "unknown".to_string()),
            manager_version: text(&source, "managerVersion"),
            profile_name: text(&source, "profileName"),
        },
        contents: vec!["mods".to_string()],
        mods,
        profiles,
        crosshairs,
        warnings,
    };
    document.drop_dangling_profile_entries();
    if !document.profiles.is_empty() {
        document.include("profiles");
    }
    if !document.crosshairs.is_empty() {
        document.include("crosshairs");
    }
    Ok(document)
}

/// The manifest inside a bundle folder, or `path` itself when it is the manifest.
pub fn manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() { path.join(MANIFEST) } else { path.to_path_buf() }
}

/// Read a bundle (its folder or its manifest) and make its file paths absolute. A path that is
/// absolute or leaves the bundle folder is dropped with a warning, so a crafted manifest can
/// never point the import at another file on disk.
pub fn read_bundle(path: &Path) -> Result<Document> {
    let manifest = manifest_path(path);
    let text = fs::read_to_string(&manifest)
        .map_err(|error| Error::msg(format!("Could not read {}: {error}", manifest.display())))?;
    let mut document = parse_document(&text)?;
    let base = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut warnings = Vec::new();
    for item in &mut document.mods {
        item.files.retain_mut(|file| {
            let relative = Path::new(&file.path);
            let inside = !relative.is_absolute()
                && relative.components().all(|part| matches!(part, Component::Normal(_)))
                && !file.path.contains('\\');
            if !inside || !is_plain_vpk_name(&file.name) {
                warnings.push(format!("{}: ignored file path outside the bundle ({})", item.name, file.path));
                return false;
            }
            file.path = base.join(relative).to_string_lossy().into_owned();
            true
        });
    }
    document.warnings.extend(warnings);
    Ok(document)
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// One installed mod to export.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExportMod {
    pub name: String,
    pub enabled: bool,
    /// Load order: the slot.
    pub slot: u16,
    pub gamebanana_id: Option<u64>,
    pub file_id: Option<u64>,
    pub sound: bool,
    pub author: Option<String>,
    pub category: Option<String>,
    pub hero: Option<String>,
    pub thumbnail_url: Option<String>,
    /// The directory VPK first, then its chunks. Empty for a document without files.
    pub files: Vec<PathBuf>,
}

/// A crosshair to export: a name, the settings, whether it is the one in use.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportCrosshair {
    pub name: String,
    pub settings: Crosshair,
    pub active: bool,
}

/// What [`export_bundle`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub folder: PathBuf,
    pub mods: usize,
    pub files: usize,
    pub bytes: u64,
    pub warnings: Vec<String>,
}

/// The game's crosshair convars as a map, the format's crosshair shape.
pub fn crosshair_convars(crosshair: &Crosshair) -> BTreeMap<String, String> {
    crosshair
        .commands()
        .iter()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, value)| (name.to_string(), value.trim().to_string()))
        .collect()
}

/// Settings from a crosshair's convars, on top of `current` for anything it does not set.
pub fn crosshair_from_convars(convars: &BTreeMap<String, String>, current: &Crosshair) -> Result<Crosshair> {
    let text: String = convars
        .iter()
        .filter(|(name, _)| name.starts_with("citadel_crosshair_"))
        .map(|(name, value)| format!("{name} {value}\n"))
        .collect();
    crate::crosshair::parse_console(&text, current)
}

/// The document key of an exported mod.
fn key_of(item: &ExportMod) -> String {
    match item.gamebanana_id {
        Some(id) => gamebanana_key(item.sound, id),
        None => local_key(item),
    }
}

fn local_key(item: &ExportMod) -> String {
    let fingerprint = item
        .files
        .first()
        .and_then(|path| crate::profiles::fingerprint_file(path).ok())
        .unwrap_or_else(|| format!("slot{}", item.slot));
    format!("local:{}", fingerprint.trim_start_matches("q1:").chars().take(16).collect::<String>())
}

/// Build a document from Sigil's library, profiles and crosshairs. With `with_files` false the
/// mods carry no files (a share that downloads from GameBanana); local mods are then left out
/// with a warning, since nothing could fetch them.
pub fn build_document(
    mods: &[ExportMod],
    profiles: &[Profile],
    active_profile: Option<&str>,
    crosshairs: &[ExportCrosshair],
    with_files: bool,
) -> Document {
    let mut document = Document::new();
    let mut keys: Vec<(String, Option<u64>, Option<String>)> = Vec::new();
    for item in mods {
        let fingerprint = item.files.first().and_then(|path| crate::profiles::fingerprint_file(path).ok());
        let key = key_of(item);
        let origin = match item.gamebanana_id {
            Some(id) => Origin::Gamebanana {
                    submission_type: if item.sound { "sound" } else { "mod" }.to_string(),
                    submission_id: id.to_string(),
                    file_id: item.file_id,
                    file_name: None,
                },
            None if !with_files => {
                document.warnings.push(format!("{}: not from GameBanana, left out of a share without files", item.name));
                continue;
            }
            None => Origin::Local { local_id: Some(key.clone()) },
        };
        if keys.iter().any(|(known, _, _)| *known == key) {
            continue;
        }
        keys.push((key.clone(), item.gamebanana_id, fingerprint));
        document.mods.push(InterchangeMod {
            key,
            name: item.name.clone(),
            enabled: item.enabled,
            order: f64::from(item.slot),
            origin,
            author: item.author.clone(),
            description: None,
            category: item.category.clone(),
            hero: item.hero.clone(),
            thumbnail_url: item.thumbnail_url.clone(),
            link: item.gamebanana_id.map(|id| {
                format!("https://gamebanana.com/{}/{id}", if item.sound { "sounds" } else { "mods" })
            }),
            nsfw: None,
            files: Vec::new(),
            extensions: Map::new(),
        });
    }

    let crosshair_key = |index: usize| format!("sigil:crosshair:{index}");
    for (index, crosshair) in crosshairs.iter().enumerate() {
        document.crosshairs.push(InterchangeCrosshair {
            key: crosshair_key(index),
            name: crosshair.name.clone(),
            active: crosshair.active,
            convars: crosshair_convars(&crosshair.settings),
        });
    }

    for profile in profiles {
        let mut entries = Vec::new();
        for slot in &profile.slots {
            let key = keys.iter().find(|(_, id, fingerprint)| {
                (slot.gamebanana_id.is_some() && *id == slot.gamebanana_id)
                    || (slot.fingerprint.is_some() && *fingerprint == slot.fingerprint)
            });
            if let Some((key, _, _)) = key {
                entries.push(InterchangeProfileMod { mod_key: key.clone(), enabled: slot.enabled, order: f64::from(slot.slot) });
            }
        }
        let crosshair = profile.crosshair.as_ref().map(|settings| {
            let index = document.crosshairs.len();
            document.crosshairs.push(InterchangeCrosshair {
                key: crosshair_key(index),
                name: format!("{} crosshair", profile.name),
                active: false,
                convars: crosshair_convars(settings),
            });
            crosshair_key(index)
        });
        document.profiles.push(InterchangeProfile {
            key: format!("sigil:profile:{}", profile.slug),
            name: profile.name.clone(),
            active: active_profile == Some(profile.slug.as_str()),
            description: None,
            mods: entries,
            crosshair_key: crosshair,
            autoexec: profile.autoexec.clone().filter(|lines| !lines.is_empty()),
        });
    }
    if !document.profiles.is_empty() {
        document.include("profiles");
    }
    if !document.crosshairs.is_empty() {
        document.include("crosshairs");
    }
    document
}

/// Write a bundle into a new folder under `parent` (app data or a folder the player picked,
/// never the game): `sigil-mods-export-<stamp>/mod-interchange.json` plus `files/<n>/<vpk>`
/// for every mod's files. Returns where it went.
pub fn export_bundle(
    parent: &Path,
    mods: &[ExportMod],
    profiles: &[Profile],
    active_profile: Option<&str>,
    crosshairs: &[ExportCrosshair],
) -> Result<ExportReport> {
    let mut document = build_document(mods, profiles, active_profile, crosshairs, true);
    let stamp = now_iso().replace([':', '.'], "-");
    let mut folder = parent.join(format!("sigil-mods-export-{stamp}"));
    let mut n = 2;
    while folder.exists() {
        folder = parent.join(format!("sigil-mods-export-{stamp}-{n}"));
        n += 1;
    }
    fs::create_dir_all(folder.join("files"))?;
    let result = (|| -> Result<(usize, u64)> {
        let (mut files, mut bytes) = (0usize, 0u64);
        for (index, entry) in document.mods.iter_mut().enumerate() {
            let Some(source) = mods.iter().find(|item| key_of(item) == entry.key) else {
                continue;
            };
            for (position, path) in source.files.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !is_plain_vpk_name(&name) {
                    continue;
                }
                let relative = format!("files/{index}/{name}");
                fs::create_dir_all(folder.join("files").join(index.to_string()))?;
                let copied = fs::copy(path, folder.join(&relative))?;
                bytes += copied;
                files += 1;
                entry.files.push(InterchangeFile {
                    name,
                    path: relative,
                    sha256: (position == 0).then(|| crate::identify::sha256_file(path).ok()).flatten(),
                    size: Some(copied),
                    selected: None,
                });
            }
        }
        fs::write(folder.join(MANIFEST), serde_json::to_vec_pretty(&document)?)?;
        Ok((files, bytes))
    })();
    match result {
        Ok((files, bytes)) => Ok(ExportReport {
            folder,
            mods: document.mods.len(),
            files,
            bytes,
            warnings: document.warnings,
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(&folder);
            Err(error)
        }
    }
}

/// Write a document alone (no files) to `path`, for a share that downloads from GameBanana.
/// `path` is a file the player picked or app data, never the game folder.
pub fn write_document(path: &Path, document: &Document) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::util::write_atomic(path, &serde_json::to_vec_pretty(document)?)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// The bundle's mods that carry a directory VPK, as import candidates for
/// [`crate::import::plan_import`] (they land switched off). Each selected `_dir.vpk` becomes one
/// mod with its `_NNN` chunks from the same entry; an entry with only a single-file VPK is one
/// mod. Mods without files are left out: download them by [`Origin::gamebanana_id`].
pub fn foreign_mods(document: &Document) -> Vec<ForeignMod> {
    let mut out = Vec::new();
    for item in &document.mods {
        let selected: Vec<&InterchangeFile> =
            item.files.iter().filter(|file| file.selected != Some(false)).collect();
        let dirs: Vec<&&InterchangeFile> = selected
            .iter()
            .filter(|file| {
                let lower = file.name.to_ascii_lowercase();
                lower.ends_with("_dir.vpk") || !is_chunk_name(&lower)
            })
            .collect();
        for dir in dirs {
            let path = PathBuf::from(&dir.path);
            if !path.is_file() {
                continue;
            }
            let lower = dir.name.to_ascii_lowercase();
            let stem = lower.strip_suffix("_dir.vpk").map(|s| format!("{s}_"));
            let mut chunks: Vec<PathBuf> = stem
                .map(|stem| {
                    selected
                        .iter()
                        .filter(|file| {
                            let name = file.name.to_ascii_lowercase();
                            name.starts_with(&stem) && is_chunk_name(&name)
                        })
                        .map(|file| PathBuf::from(&file.path))
                        .collect()
                })
                .unwrap_or_default();
            chunks.sort();
            out.push(ForeignMod {
                name: item.name.clone(),
                author: item.author.clone(),
                thumbnail_url: item.thumbnail_url.clone(),
                gamebanana_id: item.origin.gamebanana_id(),
                size_bytes: fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                file: path,
                chunks,
                enabled_in_source: item.enabled,
                archive: false,
            });
        }
    }
    out
}

fn is_chunk_name(lower: &str) -> bool {
    lower
        .strip_suffix(".vpk")
        .and_then(|stem| stem.rsplit_once('_'))
        .is_some_and(|(_, index)| index.len() == 3 && index.chars().all(|c| c.is_ascii_digit()))
}

/// The bundle's crosshairs as Sigil presets (name, settings, active), on top of `current` for
/// any convar a crosshair leaves out. One that does not parse is a warning.
pub fn crosshairs(document: &Document, current: &Crosshair) -> (Vec<ExportCrosshair>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    for crosshair in &document.crosshairs {
        match crosshair_from_convars(&crosshair.convars, current) {
            Ok(settings) => out.push(ExportCrosshair { name: crosshair.name.clone(), settings, active: crosshair.active }),
            Err(error) => warnings.push(format!("Crosshair {}: {error}", crosshair.name)),
        }
    }
    (out, warnings)
}

/// The bundle's profiles as Sigil profiles, to save with [`crate::profiles::write_profile`].
/// Each entry is identified by its GameBanana ids and, when the bundle carries the file, its
/// fingerprint; the slot is left at 0 so applying never guesses by position. A profile's
/// crosshair comes along when its key resolves.
pub fn profiles(document: &Document, current: &Crosshair) -> Vec<Profile> {
    let now = now_iso();
    document
        .profiles
        .iter()
        .map(|entry| {
            let slots = entry
                .mods
                .iter()
                .filter_map(|item| {
                    let source = document.mods.iter().find(|m| m.key == item.mod_key)?;
                    let dir = source.files.iter().find(|file| file.selected != Some(false));
                    let fingerprint = dir
                        .map(|file| PathBuf::from(&file.path))
                        .filter(|path| path.is_file())
                        .and_then(|path| crate::profiles::fingerprint_file(&path).ok());
                    Some(ProfileSlot {
                        slot: 0,
                        file: dir.map(|file| file.name.clone()).unwrap_or_default(),
                        enabled: item.enabled,
                        name: Some(source.name.clone()),
                        gamebanana_id: source.origin.gamebanana_id(),
                        gamebanana_file_id: source.origin.file_id(),
                        fingerprint,
                    })
                })
                .collect();
            let crosshair = entry
                .crosshair_key
                .as_ref()
                .and_then(|key| document.crosshairs.iter().find(|c| &c.key == key))
                .and_then(|c| crosshair_from_convars(&c.convars, current).ok());
            Profile {
                schema: PROFILE_SCHEMA,
                name: entry.name.clone(),
                slug: crate::profiles::slug_for(&entry.name),
                created_at: now.clone(),
                updated_at: now.clone(),
                slots,
                crosshair,
                autoexec: entry.autoexec.clone(),
                author: Some(document.source.manager.clone()).filter(|m| m != MANAGER),
                extensions: Map::new(),
                isolated: false,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sigil-interchange-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_a_grimoire_document_leniently() {
        let text = r#"{"format":"deadlock-mod-interchange","version":1,
          "source":{"manager":"grimoire","managerVersion":"2.1"},
          "mods":[
            {"key":"gamebanana:mod:5","name":"Neon","enabled":true,"order":1,
             "origin":{"provider":"gamebanana","submissionType":"mod","submissionId":5,"fileId":9},
             "files":[{"name":"pak01_dir.vpk","path":"files/0/pak01_dir.vpk"}]},
            {"key":"x","name":"Broken","origin":{"provider":"ftp"}},
            {"key":"gamebanana:mod:5","name":"Dupe","origin":{"provider":"local"}},
            {"name":"No key"}],
          "profiles":[{"key":"p","name":"Main","active":true,"mods":[{"modKey":"gamebanana:mod:5","enabled":true},{"modKey":"gone"}],
            "crosshairKey":"c","autoexec":["fps_max 240"]}],
          "crosshairs":[{"key":"c","name":"Dot","convars":{"citadel_crosshair_dot_size":4,"citadel_crosshair_color_r":"255"}}]}"#;
        let document = parse_document(text).unwrap();
        assert_eq!(document.mods.len(), 1);
        assert_eq!(document.mods[0].origin.gamebanana_id(), Some(5));
        assert_eq!(document.mods[0].origin.file_id(), Some(9));
        assert_eq!(document.profiles[0].mods.len(), 1, "the dangling entry is dropped");
        assert_eq!(document.warnings.len(), 4, "{:?}", document.warnings);
        assert_eq!(document.contents, ["mods", "profiles", "crosshairs"]);
        assert_eq!(document.crosshairs[0].convars["citadel_crosshair_dot_size"], "4");

        let current = Crosshair::default();
        let (presets, problems) = crosshairs(&document, &current);
        assert!(problems.is_empty());
        assert_eq!(presets[0].settings.color_r, 255);
        let imported = profiles(&document, &current);
        assert_eq!(imported[0].slots[0].gamebanana_id, Some(5));
        assert_eq!(imported[0].slots[0].slot, 0);
        assert!(imported[0].crosshair.is_some());
        assert_eq!(imported[0].autoexec.as_deref(), Some(&["fps_max 240".to_string()][..]));
        assert_eq!(imported[0].author.as_deref(), Some("grimoire"));

        assert!(parse_document(r#"{"format":"other","version":1}"#).is_err());
        assert!(parse_document(r#"{"format":"deadlock-mod-interchange","version":2}"#).is_err());
    }

    #[test]
    fn a_bundle_round_trips_and_stays_inside_its_folder() {
        let root = scratch("bundle");
        let addons = root.join("addons");
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("pak03_dir.vpk"), b"dir-bytes").unwrap();
        fs::write(addons.join("pak03_000.vpk"), b"chunk").unwrap();
        fs::write(addons.join("pak04_dir.vpk"), b"local-mod").unwrap();
        let mods = vec![
            ExportMod {
                name: "Neon".into(),
                enabled: true,
                slot: 3,
                gamebanana_id: Some(5),
                file_id: Some(9),
                files: vec![addons.join("pak03_dir.vpk"), addons.join("pak03_000.vpk")],
                ..ExportMod::default()
            },
            ExportMod { name: "Mine".into(), slot: 4, files: vec![addons.join("pak04_dir.vpk")], ..ExportMod::default() },
        ];
        let profile = Profile {
            schema: PROFILE_SCHEMA,
            name: "Main".into(),
            slug: "main".into(),
            created_at: String::new(),
            updated_at: String::new(),
            slots: vec![ProfileSlot {
                slot: 3,
                file: "pak03_dir.vpk".into(),
                enabled: true,
                name: None,
                gamebanana_id: Some(5),
                gamebanana_file_id: Some(9),
                fingerprint: None,
            }],
            crosshair: None,
            autoexec: None,
            author: None,
            extensions: Map::new(),
            isolated: false,
        };
        let crosshair = ExportCrosshair { name: "Mine".into(), settings: Crosshair::default(), active: true };
        let report = export_bundle(&root.join("out"), &mods, &[profile], Some("main"), &[crosshair]).unwrap();
        assert_eq!((report.mods, report.files), (2, 3));

        let document = read_bundle(&report.folder).unwrap();
        assert_eq!(document.source.manager, MANAGER);
        assert!(document.profiles[0].active);
        let foreign = foreign_mods(&document);
        assert_eq!(foreign.len(), 2);
        assert_eq!(foreign[0].name, "Neon");
        assert_eq!(foreign[0].chunks.len(), 1);
        assert_eq!(fs::read(&foreign[0].file).unwrap(), b"dir-bytes");
        assert_eq!(foreign[1].gamebanana_id, None);
        let (presets, _) = crosshairs(&document, &Crosshair::default());
        assert_eq!(presets[0].settings, Crosshair::default());

        // A crafted path that leaves the bundle is dropped.
        let manifest = report.folder.join(MANIFEST);
        let text = fs::read_to_string(&manifest).unwrap().replace("files/0/pak03_dir.vpk", "../../addons/pak04_dir.vpk");
        fs::write(&manifest, text).unwrap();
        let tampered = read_bundle(&manifest).unwrap();
        assert!(tampered.warnings.iter().any(|w| w.contains("outside the bundle")));
        assert!(tampered.mods[0].files.iter().all(|f| !f.path.contains("..")));

        // A share without files keeps only GameBanana mods.
        let share = build_document(&mods, &[], None, &[], false);
        assert_eq!(share.mods.len(), 1);
        assert_eq!(share.warnings.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
