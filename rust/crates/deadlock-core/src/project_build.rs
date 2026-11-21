//! Creator build helpers over a mod project's `sources/` and `staging/` folders:
//!
//! - Incremental compile: [`plan_build`] hashes each compilable source and skips the ones whose
//!   hash and compiled output match the last build (`.dah/build.json`); `rebuild` compiles all.
//!   [`record_built`] stores the hashes after a successful compile.
//! - Image compile list: [`sync_image_list`] regenerates `sources/panorama/styles/
//!   sigil_image_list.vcss`, which references every Panorama image source so one compile of that
//!   file compiles them all. Run it after images are added, moved or removed.
//! - Scan and Repair: [`scan`] finds fixable layout mistakes (source files in `staging/`,
//!   uppercase letters in game paths, Panorama images outside `panorama/images`); [`repair`]
//!   applies them after copying every touched file into `.dah/repair-undo/<stamp>/`, and
//!   [`undo_repair`] puts them back.
//! - Bulk rename: [`plan_rename`] lists the moves under a folder or file plus every project text
//!   file and game path that refers to the old name, before [`apply_rename`] moves anything.
//!
//! - Deadlock Mod Manager (`feature/modding-workbench`, behaviour only): skip unchanged assets
//!   with an explicit Rebuild, reversible Scan and Repair, bulk rename after showing addon and
//!   game references, image compiler lists kept in sync.
//!
//! Every write stays inside the project folder. Nothing here touches a game install.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Source extensions `resourcecompiler.exe` turns into `<name>.<ext>_c`.
pub const COMPILABLE: &[&str] = &["vmat", "vmdl", "vpcf", "vsndevts", "vdata", "vxml", "vcss", "vjs", "vts", "vsnd"];
/// Image sources Panorama compiles to `<name>_<ext>.vtex_c`.
pub const IMAGE_SOURCES: &[&str] = &["png", "psd", "tga", "jpg", "jpeg"];
pub const IMAGE_LIST: &str = "panorama/styles/sigil_image_list.vcss";

const MANIFEST: &str = ".dah/build.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildManifest {
    /// Source path under `sources/` to its SHA-256.
    pub hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildPlan {
    pub compile: Vec<String>,
    pub unchanged: Vec<String>,
}

fn require_project(project: &Path) -> Result<()> {
    if project.join("project.json").is_file() {
        Ok(())
    } else {
        Err(Error::msg(format!("{} is not a mod project.", project.display())))
    }
}

fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    out.sort();
    out
}

fn ext(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// The compiled game path a source produces.
pub fn compiled_path(source: &str) -> Option<String> {
    let e = ext(source);
    if COMPILABLE.contains(&e.as_str()) {
        return Some(format!("{source}_c"));
    }
    if IMAGE_SOURCES.contains(&e.as_str()) {
        let stem = &source[..source.len() - e.len() - 1];
        return Some(format!("{stem}_{e}.vtex_c"));
    }
    None
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub fn read_manifest(project: &Path) -> BuildManifest {
    fs::read(project.join(MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Which sources need a compile. `rebuild` compiles every one.
pub fn plan_build(project: &Path, rebuild: bool) -> Result<BuildPlan> {
    require_project(project)?;
    let manifest = read_manifest(project);
    let sources = project.join("sources");
    let mut plan = BuildPlan::default();
    for path in walk(&sources) {
        let Some(output) = compiled_path(&path) else { continue };
        let fresh = !rebuild
            && project.join("staging").join(&output).is_file()
            && manifest.hashes.get(&path).is_some_and(|old| hash_file(&sources.join(&path)).ok().as_deref() == Some(old));
        if fresh {
            plan.unchanged.push(path);
        } else {
            plan.compile.push(path);
        }
    }
    Ok(plan)
}

/// Record the hashes of sources that compiled.
pub fn record_built(project: &Path, built: &[String]) -> Result<()> {
    require_project(project)?;
    let mut manifest = read_manifest(project);
    for path in built {
        if let Ok(hash) = hash_file(&project.join("sources").join(path)) {
            manifest.hashes.insert(path.clone(), hash);
        }
    }
    let file = project.join(MANIFEST);
    fs::create_dir_all(file.parent().expect("manifest has a parent"))?;
    fs::write(file, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

/// Regenerate the image list from the project's Panorama image sources. Returns how many
/// images it names; with none, the list file is removed.
pub fn sync_image_list(project: &Path) -> Result<usize> {
    require_project(project)?;
    let sources = project.join("sources");
    let images: Vec<String> = walk(&sources)
        .into_iter()
        .filter(|p| p.to_ascii_lowercase().starts_with("panorama/images/") && IMAGE_SOURCES.contains(&ext(p).as_str()))
        .collect();
    let file = sources.join(IMAGE_LIST);
    if images.is_empty() {
        let _ = fs::remove_file(&file);
        return Ok(0);
    }
    let mut text = String::from("/* Written by Sigil: every Panorama image in this project, so one compile builds them all. */\n");
    for (i, image) in images.iter().enumerate() {
        text.push_str(&format!(".sigil-image-{i} {{ background-image: url(\"file://{{images}}/{}\"); }}\n", &image["panorama/images/".len()..]));
    }
    fs::create_dir_all(file.parent().expect("list has a parent"))?;
    if fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        fs::write(&file, text)?;
    }
    Ok(images.len())
}

// --- Scan and Repair -----------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    pub rule: String,
    /// Project-relative paths (`staging/...`, `sources/...`).
    pub from: String,
    pub to: String,
    pub why: String,
}

/// Fixable layout mistakes, in project-relative paths. Reads only.
pub fn scan(project: &Path) -> Result<Vec<Fix>> {
    require_project(project)?;
    let mut fixes = Vec::new();
    for path in walk(&project.join("staging")) {
        let e = ext(&path);
        if (COMPILABLE.contains(&e.as_str()) || IMAGE_SOURCES.contains(&e.as_str())) && !e.ends_with("_c") {
            fixes.push(Fix {
                rule: "source_in_staging".into(),
                from: format!("staging/{path}"),
                to: format!("sources/{path}"),
                why: "The game ignores source files. Sources belong in sources/ and compile into staging/.".into(),
            });
            continue;
        }
        let lower = path.to_ascii_lowercase();
        let mut target = lower.clone();
        if lower.starts_with("panorama/") && lower.ends_with(".vtex_c") && !lower.starts_with("panorama/images/") {
            target = format!("panorama/images/{}", &lower["panorama/".len()..]);
            fixes.push(Fix {
                rule: "misplaced_panorama_image".into(),
                from: format!("staging/{path}"),
                to: format!("staging/{target}"),
                why: "Panorama loads images from panorama/images only.".into(),
            });
        } else if target != path {
            fixes.push(Fix {
                rule: "uppercase_path".into(),
                from: format!("staging/{path}"),
                to: format!("staging/{target}"),
                why: "Game paths are lowercase; the game will not match this file.".into(),
            });
        }
    }
    Ok(fixes)
}

fn inside(project: &Path, relative: &str) -> Result<PathBuf> {
    if !crate::vpk::is_safe_vpk_path(relative) {
        return Err(Error::msg(format!("{relative} is not a safe project path.")));
    }
    Ok(project.join(relative))
}

/// Move a file, through a temporary name so a case-only rename works on a case-insensitive
/// disk. Folder case changes on the way are left to the disk.
fn move_file(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to.parent().expect("target has a parent"))?;
    let temp = from.with_extension("sigil-move");
    fs::rename(from, &temp)?;
    fs::rename(&temp, to)?;
    Ok(())
}

/// Apply fixes, keeping an undo copy. Returns the undo folder.
pub fn repair(project: &Path, fixes: &[Fix]) -> Result<PathBuf> {
    require_project(project)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%3f").to_string();
    let undo = project.join(".dah/repair-undo").join(&stamp);
    fs::create_dir_all(&undo)?;
    for fix in fixes {
        let from = inside(project, &fix.from)?;
        let to = inside(project, &fix.to)?;
        if to.exists() && !fix.from.eq_ignore_ascii_case(&fix.to) {
            return Err(Error::msg(format!("{} already exists. Nothing was moved after it.", fix.to)));
        }
        let backup = undo.join(&fix.from);
        fs::create_dir_all(backup.parent().expect("backup has a parent"))?;
        fs::copy(&from, &backup)?;
        move_file(&from, &to)?;
    }
    fs::write(undo.join("fixes.json"), serde_json::to_vec_pretty(fixes)?)?;
    Ok(undo)
}

/// Undo a repair: move each fixed file back from where the repair put it.
pub fn undo_repair(project: &Path, undo: &Path) -> Result<usize> {
    require_project(project)?;
    if !undo.starts_with(project.join(".dah/repair-undo")) {
        return Err(Error::msg("That is not one of this project's repair copies."));
    }
    let fixes: Vec<Fix> = serde_json::from_slice(&fs::read(undo.join("fixes.json"))?)?;
    for fix in &fixes {
        let to = inside(project, &fix.to)?;
        let from = inside(project, &fix.from)?;
        let _ = fs::remove_file(to);
        fs::create_dir_all(from.parent().expect("source has a parent"))?;
        fs::copy(undo.join(&fix.from), &from)?;
    }
    fs::remove_dir_all(undo)?;
    Ok(fixes.len())
}

// --- bulk rename ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlan {
    /// Project-relative moves.
    pub moves: Vec<(String, String)>,
    /// Project text files that mention the old game path, with the line number.
    pub project_references: Vec<(String, usize)>,
    /// Game paths (from the game index) under the old name: renaming away from them means the
    /// mod no longer replaces them.
    pub game_paths: Vec<String>,
}

fn game_form(path: &str) -> String {
    path.trim_matches('/').to_ascii_lowercase()
}

/// Plan renaming a game path prefix (`models/heroes/haze` to `models/heroes/haze_v2`) in
/// staging and sources. `game_index` is the game's own path list. Reads only.
pub fn plan_rename(project: &Path, from: &str, to: &str, game_index: &[String]) -> Result<RenamePlan> {
    require_project(project)?;
    let (from, to) = (game_form(from), game_form(to));
    if from.is_empty() || to.is_empty() || from == to || !crate::vpk::is_safe_vpk_path(&to) {
        return Err(Error::msg("Give a different, safe new name."));
    }
    let mut plan = RenamePlan::default();
    let under = |p: &str| {
        let l = p.to_ascii_lowercase();
        l == from || l.starts_with(&format!("{from}/")) || l.starts_with(&format!("{from}."))
    };
    for folder in ["staging", "sources"] {
        for path in walk(&project.join(folder)) {
            if under(&path) {
                let rest = &path[from.len()..];
                plan.moves.push((format!("{folder}/{path}"), format!("{folder}/{to}{rest}")));
            } else if folder == "sources"
                && !IMAGE_SOURCES.contains(&ext(&path).as_str())
                && let Ok(text) = fs::read_to_string(project.join(folder).join(&path))
            {
                for (i, line) in text.lines().enumerate() {
                    if line.to_ascii_lowercase().contains(&from) {
                        plan.project_references.push((format!("{folder}/{path}"), i + 1));
                    }
                }
            }
        }
    }
    plan.game_paths = game_index.iter().filter(|p| under(p)).take(200).cloned().collect();
    Ok(plan)
}

pub fn apply_rename(project: &Path, plan: &RenamePlan) -> Result<usize> {
    require_project(project)?;
    for (from, to) in &plan.moves {
        let (from, to) = (inside(project, from)?, inside(project, to)?);
        if to.exists() {
            return Err(Error::msg(format!("{} already exists.", to.display())));
        }
        move_file(&from, &to)?;
    }
    Ok(plan.moves.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dah-build-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sources")).unwrap();
        fs::create_dir_all(dir.join("staging")).unwrap();
        fs::write(dir.join("project.json"), "{}").unwrap();
        dir
    }

    fn put(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn unchanged_sources_skip_until_rebuild() {
        let dir = project("incr");
        put(&dir, "sources/materials/a.vmat", "one");
        put(&dir, "sources/materials/b.vmat", "two");
        assert_eq!(plan_build(&dir, false).unwrap().compile.len(), 2);
        put(&dir, "staging/materials/a.vmat_c", "x");
        put(&dir, "staging/materials/b.vmat_c", "x");
        record_built(&dir, &["materials/a.vmat".into(), "materials/b.vmat".into()]).unwrap();
        let plan = plan_build(&dir, false).unwrap();
        assert!(plan.compile.is_empty() && plan.unchanged.len() == 2);
        put(&dir, "sources/materials/b.vmat", "changed");
        assert_eq!(plan_build(&dir, false).unwrap().compile, vec!["materials/b.vmat".to_string()]);
        assert_eq!(plan_build(&dir, true).unwrap().compile.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_list_follows_the_images() {
        let dir = project("imgs");
        put(&dir, "sources/panorama/images/hud/a.png", "");
        put(&dir, "sources/panorama/images/hud/b.psd", "");
        assert_eq!(sync_image_list(&dir).unwrap(), 2);
        let list = fs::read_to_string(dir.join("sources").join(IMAGE_LIST)).unwrap();
        assert!(list.contains("hud/a.png") && list.contains("hud/b.psd"));
        fs::remove_file(dir.join("sources/panorama/images/hud/a.png")).unwrap();
        assert_eq!(sync_image_list(&dir).unwrap(), 1);
        fs::remove_file(dir.join("sources/panorama/images/hud/b.psd")).unwrap();
        assert_eq!(sync_image_list(&dir).unwrap(), 0);
        assert!(!dir.join("sources").join(IMAGE_LIST).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repair_moves_and_undo_restores() {
        let dir = project("repair");
        put(&dir, "staging/materials/a.vmat", "src");
        put(&dir, "staging/models/X.vmdl_c", "m");
        put(&dir, "staging/panorama/layout/i_png.vtex_c", "t");
        let fixes = scan(&dir).unwrap();
        let rules: Vec<&str> = fixes.iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules.len(), 3, "{rules:?}");
        let undo = repair(&dir, &fixes).unwrap();
        assert!(dir.join("sources/materials/a.vmat").is_file());
        assert!(dir.join("staging/models/x.vmdl_c").is_file());
        assert!(dir.join("staging/panorama/images/layout/i_png.vtex_c").is_file());
        assert!(scan(&dir).unwrap().is_empty());
        assert_eq!(undo_repair(&dir, &undo).unwrap(), 3);
        assert!(dir.join("staging/materials/a.vmat").is_file());
        assert!(!dir.join("sources/materials/a.vmat").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_lists_references_before_moving() {
        let dir = project("rename");
        put(&dir, "staging/models/haze/haze.vmdl_c", "m");
        put(&dir, "sources/materials/haze.vmat", "TextureColor \"models/haze/haze_color.png\"");
        let game = vec!["models/haze/haze.vmdl_c".to_string(), "models/other.vmdl_c".to_string()];
        let plan = plan_rename(&dir, "models/haze", "models/haze_v2", &game).unwrap();
        assert_eq!(plan.moves, vec![("staging/models/haze/haze.vmdl_c".into(), "staging/models/haze_v2/haze.vmdl_c".into())]);
        assert_eq!(plan.project_references, vec![("sources/materials/haze.vmat".into(), 1)]);
        assert_eq!(plan.game_paths, vec!["models/haze/haze.vmdl_c".to_string()]);
        assert!(plan_rename(&dir, "models/haze", "../x", &game).is_err());
        assert_eq!(apply_rename(&dir, &plan).unwrap(), 1);
        assert!(dir.join("staging/models/haze_v2/haze.vmdl_c").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
