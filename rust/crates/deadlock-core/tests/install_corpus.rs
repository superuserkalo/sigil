//! Regression corpus: real GameBanana Deadlock mods, planned against a fake game root.
//!
//! Ignored by default because it downloads from GameBanana (an allowed host). Run it with
//!
//! ```text
//! cd rust && cargo test -p deadlock-core --test install_corpus -- --ignored --nocapture
//! ```
//!
//! Each mod's chosen file lands in `.scratch/install-corpus/<id>/` (gitignored) and is reused on a
//! later run. Nothing is installed: every archive is only planned (download, extract, classify,
//! validate, choose a slot) against an empty fake game. The per-mod result table is printed and
//! written to `.scratch/install-corpus/results.txt`. `SIGIL_CORPUS_ONLY=679331,640378` limits
//! the run to some ids.
//!
//! The list covers every GameBanana Deadlock category and the archive shapes players hit: RAR4
//! and RAR5, 7z, ZIP, VPKs under their own names, `pakNN_dir` names with upload suffixes, loose
//! compiled trees, maps, sounds, HUD, fonts, configs and gameinfo-style mods. Chosen from the
//! GameBanana API on 2026-11-20.

use std::fs;
use std::path::{Path, PathBuf};

use deadlock_core::install::{
    DownloadPolicy, InstallArtifact, InstallRequest, PlanOptions, download, plan_install,
    vpk_choices,
};
use serde_json::Value;

/// `(mod id, file name prefix, what it exercises)`. An empty prefix takes the smallest file. A
/// description starting with `refuse:` is an upload neither Grimoire nor Deadlock Mod Manager
/// installs either (a gameinfo.gi or a .cfg, not a mod): it passes when the plan refuses it with
/// the specific "Nothing in this archive" message instead of a generic error.
const CORPUS: &[(u64, &str, &str)] = &[
    (679331, "josuke", "RAR5 holding one 107 MB self-contained VPK under its own name"),
    (640378, "pak05_dir", "RAR named after a pakNN_dir file"),
    (726666, "pak98_dir", "ZIP with an upload suffix on the pak name"),
    (742587, "pak16_dir", "7z with a pak name"),
    (742140, "pak57_dir", "7z with a pak name and suffix"),
    (664090, "pak19_dir", "2 KB skybox map tweak"),
    (696364, "seven_gangdam", "animation ZIP"),
    (693624, "parry", "animation RAR"),
    (722051, "sinclair_extra_animations_reload", "animation RAR, one of seven variants"),
    (603113, "hp_colors_v370.7z", "HUD 7z"),
    (606414, "final_v4_1_rejuv_timer", "HUD 7z"),
    (664724, "alt_mm_icons_sinners_gold", "1 KB minimap icon ZIP"),
    (743064, "deadlock_roster_gold", "HUD ZIP"),
    (742911, "codexcontra_fonts", "font mod"),
    (742501, "tony_soprano", "portrait RAR"),
    (648722, "pak69_smokin", "RAR with a pak name prefix"),
    (615358, "pak09_dir", "keyboard overlay ZIP"),
    (742339, "pak23_dir", "audio replacement"),
    (736537, "custom_hitsound", "hit sound"),
    (739521, "rem_no_pain_sounds_all", "sound removal"),
    (739148, "dlforge_sound", "DL Forge sound export"),
    (737700, "classic_valve_intro", "intro movie replacement"),
    (659570, "pak06_dir", "map RAR"),
    (669521, "jump_control", "map RAR"),
    (736994, "pkfr_cobbletowers", "map ZIP"),
    (729005, "flat", "map RAR"),
    (738133, "opustrainer", "map ZIP"),
    (627008, "tetocontainer", "model replacement ZIP"),
    (742297, "pixil_soul_container", "model replacement ZIP"),
    (741525, "sata_andagi", "hideout painting RAR"),
    (743083, "paige_074ff", "skin ZIP"),
    (743094, "sogeking", "skin RAR"),
    (630011, "fuck_you_parry", "5 KB parry sound"),
    (672151, "vinny-circle.zip", "1 KB scope tweak"),
    (722853, "all_c7614", "clutter removal pack"),
    (736352, "fov_slider_updated", "FOV slider ZIP"),
    (727968, "auto_installer", "refuse: FOV auto installer RAR"),
    (616141, "gameinfo_70", "refuse: gameinfo-style config mod"),
    (650519, "optilock_-_standard", "refuse: config RAR"),
    (656341, "sqooky_s_performance_config_v4_0", "refuse: config 7z"),
    (690233, "optilock_fps_configs", "refuse: config ZIP"),
    (742368, "multichatwheels", "refuse: chat wheel cfg ZIP"),
    (742373, "commend-all", "console cfg ZIP"),
    (741526, "littlemore", "crosshair RAR"),
    (738726, "better-icons-camps.7z", "1 KB minimap 7z"),
    (602711, "see_stats", "HUD 7z"),
    (601444, "standalone_passive_items_10_31", "HUD 7z"),
    (667277, "mapinfotemplate", "refuse: 1 KB template ZIP"),
    (682834, "fixbaba", "video settings ZIP"),
    (629803, "hudlock", "HUD mega mod RAR"),
    (615669, "main_c097a", "health bar HUD"),
    (637275, "singlebarnonnumeric", "health bar variant"),
    (742261, "oled-care-hud_40", "HUD variant of five"),
    (742083, "karin_sounds", "voice and sound RAR"),
    (742343, "glebasi", "HUD RAR"),
    (742435, "vic_typography", "typography rework"),
    (738558, "disable_doorway", "QoL RAR"),
    (741878, "menu-vyper", "main menu ZIP"),
    (725437, "build_gem", "Panorama script QoL"),
    (738405, "pro_builds", "Panorama script QoL"),
    (739609, "babeltower", "chat translator"),
    (736238, "autotranslator", "chat translator"),
    (740903, "party-check", "queue guard"),
    (742883, "stable", "Telelock"),
];

fn repo_root() -> PathBuf {
    let mut current = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if current.join("AGENTS.md").is_file() {
            return current;
        }
        assert!(current.pop(), "repo root not found");
    }
}

fn fake_game(root: &Path) -> PathBuf {
    let game = root.join("fake-game").join("Deadlock");
    let citadel = game.join("game").join("citadel");
    fs::create_dir_all(citadel.join("addons")).unwrap();
    let stock = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stock-gameinfo.gi");
    fs::copy(stock, citadel.join("gameinfo.gi")).unwrap();
    game
}

/// The GameBanana file a corpus row names: `(download url, file name)`.
fn pick_file(id: u64, prefix: &str) -> Result<(String, String), String> {
    let url = format!("https://gamebanana.com/apiv11/Mod/{id}/ProfilePage");
    let body = deadlock_core::net::get(&url)
        .header("User-Agent", "sigil-install-corpus")
        .call()
        .map_err(|error| format!("profile: {error}"))?
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("profile body: {error}"))?;
    let value: Value = serde_json::from_str(&body).map_err(|error| format!("profile json: {error}"))?;
    let files = value["_aFiles"].as_array().ok_or("no _aFiles")?;
    let mut rows: Vec<(String, String, u64)> = files
        .iter()
        .filter_map(|file| {
            Some((
                file["_sDownloadUrl"].as_str()?.to_string(),
                file["_sFile"].as_str()?.to_string(),
                file["_nFilesize"].as_u64().unwrap_or(0),
            ))
        })
        .collect();
    rows.sort_by_key(|row| row.2);
    rows.into_iter()
        .find(|(_, name, _)| prefix.is_empty() || name.to_lowercase().starts_with(&prefix.to_lowercase()))
        .map(|(url, name, _)| (url, name))
        .ok_or_else(|| format!("no file starting with {prefix:?}"))
}

fn fetch(id: u64, prefix: &str, folder: &Path) -> Result<PathBuf, String> {
    let downloads = folder.join("file");
    if let Some(existing) = fs::read_dir(&downloads)
        .ok()
        .and_then(|mut entries| entries.find_map(|entry| entry.ok().map(|entry| entry.path())))
        .filter(|path| path.is_file() && !path.to_string_lossy().ends_with(".part"))
    {
        return Ok(existing);
    }
    let (url, _) = pick_file(id, prefix)?;
    fs::create_dir_all(&downloads).map_err(|error| error.to_string())?;
    let report = download(&url, &downloads, &DownloadPolicy::gamebanana()).map_err(|error| error.to_string())?;
    Ok(report.path)
}

/// One line of the result table: the plan's verdict for one archive.
fn plan_one(archive: &Path, workspace: &Path, game: &Path) -> Result<String, String> {
    let request = InstallRequest::new(archive.display().to_string(), workspace.to_path_buf())
        .map_err(|error| error.to_string())?;
    let options = PlanOptions { game_root: Some(game.to_path_buf()), ..Default::default() };
    let describe = |plan: &deadlock_core::install::InstallPlan| {
        let artifact = match &plan.artifact {
            InstallArtifact::Vpk { chunk_paths, .. } => format!("vpk+{} chunks", chunk_paths.len()),
            InstallArtifact::Staging { .. } => "packed tree".to_string(),
        };
        format!("{} files, {artifact}, {} warning(s)", plan.files.len(), plan.warnings.len())
    };
    match plan_install(&request, &options) {
        Ok(plan) => Ok(describe(&plan)),
        Err(error) if error.to_string().contains("Allow script mods") => {
            // The owner's setting, off by default: with it on, the mod must install.
            deadlock_core::payload::set_allow_scripts(true);
            let allowed = plan_install(&request, &options);
            deadlock_core::payload::set_allow_scripts(false);
            allowed
                .map(|plan| format!("script mod, installs with Allow script mods on: {}", describe(&plan)))
                .map_err(|error| error.to_string())
        }
        Err(error) if error.to_string().contains("more than one VPK") => {
            let choices = vpk_choices(&request, &options).map_err(|error| error.to_string())?;
            let first = choices.first().ok_or("picker offered no VPK")?;
            let plan = plan_install(&request.clone().with_vpk(Some(first.path.clone())), &options)
                .map_err(|error| format!("picked {}: {error}", first.path))?;
            Ok(format!("picker of {}; first: {}", choices.len(), describe(&plan)))
        }
        Err(error) => Err(error.to_string()),
    }
}

#[test]
#[ignore = "downloads real mods from GameBanana"]
fn real_gamebanana_mods_plan_cleanly() {
    let root = repo_root().join(".scratch").join("install-corpus");
    fs::create_dir_all(&root).unwrap();
    let only: Vec<u64> = std::env::var("SIGIL_CORPUS_ONLY")
        .ok()
        .map(|value| value.split(',').filter_map(|id| id.trim().parse().ok()).collect())
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut failures = 0;
    for (id, prefix, what) in CORPUS {
        if !only.is_empty() && !only.contains(id) {
            continue;
        }
        let folder = root.join(id.to_string());
        let workspace = folder.join("workspace");
        let _ = fs::remove_dir_all(&workspace);
        let _ = fs::remove_dir_all(folder.join("fake-game"));
        let game = fake_game(&folder);
        let verdict = fetch(*id, prefix, &folder).and_then(|archive| {
            let name = archive.file_name().unwrap().to_string_lossy().into_owned();
            plan_one(&archive, &workspace, &game).map(|ok| format!("{name}: {ok}")).map_err(|error| format!("{name}: {error}"))
        });
        let _ = fs::remove_dir_all(&workspace);
        let verdict = match (what.starts_with("refuse:"), verdict) {
            (false, verdict) => verdict,
            (true, Ok(text)) => Err(format!("installed, but it is no mod: {text}")),
            (true, Err(text)) if text.contains("Nothing in this archive") => Ok(format!("refused clearly: {text}")),
            (true, Err(text)) => Err(text),
        };
        let line = match verdict {
            Ok(text) => format!("PASS {id} ({what}) {text}"),
            Err(text) => {
                failures += 1;
                format!("FAIL {id} ({what}) {}", text.replace('\n', " | "))
            }
        };
        println!("{line}");
        lines.push(line);
    }
    lines.push(format!("{} of {} passed", lines.len() - failures, lines.len()));
    fs::write(root.join("results.txt"), lines.join("\n") + "\n").unwrap();
    println!("{}", lines.last().unwrap());
    assert_eq!(failures, 0, "{failures} corpus mod(s) failed to plan; see .scratch/install-corpus/results.txt");
}
