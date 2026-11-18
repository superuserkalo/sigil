//! Which heroes a mod changes, read from the model and material paths in its VPK.
//!
//! A mod replaces a game file by exact path, so a skin's own folders name the hero it is
//! for: `models/heroes_staging/inferno_v4/inferno.vmdl_c` is Infernus and
//! `materials/models/heroes_wip/haze/haze_body.vmat_c` is Haze. The Locker already maps one
//! path to a roster hero through [`crate::locker`]'s internal `hero_of_path`. This module asks
//! that question of a whole mod and returns the sorted display names, so the hero conflict
//! warning can tell that two skins replace the same hero even when neither carries a
//! GameBanana tag.
//!
//! Only `models/` and `materials/models/` count. Sounds, particles and UI art can belong to
//! one hero and still are not a model: two sound packs for Infernus do not replace the same
//! body, and a HUD pack is not a skin. The gate matches Deadlock Mod Manager: strip a leading
//! `materials/`, then require `models/heroes*`. Every extension counts, compiled (`_c`) and
//! source alike, because a source name in a mod still says which hero its author worked on.
//!
//! Read only. Nothing here writes to a game install or contacts the network.

use std::collections::BTreeSet;
use std::path::Path;

use crate::locker::hero_of_path;
use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// Roster display names of the heroes whose model or material files these game paths replace,
/// sorted and without duplicates. Sounds, particles and UI alone do not count: two sound mods
/// for one hero do not replace the same model.
pub fn model_heroes<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut heroes = BTreeSet::new();

    for path in paths {
        // VPK paths are stored with forward slashes, but a caller may hand over a path from
        // another source (a loose file, a project manifest) that kept the platform separator.
        let lowered = path.to_ascii_lowercase().replace('\\', "/");

        if !is_model_path(&lowered) {
            continue;
        }

        if let Some(hero) = hero_of_path(&lowered) {
            heroes.insert(hero.name.to_string());
        }
    }

    heroes.into_iter().collect()
}

/// [`model_heroes`] over every file in the VPK at `path`.
pub fn model_heroes_in_vpk(path: &Path) -> crate::Result<Vec<String>> {
    // The hero answer needs the directory tree only, so skip the preload bytes.
    let directory = read_vpk_directory_with(
        path,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;

    Ok(model_heroes(
        directory.entries.iter().map(|entry| entry.path.as_str()),
    ))
}

/// True for a path under `models/heroes*` or `materials/models/heroes*`.
///
/// This is the folder pair Deadlock Mod Manager reads a hero from. Requiring `models` keeps a
/// material anyway: `materials/models/heroes_wip/haze/...` loses its leading `materials/` and
/// then looks like a model path.
fn is_model_path(path_lower: &str) -> bool {
    let without_materials = path_lower.strip_prefix("materials/").unwrap_or(path_lower);

    without_materials.starts_with("models/heroes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use crate::vpk_write::{PackOptions, pack_vpk};

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dah-mod-heroes-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn vpk(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let staging = root.join(format!("staging-{name}"));

        for (path, data) in files {
            let target = staging.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, data).unwrap();
        }

        let packed = pack_vpk(&PackOptions::new(staging, root.join("out"), name)).unwrap();
        packed.dir_path
    }

    #[test]
    fn a_model_or_material_path_names_its_hero() {
        assert_eq!(
            model_heroes(["models/heroes_staging/inferno_v4/inferno.vmdl_c"]),
            vec!["Infernus"]
        );
        assert_eq!(
            model_heroes(["materials/models/heroes_wip/haze/haze_body.vmat_c"]),
            vec!["Haze"]
        );
        // Both a compiled and a source name count.
        assert_eq!(
            model_heroes(["models/heroes_wip/billy/billy.vmdl"]),
            vec!["Billy"]
        );
        assert!(model_heroes([]).is_empty());
    }

    #[test]
    fn two_heroes_come_back_sorted_and_without_duplicates() {
        let heroes = model_heroes([
            "models/heroes_staging/inferno_v4/inferno.vmdl_c",
            "materials/models/heroes_wip/haze/haze_body.vmat_c",
            "models/heroes_staging/inferno_v4/inferno_skin.vmdl_c",
            "materials/models/heroes_staging/inferno_v4/materials/inferno_body.vmat_c",
        ]);

        assert_eq!(heroes, vec!["Haze", "Infernus"]);
    }

    #[test]
    fn sounds_particles_and_ui_alone_name_no_hero() {
        assert!(
            model_heroes([
                "sounds/abilities/inferno/a1_fire.vsnd_c",
                "sounds/heroes/haze/vo_line.vsnd_c",
                "particles/heroes/inferno/fire.vpcf_c",
                "panorama/images/heroes/inferno_card.vtex_c",
            ])
            .is_empty()
        );
    }

    #[test]
    fn a_bare_file_in_the_heroes_folder_names_no_hero() {
        assert!(model_heroes(["models/heroes/inferno.vmdl_c"]).is_empty());
        assert!(model_heroes(["materials/models/heroes/haze.vmat_c"]).is_empty());
    }

    #[test]
    fn backslashes_and_upper_case_work() {
        let heroes = model_heroes([
            "Models\\Heroes_Staging\\Inferno_V4\\Inferno.VMDL_C",
            "MATERIALS\\MODELS\\HEROES_WIP\\HAZE\\HAZE.VMAT_C",
        ]);

        assert_eq!(heroes, vec!["Haze", "Infernus"]);
    }

    #[test]
    fn reads_every_file_in_a_packed_vpk() {
        let root = temp_root("vpk");
        let path = vpk(
            &root,
            "pak01",
            &[
                (
                    "models/heroes_staging/inferno_v4/inferno.vmdl_c",
                    b"infernus",
                ),
                ("materials/models/heroes_wip/haze/haze_body.vmat_c", b"haze"),
                ("sounds/abilities/inferno/a1_fire.vsnd_c", b"sound"),
                ("particles/heroes/haze/fire.vpcf_c", b"particle"),
            ],
        );

        assert_eq!(
            model_heroes_in_vpk(&path).unwrap(),
            vec!["Haze", "Infernus"]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_vpk_is_an_error_not_a_panic() {
        let root = temp_root("missing");
        let missing = root.join("pak42_dir.vpk");

        assert!(model_heroes_in_vpk(&missing).is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
