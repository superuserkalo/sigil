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
    println!("A recolor brief records targets and a palette. Run `Sigil cli locker recolor <hero> --palette ember`.");
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
                    "No hero matches {:?}. Run `Sigil cli locker list` for the codenames.",
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

fn game_assets() -> Result<deadlock_core::game_assets::GameAssets> {
    let status = deadlock_core::discovery::read_game_status(None)
        .map_err(|error| CliError::new("locker_game", error.to_string()))?;
    let install = status.install.ok_or_else(|| {
        CliError::new(
            "locker_game",
            "No Deadlock install was found. Set DAH_GAME_DIR to the Deadlock folder.",
        )
    })?;
    deadlock_core::game_assets::GameAssets::open(&install.game_root)
        .map_err(|error| CliError::new("locker_assets", error.to_string()))
}

/// `card`, `portrait`, `ability`, `item`, `skin`, `effect` or `other` (the core's ids work too).
pub fn texture_kind(name: &str) -> Option<deadlock_core::game_assets::TextureKind> {
    use deadlock_core::game_assets::TextureKind;
    let name = name.trim().to_ascii_lowercase();
    [
        TextureKind::HeroCard,
        TextureKind::HeroPortrait,
        TextureKind::AbilityIcon,
        TextureKind::ItemIcon,
        TextureKind::SkinTexture,
        TextureKind::AbilityVfx,
        TextureKind::Other,
    ]
    .into_iter()
    .find(|kind| {
        let short = match kind {
            TextureKind::HeroCard => "card",
            TextureKind::HeroPortrait => "portrait",
            TextureKind::AbilityIcon => "ability",
            TextureKind::ItemIcon => "item",
            TextureKind::SkinTexture => "skin",
            TextureKind::AbilityVfx => "effect",
            TextureKind::Other => "other",
        };
        name == short || name == kind.id().to_ascii_lowercase()
    })
}

/// The game's textures. Reads the pak's directory only; no pixels.
pub fn textures(
    hero: Option<&str>,
    kind: Option<&str>,
    search: Option<&str>,
    limit: Option<usize>,
    json: bool,
) -> Result<()> {
    use deadlock_core::game_assets::TextureQuery;
    let kinds = match kind {
        Some(name) => vec![texture_kind(name).ok_or_else(|| {
            CliError::new(
                "locker_kind",
                format!("{name} is not a texture kind. Use card, portrait, ability, item, skin, effect or other."),
            )
        })?],
        None => Vec::new(),
    };
    let limit = limit.unwrap_or(200).clamp(1, 2000);
    let pak = game_assets()?;
    let list = pak.texture_catalog(&TextureQuery {
        hero: hero.map(str::to_string),
        kinds,
        search: search.map(str::to_string),
        limit: Some(limit),
    });
    if json {
        print_json(&json!({
            "ok": true,
            "count": list.len(),
            "textures": list.iter().map(|texture| json!({
                "path": texture.path,
                "kind": texture.kind.id(),
                "hero": texture.hero,
                "label": texture.label,
                "variant": texture.variant,
            })).collect::<Vec<_>>(),
        }));
    } else {
        for texture in &list {
            println!("{}  {}", texture.kind.id(), texture.path);
        }
    }
    Ok(())
}

/// The game's sound events for one hero, or the heroes in the sound index. The first call for a
/// game build indexes the soundevents into the workspace cache.
pub fn sounds(
    hero: Option<&str>,
    slot: Option<u8>,
    search: Option<&str>,
    limit: Option<usize>,
    workspace: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    use deadlock_core::game_assets::SoundQuery;
    let workspace = paths::workspace_root(workspace.as_deref())?;
    let cache = workspace.join(".dah").join("assets");
    let pak = game_assets()?;
    let Some(hero) = hero.filter(|hero| !hero.trim().is_empty()) else {
        let heroes = pak
            .sound_heroes(&cache)
            .map_err(|error| CliError::new("locker_sounds", error.to_string()))?;
        if json {
            print_json(&json!({
                "ok": true,
                "heroes": heroes.iter().map(|hero| json!({
                    "key": hero.key, "name": hero.name, "sounds": hero.sounds, "voiceLines": hero.voice_lines,
                })).collect::<Vec<_>>(),
            }));
        } else {
            for hero in &heroes {
                println!("{}  {} sounds", hero.name, hero.sounds);
            }
        }
        return Ok(());
    };
    if slot.is_some_and(|slot| !(1..=4).contains(&slot)) {
        return Err(CliError::new("locker_slot", "An ability slot is 1 to 4."));
    }
    let list = pak
        .sound_catalog(
            &cache,
            &SoundQuery {
                hero: hero.to_string(),
                kinds: Vec::new(),
                slot,
                search: search.map(str::to_string),
                limit: Some(limit.unwrap_or(150).clamp(1, 2000)),
            },
        )
        .map_err(|error| CliError::new("locker_sounds", error.to_string()))?;
    if json {
        print_json(&json!({
            "ok": true,
            "count": list.len(),
            "sounds": list.iter().map(|sound| json!({
                "event": sound.event,
                "kind": sound.kind.id(),
                "ability": sound.ability,
                "slot": sound.slot,
                "label": sound.label,
                "clips": sound.clips,
                "soundevents": sound.soundevents,
            })).collect::<Vec<_>>(),
        }));
    } else {
        for sound in &list {
            println!("{}  {}", sound.event, sound.label);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn texture_kinds_parse_short_and_core_ids() {
        use deadlock_core::game_assets::TextureKind;
        assert_eq!(super::texture_kind("skin"), Some(TextureKind::SkinTexture));
        assert_eq!(super::texture_kind("abilityIcon"), Some(TextureKind::AbilityIcon));
        assert_eq!(super::texture_kind("nope"), None);
    }
}
