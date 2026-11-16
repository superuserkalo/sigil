//! `dah locker`: list heroes with local art and catalog paths, and record a
//! recolor brief.
//!
//! A brief is a work order, not an edited asset. Nothing here writes to the
//! game install.

use std::path::PathBuf;

use serde_json::json;

use deadlock_core::locker::{self, LockerHero};
use deadlock_core::recolor::{self, Palette};

use crate::error::{CliError, Result};
use crate::paths;
use crate::util::print_json;

pub struct ListArgs {
    pub workspace: Option<PathBuf>,
    pub modded: bool,
    pub json: bool,
}

pub struct RecolorArgs {
    /// Hero codename or display name.
    pub hero: String,
    /// Palette preset: ember, frost, toxic or void.
    pub palette: Option<String>,
    /// Custom hue shift, -180 to 180. Overrides the preset hue.
    pub hue: Option<i32>,
    /// Custom saturation multiplier, 0 to 3.
    pub saturation: Option<f64>,
    /// Custom value multiplier, 0 to 3.
    pub value: Option<f64>,
    /// Note recorded with the brief.
    pub note: Option<String>,
    pub workspace: Option<PathBuf>,
    pub json: bool,
}

pub fn list(args: &ListArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let (heroes, assets, catalog_label) = discover(&workspace)?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "catalog": catalog_label,
            "artRoot": assets
                .heroes_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            "count": heroes.len(),
            "heroes": heroes.iter().map(|hero| json!({
                "codename": hero.codename,
                "displayName": hero.display_name,
                "hasRender": hero.render_path.is_some(),
                "mods": hero.mod_count,
                "catalogPaths": hero.asset_count,
                "recolorPaths": hero.recolor_paths,
                "inCatalog": hero.in_catalog,
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }

    let shown: Vec<&LockerHero> = heroes
        .iter()
        .filter(|hero| !args.modded || hero.mod_count > 0)
        .collect();
    println!(
        "Locker: {} of {} hero(es) · {} · art {}",
        shown.len(),
        heroes.len(),
        catalog_label,
        assets
            .heroes_dir
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "no art root".to_string())
    );
    for hero in shown {
        println!(
            "  {:<20} {:<16} {} path(s)  {} recolor target(s)  {} mod(s){}",
            hero.display_name,
            hero.codename,
            hero.asset_count,
            hero.recolor_paths.len(),
            hero.mod_count,
            if hero.render_path.is_some() { "" } else { "  no render" }
        );
    }
    println!();
    println!("A recolor brief records targets and a palette. Run `dah locker recolor <hero> --palette ember`.");
    Ok(())
}

pub fn recolor(args: &RecolorArgs) -> Result<()> {
    let workspace = paths::workspace_root(args.workspace.as_deref())?;
    let (heroes, _, _) = discover(&workspace)?;
    let key = locker::hero_asset_key(&args.hero);
    let hero = heroes
        .iter()
        .find(|hero| {
            locker::hero_asset_key(&hero.codename) == key
                || locker::hero_asset_key(&hero.display_name) == key
        })
        .ok_or_else(|| {
            CliError::new(
                "locker_hero",
                format!(
                    "No hero matches {:?}. Run `dah locker list` for the codenames.",
                    args.hero
                ),
            )
        })?;

    let mut palette = match args.palette.as_deref() {
        Some(name) => recolor::preset(name).ok_or_else(|| {
            CliError::new(
                "locker_palette",
                format!(
                    "Unknown palette {:?}. Use ember, frost, toxic or void, or pass --hue.",
                    name
                ),
            )
        })?,
        None => recolor::preset("ember").expect("ember preset"),
    };
    if let Some(hue) = args.hue {
        palette = Palette {
            name: format!("custom-h{hue}"),
            hue_shift: hue,
            ..palette
        };
    }
    if let Some(saturation) = args.saturation {
        palette.saturation = saturation;
    }
    if let Some(value) = args.value {
        palette.value = value;
    }

    let path = recolor::write_brief(
        &workspace,
        hero,
        &palette,
        args.note.as_deref().unwrap_or(""),
    )
    .map_err(|error| CliError::new("locker_recolor", error.to_string()))?;

    if args.json {
        print_json(&json!({
            "ok": true,
            "hero": hero.codename,
            "displayName": hero.display_name,
            "targets": hero.recolor_paths,
            "palette": palette,
            "brief": path.display().to_string(),
            "inGameReady": false,
            "note": "The brief records the work. It is not an edited asset; decompile and compile are still required.",
        }));
        return Ok(());
    }

    println!(
        "Recolor brief for {}: {} target(s), palette {} (hue {} deg, sat x{}, value x{})",
        hero.display_name,
        hero.recolor_paths.len(),
        palette.name,
        palette.hue_shift,
        palette.saturation,
        palette.value
    );
    for target in &hero.recolor_paths {
        println!("  {target}");
    }
    println!("Written: {}", path.display());
    println!();
    println!("This is a work order, not an asset. It is not in game ready:");
    for requirement in [
        "decompile each target with Source2Viewer-CLI",
        "edit the decoded texture and material",
        "compile with CSDK 12 resourcecompiler.exe on Windows",
        "pack, validate and deploy with approval",
        "verify with an in-game screenshot",
    ] {
        println!("  - {requirement}");
    }
    Ok(())
}

fn discover(
    workspace: &std::path::Path,
) -> Result<(
    Vec<LockerHero>,
    locker::LockerAssetPaths,
    String,
)> {
    use deadlock_core::{addons, discovery, game_catalog};

    let status = discovery::read_game_status(None)
        .map_err(|error| CliError::new("locker_game", error.to_string()))?;
    let install = status.install.ok_or_else(|| {
        CliError::new(
            "locker_game",
            "No Deadlock install was found. Set DAH_GAME_DIR to the Deadlock folder.",
        )
    })?;
    let options = game_catalog::LoadGameCatalogOptions {
        game_root: install.game_root.clone(),
        cache_dir: workspace.join(".dah").join("index"),
        force: false,
    };
    let catalog = game_catalog::load_game_catalog(options)
        .map_err(|error| CliError::new("locker_catalog", error.to_string()))?;
    let installed = addons::read_conflict_report(&install.game_root)
        .map(|report| report.addons)
        .unwrap_or_default();
    let assets = locker::LockerAssetPaths::discover();
    let heroes = locker::discover_heroes_with_assets(&catalog.catalog, &installed, &assets);
    let label = format!(
        "{} paths from {} source{}",
        catalog.catalog.entries.len(),
        catalog.catalog.sources.len(),
        if catalog.catalog.sources.len() == 1 { "" } else { "s" }
    );
    Ok((heroes, assets, label))
}
