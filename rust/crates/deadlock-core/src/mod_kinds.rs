//! Whether a mod is a map mod, read from the game paths in its VPK.
//!
//! Deadlock ships each map as its own VPK under `game/citadel/maps/`, and a map mod puts
//! entries under `maps/` inside an addon slot. Loading one while playing a normal map, or while
//! matchmaking, breaks the game, so the launcher warns before it starts. Deadlock Mod Manager
//! decides from two things: the GameBanana category (`Maps`) and the mod's own paths, where any
//! `maps/` entry counts except one under `maps/ui/`. A skin ships its hero-select scene under
//! `maps/ui/`, so that folder alone does not make a mod a map. Only the paths are available
//! here; a caller that has GameBanana metadata can treat a `Maps` category as the same answer.
//!
//! Read only. Nothing here writes to a game install or contacts the network.

use std::path::Path;

use crate::vpk::{ReadVpkOptions, read_vpk_directory_with};

/// True when these game paths make the mod a map mod: any path under `maps/` outside
/// `maps/ui/`. Case does not matter and either slash works, because a caller may hand over
/// paths from a VPK, a project manifest or a loose staging folder.
pub fn is_map_mod<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(is_map_path)
}

/// [`is_map_mod`] over every file in the VPK at `path`.
pub fn is_map_mod_vpk(path: &Path) -> crate::Result<bool> {
    // The answer needs the directory tree only, so skip the preload bytes.
    let directory = read_vpk_directory_with(
        path,
        &ReadVpkOptions {
            keep_preload: false,
        },
    )?;

    Ok(is_map_mod(
        directory.entries.iter().map(|entry| entry.path.as_str()),
    ))
}

/// True for a `maps/` path that is not a hero-select scene under `maps/ui/`.
fn is_map_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase().replace('\\', "/");

    lowered.starts_with("maps/") && !lowered.starts_with("maps/ui/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    use crate::vpk_write::{PackOptions, pack_vpk};

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dah-mod-kinds-{tag}-{}", std::process::id()));
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
    fn a_maps_path_makes_a_map_mod() {
        assert!(is_map_mod(["maps/street_test.vpk"]));
        assert!(is_map_mod([
            "materials/models/heroes_wip/haze/haze_body.vmat_c",
            "maps/street_test/entities/lights.vmdl_c",
        ]));
    }

    #[test]
    fn a_hero_select_scene_is_not_a_map() {
        assert!(!is_map_mod(["maps/ui/hero_prefabs/necro.vpk"]));
        assert!(!is_map_mod([
            "maps/ui/hero_shop/necro_shop/entities/default_ents.vents_c",
        ]));
    }

    #[test]
    fn other_folders_and_lookalikes_do_not_count() {
        assert!(!is_map_mod([]));
        assert!(!is_map_mod(["models/heroes_staging/inferno_v4/inferno.vmdl_c"]));
        assert!(!is_map_mod(["panorama/maps/ui_card.vtex_c"]));
        assert!(!is_map_mod(["mymaps/street_test.vmdl_c"]));
        // Only the exact `maps/ui/` folder is excused; a sibling starting with `ui` still counts.
        assert!(is_map_mod(["maps/ui2/street_test.vmdl_c"]));
    }

    #[test]
    fn backslashes_and_upper_case_work() {
        assert!(is_map_mod(["MAPS\\Street_Test\\World.VWRld_C"]));
        assert!(!is_map_mod(["MAPS\\UI\\Hero_Prefabs\\Necro.VPK"]));
    }

    #[test]
    fn reads_every_file_in_a_packed_vpk() {
        let root = temp_root("vpk");
        let map = vpk(
            &root,
            "pak01",
            &[
                ("maps/street_test.vpk", b"map"),
                ("maps/street_test/entities/lights.vmdl_c", b"light"),
            ],
        );
        let skin = vpk(
            &root,
            "pak02",
            &[
                ("maps/ui/hero_prefabs/necro.vpk", b"scene"),
                ("models/heroes_staging/necro/necro.vmdl_c", b"model"),
            ],
        );

        assert!(is_map_mod_vpk(&map).unwrap());
        assert!(!is_map_mod_vpk(&skin).unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_vpk_is_an_error_not_a_panic() {
        let root = temp_root("missing");
        let missing = root.join("pak42_dir.vpk");

        assert!(is_map_mod_vpk(&missing).is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
