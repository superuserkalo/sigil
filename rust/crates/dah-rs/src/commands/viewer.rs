//! `dah viewer`: export heroes and the Hideout as files for the 3D viewer.
//!
//! Read only on the game install. Output goes to a cache folder: `--out`, else
//! `<data>/viewer-cache`, the folder the app uses.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use deadlock_core::import::{library_path, read_library};
use deadlock_core::viewer::{self, HeroExportRequest, HideoutExportRequest, Variant};

use crate::domain;
use crate::error::{Result, core};
use crate::paths;
use crate::util::print_json;

pub struct ExportHeroArgs {
    pub hero: String,
    pub original: bool,
    pub slots: Vec<u16>,
    pub out: Option<PathBuf>,
    pub full_textures: bool,
    pub json: bool,
}

pub struct ExportHideoutArgs {
    pub hero: Option<String>,
    pub original: bool,
    pub slots: Vec<u16>,
    pub out: Option<PathBuf>,
    pub json: bool,
}

fn game_root() -> Result<PathBuf> {
    let status = domain::read_status()?;
    Ok(domain::require_install(&status)?.game_root.clone())
}

fn cache_dir(out: Option<&PathBuf>) -> Result<PathBuf> {
    match out {
        Some(out) => paths::absolute(out),
        None => Ok(paths::data_dir()?.join("viewer-cache")),
    }
}

fn variant(original: bool) -> Variant {
    if original {
        Variant::Original
    } else {
        Variant::Modded
    }
}

pub fn heroes(json: bool) -> Result<()> {
    let game = game_root()?;
    let list = viewer::heroes(&game).map_err(|error| core("viewer heroes", error))?;
    if json {
        print_json(&json!({ "ok": true, "heroes": list }));
        return Ok(());
    }
    for hero in &list {
        println!(
            "  {:<16} {:<14} {}",
            hero.display_name, hero.codename, hero.model_entry
        );
    }
    println!("{} hero(es).", list.len());
    Ok(())
}

pub fn mod_heroes(vpk: &std::path::Path, json: bool) -> Result<()> {
    let codenames = viewer::heroes_in_vpk(vpk).map_err(|error| core("viewer mod-heroes", error))?;
    if json {
        print_json(&json!({ "ok": true, "heroes": codenames }));
    } else if codenames.is_empty() {
        println!("No hero model or material files in {}.", vpk.display());
    } else {
        println!("{}", codenames.join(", "));
    }
    Ok(())
}

pub fn export_hero(args: &ExportHeroArgs) -> Result<()> {
    let game = game_root()?;
    let cache = cache_dir(args.out.as_ref())?;
    let library = read_library(&library_path(&paths::data_dir()?)).ok();
    let mut request = HeroExportRequest::new(&game, &cache, &args.hero, variant(args.original));
    request.extra_slots = &args.slots;
    request.library = library.as_ref();
    if args.full_textures {
        request.max_texture_edge = None;
    }
    let started = std::time::Instant::now();
    let exported = viewer::export_hero_with(&request, &AtomicBool::new(false))
        .map_err(|error| core("viewer export-hero", error))?;
    let seconds = started.elapsed().as_secs_f64();
    if args.json {
        print_json(&json!({
            "ok": true,
            "glb": exported.glb,
            "sidecar": exported.sidecar,
            "cloth": exported.cloth,
            "key": exported.key,
            "variant": exported.variant,
            "cached": exported.cached,
            "seconds": seconds,
        }));
        return Ok(());
    }
    println!(
        "{} ({}, {:.1} s{})",
        exported.glb.display(),
        exported.variant.as_str(),
        seconds,
        if exported.cached { ", cached" } else { "" }
    );
    println!("{}", exported.sidecar.display());
    if let Some(cloth) = &exported.cloth {
        println!("{}", cloth.display());
    }
    Ok(())
}

pub fn export_hideout(args: &ExportHideoutArgs) -> Result<()> {
    let game = game_root()?;
    let cache = cache_dir(args.out.as_ref())?;
    let library = read_library(&library_path(&paths::data_dir()?)).ok();
    let mut request = HideoutExportRequest::new(&game, &cache, variant(args.original));
    request.hero = args.hero.as_deref();
    request.extra_slots = &args.slots;
    request.library = library.as_ref();
    let started = std::time::Instant::now();
    let scene = viewer::export_hideout_with(&request, &AtomicBool::new(false))
        .map_err(|error| core("viewer export-hideout", error))?;
    let seconds = started.elapsed().as_secs_f64();
    if args.json {
        print_json(&json!({ "ok": true, "scene": scene, "seconds": seconds }));
    } else {
        println!("{} ({:.1} s)", scene.display(), seconds);
    }
    Ok(())
}

pub fn prune(out: Option<&PathBuf>, keep_mb: u64, json: bool) -> Result<()> {
    let cache = cache_dir(out)?;
    let report = viewer::prune_cache(&cache, keep_mb.saturating_mul(1024 * 1024))
        .map_err(|error| core("viewer prune", error))?;
    if json {
        print_json(&json!({ "ok": true, "report": report }));
    } else {
        println!(
            "Removed {} entr{} ({} MB). {} MB kept.",
            report.removed,
            if report.removed == 1 { "y" } else { "ies" },
            report.freed_bytes / (1024 * 1024),
            report.kept_bytes / (1024 * 1024)
        );
    }
    Ok(())
}
