//! Plans for three game-folder writes the owner's app actions perform. Everything here reads and
//! plans; the only files it writes are in app-data folders the caller gives. The apply steps are
//! listed on each plan.
//!
//! 1. `gameinfo.gi` repair that keeps edits made outside Sigil (Deadlock Mod Manager keeps
//!    custom mounts and other tools' marked blocks when it rewrites the file). A game update or
//!    a Steam file verify resets `gameinfo.gi`; Sigil's repair already inserts only the lines it
//!    needs and keeps every other byte. [`plan_gameinfo_repair`] also carries over the custom
//!    `SearchPaths` lines (another tool's mount, the player's own) from the last copy Sigil saw
//!    before the reset, in the same place, and records the hash of the file it planned against
//!    so the write can refuse a file that changed since.
//! 2. Configs shipped inside mod packs (DMM 2.4): an archive that carries a `gameinfo.gi`, an
//!    `autoexec.cfg` or other `.cfg`, or a `video.txt` next to its VPKs. The installer ignores
//!    those today. [`find_bundled_configs`] lists them with what each would change, and
//!    [`stage_bundled_configs`] copies them into app data for review; applying one goes through
//!    the existing Performance import or the autoexec editor.
//! 3. The crosshair in the Steam account's `citadel_hero_settings.lst` (DMM): the game reads its
//!    crosshair from that KeyValues3 file as well as from convars, so the autoexec block alone
//!    can be overridden by the in-game editor's saved values. [`plan_crosshair_write`] edits only
//!    the `crosshair_*` keys in `base`, keeps hero overrides, comments and every other setting,
//!    and records each key's original value in a backup kept in app data, so "turn off" puts
//!    back exactly what was there (DMM's `citadel_hero_settings.dmm-crosshair.json`, kept in app
//!    data instead of the game folder).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crosshair::Crosshair;
use crate::error::{Error, Result};

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// 1. gameinfo.gi repair that keeps outside edits
// ---------------------------------------------------------------------------

/// What a repair of `gameinfo.gi` will write.
///
/// Apply (owner): read `gameinfo.gi`; refuse when its sha256 is not `before_sha256`; copy it to
/// `gameinfo.gi.dah-backup-<stamp>`; write `text` atomically. Skip when `changed` is false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameinfoPlan {
    pub before_sha256: String,
    pub text: String,
    pub changed: bool,
    /// Lines Sigil needs that were missing (the addons line, language mounts, Mod and Write).
    pub inserted: Vec<String>,
    /// Custom lines carried over from the copy before the reset.
    pub restored: Vec<String>,
}

fn code_of(line: &str) -> &str {
    let line = line.trim_end_matches(['\r', '\n']);
    line.find("//").map_or(line, |at| &line[..at]).trim()
}

/// `(key, path)` of a SearchPaths line, lowercased with forward slashes.
fn mount_of(line: &str) -> Option<(String, String)> {
    let mut words = code_of(line).split_whitespace();
    let key = words.next()?.to_ascii_lowercase();
    let path = words.next()?.replace('\\', "/").to_ascii_lowercase();
    words.next().is_none().then_some((key, path))
}

/// True for a mount Sigil (or the game itself) owns: the base game, the addons folders, the
/// required language, Mod and Write paths, a profile's folder and the Global folder. Sigil puts
/// those back through its own flows, so a repair never copies them from an old file.
fn managed_mount(key: &str, path: &str) -> bool {
    let path = path.trim_end_matches('/');
    matches!(path, "citadel" | "core" | "citadel/addons")
        || path.starts_with(&format!("citadel/{}", crate::overflow::FOLDER_PREFIX))
        || path.starts_with("citadel/addons/sigil-")
        || path == format!("citadel/{}", crate::locker::GLOBAL_FOLDER)
        || crate::discovery::REQUIRED_SEARCH_PATHS
            .iter()
            .any(|(k, p)| k.eq_ignore_ascii_case(key) && p.eq_ignore_ascii_case(path))
}

/// One custom group in a SearchPaths block: the mount line plus the comment-only lines right
/// around it (another tool's `// BEGIN` / `// END` markers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomMount {
    pub lines: Vec<String>,
    pub key: String,
    pub path: String,
    /// It sat above `Game citadel` (it loads before the game's own files).
    pub above_citadel: bool,
}

/// The custom mounts in a `gameinfo.gi` text, in file order.
pub fn custom_mounts(text: &str) -> Vec<CustomMount> {
    let Some((start, end)) = crate::discovery::search_paths_block(text) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let citadel_at = (start..end).find(|&i| mount_of(lines[i]).is_some_and(|(k, p)| k == "game" && p == "citadel"));
    let comment_only = |i: usize| {
        let trimmed = lines[i].trim();
        trimmed.starts_with("//") && code_of(lines[i]).is_empty()
    };
    let mut out = Vec::new();
    for index in start + 1..end {
        let Some((key, path)) = mount_of(lines[index]) else {
            continue;
        };
        if managed_mount(&key, &path) {
            continue;
        }
        let mut first = index;
        while first > start + 1 && comment_only(first - 1) {
            first -= 1;
        }
        let mut last = index;
        while last + 1 < end && comment_only(last + 1) {
            last += 1;
        }
        out.push(CustomMount {
            lines: (first..=last).map(|i| lines[i].trim().to_string()).collect(),
            key,
            path,
            above_citadel: citadel_at.is_some_and(|at| index < at),
        });
    }
    out
}

/// Plan a repair of `current` (the file now) that also restores the custom mounts `previous`
/// (Sigil's last copy before the reset, for example the newest `gameinfo.gi.dah-backup-*`) had
/// and `current` lacks. Errors when the file has no `Game citadel` line or no readable
/// SearchPaths block, like the repair itself.
pub fn plan_gameinfo_repair(current: &str, previous: Option<&str>) -> Result<GameinfoPlan> {
    let status = crate::discovery::game_info_status_of(current);
    let before_sha256 = sha256_hex(current.as_bytes());
    let needs_repair = !status.addons_search_path || !status.missing_paths.is_empty();
    let mut inserted = Vec::new();
    if !status.addons_search_path {
        inserted.push(crate::deploy::GAMEINFO_ADDONS_LINE.to_string());
    }
    inserted.extend(status.missing_paths.iter().cloned());
    let mut text = if needs_repair {
        crate::deploy::repaired_gameinfo_text(current, &status)
            .map_err(|reason| Error::msg(format!("gameinfo.gi {reason}")))?
            .0
    } else {
        current.to_string()
    };

    let present: Vec<(String, String)> =
        custom_mounts(&text).into_iter().map(|m| (m.key, m.path)).collect();
    let missing: Vec<CustomMount> = previous
        .map(custom_mounts)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| !present.contains(&(m.key.clone(), m.path.clone())))
        .collect();
    let mut restored = Vec::new();
    if !missing.is_empty() {
        let ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let (_, end) = crate::discovery::search_paths_block(&text)
            .ok_or_else(|| Error::msg("gameinfo.gi has no SearchPaths block Sigil can read."))?;
        let citadel_at = lines
            .iter()
            .position(|line| mount_of(line).is_some_and(|(k, p)| k == "game" && p == "citadel"))
            .ok_or_else(|| Error::msg("gameinfo.gi has no `Game citadel` SearchPath line."))?;
        let citadel = &lines[citadel_at];
        let indent = citadel[..citadel.len() - citadel.trim_start().len()].to_string();
        let mut above = Vec::new();
        let mut below = Vec::new();
        for group in &missing {
            restored.extend(group.lines.iter().cloned());
            let target = if group.above_citadel { &mut above } else { &mut below };
            target.extend(group.lines.iter().map(|line| format!("{indent}{line}")));
        }
        // Custom mounts that loaded before the game go just above the Sigil-managed addons line
        // (or `Game citadel`), so Sigil's mods still win over them as before the reset.
        let addons_at = lines.iter().position(|line| {
            mount_of(line).is_some_and(|(k, p)| k == "game" && (p == "citadel/addons" || p.starts_with("citadel/addons/sigil-")))
        });
        let above_at = addons_at.filter(|at| *at < citadel_at).map_or(citadel_at, |at| at + 1);
        let mut output: Vec<String> = Vec::with_capacity(lines.len() + above.len() + below.len());
        for (index, line) in lines.into_iter().enumerate() {
            if index == above_at {
                output.append(&mut above);
            }
            if index == end {
                output.append(&mut below);
            }
            output.push(line);
        }
        let trailing = text.ends_with('\n');
        text = output.join(ending);
        if trailing {
            text.push_str(ending);
        }
    }
    Ok(GameinfoPlan { changed: text != current, before_sha256, text, inserted, restored })
}

// ---------------------------------------------------------------------------
// 2. Configs inside mod packs
// ---------------------------------------------------------------------------

/// What kind of config a pack carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BundledKind {
    /// A `gameinfo.gi`: its ConVars can become a Performance config.
    Gameinfo,
    /// `autoexec.cfg`: lines to merge into the player's autoexec.
    Autoexec,
    /// Another `.cfg` (a config the author's autoexec calls with `exec`).
    Cfg,
    /// `video.txt`: graphics settings, reviewed through Performance.
    Video,
}

/// One config file in an extracted pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledConfig {
    /// Path inside the archive, forward slashes.
    pub path: String,
    pub kind: BundledKind,
    pub bytes: u64,
    /// ConVars a `gameinfo.gi` sets, commands a `.cfg` runs, settings a `video.txt` holds.
    pub settings: usize,
}

/// Files bigger than this are not configs.
pub const MAX_BUNDLED_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

fn bundled_kind(path: &str) -> Option<BundledKind> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    match name.as_str() {
        "gameinfo.gi" => Some(BundledKind::Gameinfo),
        "autoexec.cfg" => Some(BundledKind::Autoexec),
        "video.txt" | "videoconfig.txt" => Some(BundledKind::Video),
        _ if name.ends_with(".cfg") => Some(BundledKind::Cfg),
        _ => None,
    }
}

/// Commands in a `.cfg`: lines that are not empty or comments.
fn cfg_commands(text: &str) -> usize {
    text.lines().filter(|line| !code_of(line).is_empty()).count()
}

/// The configs among an extracted pack's files (`root` plus the relative `files` the extractor
/// reported). Read only.
pub fn find_bundled_configs(root: &Path, files: &[String]) -> Vec<BundledConfig> {
    let mut out = Vec::new();
    for relative in files {
        let relative = relative.replace('\\', "/");
        let Some(kind) = bundled_kind(&relative) else {
            continue;
        };
        let path = relative.split('/').fold(root.to_path_buf(), |path, part| path.join(part));
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_BUNDLED_CONFIG_BYTES {
            continue;
        }
        let text = fs::read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let settings = match kind {
            BundledKind::Gameinfo => crate::performance::exchange::parse(&text).entries.len(),
            BundledKind::Autoexec | BundledKind::Cfg => cfg_commands(&text),
            BundledKind::Video => text.lines().filter(|line| line.trim_start().starts_with('"') && line.matches('"').count() >= 4).count(),
        };
        out.push(BundledConfig { path: relative, kind, bytes: meta.len(), settings });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Copy the configs into `staging` (an app-data folder) at their archive paths, for review after
/// the extraction folder is gone. Returns the staged files.
pub fn stage_bundled_configs(root: &Path, configs: &[BundledConfig], staging: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for config in configs {
        if config.path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
            continue;
        }
        let from = config.path.split('/').fold(root.to_path_buf(), |path, part| path.join(part));
        let to = config.path.split('/').fold(staging.to_path_buf(), |path, part| path.join(part));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&from, &to)?;
        out.push(to);
    }
    Ok(out)
}

/// The lines of a bundled `autoexec.cfg` the player's autoexec does not already run, in order:
/// what "merge into my autoexec" would append (through the approved autoexec write).
pub fn autoexec_additions(current: &str, bundled: &str) -> Vec<String> {
    let have: Vec<String> = current.lines().map(|l| code_of(l).to_ascii_lowercase()).filter(|l| !l.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();
    for line in bundled.lines() {
        let code = code_of(line);
        if code.is_empty() {
            continue;
        }
        let key = code.to_ascii_lowercase();
        if !have.contains(&key) && !out.iter().any(|known| known.eq_ignore_ascii_case(code)) {
            out.push(code.to_string());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 3. Crosshair in citadel_hero_settings.lst
// ---------------------------------------------------------------------------

pub const HERO_SETTINGS_FILE: &str = "citadel_hero_settings.lst";
/// An empty settings file, as the game writes one.
pub const EMPTY_HERO_SETTINGS: &str = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:generic:version{7412167c-06e9-4698-aff2-e63eb59037e7} -->\n{\n\tbase = {}\n\theroes = {}\n}\n";

/// Where the game and Steam keep the file: `game/citadel/cfg/` under the install, and the Steam
/// Cloud copy `<steam>/userdata/<account>/1422450/remote/cfg/` for each account given.
pub fn hero_settings_paths(game_root: &Path, steam_root: Option<&Path>, accounts: &[u32]) -> Vec<PathBuf> {
    let mut out = vec![game_root.join("game").join("citadel").join("cfg").join(HERO_SETTINGS_FILE)];
    if let Some(steam) = steam_root {
        for account in accounts {
            out.push(
                steam
                    .join("userdata")
                    .join(account.to_string())
                    .join(crate::discovery::DEADLOCK_APP_ID)
                    .join("remote")
                    .join("cfg")
                    .join(HERO_SETTINGS_FILE),
            );
        }
    }
    out
}

/// One `name = value` field in the `base` object, by byte ranges.
#[derive(Debug, Clone)]
struct Field {
    name: String,
    start: usize,
    value: std::ops::Range<usize>,
}

/// The parsed `base` object of a hero settings file.
struct BaseBlock {
    fields: Vec<Field>,
    /// Byte offset of `base`'s closing brace.
    close: usize,
    /// True when the object is written `{}` on one line.
    inline_empty: bool,
}

struct Scanner<'a> {
    s: &'a [u8],
    pos: usize,
}

fn invalid() -> Error {
    Error::msg("citadel_hero_settings.lst is not a settings file Sigil can read. Nothing was planned.")
}

impl Scanner<'_> {
    fn skip(&mut self) -> Result<()> {
        loop {
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            let rest = &self.s[self.pos..];
            if rest.starts_with(b"//") {
                self.pos += rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
            } else if rest.starts_with(b"/*") {
                self.pos += rest.windows(2).position(|w| w == b"*/").ok_or_else(invalid)? + 2;
            } else if rest.starts_with(b"<!--") {
                self.pos += rest.windows(3).position(|w| w == b"-->").ok_or_else(invalid)? + 3;
            } else {
                return Ok(());
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        self.skip()?;
        if self.peek() != Some(byte) {
            return Err(invalid());
        }
        self.pos += 1;
        Ok(())
    }

    /// A key: bare word or quoted.
    fn key(&mut self) -> Result<String> {
        self.skip()?;
        let start = self.pos;
        if self.peek() == Some(b'"') {
            self.string()?;
            return Ok(String::from_utf8_lossy(&self.s[start + 1..self.pos - 1]).into_owned());
        }
        while self.pos < self.s.len() && (self.s[self.pos].is_ascii_alphanumeric() || matches!(self.s[self.pos], b'_' | b'.' | b':' | b'-')) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(invalid());
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.pos]).into_owned())
    }

    fn string(&mut self) -> Result<()> {
        self.pos += 1;
        while let Some(b) = self.peek() {
            self.pos += 1;
            match b {
                b'"' => return Ok(()),
                b'\\' => self.pos += 1,
                _ => {}
            }
        }
        Err(invalid())
    }

    /// Skip one value, returning its range.
    fn value(&mut self, depth: usize) -> Result<std::ops::Range<usize>> {
        if depth > 64 {
            return Err(invalid());
        }
        self.skip()?;
        let start = self.pos;
        match self.peek().ok_or_else(invalid)? {
            b'"' => self.string()?,
            b'{' => {
                self.pos += 1;
                loop {
                    self.skip()?;
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                        break;
                    }
                    self.key()?;
                    self.expect(b'=')?;
                    self.value(depth + 1)?;
                    self.skip()?;
                    if self.peek() == Some(b',') {
                        self.pos += 1;
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                loop {
                    self.skip()?;
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                    self.value(depth + 1)?;
                    self.skip()?;
                    if self.peek() == Some(b',') {
                        self.pos += 1;
                    }
                }
            }
            _ => {
                while self.pos < self.s.len()
                    && !self.s[self.pos].is_ascii_whitespace()
                    && !matches!(self.s[self.pos], b'}' | b']' | b',' | b'=')
                {
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(invalid());
                }
            }
        }
        Ok(start..self.pos)
    }
}

fn parse_base(text: &str) -> Result<BaseBlock> {
    let mut sc = Scanner { s: text.as_bytes(), pos: 0 };
    sc.expect(b'{')?;
    loop {
        sc.skip()?;
        match sc.peek() {
            Some(b'}') | None => return Err(invalid()),
            _ => {}
        }
        let key = sc.key()?;
        sc.expect(b'=')?;
        sc.skip()?;
        if key == "base" && sc.peek() == Some(b'{') {
            let open = sc.pos;
            sc.pos += 1;
            let mut fields = Vec::new();
            loop {
                sc.skip()?;
                if sc.peek() == Some(b'}') {
                    let close = sc.pos;
                    let inline_empty = fields.is_empty() && !text[open..close].contains('\n');
                    return Ok(BaseBlock { fields, close, inline_empty });
                }
                let start = sc.pos;
                let name = sc.key()?;
                sc.expect(b'=')?;
                let value = sc.value(1)?;
                fields.push(Field { name, start, value });
                sc.skip()?;
                if sc.peek() == Some(b',') {
                    sc.pos += 1;
                }
            }
        }
        sc.value(1)?;
    }
}

/// The value of `key` in the `base` object (quotes kept), None when it is not set.
pub fn hero_setting(text: &str, key: &str) -> Result<Option<String>> {
    let base = parse_base(text)?;
    Ok(base.fields.iter().find(|f| f.name == key).map(|f| text[f.value.clone()].to_string()))
}

/// Every `name = value` field of the `base` object, in file order, values with their quotes
/// taken off. Hero overrides under `heroes` are not read.
pub fn hero_settings_base(text: &str) -> Result<Vec<(String, String)>> {
    let base = parse_base(text)?;
    Ok(base
        .fields
        .iter()
        .map(|f| {
            let raw = text[f.value.clone()].trim();
            let value = raw.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(raw);
            (f.name.clone(), value.to_string())
        })
        .collect())
}

/// `text` with each key in `changes` set (Some) or removed (None) in `base`. Every other byte,
/// hero overrides and comments included, stays as it was.
pub fn update_hero_settings(text: &str, changes: &BTreeMap<String, Option<String>>) -> Result<String> {
    let base = parse_base(text)?;
    // (start, end, replacement), applied from the end so offsets stay valid.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    for (key, value) in changes {
        match (base.fields.iter().find(|f| &f.name == key), value) {
            (Some(field), Some(value)) => edits.push((field.value.start, field.value.end, value.clone())),
            (Some(field), None) => {
                // Remove the whole line when the field is alone on it.
                let line_start = text[..field.start].rfind('\n').map_or(0, |at| at + 1);
                let line_end = text[field.value.end..].find('\n').map_or(text.len(), |at| field.value.end + at + 1);
                let before = text[line_start..field.start].trim();
                let after = code_of(&text[field.value.end..line_end]);
                if before.is_empty() && after.is_empty() {
                    edits.push((line_start, line_end, String::new()));
                } else {
                    edits.push((field.start, field.value.end, String::new()));
                }
            }
            (None, Some(value)) => added.push(format!("{key} = {value}")),
            (None, None) => {}
        }
    }
    if !added.is_empty() {
        if base.inline_empty {
            let open = text[..base.close].rfind('{').ok_or_else(invalid)?;
            let body: String = added.iter().map(|line| format!("\t\t{line}\n")).collect();
            edits.push((open, base.close + 1, format!("{{\n{body}\t}}")));
        } else {
            let line_start = text[..base.close].rfind('\n').map_or(0, |at| at + 1);
            let insert_at = if text[line_start..base.close].trim().is_empty() { line_start } else { base.close };
            let indent = base
                .fields
                .last()
                .map(|f| {
                    let start = text[..f.start].rfind('\n').map_or(0, |at| at + 1);
                    text[start..f.start].to_string()
                })
                .filter(|s| s.trim().is_empty())
                .unwrap_or_else(|| "\t\t".to_string());
            let body: String = added.iter().map(|line| format!("{indent}{line}\n")).collect();
            let body = if insert_at == base.close { format!("\n{body}") } else { body };
            edits.push((insert_at, insert_at, body));
        }
    }
    edits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut out = text.to_string();
    for (start, end, replacement) in edits {
        out.replace_range(start..end, &replacement);
    }
    parse_base(&out)?;
    Ok(out)
}

/// The `crosshair_*` settings a crosshair writes, values quoted as the game writes them, plus
/// `crosshair_themed = "false"` so the custom look shows (DMM sets the same).
pub fn crosshair_hero_settings(crosshair: &Crosshair) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = crosshair
        .commands()
        .iter()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| !name.ends_with("disable_hero_specific_crosshairs"))
        .filter_map(|(name, value)| Some((name.strip_prefix("citadel_")?.to_string(), format!("\"{}\"", value.trim()))))
        .collect();
    out.insert("crosshair_themed".to_string(), "\"false\"".to_string());
    out
}

/// One key's value before Sigil changed it, and what Sigil wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSetting {
    pub original: Option<String>,
    pub applied: String,
}

/// The restore record, per settings file path. Kept in app data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrosshairBackup {
    pub files: BTreeMap<String, BTreeMap<String, SavedSetting>>,
}

/// Read the backup from `path` (app data); missing is empty.
pub fn read_crosshair_backup(path: &Path) -> Result<CrosshairBackup> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| Error::msg(format!("The crosshair backup is not readable: {e}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(CrosshairBackup::default()),
        Err(error) => Err(error.into()),
    }
}

/// Save the backup to `path` (app data, never the game folder).
pub fn save_crosshair_backup(path: &Path, backup: &CrosshairBackup) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::util::write_atomic(path, &serde_json::to_vec_pretty(backup)?)
}

/// One settings file to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsEdit {
    pub path: PathBuf,
    /// The file as planned against (None: it did not exist).
    pub original: Option<String>,
    pub updated: String,
}

/// What writing (or removing) the crosshair does.
///
/// Apply (owner): 1. `save_crosshair_backup(backup_path, &plan.backup)` (app data) BEFORE any
/// game file changes. 2. For each edit: read the file; refuse the whole apply when it is not
/// `original` (the game or Steam wrote it since); create the parent folder; write `updated`
/// atomically. 3. On a failure part way, write back the `original` of the edits already done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrosshairWritePlan {
    pub edits: Vec<SettingsEdit>,
    pub backup: CrosshairBackup,
}

fn same_value(current: Option<&str>, applied: &str) -> bool {
    let Some(current) = current else {
        return false;
    };
    let (a, b) = (current.trim_matches('"'), applied.trim_matches('"'));
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Plan writing `crosshair` into each settings file (Some), or putting back what was there
/// before Sigil wrote (None: only keys still holding Sigil's value are restored, so a change the
/// player made in game since is kept). A file that does not exist is created from
/// [`EMPTY_HERO_SETTINGS`] when writing and skipped when restoring.
pub fn plan_crosshair_write(
    settings_paths: &[PathBuf],
    backup: &CrosshairBackup,
    crosshair: Option<&Crosshair>,
) -> Result<CrosshairWritePlan> {
    let wanted = crosshair.map(crosshair_hero_settings);
    let mut next = backup.clone();
    let mut edits = Vec::new();
    for path in settings_paths {
        let original = match fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let key = path.to_string_lossy().into_owned();
        let saved = next.files.entry(key.clone()).or_default();
        let mut changes: BTreeMap<String, Option<String>> = BTreeMap::new();
        let base = match (&original, &wanted) {
            (None, None) => {
                next.files.remove(&key);
                continue;
            }
            (Some(text), _) => text.clone(),
            (None, Some(_)) => EMPTY_HERO_SETTINGS.to_string(),
        };
        match &wanted {
            Some(wanted) => {
                for (name, applied) in wanted {
                    let current = hero_setting(&base, name)?;
                    let first = match saved.get(name) {
                        Some(prior) if same_value(current.as_deref(), &prior.applied) => prior.original.clone(),
                        _ => current,
                    };
                    changes.insert(name.clone(), Some(applied.clone()));
                    saved.insert(name.clone(), SavedSetting { original: first, applied: applied.clone() });
                }
            }
            None => {
                for (name, prior) in saved.iter() {
                    if same_value(hero_setting(&base, name)?.as_deref(), &prior.applied) {
                        changes.insert(name.clone(), prior.original.clone());
                    }
                }
                next.files.remove(&key);
            }
        }
        let updated = update_hero_settings(&base, &changes)?;
        if Some(&updated) != original.as_ref() {
            edits.push(SettingsEdit { path: path.clone(), original, updated });
        }
    }
    Ok(CrosshairWritePlan { edits, backup: next })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAMEINFO: &str = "GameInfo\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\t// TOOL BEGIN\n\t\t\tGame\t\tcitadel/tool\n\t\t\t// TOOL END\n\t\t\tGame\t\tcitadel/addons\n\t\t\tGame\t\tcitadel\n\t\t\tGame\t\tcore\n\t\t\tGame\t\tcitadel/late // mine\n\t\t\tMod\t\tcitadel\n\t\t\tWrite\t\tcitadel\n\t\t\tMod\t\tcore\n\t\t\tWrite\t\tcore\n\t\t\tGame_UILanguage\tcitadel_*LANGUAGE*\n\t\t\tGame_LowViolence\tcitadel_lv\n\t\t}\n\t}\n\tConVars { fps_max 240 }\n}\n";

    fn reset() -> String {
        // What a Steam verify leaves: the stock file without the addons line or custom mounts.
        GAMEINFO
            .replace("\t\t\t// TOOL BEGIN\n\t\t\tGame\t\tcitadel/tool\n\t\t\t// TOOL END\n", "")
            .replace("\t\t\tGame\t\tcitadel/addons\n", "")
            .replace("\t\t\tGame\t\tcitadel/late // mine\n", "")
    }

    #[test]
    fn custom_mounts_skip_what_sigil_manages() {
        let mounts = custom_mounts(GAMEINFO);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].path, "citadel/tool");
        assert_eq!(mounts[0].lines, vec!["// TOOL BEGIN", "Game\t\tcitadel/tool", "// TOOL END"]);
        assert!(mounts[0].above_citadel);
        assert_eq!(mounts[1].path, "citadel/late");
        assert!(!mounts[1].above_citadel);
    }

    #[test]
    fn a_repair_after_a_reset_keeps_outside_edits() {
        let current = reset();
        let plan = plan_gameinfo_repair(&current, Some(GAMEINFO)).unwrap();
        assert!(plan.changed);
        assert_eq!(plan.inserted, vec!["Game citadel/addons".to_string()]);
        assert_eq!(plan.restored.len(), 4);
        let tool = plan.text.find("citadel/tool").unwrap();
        let addons = plan.text.find("citadel/addons").unwrap();
        let citadel = plan.text.find("Game\t\tcitadel\n").unwrap();
        let late = plan.text.find("citadel/late").unwrap();
        assert!(addons < tool && tool < citadel, "the tool mount stays above the game and below Sigil's addons");
        assert!(citadel < late);
        assert!(plan.text.contains("ConVars { fps_max 240 }"));
        assert_eq!(plan.before_sha256, sha256_hex(current.as_bytes()));

        let again = plan_gameinfo_repair(&plan.text, Some(GAMEINFO)).unwrap();
        assert!(!again.changed, "a second plan changes nothing");
        let plain = plan_gameinfo_repair(&current, None).unwrap();
        assert!(plain.restored.is_empty());
        assert!(plain.text.contains("citadel/addons"));
    }

    #[test]
    fn finds_and_stages_bundled_configs() {
        let root = std::env::temp_dir().join(format!("sigil-bundled-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("pack/cfg")).unwrap();
        fs::write(root.join("pack/gameinfo.gi"), "ConVars\n{\n\tr_shadows \"0\"\n\tfps_max \"300\"\n}\n").unwrap();
        fs::write(root.join("pack/cfg/autoexec.cfg"), "// hi\nfps_max 300\nexec fast\n").unwrap();
        fs::write(root.join("pack/pak01_dir.vpk"), b"vpk").unwrap();
        let files = vec!["pack/gameinfo.gi".to_string(), "pack/cfg/autoexec.cfg".to_string(), "pack/pak01_dir.vpk".to_string()];
        let found = find_bundled_configs(&root, &files);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, BundledKind::Autoexec);
        assert_eq!(found[0].settings, 2);
        assert_eq!(found[1].kind, BundledKind::Gameinfo);
        assert_eq!(found[1].settings, 2);
        let staged = stage_bundled_configs(&root, &found, &root.join("staging")).unwrap();
        assert!(staged.iter().all(|p| p.is_file()));
        assert_eq!(autoexec_additions("FPS_MAX 300 // set\n", "fps_max 300\nexec fast\nexec fast\n"), vec!["exec fast".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    const SETTINGS: &str = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:generic:version{7412167c-06e9-4698-aff2-e63eb59037e7} -->\n{\n\tbase = \n\t{\n\t\tcrosshair_themed = \"true\"\n\t\tcrosshair_color_r = \"125\" // keep comment\n\t\tmouse_sensitivity = \"1.25\"\n\t}\n\theroes = { hero_haze = { crosshair_color_r = \"77\" note = \"braces { } and \\\"quotes\\\"\" } }\n}\n";

    #[test]
    fn hero_settings_edit_only_base_keys() {
        assert_eq!(hero_setting(SETTINGS, "crosshair_color_r").unwrap().as_deref(), Some("\"125\""));
        assert_eq!(hero_setting(SETTINGS, "missing").unwrap(), None);
        let changes = BTreeMap::from([
            ("crosshair_color_r".to_string(), Some("\"255\"".to_string())),
            ("crosshair_pip_gap".to_string(), Some("\"4\"".to_string())),
            ("crosshair_themed".to_string(), None),
        ]);
        let updated = update_hero_settings(SETTINGS, &changes).unwrap();
        assert_eq!(hero_setting(&updated, "crosshair_color_r").unwrap().as_deref(), Some("\"255\""));
        assert_eq!(hero_setting(&updated, "crosshair_pip_gap").unwrap().as_deref(), Some("\"4\""));
        assert_eq!(hero_setting(&updated, "crosshair_themed").unwrap(), None);
        assert!(updated.contains("// keep comment"));
        assert!(updated.contains("mouse_sensitivity = \"1.25\""));
        assert_eq!(updated.split("heroes = ").nth(1), SETTINGS.split("heroes = ").nth(1));

        let empty = update_hero_settings(EMPTY_HERO_SETTINGS, &BTreeMap::from([("a".to_string(), Some("\"1\"".to_string()))])).unwrap();
        assert_eq!(hero_setting(&empty, "a").unwrap().as_deref(), Some("\"1\""));
        assert!(update_hero_settings("not kv3", &BTreeMap::new()).is_err());
    }

    #[test]
    fn crosshair_write_and_restore_round_trip() {
        let root = std::env::temp_dir().join(format!("sigil-hero-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let game = root.join("Deadlock");
        let paths = hero_settings_paths(&game, Some(&root.join("Steam")), &[42]);
        assert_eq!(paths.len(), 2);
        assert!(paths[1].ends_with("userdata/42/1422450/remote/cfg/citadel_hero_settings.lst"));
        fs::create_dir_all(paths[0].parent().unwrap()).unwrap();
        fs::write(&paths[0], SETTINGS).unwrap();

        let mut crosshair = Crosshair { color_r: 255, ..Crosshair::default() };
        let plan = plan_crosshair_write(&paths, &CrosshairBackup::default(), Some(&crosshair)).unwrap();
        assert_eq!(plan.edits.len(), 2, "the missing Steam copy is created");
        assert_eq!(plan.edits[1].original, None);
        let saved = &plan.backup.files[&paths[0].to_string_lossy().into_owned()];
        assert_eq!(saved["crosshair_color_r"].original.as_deref(), Some("\"125\""));
        assert_eq!(saved["crosshair_pip_gap"].original, None);
        // Apply the way the owner's step does.
        for edit in &plan.edits {
            fs::create_dir_all(edit.path.parent().unwrap()).unwrap();
            fs::write(&edit.path, &edit.updated).unwrap();
        }
        let written = fs::read_to_string(&paths[0]).unwrap();
        assert_eq!(hero_setting(&written, "crosshair_color_r").unwrap().as_deref(), Some("\"255\""));
        assert_eq!(hero_setting(&written, "crosshair_themed").unwrap().as_deref(), Some("\"false\""));

        // A second write keeps the first originals.
        crosshair.color_r = 10;
        let second = plan_crosshair_write(&paths[..1], &plan.backup, Some(&crosshair)).unwrap();
        let saved = &second.backup.files[&paths[0].to_string_lossy().into_owned()];
        assert_eq!(saved["crosshair_color_r"].original.as_deref(), Some("\"125\""));
        for edit in &second.edits {
            fs::write(&edit.path, &edit.updated).unwrap();
        }

        // The player changes one key in game; turning off restores the rest and keeps that one.
        let changed = update_hero_settings(
            &fs::read_to_string(&paths[0]).unwrap(),
            &BTreeMap::from([("crosshair_pip_gap".to_string(), Some("\"9\"".to_string()))]),
        )
        .unwrap();
        fs::write(&paths[0], &changed).unwrap();
        let off = plan_crosshair_write(&paths[..1], &second.backup, None).unwrap();
        let restored = &off.edits[0].updated;
        assert_eq!(hero_setting(restored, "crosshair_color_r").unwrap().as_deref(), Some("\"125\""));
        assert_eq!(hero_setting(restored, "crosshair_themed").unwrap().as_deref(), Some("\"true\""));
        assert_eq!(hero_setting(restored, "crosshair_pip_gap").unwrap().as_deref(), Some("\"9\""));
        assert!(!off.backup.files.contains_key(&paths[0].to_string_lossy().into_owned()));
        assert!(off.backup.files.contains_key(&paths[1].to_string_lossy().into_owned()), "a file not in this plan keeps its record");

        let backup_path = root.join("data").join("crosshair-backup.json");
        save_crosshair_backup(&backup_path, &second.backup).unwrap();
        assert_eq!(read_crosshair_backup(&backup_path).unwrap(), second.backup);
        let _ = fs::remove_dir_all(&root);
    }
}
