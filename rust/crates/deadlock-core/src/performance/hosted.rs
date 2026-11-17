//! Configs hosted on GameBanana: fetch the page, download the author's archive,
//! find the gameinfo file the player wants and cache it.
//!
//! Sigil never bundles these configs and never redistributes them. A download
//! runs only from the button on the hosted page, the archive is extracted under
//! the app data directory with the same limits as a mod install, and the player
//! reviews which variant and how many settings before anything is cached or
//! applied. The GameBanana page supplies the author, the file list and the
//! license to show.
//!
//! Once cached, the config is an ordinary catalog entry: `load` classifies it,
//! the tier list shows it, and applying goes through the same `gameinfo.gi`
//! writer with its backup and removal.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::gamebanana::{self, ModFile, ModProfile};
use crate::install::{
    self, DownloadPolicy, ExtractLimits, ToolPaths, detect_archive_format, extract_archive,
};

use super::fetch::{self, CachedConfig};
use super::{Config, GameConvars, catalog, generate, remove_text};

/// One hosted config page and the files it offers.
#[derive(Debug, Clone)]
pub struct Page {
    pub name: String,
    pub author: String,
    pub license: Option<String>,
    pub url: String,
    pub version: Option<String>,
    pub text: String,
    pub files: Vec<ModFile>,
}

impl Page {
    /// The newest file, or the one the catalog pins.
    pub fn chosen_file(&self, pinned: Option<u64>) -> Option<&ModFile> {
        if let Some(pinned) = pinned
            && let Some(file) = self.files.iter().find(|file| file.id == pinned)
        {
            return Some(file);
        }
        self.files
            .iter()
            .max_by_key(|file| (file.added_ts, file.id))
    }
}

/// Read a GameBanana config's page. Network; run off the UI thread.
pub fn fetch_page(id: &str) -> Result<Page> {
    let def = catalog::config(id)
        .ok_or_else(|| Error::msg(format!("No config named {id}.")))?;
    let hosted = def
        .gamebanana
        .ok_or_else(|| Error::msg(format!("{} is not hosted on GameBanana.", def.name)))?;
    let profile: ModProfile = gamebanana::fetch_mod_profile(hosted.mod_id)?;
    let mut files = profile.files;
    files.retain(|file| !file.download_url.is_empty());
    files.sort_by_key(|file| (file.added_ts, file.id));
    files.reverse();
    Ok(Page {
        name: profile.name,
        author: profile
            .submitter
            .map(|member| member.name)
            .unwrap_or_else(|| def.author.to_string()),
        license: profile.license,
        url: format!("https://gamebanana.com/mods/{}", hosted.mod_id),
        version: profile.version,
        text: profile.text,
        files,
    })
}

/// One gameinfo file found in the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// Path inside the archive, forward slashes.
    pub path: String,
    /// The folder name when the file is `gameinfo.gi`, else the file name.
    pub label: String,
    /// ConVars that will be written.
    pub convars: usize,
    /// Engine section edits the config carries.
    pub engine: usize,
    pub ignored: usize,
    /// video.txt files next to it.
    pub videos: Vec<String>,
    /// The candidate the catalog's hint points at.
    pub preferred: bool,
}

/// A downloaded archive, read and ready for review.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub id: String,
    pub file_id: u64,
    pub file_name: String,
    pub version: String,
    pub date: String,
    pub variants: Vec<Variant>,
    pub chosen: usize,
    /// The install's stock gameinfo, for the baseline when GitHub was not fetched yet.
    pub stock: String,
    dir: PathBuf,
}

impl Prepared {
    pub fn chosen(&self) -> Option<&Variant> {
        self.variants.get(self.chosen)
    }
}

/// Download `file`, extract it and list the gameinfo variants inside.
/// `stock` is the install's own `gameinfo.gi` with Sigil's block removed.
/// Network and disk; run off the UI thread.
pub fn prepare(
    id: &str,
    file: &ModFile,
    data_dir: &Path,
    stock: &str,
) -> Result<Prepared> {
    let def = catalog::config(id)
        .ok_or_else(|| Error::msg(format!("No config named {id}.")))?;
    let hosted = def
        .gamebanana
        .ok_or_else(|| Error::msg(format!("{} is not hosted on GameBanana.", def.name)))?;

    let root = fetch::cache_root(data_dir).join("hosted").join(def.id);
    let _ = std::fs::remove_dir_all(&root);
    let download_dir = root.join("download");
    let files_dir = root.join("files");
    let report = install::download(
        &file.download_url,
        &download_dir,
        &DownloadPolicy::gamebanana(),
    )?;
    let format = detect_archive_format(&report.path);
    if format == install::ArchiveFormat::Unknown {
        return Err(Error::msg(format!(
            "{} is not an archive Sigil can read. Open the GameBanana page and download the gameinfo file by hand.",
            file.name
        )));
    }
    extract_archive(
        &report.path,
        &files_dir,
        &ExtractLimits::default(),
        &ToolPaths::from_environment(),
    )?;

    let game = std::fs::read_to_string(fetch::file_path(
        data_dir,
        fetch::CONVARS_ID,
        fetch::CONVARS_FILE,
    ))
    .ok()
    .map(|text| GameConvars::parse(&text))
    .filter(|game| game.len() > 1000);
    let baseline = generate::parse(&remove_text(stock));
    let version = if file.name.is_empty() {
        "GameBanana".to_string()
    } else {
        file.name.clone()
    };
    let date = gamebanana::calendar_date(file.added_ts);
    let hint = hosted.hint.to_ascii_lowercase().replace('\\', "/");

    let mut variants = Vec::new();
    for path in gameinfo_files(&files_dir) {
        let Ok(text) = std::fs::read_to_string(files_dir.join(&path)) else {
            continue;
        };
        let relative = path.replace('\\', "/");
        let videos = video_files(&files_dir, &path);
        let meta = CachedConfig {
            id: def.id.to_string(),
            version: version.clone(),
            commit: format!("gb{}", file.id),
            date: date.clone(),
            path: relative.clone(),
            blob: fetch::git_blob_sha(text.as_bytes()),
            videos: videos
                .iter()
                .map(|(name, _)| (name.clone(), String::new()))
                .collect(),
        };
        let config = Config::from_texts(
            def,
            &meta,
            &baseline,
            &text,
            &videos,
            game.as_ref(),
        );
        if config.body.convars.is_empty() && config.body.section_ops.is_empty() {
            continue;
        }
        let label = relative
            .rsplit_once('/')
            .map(|(folder, _)| folder.rsplit('/').next().unwrap_or(folder))
            .filter(|folder| !folder.is_empty())
            .unwrap_or("gameinfo.gi")
            .to_string();
        let preferred = relative.to_ascii_lowercase().ends_with(&hint);
        variants.push(Variant {
            path: relative,
            label,
            convars: config.body.convars.len(),
            engine: config.body.section_ops.len(),
            ignored: config.ignored.len(),
            videos: videos.into_iter().map(|(name, _)| name).collect(),
            preferred,
        });
    }
    if variants.is_empty() {
        return Err(Error::msg(format!(
            "No gameinfo file with settings was found in {}. Another file on the mod page may hold the config.",
            file.name
        )));
    }
    variants.sort_by(|a, b| b.preferred.cmp(&a.preferred).then(a.path.cmp(&b.path)));
    let chosen = variants.iter().position(|variant| variant.preferred).unwrap_or(0);
    Ok(Prepared {
        id: def.id.to_string(),
        file_id: file.id,
        file_name: file.name.clone(),
        version,
        date,
        variants,
        chosen,
        stock: stock.to_string(),
        dir: files_dir,
    })
}

/// Cache the reviewed variant under the config's id. The next `load` shows it
/// in its tier; applying is still the player's separate Apply step.
/// Disk only; run off the UI thread.
pub fn install(prepared: &Prepared, variant: usize, data_dir: &Path) -> Result<()> {
    let chosen = prepared
        .variants
        .get(variant)
        .ok_or_else(|| Error::msg("That file is no longer in the archive."))?;
    let path = prepared.dir.join(&chosen.path);
    let text = std::fs::read_to_string(&path)?;
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut videos = Vec::new();
    for name in &chosen.videos {
        let video = folder.join(name);
        if let Ok(text) = std::fs::read_to_string(&video) {
            videos.push((name.clone(), text));
        }
    }
    let meta = CachedConfig {
        id: prepared.id.clone(),
        version: prepared.version.clone(),
        commit: format!("gb{}", prepared.file_id),
        date: prepared.date.clone(),
        path: chosen.path.clone(),
        blob: fetch::git_blob_sha(text.as_bytes()),
        videos: videos
            .iter()
            .map(|(name, text)| (name.clone(), fetch::git_blob_sha(text.as_bytes())))
            .collect(),
    };
    fetch::store_hosted(data_dir, &meta, &text, &videos, Some(&prepared.stock))
}

/// Every file in `dir` that looks like a gameinfo, relative and sorted.
fn gameinfo_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out, 0);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 8 || out.len() > 400 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            walk(root, &path, out, depth + 1);
            continue;
        }
        let lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if (lower.ends_with(".gi") || lower == "gameinfo")
            && let Ok(relative) = path.strip_prefix(root)
        {
            out.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// `video*.txt` files beside `gameinfo`, by name.
fn video_files(root: &Path, gameinfo: &str) -> Vec<(String, String)> {
    let folder = Path::new(gameinfo).parent().map(Path::to_path_buf).unwrap_or_default();
    let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("video")
            && lower.ends_with(".txt")
            && let Ok(text) = std::fs::read_to_string(entry.path())
        {
            out.push((name, text));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_file_wins_over_the_newest() {
        let page = Page {
            name: "x".into(),
            author: "y".into(),
            license: None,
            url: String::new(),
            version: None,
            text: String::new(),
            files: vec![
                ModFile { id: 1, added_ts: 10, ..ModFile::default() },
                ModFile { id: 2, added_ts: 20, ..ModFile::default() },
            ],
        };
        assert_eq!(page.chosen_file(Some(1)).map(|file| file.id), Some(1));
        assert_eq!(page.chosen_file(None).map(|file| file.id), Some(2));
    }

    #[test]
    fn only_gameinfo_and_video_files_are_picked() {
        let root = std::env::temp_dir().join(format!("sigil-hosted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).expect("temp dir");
        std::fs::write(root.join("a/gameinfo.gi"), "GameInfo {}").expect("write");
        std::fs::write(root.join("a/video.txt"), "VideoConfig {}").expect("write");
        std::fs::write(root.join("a/readme.txt"), "hello").expect("write");
        let games = gameinfo_files(&root);
        assert_eq!(games, vec!["a/gameinfo.gi".to_string()]);
        let videos = video_files(&root, &games[0]);
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].0, "video.txt");
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// A stand-in archive for the headless scenes. It owns no files, so `install`
/// would fail; scenes only show the review.
pub fn sample() -> Prepared {
    Prepared {
        id: "gb-hanturaya-competitive".to_string(),
        file_id: 1712001,
        file_name: "hanturaya-competitive.zip".to_string(),
        version: "2.4".to_string(),
        date: "2026-11-03".to_string(),
        variants: vec![
            Variant {
                path: "hanturaya/gameinfo.gi".to_string(),
                label: "hanturaya".to_string(),
                convars: 286,
                engine: 0,
                ignored: 14,
                videos: Vec::new(),
                preferred: true,
            },
            Variant {
                path: "hanturaya/old/gameinfo.gi".to_string(),
                label: "old".to_string(),
                convars: 274,
                engine: 0,
                ignored: 15,
                videos: vec!["video.txt".to_string()],
                preferred: false,
            },
        ],
        chosen: 0,
        stock: String::new(),
        dir: std::path::PathBuf::from("/nonexistent/GameBanana"),
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::performance::{self, catalog};

    const STOCK: &str = include_str!("../../tests/fixtures/stock-gameinfo.gi");

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sigil-hosted-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_reviewed_variant_becomes_a_catalog_config() {
        let data = temp_dir("cache");
        let files = data.join("files");
        std::fs::create_dir_all(&files).expect("files dir");
        let text = STOCK.replace(
            "sv_lagcomp_filterbyviewangle \"false\"",
            "sv_lagcomp_filterbyviewangle \"true\"",
        );
        std::fs::write(files.join("gameinfo.gi"), &text).expect("gameinfo");
        std::fs::write(
            files.join("video.txt"),
            "VideoConfig\n{\n\t\"setting.r_shadows\"\t\t\"0\"\n}\n",
        )
        .expect("video");
        let prepared = Prepared {
            id: "gb-hanturaya-competitive".into(),
            file_id: 1712001,
            file_name: "config.zip".into(),
            version: "2.4".into(),
            date: "2026-11-03".into(),
            variants: vec![Variant {
                path: "gameinfo.gi".into(),
                label: "gameinfo.gi".into(),
                convars: 1,
                engine: 0,
                ignored: 0,
                videos: vec!["video.txt".into()],
                preferred: true,
            }],
            chosen: 0,
            stock: STOCK.into(),
            dir: files,
        };
        install(&prepared, 0, &data).expect("install");
        let catalog = performance::load(&data);
        let config = catalog.get("gb-hanturaya-competitive").expect("hosted config");
        assert_eq!(config.name(), "Hanturaya's competitive");
        assert_eq!(config.def.tier, catalog::Tier::MaxFps);
        assert_eq!(config.version, "2.4");
        assert_eq!(config.videos.len(), 1);
        assert_eq!(config.videos[0].file, "video.txt");
        assert!(config
            .body
            .convars
            .iter()
            .any(|(key, value)| key == "sv_lagcomp_filterbyviewangle" && value == "true"));
        let _ = std::fs::remove_dir_all(&data);
    }
}
