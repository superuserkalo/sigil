//! Recolor briefs: a durable work order for a hero recolor.
//!
//! A compiled `vtex_c` cannot be recolored without decompiling it and compiling
//! the result back. That toolchain is external and often absent, so this module
//! records the exact targets and the palette in the workspace instead of
//! pretending to edit a game file. When a toolchain exists, the agent executes
//! the brief and the record is updated.
//!
//! Nothing here writes to a game install.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::locker::{LockerHero, hero_asset_key};
use crate::util::now_iso;

pub const BRIEF_SCHEMA: u32 = 1;
pub const RECOLOR_DIR: &str = "generated/recolors";

/// A hue and saturation direction. Values are applied by the image editor step,
/// not by this crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    pub name: String,
    /// Degrees added to the source hue, -180 to 180.
    pub hue_shift: i32,
    /// Multiplier on saturation, 0 to 3.
    pub saturation: f64,
    /// Multiplier on value, 0 to 3.
    pub value: f64,
    pub note: String,
}

/// The palette choices the app offers. A brief can also carry a custom palette.
pub fn presets() -> Vec<Palette> {
    vec![
        Palette {
            name: "ember".to_string(),
            hue_shift: -35,
            saturation: 1.15,
            value: 1.0,
            note: "warm shift toward orange".to_string(),
        },
        Palette {
            name: "frost".to_string(),
            hue_shift: 155,
            saturation: 0.9,
            value: 1.05,
            note: "cool shift toward cyan".to_string(),
        },
        Palette {
            name: "toxic".to_string(),
            hue_shift: 95,
            saturation: 1.35,
            value: 1.0,
            note: "green shift, higher saturation".to_string(),
        },
        Palette {
            name: "void".to_string(),
            hue_shift: 240,
            saturation: 0.6,
            value: 0.8,
            note: "violet shift, darkened".to_string(),
        },
    ]
}

pub fn preset(name: &str) -> Option<Palette> {
    presets()
        .into_iter()
        .find(|palette| palette.name.eq_ignore_ascii_case(name))
}

/// A recorded recolor request for one hero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecolorBrief {
    pub schema: u32,
    pub hero: String,
    pub display_name: String,
    pub created_at: String,
    /// Exact game paths the recolor edits, from the catalog.
    pub targets: Vec<String>,
    pub palette: Palette,
    pub note: String,
    /// Steps that must exist before the brief can be executed.
    pub requires: Vec<String>,
    /// Always false. A brief is not an edited asset.
    pub in_game_ready: bool,
}

impl RecolorBrief {
    pub fn is_executable(&self) -> bool {
        false
    }
}

/// Validate a palette before it is recorded.
pub fn validate_palette(palette: &Palette) -> Result<()> {
    if palette.name.trim().is_empty() {
        return Err(Error::msg("A palette needs a name."));
    }
    if !(-180..=180).contains(&palette.hue_shift) {
        return Err(Error::msg(format!(
            "Hue shift is {}; the accepted range is -180 to 180 degrees.",
            palette.hue_shift
        )));
    }
    for (label, value) in [
        ("saturation", palette.saturation),
        ("value", palette.value),
    ] {
        if !(0.0..=3.0).contains(&value) || !value.is_finite() {
            return Err(Error::msg(format!(
                "Palette {label} is {value}; the accepted range is 0 to 3."
            )));
        }
    }
    Ok(())
}

fn slug_for(hero: &LockerHero) -> String {
    let key = hero_asset_key(&hero.codename);
    if key.is_empty() {
        hero_asset_key(&hero.display_name)
    } else {
        key
    }
}

pub fn recolor_dir(workspace: &Path) -> PathBuf {
    workspace.join(RECOLOR_DIR)
}

pub fn brief_path(workspace: &Path, hero: &LockerHero) -> Result<PathBuf> {
    let slug = slug_for(hero);
    if slug.is_empty() {
        return Err(Error::msg("The hero has no usable codename for a brief."));
    }
    Ok(recolor_dir(workspace).join(format!("{slug}.json")))
}

/// Write a brief for one hero. The hero must carry at least one catalog path:
/// an art-only hero has nothing to recolor.
pub fn write_brief(
    workspace: &Path,
    hero: &LockerHero,
    palette: &Palette,
    note: &str,
) -> Result<PathBuf> {
    validate_palette(palette)?;
    if hero.recolor_paths.is_empty() {
        return Err(Error::msg(format!(
            "{} has no recolorable catalog path. {}",
            hero.display_name,
            if hero.in_catalog {
                "The catalog has no material or texture under its folder."
            } else {
                "The hero is known only from local art, not from the game catalog."
            }
        )));
    }

    let brief = RecolorBrief {
        schema: BRIEF_SCHEMA,
        hero: hero.codename.clone(),
        display_name: hero.display_name.clone(),
        created_at: now_iso(),
        targets: hero.recolor_paths.clone(),
        palette: palette.clone(),
        note: note.trim().to_string(),
        requires: vec![
            "Source2Viewer-CLI to decompile each target _c asset".to_string(),
            "an image editor step for the decoded texture".to_string(),
            "numeric edits to the material for tint or colour parameters".to_string(),
            "CSDK 12 resourcecompiler.exe on Windows to compile".to_string(),
            "pack, validate, then deploy with approval".to_string(),
            "an in-game screenshot before the brief is called done".to_string(),
        ],
        in_game_ready: false,
    };

    let path = brief_path(workspace, hero)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&brief)?;
    bytes.push(b'\n');
    crate::util::write_atomic(&path, &bytes)?;
    Ok(path)
}

pub fn read_brief(workspace: &Path, hero: &LockerHero) -> Result<Option<RecolorBrief>> {
    let path = brief_path(workspace, hero)?;
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|error| Error::msg(format!("Could not parse {}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::msg(format!(
            "Could not read {}: {error}",
            path.display()
        ))),
    }
}

/// Every brief in the workspace, newest first.
pub fn list_briefs(workspace: &Path) -> Result<Vec<RecolorBrief>> {
    let directory = recolor_dir(workspace);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut briefs = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        match serde_json::from_str::<RecolorBrief>(&text) {
            Ok(brief) => briefs.push(brief),
            Err(error) => {
                return Err(Error::msg(format!(
                    "Could not parse {}: {error}. Fix or remove the brief.",
                    path.display()
                )));
            }
        }
    }
    briefs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(briefs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hero() -> LockerHero {
        LockerHero {
            codename: "punkgoat".to_string(),
            display_name: "Billy".to_string(),
            render_path: None,
            name_art_path: None,
            mod_count: 1,
            asset_count: 42,
            in_catalog: true,
            recolor_paths: vec![
                "materials/models/heroes_wip/punkgoat/punkgoat_body.vmat_c".to_string(),
                "materials/models/heroes_wip/punkgoat/punkgoat_head.vtex_c".to_string(),
            ],
        }
    }

    #[test]
    fn writes_a_brief_with_targets_and_requirements() {
        let workspace = std::env::temp_dir().join(format!("dah-recolor-{}", std::process::id()));
        let path = write_brief(&workspace, &hero(), &preset("ember").unwrap(), "test").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let brief: RecolorBrief = serde_json::from_str(&text).unwrap();
        assert_eq!(brief.targets.len(), 2);
        assert!(!brief.in_game_ready);
        assert!(brief.requires.iter().any(|line| line.contains("resourcecompiler")));
        assert!(!brief.is_executable());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn an_art_only_hero_has_nothing_to_recolor() {
        let mut hero = hero();
        hero.in_catalog = false;
        hero.recolor_paths.clear();
        let workspace = std::env::temp_dir().join(format!("dah-recolor-empty-{}", std::process::id()));
        assert!(write_brief(&workspace, &hero, &preset("frost").unwrap(), "").is_err());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn palettes_are_bounded() {
        let mut palette = preset("void").unwrap();
        palette.hue_shift = 400;
        assert!(validate_palette(&palette).is_err());
        palette.hue_shift = 40;
        palette.saturation = 5.0;
        assert!(validate_palette(&palette).is_err());
    }

    #[test]
    fn presets_are_stable() {
        assert_eq!(presets().len(), 4);
        assert!(preset("EMBER").is_some());
        assert!(preset("nope").is_none());
    }
}
