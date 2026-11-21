//! Settings files that come inside a mod: a `gameinfo.gi`, `.cfg` files, a `video.txt`.
//!
//! None of them is a VPK mod, and none is ever copied into the game as it ships: a copied
//! `gameinfo.gi` drops the addons line, a copied `video.txt` overwrites the player's graphics
//! settings. Deadlock Mod Manager stages such files for review (GPL-3.0, behaviour only). Sigil
//! reads them out of the unpacked archive and shows what each would change; applying goes
//! through Sigil's own writers with their usual confirm and undo:
//!
//! - a `gameinfo.gi` becomes the Performance page's "Config from a mod" ([`stage`]), applied as a
//!   tagged, reversible merge into the player's own file;
//! - its `video*.txt` (or one alone) rides along, written by Performance's video step;
//! - `.cfg` commands are added to the Autoexec page, where Save writes them.
//!
//! Reading is bounded: eight folders deep, 400 files looked at, 512 KiB a file, 2,000 lines kept
//! a file. Nothing here writes outside the app data directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::autoexec::{self, CfgLineKind, Placement};
use crate::error::{Error, Result};
use crate::performance::{self, catalog, fetch, generate};

/// The Performance catalog id a mod's gameinfo is cached under.
pub const CONFIG_ID: &str = catalog::FROM_MOD_ID;

const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 400;
const MAX_BYTES: u64 = 512 * 1024;
const MAX_LINES: usize = 2_000;

/// What a settings file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Gameinfo,
    Config,
    Video,
}

/// One settings file found in the unpacked archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub kind: Kind,
    /// Path inside the archive, forward slashes.
    pub path: String,
    pub text: String,
}

/// One convar a gameinfo would set: its stock value (None when the stock file has none) and
/// the mod's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub key: String,
    pub from: Option<String>,
    pub to: String,
}

/// A gameinfo in the mod, measured against the player's stock file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gameinfo {
    pub path: String,
    pub changes: Vec<Change>,
    /// Engine section edits (search paths and the like are never among them).
    pub engine: usize,
    /// `video*.txt` beside it, by name.
    pub videos: Vec<String>,
}

/// One command of a `.cfg` and where it would land in the player's autoexec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub line: String,
    pub placement: Placement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub path: String,
    pub commands: Vec<Command>,
}

/// A `video*.txt` and the graphics settings it would change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub path: String,
    /// Settings that differ from the player's video.txt (all of them when there is none).
    pub changes: Vec<Change>,
}

/// Everything a mod's settings files would do, for the review.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Review {
    pub mod_name: String,
    pub mod_id: Option<u64>,
    pub gameinfo: Vec<Gameinfo>,
    pub configs: Vec<Config>,
    pub videos: Vec<Video>,
    /// The unpacked archive the files were read from.
    pub dir: PathBuf,
    /// The player's gameinfo with Sigil's block taken out, the diff's baseline.
    pub stock: String,
}

impl Review {
    pub fn is_empty(&self) -> bool {
        self.gameinfo.is_empty() && self.configs.is_empty() && self.videos.is_empty()
    }

    /// Commands the Autoexec page does not hold yet.
    pub fn new_commands(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for config in &self.configs {
            for command in &config.commands {
                if !matches!(command.placement, Placement::Present(_)) && !out.contains(&command.line) {
                    out.push(command.line.clone());
                }
            }
        }
        out
    }
}

/// What kind of settings file `path` is, by name.
pub fn kind_of(path: &str) -> Option<Kind> {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if name == "gameinfo.gi" || name.ends_with(".gi") {
        Some(Kind::Gameinfo)
    } else if name.starts_with("video") && name.ends_with(".txt") {
        Some(Kind::Video)
    } else if name.ends_with(".cfg") {
        Some(Kind::Config)
    } else {
        None
    }
}

/// Every settings file under `dir`, sorted by kind then path. Symlinks are not followed.
pub fn scan(dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let mut seen = 0usize;
    walk(dir, dir, 0, &mut seen, &mut out);
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.path.cmp(&b.path)));
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, seen: &mut usize, out: &mut Vec<Found>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if *seen >= MAX_FILES {
            return;
        }
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(root, &path, depth + 1, seen, out);
            continue;
        }
        *seen += 1;
        let Ok(relative) = path.strip_prefix(root) else { continue };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.starts_with("__MACOSX/") {
            continue;
        }
        let Some(file_kind) = kind_of(&relative) else { continue };
        if entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > MAX_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let text: String = String::from_utf8_lossy(&bytes)
            .split_inclusive('\n')
            .take(MAX_LINES)
            .collect();
        out.push(Found { kind: file_kind, path: relative, text });
    }
}

fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// Measure what was found against the player's files: `stock` is the install's gameinfo
/// (Sigil's own block is taken out here), `autoexec` the autoexec text, `video` the video.txt
/// the game reads. None when nothing would change anything.
pub fn review(found: &[Found], stock: &str, autoexec_text: &str, video: Option<&str>) -> Review {
    let stock = performance::remove_text(stock);
    let baseline = generate::parse(&stock);
    let stock_values = performance::convar_values(&stock);
    let current_lines: Vec<&str> = autoexec::list_lines(autoexec_text);
    let current_video: BTreeMap<String, String> =
        video.map(generate::parse_video).unwrap_or_default().into_iter().collect();
    let mut out = Review { stock: stock.clone(), ..Review::default() };
    for file in found {
        match file.kind {
            Kind::Gameinfo => {
                let body = generate::generate(&baseline, &generate::parse(&file.text));
                let changes: Vec<Change> = body
                    .convars
                    .iter()
                    .map(|(key, to)| Change { key: key.clone(), from: stock_values.get(key).cloned(), to: to.clone() })
                    .collect();
                if changes.is_empty() && body.section_ops.is_empty() {
                    continue;
                }
                let folder = folder_of(&file.path);
                let videos = found
                    .iter()
                    .filter(|f| f.kind == Kind::Video && folder_of(&f.path) == folder)
                    .map(|f| f.path.rsplit('/').next().unwrap_or(&f.path).to_string())
                    .collect();
                out.gameinfo.push(Gameinfo { path: file.path.clone(), changes, engine: body.section_ops.len(), videos });
            }
            Kind::Config => {
                let commands: Vec<Command> = autoexec::parse_cfg(&file.text)
                    .lines
                    .into_iter()
                    .map(|line| line.text.trim().to_string())
                    .filter(|line| autoexec::classify_line(line) == CfgLineKind::Command)
                    .map(|line| Command { placement: autoexec::place_command(&current_lines, &line), line })
                    .collect();
                if !commands.is_empty() {
                    out.configs.push(Config { path: file.path.clone(), commands });
                }
            }
            Kind::Video => {
                let changes: Vec<Change> = generate::parse_video(&file.text)
                    .into_iter()
                    .filter(|(key, to)| current_video.get(key) != Some(to))
                    .map(|(key, to)| Change { from: current_video.get(&key).cloned(), key, to })
                    .collect();
                if !changes.is_empty() {
                    out.videos.push(Video { path: file.path.clone(), changes });
                }
            }
        }
    }
    out
}

/// Read and measure the settings files of an unpacked archive against the game at
/// `game_root`. Disk only; run off the UI thread.
pub fn review_dir(dir: &Path, game_root: &Path, extra_steam_roots: &[PathBuf]) -> Review {
    let found = scan(dir);
    if found.is_empty() {
        return Review { dir: dir.to_path_buf(), ..Review::default() };
    }
    let stock = std::fs::read_to_string(performance::gameinfo_path(game_root)).unwrap_or_default();
    let autoexec_text = autoexec::read_autoexec(game_root).ok().flatten().unwrap_or_default();
    let video = performance::video::locate(game_root, extra_steam_roots).and_then(|p| std::fs::read_to_string(p).ok());
    let mut out = review(&found, &stock, &autoexec_text, video.as_deref());
    out.dir = dir.to_path_buf();
    out
}

/// Cache the reviewed gameinfo (or, for a mod that ships only a video.txt, the stock file with
/// it) as the Performance page's "Config from a mod". Applying is still Performance's Apply
/// with its confirm. App data only; run off the UI thread.
pub fn stage(review: &Review, gameinfo: Option<usize>, data_dir: &Path) -> Result<()> {
    let (text, folder, path) = match gameinfo.and_then(|i| review.gameinfo.get(i)) {
        Some(chosen) => {
            let text = std::fs::read_to_string(review.dir.join(&chosen.path))?;
            (text, folder_of(&chosen.path).to_string(), chosen.path.clone())
        }
        None => {
            let first = review
                .videos
                .first()
                .ok_or_else(|| Error::msg("This mod ships no gameinfo.gi or video.txt with settings."))?;
            (review.stock.clone(), folder_of(&first.path).to_string(), "gameinfo.gi".to_string())
        }
    };
    let mut videos = Vec::new();
    for found in scan(&review.dir).into_iter().filter(|f| f.kind == Kind::Video && folder_of(&f.path) == folder) {
        let name = found.path.rsplit('/').next().unwrap_or(&found.path).to_string();
        videos.push((name, found.text));
    }
    let name = if review.mod_name.trim().is_empty() { "A mod".to_string() } else { review.mod_name.trim().to_string() };
    let meta = fetch::CachedConfig {
        id: CONFIG_ID.to_string(),
        version: name,
        commit: review.mod_id.map_or_else(|| "mod".to_string(), |id| format!("mod{id}")),
        date: crate::util::now_iso().chars().take(10).collect(),
        path,
        blob: fetch::git_blob_sha(text.as_bytes()),
        videos: videos.iter().map(|(name, text)| (name.clone(), fetch::git_blob_sha(text.as_bytes()))).collect(),
    };
    fetch::store_hosted(data_dir, &meta, &text, &videos, Some(&review.stock))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "\"GameInfo\"\n{\n\tFileSystem\n\t{\n\t\tSearchPaths\n\t\t{\n\t\t\tGame citadel/addons\n\t\t\tGame citadel\n\t\t}\n\t}\n\tConVars\n\t{\n\t\t\"r_farz\" \"10000\"\n\t\t\"r_shadows\" \"1\"\n\t}\n}\n";

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sigil-mod-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn files_are_told_apart_by_name() {
        assert_eq!(kind_of("Potato/gameinfo.gi"), Some(Kind::Gameinfo));
        assert_eq!(kind_of("cfg/autoexec.cfg"), Some(Kind::Config));
        assert_eq!(kind_of("video(No Shadows).txt"), Some(Kind::Video));
        assert_eq!(kind_of("readme.txt"), None);
        assert_eq!(kind_of("pak01_dir.vpk"), None);
    }

    #[test]
    fn a_review_says_what_each_file_would_change() {
        let mod_gameinfo = STOCK.replace("\"r_shadows\" \"1\"", "\"r_shadows\" \"0\"\n\t\t\"r_fog\" \"0\"");
        let found = vec![
            Found { kind: Kind::Gameinfo, path: "fps/gameinfo.gi".into(), text: mod_gameinfo },
            Found { kind: Kind::Config, path: "autoexec.cfg".into(), text: "// tuned\nfps_max 240\nr_fullscreen_gamma 2.2\n\n".into() },
            Found { kind: Kind::Video, path: "fps/video.txt".into(), text: "\"VideoConfig\"\n{\n\t\"setting.r_texture_quality\"\t\"0\"\n\t\"setting.mat_vsync\"\t\"0\"\n}\n".into() },
        ];
        let video = "\"VideoConfig\"\n{\n\t\"setting.mat_vsync\"\t\"0\"\n}\n";
        let review = review(&found, STOCK, "fps_max 144\n", Some(video));
        assert_eq!(review.gameinfo.len(), 1);
        let shadows = review.gameinfo[0].changes.iter().find(|c| c.key == "r_shadows").unwrap();
        assert_eq!((shadows.from.as_deref(), shadows.to.as_str()), (Some("1"), "0"));
        let fog = review.gameinfo[0].changes.iter().find(|c| c.key == "r_fog").unwrap();
        assert_eq!(fog.from, None);
        assert_eq!(review.gameinfo[0].videos, vec!["video.txt".to_string()]);
        let commands = &review.configs[0].commands;
        assert_eq!(commands.len(), 2, "the comment is not a command");
        assert_eq!(commands[0].placement, Placement::Replace(0), "fps_max replaces the player's own");
        assert_eq!(commands[1].placement, Placement::Append);
        assert_eq!(review.videos[0].changes.len(), 1, "a setting the player already has is no change");
        assert_eq!(review.new_commands(), vec!["fps_max 240".to_string(), "r_fullscreen_gamma 2.2".to_string()]);
        assert!(!review.is_empty());
    }

    #[test]
    fn a_gameinfo_equal_to_stock_changes_nothing() {
        let found = vec![Found { kind: Kind::Gameinfo, path: "gameinfo.gi".into(), text: STOCK.into() }];
        assert!(review(&found, STOCK, "", None).is_empty());
    }

    #[test]
    fn scan_reads_settings_files_only_and_stage_caches_the_config() {
        let root = scratch("scan");
        std::fs::create_dir_all(root.join("fps")).unwrap();
        std::fs::create_dir_all(root.join("__MACOSX/fps")).unwrap();
        std::fs::write(root.join("fps/gameinfo.gi"), STOCK.replace("\"r_shadows\" \"1\"", "\"r_shadows\" \"0\"")).unwrap();
        std::fs::write(root.join("fps/video.txt"), "\"VideoConfig\"\n{\n\t\"setting.mat_vsync\"\t\"1\"\n}\n").unwrap();
        std::fs::write(root.join("__MACOSX/fps/gameinfo.gi"), "junk").unwrap();
        std::fs::write(root.join("readme.txt"), "hi").unwrap();
        let found = scan(&root);
        let paths: Vec<&str> = found.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["fps/gameinfo.gi", "fps/video.txt"]);

        let mut review = review(&found, STOCK, "", None);
        review.dir = root.clone();
        review.mod_name = "Potato FPS".into();
        review.mod_id = Some(42);
        let data = scratch("data");
        stage(&review, Some(0), &data).unwrap();
        let index = fetch::read_index(&data);
        let meta = index.configs.get(CONFIG_ID).expect("cached");
        assert_eq!(meta.version, "Potato FPS");
        assert_eq!(meta.commit, "mod42");
        assert_eq!(meta.videos.len(), 1);
        let catalog = performance::load(&data);
        let config = catalog.get(CONFIG_ID).expect("listed on the Performance page");
        assert!(config.body.convars.iter().any(|(k, v)| k == "r_shadows" && v == "0"));
        assert_eq!(config.link(), "https://gamebanana.com/mods/42");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&data);
    }
}
