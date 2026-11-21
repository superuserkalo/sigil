//! Pack and Test: one app action that validates and packs a project's staging folder into a
//! throwaway build, shows the install plan, and installs it into an addon slot once the player
//! confirms.
//!
//! - Deadlock Mod Manager (`feature/modding-workbench`, behaviour only): close the game, pack,
//!   install into a slot and open the addon folder.
//! - Sigil: [`prepare`] writes only inside the project (`.dah/test-build`). [`install`] is the
//!   one write to the game install; it is called from the desktop app's confirm dialog, refuses
//!   while Deadlock runs (Sigil does not close the game for you) and goes through the same
//!   [`apply_deploy`] as a release, with its backups. No agent tool reaches this module.

use std::fs;
use std::path::{Path, PathBuf};

use crate::addons::list_installed;
use crate::deploy::{DeployPlan, DeployResult, apply_deploy, plan_deploy};
use crate::error::{Error, Result};
use crate::payload::{PayloadOptions, format_payload_issues, has_errors, validate_payload};
use crate::project::{ModProject, ProjectHistoryEntry, read_project, write_project};
use crate::release::ReleaseManifest;
use crate::vpk_write::{PackOptions, collect_staging_entries, pack_vpk};

const TEST_BUILD: &str = ".dah/test-build";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestBuild {
    pub dir: PathBuf,
    pub manifest: ReleaseManifest,
    pub plan: DeployPlan,
    /// Payload warnings, as `entry: message`.
    pub warnings: Vec<String>,
}

/// The slot a test build goes to: where this project is installed, else its own slot when free,
/// else the first free one. A test never takes another mod's slot by default.
pub fn choose_slot(project: &ModProject, occupied: &[u16]) -> Result<u16> {
    if let Some(slot) = project.deployed_slot.filter(|slot| (1..=99).contains(slot)) {
        return Ok(slot);
    }
    if let Some(slot) = project.slot.filter(|slot| (1..=99).contains(slot) && !occupied.contains(slot)) {
        return Ok(slot);
    }
    (1..=99)
        .find(|slot| !occupied.contains(slot))
        .ok_or_else(|| Error::msg("Every addon slot from 01 to 99 is taken. Free one first."))
}

/// Validate and pack the staging folder, then plan the install. Writes inside the project only.
pub fn prepare(project_dir: &Path, game_root: &Path) -> Result<TestBuild> {
    let project = read_project(project_dir)?;
    let staging = project_dir.join("staging");
    if !staging.is_dir() {
        return Err(Error::msg("This project has no staging folder yet. Compile or export files into it first."));
    }
    let entries = collect_staging_entries(&staging)?;
    let issues = validate_payload(
        &entries,
        &PayloadOptions {
            staging_folder: Some("staging".to_string()),
        },
    );
    if has_errors(&issues) {
        return Err(Error::msg(format!(
            "The payload has errors, so nothing was packed.\n{}",
            format_payload_issues(&issues)
        )));
    }
    let warnings = issues
        .iter()
        .map(|issue| match &issue.entry {
            Some(entry) => format!("{entry}: {}", issue.message),
            None => issue.message.clone(),
        })
        .collect();

    let occupied: Vec<u16> = list_installed(game_root)?.iter().map(|addon| addon.slot).collect();
    let slot = choose_slot(&project, &occupied)?;

    let dir = project_dir.join(TEST_BUILD);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    let packed = pack_vpk(&PackOptions::new(staging, dir.clone(), format!("pak{slot:02}")))?;
    let vpk_file = file_name(&packed.dir_path);
    let mut manifest = ReleaseManifest::new(&project.slug, &project.name, &project.version, slot, vpk_file);
    manifest.chunk_files = packed.chunk_paths.iter().map(|path| file_name(path)).collect();
    manifest.entries = packed.entry_count;
    manifest.total_bytes = packed.total_bytes;
    manifest.notes = Some("Pack and Test build".to_string());
    let plan = plan_deploy(game_root, &dir, &manifest, Some(slot))?;
    Ok(TestBuild { dir, manifest, plan, warnings })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Install a prepared build. The app calls this after the player confirmed the plan.
pub fn install(project_dir: &Path, game_root: &Path, build: &TestBuild) -> Result<DeployResult> {
    if crate::launch::game_running() {
        return Err(Error::msg("Close Deadlock first. Nothing was installed."));
    }
    install_checked(project_dir, game_root, build)
}

fn install_checked(project_dir: &Path, game_root: &Path, build: &TestBuild) -> Result<DeployResult> {
    let backups = project_dir.join(".dah").join("backups");
    let result = apply_deploy(game_root, &build.dir, &build.manifest, &build.plan, true, &backups)?;
    let mut project = read_project(project_dir)?;
    project.deployed_slot = Some(build.plan.slot);
    project.deployed_at = Some(build.manifest.created_at.clone());
    project.history.push(ProjectHistoryEntry::new(format!(
        "pack and test into pak{:02}_dir.vpk",
        build.plan.slot
    )));
    write_project(project_dir, &project)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_build_never_takes_another_mods_slot() {
        let mut project = ModProject::new("Test", "test");
        assert_eq!(choose_slot(&project, &[1, 2]).unwrap(), 3);
        project.slot = Some(7);
        assert_eq!(choose_slot(&project, &[1]).unwrap(), 7);
        assert_eq!(choose_slot(&project, &[7]).unwrap(), 1);
        project.deployed_slot = Some(7);
        assert_eq!(choose_slot(&project, &[7]).unwrap(), 7, "its own install is replaced");
        project.deployed_slot = None;
        project.slot = None;
        let full: Vec<u16> = (1..=99).collect();
        assert!(choose_slot(&project, &full).is_err());
    }

    #[test]
    fn prepare_writes_only_in_the_project_and_refuses_a_bad_payload() {
        let base = std::env::temp_dir().join(format!("dah-pack-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let project_dir = base.join("project");
        let game = base.join("game-root");
        fs::create_dir_all(game.join("game/citadel/addons")).unwrap();
        fs::write(
            game.join("game/citadel/gameinfo.gi"),
            "SearchPaths\n{\nGame citadel/addons\nMod citadel\nGame citadel\n}\n",
        )
        .unwrap();
        crate::project::create_project(&project_dir, "Test", "test").unwrap();
        let file = project_dir.join("staging/materials/a.vmat_c");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"compiled").unwrap();

        let build = prepare(&project_dir, &game).unwrap();
        assert_eq!(build.plan.slot, 1);
        assert!(build.dir.join("pak01_dir.vpk").is_file());
        assert!(!game.join("game/citadel/addons/pak01_dir.vpk").exists(), "prepare never installs");

        install_checked(&project_dir, &game, &build).unwrap();
        assert!(game.join("game/citadel/addons/pak01_dir.vpk").is_file());
        assert_eq!(read_project(&project_dir).unwrap().deployed_slot, Some(1));

        fs::write(project_dir.join("staging/materials/b.vmat"), b"source").unwrap();
        assert!(prepare(&project_dir, &game).is_err());
        let _ = fs::remove_dir_all(&base);
    }
}
