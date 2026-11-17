//! The video.txt half of a config: the in-game graphics settings the author
//! tuned the config for.
//!
//! Deadlock keeps video.txt in Steam userdata
//! (`userdata/<account>/1422450/local/cfg/video.txt`) and falls back to
//! `game/citadel/cfg/video.txt` only while that copy does not exist. The game
//! rewrites the file whenever a graphics setting changes in game and drops
//! comments, so markers would not survive. What Sigil wrote is recorded in a
//! sidecar under the app data directory instead: per key, the value written and
//! the value it replaced. Turning it off restores only keys that still hold
//! Sigil's value; one changed in game since is the player's.
//!
//! Only `setting.*` lines are edited. The header (Version, VendorID, DeviceID)
//! and display settings are never touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util::write_atomic;

const RECORD: &str = "video-applied.json";
const BACKUP_SUFFIX: &str = "sigil-bak";

/// What Sigil wrote to a video.txt.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRecord {
    pub config: String,
    pub file: String,
    pub path: PathBuf,
    /// Value each key had before; `None` when the key was absent.
    pub original: BTreeMap<String, Option<String>>,
    /// Value Sigil wrote, per key.
    pub written: BTreeMap<String, String>,
}

fn setting_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"^([ \t]*)"setting\.([A-Za-z_]\w*)"([ \t]+)"([^"\r\n]*)""#)
            .expect("video regex")
    })
}

/// The game writes `3` back as `3.000000`; that is not the player changing it.
fn same_value(current: Option<&str>, value: &str) -> bool {
    match current {
        Some(current) if current == value => true,
        Some(current) => match (current.trim().parse::<f64>(), value.trim().parse::<f64>()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        },
        None => false,
    }
}

fn split(text: &str) -> (Vec<String>, bool) {
    let crlf = text.contains("\r\n");
    let lf = if crlf {
        text.replace("\r\n", "\n")
    } else {
        text.to_string()
    };
    (lf.split('\n').map(str::to_string).collect(), crlf)
}

fn join(lines: &[String], crlf: bool) -> String {
    let text = lines.join("\n");
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

fn current_values(lines: &[String]) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in lines {
        if let Some(caps) = setting_re().captures(line) {
            values.insert(caps[2].to_string(), caps[4].to_string());
        }
    }
    values
}

/// Set every occurrence of `key`, or delete them for `None`. Whether it had one.
fn set_value(lines: &mut Vec<String>, key: &str, value: Option<&str>) -> bool {
    let mut found = false;
    let mut index = lines.len();
    while index > 0 {
        index -= 1;
        let Some(caps) = setting_re().captures(&lines[index]) else {
            continue;
        };
        if &caps[2] != key {
            continue;
        }
        found = true;
        match value {
            None => {
                lines.remove(index);
            }
            Some(value) => {
                let rest = lines[index][caps.get(0).unwrap().end()..].to_string();
                lines[index] =
                    format!("{}\"setting.{key}\"{}\"{value}\"{rest}", &caps[1], &caps[3]);
            }
        }
    }
    found
}

/// Put back what `record` replaced, for keys that still hold Sigil's value.
pub fn revert_text(text: &str, record: &VideoRecord) -> String {
    let (mut lines, crlf) = split(text);
    let current = current_values(&lines);
    for (key, written) in &record.written {
        if !same_value(current.get(key).map(String::as_str), written) {
            continue;
        }
        let original = record.original.get(key).cloned().flatten();
        set_value(&mut lines, key, original.as_deref());
    }
    join(&lines, crlf)
}

/// Write `settings` into video.txt text. A previous Sigil write is reverted
/// first. Missing keys go in before the closing brace, formatted like the
/// file's own entries.
pub fn apply_text(
    text: &str,
    settings: &[(String, String)],
    previous: Option<&VideoRecord>,
) -> Result<(
    String,
    BTreeMap<String, Option<String>>,
    BTreeMap<String, String>,
)> {
    let base = match previous {
        Some(previous) => revert_text(text, previous),
        None => text.to_string(),
    };
    let (mut lines, crlf) = split(&base);
    let close = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .filter(|at| lines[*at].trim() == "}");
    let Some(close) = close else {
        return Err(Error::msg(
            "video.txt is not in the format Deadlock writes.",
        ));
    };
    if !lines.iter().any(|line| line.contains('{')) {
        return Err(Error::msg(
            "video.txt is not in the format Deadlock writes.",
        ));
    }
    let current = current_values(&lines);
    let sample = lines.iter().find_map(|line| setting_re().captures(line));
    let indent = sample
        .as_ref()
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| "\t".into());
    let gap = sample
        .as_ref()
        .map(|c| c[3].to_string())
        .unwrap_or_else(|| "\t\t".into());

    let mut original = BTreeMap::new();
    let mut written = BTreeMap::new();
    let mut missing = Vec::new();
    for (key, value) in settings {
        if same_value(current.get(key).map(String::as_str), value) {
            continue;
        }
        let before = match previous {
            Some(previous) if previous.written.contains_key(key) => {
                previous.original.get(key).cloned().flatten()
            }
            _ => current.get(key).cloned(),
        };
        original.insert(key.clone(), before);
        written.insert(key.clone(), value.clone());
        if !set_value(&mut lines, key, Some(value)) {
            missing.push(format!("{indent}\"setting.{key}\"{gap}\"{value}\""));
        }
    }
    for (offset, line) in missing.into_iter().enumerate() {
        lines.insert(close + offset, line);
    }
    Ok((join(&lines, crlf), original, written))
}

/// How many values Sigil wrote the file still holds.
pub fn still_written(text: &str, record: &VideoRecord) -> usize {
    let (lines, _) = split(text);
    let current = current_values(&lines);
    record
        .written
        .iter()
        .filter(|(key, value)| same_value(current.get(*key).map(String::as_str), value))
        .count()
}

/// The video.txt Deadlock reads for this install: the userdata copy of the
/// Steam account that launched the game last, else the game folder copy.
/// `None` until the game has written one.
pub fn locate(game_root: &Path, extra_steam_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    // <steam>/steamapps/common/Deadlock
    if let Some(steam) = game_root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
    {
        roots.push(steam.to_path_buf());
    }
    roots.extend(extra_steam_roots.iter().cloned());
    let mut best: Option<(PathBuf, std::time::SystemTime)> = None;
    for root in roots {
        let Ok(accounts) = std::fs::read_dir(root.join("userdata")) else {
            continue;
        };
        for account in accounts.flatten() {
            let name = account.file_name().to_string_lossy().into_owned();
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let cfg = account
                .path()
                .join(crate::discovery::DEADLOCK_APP_ID)
                .join("local")
                .join("cfg");
            let stamp = ["machine_convars.vcfg", "video.txt"]
                .iter()
                .map(|file| cfg.join(file))
                .find(|path| path.exists());
            let Some(stamp) = stamp else {
                continue;
            };
            let Ok(modified) = std::fs::metadata(&stamp).and_then(|meta| meta.modified()) else {
                continue;
            };
            if best.as_ref().is_none_or(|(_, at)| modified > *at) {
                best = Some((cfg, modified));
            }
        }
    }
    if let Some((cfg, _)) = best {
        let path = cfg.join("video.txt");
        return path.exists().then_some(path);
    }
    let local = game_root
        .join("game")
        .join("citadel")
        .join("cfg")
        .join("video.txt");
    local.exists().then_some(local)
}

fn record_path(data_dir: &Path) -> PathBuf {
    super::fetch::cache_root(data_dir).join(RECORD)
}

pub fn read_record(data_dir: &Path) -> Option<VideoRecord> {
    std::fs::read_to_string(record_path(data_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// GAME WRITE. Write a config's video settings into the player's video.txt.
/// Called only from the explicit "Write video settings" button. Keeps a
/// one-time backup next to the file, writes atomically and records what it
/// replaced so `revert_install` can put it back. Returns how many values changed.
pub fn apply_install(
    data_dir: &Path,
    path: &Path,
    config: &str,
    file: &str,
    settings: &[(String, String)],
) -> Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let previous = read_record(data_dir).filter(|record| record.path == path);
    let (next, original, written) = apply_text(&text, settings, previous.as_ref())?;
    let backup = backup_path(path);
    if !backup.exists() {
        write_atomic(&backup, text.as_bytes())?;
    }
    let record = VideoRecord {
        config: config.to_string(),
        file: file.to_string(),
        path: path.to_path_buf(),
        original,
        written,
    };
    // Record first: a crash after it leaves a record of a write that did not
    // happen, which reverts nothing. The other order could lose the originals.
    write_atomic(
        &record_path(data_dir),
        serde_json::to_string_pretty(&record)?.as_bytes(),
    )?;
    write_atomic(path, next.as_bytes())?;
    Ok(record.written.len())
}

/// Let the values Sigil wrote become the player's: drop the record without
/// touching video.txt, so Restore and Turn off no longer revert them. Runs from
/// the "Keep these values" confirm.
pub fn keep_install(data_dir: &Path) -> Result<bool> {
    let path = record_path(data_dir);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

/// GAME WRITE. Put back what Sigil wrote to video.txt. Called from Turn off
/// (after its confirm) and the "Restore video settings" button.
pub fn revert_install(data_dir: &Path) -> Result<bool> {
    let Some(record) = read_record(data_dir) else {
        return Ok(false);
    };
    if let Ok(text) = std::fs::read_to_string(&record.path) {
        let next = revert_text(&text, &record);
        if next != text {
            write_atomic(&record.path, next.as_bytes())?;
        }
    }
    std::fs::remove_file(record_path(data_dir))?;
    Ok(true)
}

pub fn backup_path(path: &Path) -> PathBuf {
    path.with_extension(format!("txt.{BACKUP_SUFFIX}"))
}

/// A plain name for a video setting the in-game menu shows, if it has one.
pub fn label(key: &str) -> Option<&'static str> {
    Some(match key {
        "cpu_level" => "Effects detail",
        "gpu_level" => "Shader detail",
        "gpu_mem_level" => "Texture memory",
        "mem_level" => "Model detail",
        "mat_viewportscale" => "Render scale",
        "r_citadel_antialiasing" => "Anti-aliasing",
        "r_citadel_upscaling" => "Upscaling",
        "r_citadel_dlss_settings_mode" => "DLSS mode",
        "r_dlss_preset" => "DLSS model",
        "r_citadel_fsr_rcas_sharpness" => "FSR sharpness",
        "r_citadel_fsr2_sharpness" => "FSR 2 sharpness",
        "r_effects_bloom" => "Effects bloom",
        "r_post_bloom" => "Bloom",
        "r_depth_of_field" => "Depth of field",
        "r_citadel_motion_blur" => "Motion blur",
        "r_reduce_flash" => "Reduce flashes",
        "r_low_latency" => "Low latency (Reflex)",
        "r_citadel_ssao_quality" => "Ambient occlusion",
        "r_citadel_shadow_quality" => "Shadow quality",
        "r_citadel_fog_quality" => "Fog quality",
        "r_texture_stream_mip_bias" => "Texture quality",
        "r_arealights" => "Area lights",
        "r_light_sensitivity_mode" => "Light sensitivity mode",
        "r_particle_max_detail_level" => "Particle detail",
        "r_texturefilteringquality" => "Texture filtering",
        "r_ssao" => "Ambient occlusion",
        _ => return None,
    })
}
