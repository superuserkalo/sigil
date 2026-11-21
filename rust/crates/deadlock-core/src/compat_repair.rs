//! Compatibility repairs after a game update, built on vpkmerge's healers.
//!
//! Deadlock names every asset by path, and compiled resources carry values from the build they
//! were made against: skeleton bone tables, the `root_motion` attachment frame that carries
//! cameras, sound event bindings, game data fields and text. A game update changes those in the
//! game's own `pak01`, and an old mod keeps the old copies, so the hero can look wrong, a camera
//! can sit in the wrong place or footsteps can go silent.
//!
//! This module is the repair half of [`crate::mod_health`], which only detects. [`check`] is the
//! dry run: it compares one mod with the installed game and reports what a repair would change.
//! [`repair_to`] writes one repaired archive to a staging path the caller picks, then verifies it
//! the way Grimoire does before returning.
//!
//! Repairs come in two families. [`RepairKind::Data`] fills absent fields in `scripts/heroes.vdata`
//! and `scripts/abilities.vdata`, merges the game's text, refreshes model skeleton references and
//! repairs sound event bindings. [`RepairKind::Cameras`] rewrites the `root_motion` attachments
//! that carry camera inputs and restores camera settings the model is missing.
//!
//! Nothing here writes to the game install or to the mod. The app owns the backup, the replace and
//! the revert, under the same running-game guard as its other install actions.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use vpkmerge_core::{
    AttachmentHealer, AttachmentReport, AttachmentStatus, CompatibilityRepairer, RepairStatus,
};

use crate::error::{Error, Result};
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Suffixes of compiled animation resources that compete with a rig repair.
const ANIMATION_SUFFIXES: [&str; 6] = [
    ".vnmskel_c",
    ".vnmclip_c",
    ".vnmgraph_c",
    ".vanmgrph_c",
    ".vanim_c",
    ".vagrp_c",
];

/// The family of repairs to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepairKind {
    /// vpkmerge's compatibility heal: data fields, text, sound bindings and skeletons.
    Data,
    /// vpkmerge's attachments heal: the `root_motion` frame that carries camera inputs.
    Cameras,
}

impl RepairKind {
    pub fn label(self) -> &'static str {
        match self {
            RepairKind::Data => "Data",
            RepairKind::Cameras => "Cameras",
        }
    }
}

/// One entry a repair looked at.
///
/// `kind` is the upstream repair family that applies, for example `data`, `localization`,
/// `skeleton`, `sounds` or `cameras`. `repairable` is false when the entry cannot be fixed: an
/// undecodable resource, or a model the game no longer ships.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFinding {
    /// Path inside the mod.
    pub entry: String,
    /// Repair family that applies to the entry.
    pub kind: String,
    /// Whether a repair changes this entry.
    pub repairable: bool,
    /// Number of values or attachments a repair changes.
    pub changes: usize,
    /// Why the entry is not repairable, or what changed. Empty when vpkmerge has nothing to add.
    pub detail: String,
}

/// Dry run: what a repair would change in `mod_vpk` against the installed game's `pak01_dir.vpk`.
///
/// Read only. Returns the entries a repair would touch, plus entries that cannot be repaired and
/// why. A mod with nothing to fix returns an empty list. Fails when either archive is missing. A
/// split mod is combined into a temporary single-file copy first (see [`SingleFile`]). `animation_overrides` only reaches the data heal: pass
/// [`has_animation_overrides`] for the other enabled mods. The camera repair has no such flag
/// upstream.
pub fn check(
    base_pak01: &Path,
    mod_vpk: &Path,
    kind: RepairKind,
    animation_overrides: bool,
) -> Result<Vec<RepairFinding>> {
    require_archive(base_pak01, "game pak01")?;
    require_archive(mod_vpk, "mod")?;
    let scratch = std::env::temp_dir().join(format!(
        "sigil-compat-check-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let single = SingleFile::of(mod_vpk, &scratch)?;
    let result = check_single(base_pak01, single.path(), kind, animation_overrides);
    drop(single);
    let _ = fs::remove_dir(&scratch);
    result
}

fn check_single(
    base_pak01: &Path,
    mod_vpk: &Path,
    kind: RepairKind,
    animation_overrides: bool,
) -> Result<Vec<RepairFinding>> {
    match kind {
        RepairKind::Data => {
            refuse_damaged_game_text(base_pak01, mod_vpk)?;
            let mut repairer = CompatibilityRepairer::new(base_pak01).map_err(|error| {
                Error::msg(format!(
                    "Could not open the game pak01 {} for repair: {error:#}",
                    base_pak01.display()
                ))
            })?;
            let reports = repairer
                .check(mod_vpk, animation_overrides)
                .map_err(|error| {
                    Error::msg(format!(
                        "Could not check {} for compatibility: {error:#}",
                        mod_vpk.display()
                    ))
                })?;
            Ok(reports.into_iter().map(finding_from_report).collect())
        }
        RepairKind::Cameras => {
            let mut healer = AttachmentHealer::new(base_pak01);
            let reports = healer.check(mod_vpk).map_err(|error| {
                Error::msg(format!(
                    "Could not check the models in {}: {error:#}",
                    mod_vpk.display()
                ))
            })?;
            Ok(reports
                .into_iter()
                .filter_map(finding_from_attachment)
                .collect())
        }
    }
}

/// A game text file that cannot be used as the repair's baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DamagedGameText {
    /// Path inside the game's `pak01` or its loose files, for example
    /// `resource/localization/citadel_heroes/citadel_heroes_english.txt`.
    pub entry: String,
    /// What the strict read found wrong.
    pub detail: String,
}

/// The repair merges game text only from a complete base table (DMM PR #1397 and #1400). A
/// damaged game `lang` file, half written by a patch or a Steam verify, would otherwise be
/// used as the baseline and could drop tokens the mod needs. Read the game's copy of every
/// localization file the mod ships and stop when one does not parse.
///
/// Loose game files beside `pak01` take precedence, as they do for the game and for vpkmerge.
/// A localization file the game does not have is skipped: the mod adds its own text.
pub fn game_text_problem(base_pak01: &Path, mod_vpk: &Path) -> Result<Option<DamagedGameText>> {
    let directory = read_vpk_directory_with(
        mod_vpk,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;
    let mut entries: Vec<String> = directory
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .filter(|path| is_game_text_path(path))
        .collect();
    entries.sort();
    entries.dedup();
    if entries.is_empty() {
        return Ok(None);
    }
    let loose_root = base_pak01.parent().unwrap_or_else(|| Path::new("."));
    for entry in entries {
        let loose = loose_root.join(&entry);
        let bytes = if loose.is_file() {
            fs::read(&loose)?
        } else {
            let mut wanted = entry_bytes(base_pak01, std::slice::from_ref(&entry))?;
            let Some(bytes) = wanted.remove(&entry.to_ascii_lowercase()) else {
                continue;
            };
            bytes
        };
        if let Err(detail) = strict_text_table(&bytes) {
            return Ok(Some(DamagedGameText { entry, detail }));
        }
    }
    Ok(None)
}

/// Stop a repair when the game's own text is damaged, naming the file and the way back.
fn refuse_damaged_game_text(base_pak01: &Path, mod_vpk: &Path) -> Result<()> {
    let Some(problem) = game_text_problem(base_pak01, mod_vpk)? else {
        return Ok(());
    };
    Err(Error::msg(format!(
        "The game's text file {} is damaged ({}). Steam can replace it: right click Deadlock in Steam, open Properties, Installed Files, and press Verify integrity of game files. Nothing was repaired.",
        problem.entry, problem.detail
    )))
}

/// True for a localization table the repairs merge: `resource/localization/.../*.txt`.
fn is_game_text_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("resource/localization/") && lower.ends_with(".txt")
}

/// Decode and strictly read a Valve-KeyValues `lang { Tokens { ... } }` table. The same shape
/// vpkmerge reads, so a table this function accepts is one the repair can merge from.
fn strict_text_table(bytes: &[u8]) -> std::result::Result<(), String> {
    let text = decode_text(bytes)?;
    let tokens = kv_tokens(&text)?;
    if tokens.len() < 6 {
        return Err("the file is too short to hold a Tokens table".to_string());
    }
    if !tokens[0].eq_ignore_ascii_case("lang") || tokens[1] != "{" {
        return Err("the file does not open a lang block".to_string());
    }
    let mut pos = 2;
    let mut rows: Option<usize> = None;
    while pos + 1 < tokens.len() && tokens[pos] != "}" {
        if tokens[pos].eq_ignore_ascii_case("tokens") {
            if rows.is_some() {
                return Err("the Tokens block appears twice".to_string());
            }
            if tokens[pos + 1] != "{" {
                return Err("the Tokens block does not open".to_string());
            }
            pos += 2;
            let mut keys = BTreeSet::new();
            let mut count = 0;
            while pos < tokens.len() && tokens[pos] != "}" {
                let key = tokens.get(pos).ok_or("a token key is missing")?;
                let value = tokens.get(pos + 1).ok_or("a token value is missing")?;
                if matches!(key.as_str(), "{" | "}") || matches!(value.as_str(), "{" | "}") {
                    return Err("a token is nested or incomplete".to_string());
                }
                let normalized = key.split(':').next().unwrap_or(key).to_ascii_lowercase();
                if !keys.insert(normalized) {
                    return Err(format!("the token {key} appears twice"));
                }
                count += 1;
                pos += 2;
            }
            if pos >= tokens.len() {
                return Err("the Tokens block never closes".to_string());
            }
            rows = Some(count);
            pos += 1;
        } else {
            if tokens.get(pos + 1).is_none_or(|next| next == "{") {
                return Err(format!("the block {} is unknown", tokens[pos]));
            }
            pos += 2;
        }
    }
    if pos + 1 != tokens.len() || tokens[pos] != "}" {
        return Err("the file has unclosed or extra blocks".to_string());
    }
    match rows {
        None => Err("there is no Tokens block".to_string()),
        Some(0) => Err("the Tokens block is empty".to_string()),
        Some(_) => Ok(()),
    }
}

fn decode_text(bytes: &[u8]) -> std::result::Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err("the UTF-16 text ends on half a character".to_string());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect();
        return String::from_utf16(&units).map_err(|_| "the UTF-16 text is damaged".to_string());
    }
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let body = if bom { &bytes[3..] } else { bytes };
    std::str::from_utf8(body)
        .map(str::to_string)
        .map_err(|_| "the text is not valid UTF-8".to_string())
}

/// Split Valve-KeyValues text into quoted values, bare values and braces. Comments are skipped.
fn kv_tokens(text: &str) -> std::result::Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    for c in chars.by_ref() {
                        if c == '\n' {
                            break;
                        }
                    }
                } else {
                    return Err("a stray slash sits outside a value".to_string());
                }
            }
            '{' | '}' => {
                out.push(c.to_string());
                chars.next();
            }
            '"' => {
                chars.next();
                let mut value = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.next() {
                            Some('n') => value.push('\n'),
                            Some('t') => value.push('\t'),
                            Some(other) => value.push(other),
                            None => break,
                        },
                        other => value.push(other),
                    }
                }
                if !closed {
                    return Err("a quoted value never closes".to_string());
                }
                out.push(value);
            }
            _ => {
                // vpkmerge's reader takes only quoted values, so anything else means the
                // game's table is not one the repair could merge from.
                return Err("an unquoted value sits in the table".to_string());
            }
        }
    }
    Ok(out)
}

/// Write the repaired archive to `out_vpk` (a staging path the caller picks), then verify it.
///
/// Returns the findings that were applied. An empty list means there was nothing to repair and no
/// file was written. `out_vpk` must not exist yet, and must not be the mod or the game `pak01`.
/// `animation_overrides` only reaches the data heal; the camera heal compares attachments with
/// the game directly.
///
/// Verification follows Grimoire's guard: the repaired archive has the same entry set as the mod,
/// only the reported entries changed size or CRC, and the game `pak01` kept its size and
/// modification time for the whole run. A failed verification removes the staged file and returns
/// an error so the app never installs a doubtful archive.
pub fn repair_to(
    base_pak01: &Path,
    mod_vpk: &Path,
    out_vpk: &Path,
    kind: RepairKind,
    animation_overrides: bool,
) -> Result<Vec<RepairFinding>> {
    require_archive(base_pak01, "game pak01")?;
    require_archive(mod_vpk, "mod")?;

    if out_vpk.exists() {
        return Err(Error::msg(format!(
            "Repair output {} already exists. Pick a new staging path.",
            out_vpk.display()
        )));
    }

    // A split mod (Sigil's own builds, most packers) is combined into one archive beside the
    // output first; vpkmerge heals that copy. The combined copy is removed on every path.
    let scratch = out_vpk.parent().map(Path::to_path_buf).unwrap_or_default();
    let single = SingleFile::of(mod_vpk, &scratch)?;
    let mod_vpk = single.path();

    let base_stamp = file_stamp(base_pak01)?;

    let (applied, repaired_entries) = match kind {
        RepairKind::Data => {
            refuse_damaged_game_text(base_pak01, mod_vpk)?;
            let mut repairer = CompatibilityRepairer::new(base_pak01).map_err(|error| {
                Error::msg(format!(
                    "Could not open the game pak01 {} for repair: {error:#}",
                    base_pak01.display()
                ))
            })?;
            let reports = repairer
                .heal(mod_vpk, out_vpk, animation_overrides)
                .map_err(|error| {
                    Error::msg(format!("Could not repair {}: {error:#}", mod_vpk.display()))
                })?;
            let applied: Vec<RepairFinding> = reports
                .into_iter()
                .filter(|report| matches!(report.status, RepairStatus::Repairable))
                .map(finding_from_report)
                .collect();
            let entries = entries_of(&applied);
            (applied, entries)
        }
        RepairKind::Cameras => {
            let mut healer = AttachmentHealer::new(base_pak01);
            let healed = healer.heal(mod_vpk, out_vpk).map_err(|error| {
                Error::msg(format!(
                    "Could not repair the models in {}: {error:#}",
                    mod_vpk.display()
                ))
            })?;
            // DMM PR #1399: attachments already in the current frame keep the author's values.
            let healed = keep_current_attachments(base_pak01, mod_vpk, out_vpk, healed)?;
            let applied: Vec<RepairFinding> = healed
                .into_iter()
                .map(|model| RepairFinding {
                    entry: model.entry,
                    kind: "cameras".to_string(),
                    repairable: true,
                    changes: model.attachments.len(),
                    detail: format!(
                        "Updated {} camera inputs across {} mesh blocks.",
                        model.attachments.len(),
                        model.blocks
                    ),
                })
                .collect();
            let entries = entries_of(&applied);
            (applied, entries)
        }
    };

    if let Err(error) = ensure_unchanged(base_pak01, base_stamp) {
        let _ = fs::remove_file(out_vpk);
        return Err(error);
    }

    if applied.is_empty() {
        // vpkmerge writes nothing when there is nothing to repair. Clear a stray file so the
        // caller's staging folder never keeps an archive that was not verified.
        let _ = fs::remove_file(out_vpk);
        return Ok(Vec::new());
    }

    if let Err(error) = verify_output(mod_vpk, out_vpk, &repaired_entries) {
        let _ = fs::remove_file(out_vpk);
        return Err(error);
    }

    Ok(applied)
}

/// The repair rules' revision. A repair made under an older revision is rebuilt from its
/// backup (DMM bumps its repair revision the same way). 2: the camera repair keeps attachments
/// already in the current frame ([`cameras`]).
pub const REPAIR_RULE: u32 = 2;

/// Run [`cameras::keep_current`] over every model vpkmerge healed into `out_vpk`,
/// write the kept values back into `out_vpk`, and drop the kept names from the report.
fn keep_current_attachments(
    base_pak01: &Path,
    mod_vpk: &Path,
    out_vpk: &Path,
    mut healed: Vec<vpkmerge_core::attachment_heal::HealedModel>,
) -> Result<Vec<vpkmerge_core::attachment_heal::HealedModel>> {
    if healed.is_empty() || !out_vpk.is_file() {
        return Ok(healed);
    }
    let entries: Vec<String> = healed.iter().map(|model| model.entry.clone()).collect();
    let original = entry_bytes(mod_vpk, &entries)?;
    let repaired = entry_bytes(out_vpk, &entries)?;
    let game = entry_bytes(base_pak01, &entries)?;
    let mut replaced: Vec<(String, Vec<u8>)> = Vec::new();
    for model in &mut healed {
        let key = model.entry.to_ascii_lowercase();
        let (Some(original), Some(repaired), Some(game)) = (original.get(&key), repaired.get(&key), game.get(&key)) else {
            continue;
        };
        if let Some((bytes, kept)) = cameras::keep_current(original, repaired, game)? {
            model.attachments.retain(|name| !kept.contains(name));
            replaced.push((model.entry.clone(), bytes));
        }
    }
    if replaced.is_empty() {
        return Ok(healed);
    }
    let raw = out_vpk.with_file_name(format!(
        "{}-raw_dir.vpk",
        archive_stem(out_vpk).unwrap_or_else(|| "cameras".to_string())
    ));
    let _ = fs::remove_file(&raw);
    fs::rename(out_vpk, &raw)?;
    let extras: Vec<(&str, &[u8])> = replaced.iter().map(|(entry, bytes)| (entry.as_str(), bytes.as_slice())).collect();
    let written = vpkmerge_core::embed_metadata(&raw, None, &extras, &[], out_vpk);
    let _ = fs::remove_file(&raw);
    written.map_err(|error| Error::msg(format!("Could not write the repaired models: {error:#}")))?;
    Ok(healed)
}

/// The bytes of these entries in `vpk`, keyed by lowercase path. Missing entries are left out.
fn entry_bytes(vpk: &Path, entries: &[String]) -> Result<BTreeMap<String, Vec<u8>>> {
    let wanted: BTreeSet<String> = entries.iter().map(|entry| entry.to_ascii_lowercase()).collect();
    let directory = read_vpk_directory_with(vpk, &ReadVpkOptions { keep_preload: true })?;
    let mut out = BTreeMap::new();
    for entry in &directory.entries {
        let key = entry.path.to_ascii_lowercase();
        if wanted.contains(&key) {
            out.insert(key, crate::vpk::read_vpk_entry_data(&directory, entry)?);
        }
    }
    Ok(out)
}

/// Grimoire's guard: true when any of these other VPKs ships animation resources
/// (`vnmskel`, `vnmclip`, `vnmgraph`, `vanmgrph`, `vanim`, `vagrp`, compiled with `_c`).
///
/// A competing animation provider makes a rig repair uncertain: the mod would be healed against
/// the game while another enabled mod keeps supplying the old clips or skeleton. An archive that
/// cannot be read also returns true, because Sigil cannot rule the override out.
pub fn has_animation_overrides(other_vpks: &[PathBuf]) -> bool {
    other_vpks.iter().any(|path| {
        match read_vpk_directory_with(
            path,
            &ReadVpkOptions {
                keep_preload: false,
            },
        ) {
            Ok(directory) => directory
                .entries
                .iter()
                .any(|entry| is_animation_path(&entry.path)),
            Err(_) => true,
        }
    })
}

fn finding_from_report(report: vpkmerge_core::CompatibilityReport) -> RepairFinding {
    RepairFinding {
        entry: report.entry,
        kind: report.kind,
        repairable: matches!(report.status, RepairStatus::Repairable),
        changes: report.changes,
        detail: report.detail,
    }
}

fn finding_from_attachment(report: AttachmentReport) -> Option<RepairFinding> {
    let detail = match report.status {
        AttachmentStatus::Stale => format!("Stale camera inputs: {}", report.stale.join(", ")),
        AttachmentStatus::NotInGame => "The game ships no model at this path.".to_string(),
        AttachmentStatus::Undecodable => report
            .error
            .clone()
            .unwrap_or_else(|| "The model could not be read.".to_string()),
        // Nothing to repair, so these are not findings.
        AttachmentStatus::Current | AttachmentStatus::Unaffected => return None,
    };
    Some(RepairFinding {
        entry: report.entry,
        kind: "cameras".to_string(),
        repairable: matches!(report.status, AttachmentStatus::Stale),
        changes: report.stale.len(),
        detail,
    })
}

fn entries_of(findings: &[RepairFinding]) -> BTreeSet<String> {
    findings
        .iter()
        .map(|finding| finding.entry.to_ascii_lowercase())
        .collect()
}

fn is_animation_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    ANIMATION_SUFFIXES
        .iter()
        .any(|suffix| lowered.ends_with(suffix))
}

fn require_archive(path: &Path, what: &str) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(Error::msg(format!(
            "The {what} archive {} is not a file.",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(Error::msg(format!(
            "The {what} archive {} is missing.",
            path.display()
        ))),
        Err(error) => Err(error.into()),
    }
}

/// The archive a repair reads: the mod itself when its data is inline, else a single-file copy
/// made with [`crate::vpk_write::combine_chunks`] in `scratch`, removed when this is dropped.
///
/// Repairing one chunk of a split archive would leave the rest stale, so Grimoire refuses split
/// mods. Combining first lets the repair run on every mod, including Sigil's own chunked builds,
/// and the repaired result is one self-contained `_dir.vpk`. Chunk files left beside it are no
/// longer referenced; the game opens only the chunks its directory names.
struct SingleFile {
    path: PathBuf,
    temporary: bool,
}

impl SingleFile {
    fn of(vpk: &Path, scratch: &Path) -> Result<SingleFile> {
        if !crate::vpk_write::is_split(vpk)? {
            return Ok(SingleFile { path: vpk.to_path_buf(), temporary: false });
        }
        fs::create_dir_all(scratch)?;
        let stem = archive_stem(vpk).unwrap_or_else(|| "mod".to_string());
        let path = scratch.join(format!("{stem}-combined-{}_dir.vpk", std::process::id()));
        let _ = fs::remove_file(&path);
        crate::vpk_write::combine_chunks(vpk, &path)?;
        Ok(SingleFile { path, temporary: true })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SingleFile {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn archive_stem(vpk: &Path) -> Option<String> {
    let name = vpk.file_name()?.to_str()?;
    let lowered = name.to_ascii_lowercase();
    let stem = if lowered.ends_with("_dir.vpk") {
        &name[..name.len() - "_dir.vpk".len()]
    } else if lowered.ends_with(".vpk") {
        &name[..name.len() - ".vpk".len()]
    } else {
        return None;
    };
    Some(stem.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: Option<SystemTime>,
}

fn file_stamp(path: &Path) -> Result<FileStamp> {
    let metadata = fs::metadata(path)?;
    Ok(FileStamp {
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn ensure_unchanged(path: &Path, before: FileStamp) -> Result<()> {
    if file_stamp(path)? != before {
        return Err(Error::msg(format!(
            "{} changed during the repair. A game update may be running; try again.",
            path.display()
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryStamp {
    crc32: u32,
    total_length: u32,
}

fn snapshot(vpk: &Path) -> Result<BTreeMap<String, EntryStamp>> {
    let directory = read_vpk_directory_with(
        vpk,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;
    Ok(directory
        .entries
        .into_iter()
        .map(|entry| {
            (
                entry.path.to_ascii_lowercase(),
                EntryStamp {
                    crc32: entry.crc32,
                    total_length: entry.total_length,
                },
            )
        })
        .collect())
}

/// Grimoire's output guard: same entry set as the mod, and only the entries a repair reported may
/// differ in size or CRC. Every other entry must be byte-for-byte the same file.
fn verify_output(original: &Path, output: &Path, repaired: &BTreeSet<String>) -> Result<()> {
    let before = snapshot(original)?;
    let after = snapshot(output)?;

    if before.len() != after.len() || before.keys().any(|path| !after.contains_key(path)) {
        return Err(Error::msg(
            "The repaired archive changed its entry set. The repair was discarded.",
        ));
    }

    for (path, entry) in &before {
        if repaired.contains(path) {
            continue;
        }
        let Some(result) = after.get(path) else {
            return Err(Error::msg(
                "The repaired archive changed its entry set. The repair was discarded.",
            ));
        };
        if result != entry {
            return Err(Error::msg(format!(
                "The repaired archive changed {path}, which no repair reported. The repair was discarded."
            )));
        }
    }

    Ok(())
}

/// What an applied repair did, so the app can retarget its records and offer Revert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedRepair {
    pub findings: Vec<RepairFinding>,
    /// The original mod, kept in app data under its content hash for [`revert_repair`].
    pub backup: PathBuf,
    pub old_size: u64,
    pub new_size: u64,
    /// SHA-256 of the repaired file now in the game folder.
    pub new_sha256: String,
    /// SHA-256 of the mod before the repair (also the backup's name prefix).
    #[serde(default)]
    pub old_sha256: String,
}

/// File that maps repaired mods back to the files they were installed as, in `backup_root`.
pub const REPAIR_ALIASES_FILE: &str = "repair-aliases.json";

/// One repaired file and the file it was before. The update check and anything else keyed by a
/// mod's size or hash looks the original up here, so a repair never makes a mod look unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairAlias {
    pub repaired_bytes: u64,
    pub repaired_sha256: String,
    pub original_bytes: u64,
    pub original_sha256: String,
}

/// Every alias [`apply_repair`] recorded under `backup_root`, oldest first. Empty when none.
pub fn read_repair_aliases(backup_root: &Path) -> Vec<RepairAlias> {
    fs::read(backup_root.join(REPAIR_ALIASES_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Add one alias to the ledger in `backup_root` (app data). A repeat of the same pair is kept once.
pub fn record_repair_alias(backup_root: &Path, alias: RepairAlias) -> Result<()> {
    let mut aliases = read_repair_aliases(backup_root);
    if aliases.contains(&alias) {
        return Ok(());
    }
    aliases.push(alias);
    fs::create_dir_all(backup_root)?;
    crate::util::write_atomic(
        &backup_root.join(REPAIR_ALIASES_FILE),
        &serde_json::to_vec_pretty(&aliases)?,
    )
}

/// The size and hash the mod was installed with, following repairs back (a mod repaired twice
/// resolves to its first file). Unknown files come back unchanged.
pub fn original_of(aliases: &[RepairAlias], bytes: u64, sha256: Option<&str>) -> (u64, Option<String>) {
    let mut current = (bytes, sha256.map(str::to_string));
    for _ in 0..16 {
        let next = aliases.iter().rev().find(|alias| {
            alias.repaired_bytes == current.0
                && current.1.as_deref().is_none_or(|sha| sha.eq_ignore_ascii_case(&alias.repaired_sha256))
        });
        match next {
            Some(alias) if (alias.original_bytes, Some(alias.original_sha256.as_str())) != (current.0, current.1.as_deref()) => {
                current = (alias.original_bytes, Some(alias.original_sha256.clone()));
            }
            _ => break,
        }
    }
    current
}

/// Repair one installed mod in place: the app action behind Repair and Fix all.
///
/// Refuses when `approve` is false or the game is running. Backs the original up under its
/// SHA-256 in `backup_root` and checks the copy, runs each repair family in turn through
/// [`repair_to`] in `staging` (app data, never the game folder), confirms the mod did not change
/// during the run, then swaps the repaired file in through a temporary sibling and a rename.
/// Returns `None` when nothing needed repairing; the backup is still kept then only if it already
/// existed. On any failure the original stays in place. A success also records a
/// [`RepairAlias`] in `backup_root`, so [`crate::mod_updates::baseline_for_repaired`] still finds
/// the install the mod came from.
pub fn apply_repair(
    base_pak01: &Path,
    mod_vpk: &Path,
    kinds: &[RepairKind],
    animation_overrides: bool,
    backup_root: &Path,
    staging: &Path,
    approve: bool,
) -> Result<Option<AppliedRepair>> {
    if !approve {
        return Err(Error::msg("The repair was not approved. Nothing was written."));
    }
    if crate::profiles::game_running() {
        return Err(Error::msg("Close Deadlock before repairing mods. Nothing was written."));
    }
    let original_sha = crate::identify::sha256_file(mod_vpk)?;
    let old_size = fs::metadata(mod_vpk)?.len();
    fs::create_dir_all(staging)?;
    let run = staging.join(format!("repair-{}-{}", std::process::id(), &original_sha[..12]));
    let _ = fs::remove_dir_all(&run);
    fs::create_dir_all(&run)?;
    let outcome = stage_repairs(base_pak01, mod_vpk, kinds, animation_overrides, &run);
    let staged = match outcome {
        Ok(Some(staged)) => staged,
        Ok(None) => {
            let _ = fs::remove_dir_all(&run);
            return Ok(None);
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&run);
            return Err(error);
        }
    };
    let (staged_file, findings) = staged;
    let result = (|| {
        let backup = backup_original(mod_vpk, &original_sha, backup_root)?;
        if crate::identify::sha256_file(mod_vpk)? != original_sha {
            return Err(Error::msg(
                "The mod changed while it was being repaired. Nothing was replaced; try again.",
            ));
        }
        let new_sha256 = crate::identify::sha256_file(&staged_file)?;
        replace_file(&staged_file, mod_vpk)?;
        let new_size = fs::metadata(mod_vpk)?.len();
        // The update check keys mods by size: remember what this file was installed as. A
        // failed write only costs the update badge, never the repair.
        let _ = record_repair_alias(
            backup_root,
            RepairAlias {
                repaired_bytes: new_size,
                repaired_sha256: new_sha256.clone(),
                original_bytes: old_size,
                original_sha256: original_sha.clone(),
            },
        );
        Ok(AppliedRepair {
            findings,
            backup,
            old_size,
            new_size,
            new_sha256,
            old_sha256: original_sha.clone(),
        })
    })();
    let _ = fs::remove_dir_all(&run);
    result.map(Some)
}

/// Put the original mod back from the backup an [`apply_repair`] made. Refuses when `approve` is
/// false, the game is running, or the file in the game folder is no longer the repaired one
/// (`repaired_sha256`), so a mod updated since is never overwritten with an old copy.
pub fn revert_repair(backup: &Path, mod_vpk: &Path, repaired_sha256: &str, approve: bool) -> Result<()> {
    if !approve {
        return Err(Error::msg("Undoing the repair was not approved. Nothing was written."));
    }
    if crate::profiles::game_running() {
        return Err(Error::msg("Close Deadlock before undoing a repair. Nothing was written."));
    }
    if crate::identify::sha256_file(mod_vpk)? != repaired_sha256 {
        return Err(Error::msg(
            "This mod changed since it was repaired, so the old copy was not put back.",
        ));
    }
    replace_file(backup, mod_vpk)
}

/// Run the families one after another, each on the previous output. `None` when no family had
/// anything to repair.
fn stage_repairs(
    base_pak01: &Path,
    mod_vpk: &Path,
    kinds: &[RepairKind],
    animation_overrides: bool,
    run: &Path,
) -> Result<Option<(PathBuf, Vec<RepairFinding>)>> {
    let mut current = mod_vpk.to_path_buf();
    let mut applied = Vec::new();
    for (step, kind) in kinds.iter().enumerate() {
        let out = run.join(format!("step{step}_dir.vpk"));
        let findings = repair_to(base_pak01, &current, &out, *kind, animation_overrides)?;
        if !findings.is_empty() && out.is_file() {
            applied.extend(findings);
            current = out;
        }
    }
    Ok((current != mod_vpk).then_some((current, applied)))
}

fn backup_original(mod_vpk: &Path, sha: &str, backup_root: &Path) -> Result<PathBuf> {
    let name = mod_vpk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let backup = backup_root.join(format!("{sha}-{name}"));
    if !backup.is_file() || crate::identify::sha256_file(&backup)? != sha {
        fs::create_dir_all(backup_root)?;
        let partial = crate::util::temp_path(&backup);
        fs::copy(mod_vpk, &partial)?;
        fs::rename(&partial, &backup)?;
    }
    if crate::identify::sha256_file(&backup)? != sha {
        return Err(Error::msg("The backup copy of the mod did not match it. Nothing was replaced."));
    }
    Ok(backup)
}

/// Copy `from` next to `to`, then rename over it, so the game folder never holds a half file.
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    let partial = crate::util::temp_path(to);
    if let Err(error) = fs::copy(from, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&partial, to) {
        let _ = fs::remove_file(&partial);
        return Err(error.into());
    }
    Ok(())
}

#[path = "compat_cameras.rs"]
pub mod cameras;

#[cfg(test)]
#[path = "compat_repair_tests.rs"]
mod tests;
