//! Fixture tests for the viewer pipeline. No game files: VPKs are packed from small staging
//! trees under `.scratch/viewer-test`. Tests that need the real game are `#[ignore]` and read
//! `SIGIL_GAME_ROOT`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use super::cache::{Staging, key, prune_cache};
use super::clips::{ClipCandidate, Playback, select_clips};
use super::hero::{
    HeroExportRequest, HeroSidecar, canonical_clip_name, export_hero, export_hero_with, find_hero,
    heroes, heroes_in_vpk, plan_hero,
};
use super::hideout::{map_light, realtime_light};
use super::space;
use super::stack::{SourceKind, Stack, sources};
use super::{PIPELINE_VERSION, SCHEMA, Variant, is_cancelled};
use crate::vpk_write::{PackOptions, pack_vpk_single};

const INFERNO: &str = "models/heroes_wip/inferno/inferno.vmdl_c";
const HAZE: &str = "models/heroes_staging/haze/haze.vmdl_c";

fn root(tag: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.scratch/viewer-test")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn vpk(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let staging = dir.join(format!(".staging-{name}"));
    let _ = fs::remove_dir_all(&staging);
    for (path, data) in files {
        let target = staging.join(path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(target, data).unwrap();
    }
    let packed =
        pack_vpk_single(&PackOptions::new(staging.clone(), dir.to_path_buf(), name)).unwrap();
    fs::remove_dir_all(&staging).unwrap();
    packed.dir_path
}

/// A fake install: base and core VPKs, two enabled addons and one disabled one.
fn game(tag: &str) -> PathBuf {
    let root = root(tag);
    let citadel = root.join("game").join("citadel");
    vpk(
        &citadel,
        "pak01",
        &[
            (INFERNO, b"base inferno"),
            (HAZE, b"base haze"),
            (
                "materials/models/heroes_wip/inferno/body_color.vtex_c",
                b"base texture",
            ),
            ("sounds/ui/click.vsnd_c", b"base sound"),
        ],
    );
    vpk(
        &root.join("game").join("core"),
        "pak01",
        &[("materials/default.vtex_c", b"core")],
    );
    let addons = citadel.join("addons");
    vpk(
        &addons,
        "pak01",
        &[(
            "materials/models/heroes_wip/inferno/body_color.vtex_c",
            b"slot one texture",
        )],
    );
    vpk(
        &addons,
        "pak02",
        &[
            ("sounds/ui/click.vsnd_c", b"slot two sound"),
            (
                "materials/models/heroes_wip/inferno/body_color.vtex_c",
                b"slot two texture",
            ),
        ],
    );
    let haze = vpk(&addons, "pak03", &[(HAZE, b"haze skin")]);
    fs::rename(&haze, addons.join("pak03_dir.vpk.disabled")).unwrap();
    root
}

#[test]
fn viewer_sources_follow_game_precedence() {
    let root = game("sources");
    let modded = sources(&root, Variant::Modded, None);
    let files: Vec<&str> = modded.iter().map(|source| source.file.as_str()).collect();
    assert_eq!(
        files,
        [
            "addons/pak01_dir.vpk",
            "addons/pak02_dir.vpk",
            "pak01_dir.vpk",
            "core/pak01_dir.vpk"
        ]
    );
    assert_eq!(modded[0].slot, Some(1));
    assert_eq!(modded[2].kind, SourceKind::Base);
    assert_eq!(modded[3].kind, SourceKind::Core);

    let preview = sources(&root, Variant::Modded, Some(&[3]));
    let slots: Vec<Option<u16>> = preview.iter().map(|source| source.slot).collect();
    assert_eq!(slots, [Some(1), Some(2), Some(3), None, None]);
    assert!(!preview[2].enabled);
    assert_eq!(preview[2].file, "addons/pak03_dir.vpk.disabled");

    let original = sources(&root, Variant::Original, Some(&[3]));
    assert!(original.iter().all(|source| !source.kind.is_mod()));
    assert_eq!(original.len(), 2);
}

#[test]
fn viewer_stack_resolves_lower_slot_first_and_counts_reads() {
    let root = game("stack");
    let stack = Stack::open(&sources(&root, Variant::Modded, None)).unwrap();
    let texture = "materials/models/heroes_wip/inferno/body_color.vtex_c";
    assert_eq!(stack.read(texture).unwrap(), b"slot one texture");
    // Paths compare case-insensitively, as the engine's do.
    assert_eq!(
        stack.read(&texture.to_ascii_uppercase()).unwrap(),
        b"slot one texture"
    );
    assert_eq!(
        stack.read("sounds/ui/click.vsnd_c").unwrap(),
        b"slot two sound"
    );
    assert_eq!(stack.read(INFERNO).unwrap(), b"base inferno");
    assert_eq!(
        stack
            .read_from(stack.first_game_layer(), texture)
            .unwrap()
            .0,
        b"base texture"
    );
    assert!(stack.read("missing.vtex_c").is_none());

    let usage = stack.usage();
    assert_eq!(usage[0].used, 1 + 1);
    assert_eq!(usage[1].used, 1);
    assert_eq!(usage[2].used, 2);
    assert_eq!(usage[3].used, 0);

    let listed = stack.list("materials/models/heroes_wip/inferno/", ".vtex_c");
    assert_eq!(listed.get(texture), Some(&0));
}

#[test]
fn viewer_heroes_fall_back_to_reviewed_model_entries() {
    // The fixture has no heroes.vdata_c, so the roster comes from Grimoire's reviewed table,
    // limited to models the base VPK ships.
    let root = game("heroes");
    let list = heroes(&root).unwrap();
    let names: Vec<&str> = list.iter().map(|hero| hero.display_name.as_str()).collect();
    assert_eq!(names, ["Haze", "Infernus"]);
    assert_eq!(list[1].model_entry, INFERNO);
    assert_eq!(
        find_hero(&root, "inferno").unwrap().display_name,
        "Infernus"
    );
    assert_eq!(find_hero(&root, "Infernus").unwrap().codename, "inferno");
    assert!(find_hero(&root, "nobody").is_err());
}

#[test]
fn viewer_heroes_in_vpk_names_the_skinned_heroes() {
    let root = game("in-vpk");
    let addons = root.join("game/citadel/addons");
    assert_eq!(
        heroes_in_vpk(&addons.join("pak01_dir.vpk")).unwrap(),
        ["inferno"]
    );
    assert_eq!(
        heroes_in_vpk(&addons.join("pak03_dir.vpk.disabled")).unwrap(),
        ["haze"]
    );
    let sounds = vpk(
        &root,
        "sounds",
        &[("sounds/heroes/inferno/fire.vsnd_c", b"x")],
    );
    assert!(heroes_in_vpk(&sounds).unwrap().is_empty());
}

#[test]
fn viewer_plan_keeps_only_mods_that_change_the_hero() {
    let root = game("plan");
    let cache = root.join("cache");
    let inferno = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "inferno",
        Variant::Modded,
    ))
    .unwrap();
    let files: Vec<&str> = inferno
        .sources
        .iter()
        .map(|source| source.file.as_str())
        .collect();
    assert_eq!(
        files,
        [
            "addons/pak01_dir.vpk",
            "addons/pak02_dir.vpk",
            "pak01_dir.vpk",
            "core/pak01_dir.vpk"
        ]
    );
    assert_eq!(inferno.variant, Variant::Modded);

    // No enabled mod touches Haze: the modded preview is the original, one cache entry.
    let haze_modded = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "haze",
        Variant::Modded,
    ))
    .unwrap();
    let haze_original = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "haze",
        Variant::Original,
    ))
    .unwrap();
    assert_eq!(haze_modded.variant, Variant::Original);
    assert_eq!(haze_modded.key, haze_original.key);

    // Previewing the disabled Haze skin brings it in.
    let mut preview = HeroExportRequest::new(&root, &cache, "haze", Variant::Modded);
    preview.extra_slots = &[3];
    let haze_preview = plan_hero(&preview).unwrap();
    assert_eq!(haze_preview.variant, Variant::Modded);
    assert_ne!(haze_preview.key, haze_original.key);
    assert_eq!(haze_preview.sources[0].slot, Some(3));

    // Leaving out an enabled slot shows the hero without that mod.
    let mut without = HeroExportRequest::new(&root, &cache, "inferno", Variant::Modded);
    without.exclude_slots = &[1];
    let inferno_without = plan_hero(&without).unwrap();
    let slots: Vec<Option<u16>> = inferno_without
        .sources
        .iter()
        .map(|source| source.slot)
        .collect();
    assert_eq!(slots, [Some(2), None, None]);
    assert_ne!(inferno_without.key, inferno.key);
    without.exclude_slots = &[1, 2];
    let inferno_bare = plan_hero(&without).unwrap();
    assert_eq!(inferno_bare.variant, Variant::Original);
}

fn sidecar(codename: &str, key: &str) -> HeroSidecar {
    HeroSidecar {
        schema: SCHEMA,
        hero: codename.into(),
        display_name: "Infernus".into(),
        variant: Variant::Modded,
        clips: vec!["shop_menu_base".into()],
        idle_clip: Some("shop_menu_base".into()),
        sources: Vec::new(),
        model_entry: INFERNO.into(),
        clip_info: Vec::new(),
        cloth: None,
        particles: None,
        units: "meters".into(),
        up: "+Y".into(),
        forward: "+Z".into(),
        key: key.into(),
        issues: Vec::new(),
        pipeline: PIPELINE_VERSION.into(),
    }
}

#[test]
fn viewer_export_returns_the_cache_without_decoding() {
    let root = game("cache-hit");
    let cache = root.join("cache");
    let plan = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "inferno",
        Variant::Modded,
    ))
    .unwrap();
    let entry = cache.join("heroes").join(&plan.key);
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("inferno.glb"), b"glTF").unwrap();
    fs::write(
        entry.join("inferno.json"),
        serde_json::to_vec(&sidecar("inferno", &plan.key)).unwrap(),
    )
    .unwrap();

    let never = AtomicBool::new(false);
    let found = export_hero(&root, &cache, "Infernus", Variant::Modded, &[], &never).unwrap();
    assert!(found.cached);
    assert_eq!(found.key, plan.key);
    assert_eq!(found.glb, entry.join("inferno.glb"));
    assert_eq!(found.sidecar, entry.join("inferno.json"));

    // A changed mod is a new key: the fixture's model bytes are not a model, so the export
    // now really decodes and fails, proving the cache was not reused.
    std::thread::sleep(Duration::from_millis(20));
    let addons = root.join("game/citadel/addons");
    vpk(
        &addons,
        "pak01",
        &[(
            "materials/models/heroes_wip/inferno/body_color.vtex_c",
            b"slot one texture v2",
        )],
    );
    let error = export_hero(&root, &cache, "inferno", Variant::Modded, &[], &never).unwrap_err();
    assert!(!is_cancelled(&error));
    assert!(error.to_string().contains("decoding"), "{error}");
    // The failed export left no entry and no staging folder behind.
    let left: Vec<String> = fs::read_dir(cache.join("heroes"))
        .unwrap()
        .flatten()
        .map(|child| child.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, std::slice::from_ref(&plan.key));
}

#[test]
fn viewer_cache_hit_refreshes_mod_names() {
    let root = game("names");
    let cache = root.join("cache");
    let plan = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "inferno",
        Variant::Modded,
    ))
    .unwrap();
    let entry = cache.join("heroes").join(&plan.key);
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("inferno.glb"), b"glTF").unwrap();
    let mut stale = sidecar("inferno", &plan.key);
    stale.sources = vec![super::SourceUsage {
        slot: Some(1),
        file: "addons/pak01_dir.vpk".into(),
        name: "old name".into(),
        kind: SourceKind::Addon,
        enabled: true,
        used: 3,
    }];
    fs::write(
        entry.join("inferno.json"),
        serde_json::to_vec(&stale).unwrap(),
    )
    .unwrap();

    let size = fs::metadata(root.join("game/citadel/addons/pak01_dir.vpk"))
        .unwrap()
        .len();
    let library = crate::import::Library {
        schema: 1,
        mods: vec![crate::import::ModMeta {
            slot: 1,
            size_bytes: size,
            name: "Ember Infernus".into(),
            source: "browse".into(),
            ..Default::default()
        }],
    };
    let mut request = HeroExportRequest::new(&root, &cache, "inferno", Variant::Modded);
    request.library = Some(&library);
    let found = export_hero_with(&request, &AtomicBool::new(false)).unwrap();
    assert!(found.cached);
    let written: HeroSidecar = serde_json::from_slice(&fs::read(found.sidecar).unwrap()).unwrap();
    assert_eq!(written.sources[0].name, "Ember Infernus");
}

#[test]
fn viewer_cancel_flag_stops_an_export() {
    let root = game("cancel");
    let cancel = AtomicBool::new(true);
    let error = export_hero(
        &root,
        &root.join("cache"),
        "inferno",
        Variant::Original,
        &[],
        &cancel,
    )
    .unwrap_err();
    assert!(is_cancelled(&error));
    let error = super::export_hideout(&root, &root.join("cache"), Variant::Original, None, &cancel)
        .unwrap_err();
    assert!(is_cancelled(&error));
}

#[test]
fn viewer_staging_commits_whole_folders_once() {
    let root = root("staging");
    let first = Staging::begin(&root, "abc").unwrap();
    fs::write(first.dir.join("a.glb"), b"first").unwrap();
    let second = Staging::begin(&root, "abc").unwrap();
    fs::write(second.dir.join("a.glb"), b"second").unwrap();
    assert!(!root.join("abc").exists());
    let committed = first.commit().unwrap();
    assert_eq!(fs::read(committed.join("a.glb")).unwrap(), b"first");
    // A racing export of the same key keeps the first copy and cleans up after itself.
    let again = second.commit().unwrap();
    assert_eq!(again, committed);
    assert_eq!(fs::read(again.join("a.glb")).unwrap(), b"first");
    let dropped = Staging::begin(&root, "def").unwrap();
    let dropped_dir = dropped.dir.clone();
    drop(dropped);
    assert!(!dropped_dir.exists());
    let names: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|child| child.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["abc"]);
}

fn aged_entry(dir: &Path, bytes: usize, age: Duration) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("model.glb"), vec![0u8; bytes]).unwrap();
    let marker = dir.join(".used");
    fs::write(&marker, b"").unwrap();
    let file = fs::File::options().write(true).open(&marker).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

#[test]
fn viewer_prune_removes_least_recently_used_first() {
    let root = root("prune");
    let hour = Duration::from_secs(3600);
    aged_entry(&root.join("heroes/old"), 1000, hour * 48);
    aged_entry(&root.join("heroes/middle"), 1000, hour * 24);
    aged_entry(&root.join("hideout/new"), 1000, hour);
    aged_entry(&root.join("heroes/open"), 1000, Duration::from_secs(10));

    let report = prune_cache(&root, 2500).unwrap();
    assert_eq!(report.removed, 2);
    assert!(!root.join("heroes/old").exists());
    assert!(!root.join("heroes/middle").exists());
    assert!(root.join("hideout/new").exists());
    assert!(root.join("heroes/open").exists());
    assert!(report.kept_bytes <= 2500);

    // An entry in use stays even past the budget.
    let report = prune_cache(&root, 0).unwrap();
    assert_eq!(report.removed, 1);
    assert!(root.join("heroes/open").exists());
}

#[test]
fn viewer_cache_keys_are_stable_and_ordered() {
    let a = key(&["a".into(), "b".into()]);
    assert_eq!(a.len(), 24);
    assert_eq!(a, key(&["a".into(), "b".into()]));
    assert_ne!(a, key(&["b".into(), "a".into()]));
    assert_ne!(a, key(&["ab".into()]));
}

#[test]
fn viewer_space_matches_morphic_model_wrapper() {
    use morphic::model::{Mat4, Quat, Vec3};
    use std::f32::consts::FRAC_PI_2;
    // morphic's own Source to glTF node transform (row vectors, row major).
    let wrapper = Mat4::from_scale(0.0254).mul(&Mat4::from_quaternion(Quat::from_yaw_pitch_roll(
        0.0, -FRAC_PI_2, -FRAC_PI_2,
    )));
    for source in [
        [100.0, 0.0, 0.0],
        [0.0, 100.0, 0.0],
        [0.0, 0.0, 100.0],
        [12.0, -34.0, 56.0],
    ] {
        let theirs = wrapper.transform_point(Vec3 {
            x: source[0] as f32,
            y: source[1] as f32,
            z: source[2] as f32,
        });
        let ours = space::point(source);
        assert!(
            (theirs.x as f64 - ours[0]).abs() < 1e-4,
            "{source:?}: {theirs:?} vs {ours:?}"
        );
        assert!(
            (theirs.y as f64 - ours[1]).abs() < 1e-4,
            "{source:?}: {theirs:?} vs {ours:?}"
        );
        assert!(
            (theirs.z as f64 - ours[2]).abs() < 1e-4,
            "{source:?}: {theirs:?} vs {ours:?}"
        );
    }
}

#[test]
fn viewer_placements_convert_to_gltf() {
    // A Source placement 100 inches forward, turned 90 degrees left.
    let source = space::entity_matrix([100.0, 0.0, 0.0], [0.0, 90.0, 0.0], [1.0; 3]);
    let gltf = space::to_gltf(&source);
    // Translation: Source +X is glTF +Z, in meters.
    assert!((gltf[12]).abs() < 1e-9 && (gltf[13]).abs() < 1e-9 && (gltf[14] - 2.54).abs() < 1e-9);
    // The model's forward (glTF +Z, Source +X) turned to Source +Y, which is glTF +X.
    let forward = [gltf[8], gltf[9], gltf[10]];
    assert!(
        (forward[0] - 1.0).abs() < 1e-9 && forward[1].abs() < 1e-9 && forward[2].abs() < 1e-9,
        "{forward:?}"
    );
    // Up stays up and scale stays one: S * M * S^-1 is a rigid motion here.
    assert!((gltf[5] - 1.0).abs() < 1e-9);
    let identity = space::to_gltf(&space::IDENTITY);
    assert!(
        identity
            .iter()
            .zip(space::IDENTITY)
            .all(|(a, b)| (a - b).abs() < 1e-12)
    );
    let rows = [
        [1.0, 0.0, 0.0, 5.0],
        [0.0, 1.0, 0.0, 6.0],
        [0.0, 0.0, 1.0, 7.0],
    ];
    let matrix = space::rows_matrix(&rows);
    assert_eq!(&matrix[12..15], &[5.0, 6.0, 7.0]);
}

#[test]
fn viewer_hideout_keeps_realtime_and_fog_lights() {
    assert!(realtime_light(2.0, -1.0));
    assert!(!realtime_light(0.0, 1.0));
    assert!(!realtime_light(1.0, -1.0));
    assert!(realtime_light(1.0, 2.0));

    let entity = |json: serde_json::Value| json.as_object().unwrap().clone();
    let barn = map_light(&entity(serde_json::json!({
        "classname": "light_barn", "origin": "100 200 300", "angles": [0, 90, 0], "color": [255, 128, 0],
        "range": 500, "brightness_lumens": 1000, "inner_angle": 20, "outer_angle": 45, "directlight": 2,
    })))
    .unwrap();
    assert_eq!(barn.kind, "spot");
    assert_eq!(barn.cone, Some([20.0, 45.0]));
    assert!((barn.range - 12.7).abs() < 1e-4);
    assert!((barn.position[0] - 5.08).abs() < 1e-4 && (barn.position[1] - 7.62).abs() < 1e-4);
    // Facing Source +Y, which is glTF +X.
    assert!((barn.direction[0] - 1.0).abs() < 1e-4);
    assert!((barn.intensity - (1000.0 / (4.0 * std::f32::consts::PI))).abs() < 1e-2);
    assert!((barn.color[1] - 128.0 / 255.0).abs() < 1e-4);

    let baked = serde_json::json!({ "classname": "light_omni2", "origin": [0, 0, 0], "color": [1, 1, 1], "range": 10, "directlight": 1 });
    assert!(map_light(&entity(baked.clone())).is_none());
    let mut fog = baked;
    fog["fog"] = serde_json::json!(1);
    fog["fogstrength"] = serde_json::json!(0.5);
    let fog_light = map_light(&entity(fog)).unwrap();
    assert!(!fog_light.realtime);
    assert_eq!(fog_light.fog_strength, Some(0.5));
    assert!(
        map_light(&entity(
            serde_json::json!({ "classname": "prop_static", "origin": [0, 0, 0] })
        ))
        .is_none()
    );
}

fn clip(name: &str, frames: usize) -> ClipCandidate {
    ClipCandidate::new(name, frames, 30.0)
}

#[test]
fn viewer_clip_selection_follows_grimoire_recipes() {
    let offered = [
        clip("primary_stand_reload", 60),
        clip("shop_menu_base", 90),
        clip("aim_layer_idle", 30),
        clip("primary_stand_idle", 45),
    ];
    let haze = select_clips(&offered, "Haze");
    let names: Vec<&str> = haze.iter().map(|choice| choice.name.as_str()).collect();
    assert_eq!(names, ["shop_menu_base", "primary_stand_reload"]);
    assert_eq!(haze[1].playback, Playback::Hold);
    assert_eq!(
        super::clips::default_clip(&haze, "Haze").as_deref(),
        Some("shop_menu_base")
    );

    // An unknown hero gets the default standing idle. Layer clips never qualify.
    let other = select_clips(&offered, "Somebody New");
    assert_eq!(other[0].name, "primary_stand_idle");
    assert!(!other.iter().any(|choice| choice.name.starts_with("aim")));

    // No recipe clip: the menu pose vpkmerge would bake, held.
    let posed = select_clips(&[clip("ui_hero_pose", 1)], "Haze");
    assert_eq!(posed[0].name, "ui_hero_pose");
    assert_eq!(posed[0].playback, Playback::Hold);

    let mut additive = clip("primary_stand_idle", 40);
    additive.additive = true;
    assert!(select_clips(&[additive], "Somebody New").is_empty());
}

#[test]
fn viewer_clip_file_names_drop_the_hero_prefix() {
    let wanted = super::clips::wanted_names("Yamato");
    assert_eq!(
        canonical_clip_name("yamato_ui_hero_select", "yamato", "yamato", &wanted).as_deref(),
        Some("ui_hero_select")
    );
    assert_eq!(
        canonical_clip_name("primary_stand_idle", "yamato", "yamato", &wanted).as_deref(),
        Some("primary_stand_idle")
    );
    assert_eq!(
        canonical_clip_name("vampirebat_hero_pose", "vampirebat", "vampirebat", &wanted).as_deref(),
        Some("hero_pose")
    );
    assert_eq!(
        canonical_clip_name("yamato_flinch_back", "yamato", "yamato", &wanted),
        None
    );
}

#[test]
fn viewer_clip_files_decode_with_morphic() {
    // morphic's own fixture of a Yamato clip file, when the vendored checkout is present.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../vendor/vpkmerge/morphic/fixtures/nm/yamato_ui_hero_select.vnmclip_c");
    let Ok(bytes) = fs::read(&path) else {
        eprintln!("skipping: {} is missing", path.display());
        return;
    };
    let clip = morphic::model::decode_nm_clip(&bytes).unwrap();
    assert!(clip.frame_count > 1);
    assert!(!clip.additive);
    assert!(clip.skeleton_ref.ends_with(".vnmskel"));
}

#[test]
fn viewer_sidecar_contract_field_names() {
    let json = serde_json::to_value(sidecar("inferno", "k")).unwrap();
    for field in [
        "schema",
        "hero",
        "displayName",
        "variant",
        "clips",
        "idleClip",
        "sources",
        "clipInfo",
        "modelEntry",
        "units",
    ] {
        assert!(json.get(field).is_some(), "missing {field}");
    }
    assert_eq!(json["variant"], "modded");
    assert_eq!(json["schema"], 1);
}

/// Real exports from an installed game. Run with
/// `SIGIL_GAME_ROOT=<Deadlock folder> cargo test -p deadlock-core viewer -- --ignored`.
/// Output goes to `SIGIL_VIEWER_OUT`, else `.scratch/viewer-samples/cache`.
#[test]
#[ignore = "needs the real game: set SIGIL_GAME_ROOT"]
fn viewer_real_game_exports() {
    let Some(game) = std::env::var_os("SIGIL_GAME_ROOT").map(PathBuf::from) else {
        eprintln!("SIGIL_GAME_ROOT is not set");
        return;
    };
    let cache = std::env::var_os("SIGIL_VIEWER_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.scratch/viewer-samples/cache")
        });
    let cancel = AtomicBool::new(false);
    let list = heroes(&game).unwrap();
    assert!(list.len() > 30, "{} heroes", list.len());
    let hero = std::env::var("SIGIL_VIEWER_HERO").unwrap_or_else(|_| "inferno".into());
    for variant in [Variant::Original, Variant::Modded] {
        let started = std::time::Instant::now();
        let first = export_hero(&game, &cache, &hero, variant, &[], &cancel).unwrap();
        let cold = started.elapsed();
        let started = std::time::Instant::now();
        let second = export_hero(&game, &cache, &hero, variant, &[], &cancel).unwrap();
        assert!(second.cached);
        assert_eq!(first.key, second.key);
        let glb = fs::read(&first.glb).unwrap();
        assert_eq!(&glb[..4], b"glTF");
        eprintln!(
            "{variant:?}: {} ({} bytes) cold {cold:?}, warm {:?}",
            first.glb.display(),
            glb.len(),
            started.elapsed()
        );
    }
    let scene =
        super::export_hideout(&game, &cache, Variant::Modded, Some(&hero), &cancel).unwrap();
    let parsed: super::HideoutScene = serde_json::from_slice(&fs::read(&scene).unwrap()).unwrap();
    assert!(!parsed.models.is_empty());
    assert!(parsed.hero.is_some());
    eprintln!(
        "hideout: {} ({} models, {} lights)",
        scene.display(),
        parsed.models.len(),
        parsed.lights.len()
    );
}

fn showcase(state: super::clips::ShowcaseState, playback: Playback) -> super::clips::Showcase {
    super::clips::Showcase {
        graph: "models/heroes_staging/haze/haze_ui.vnmgraph_c".into(),
        state,
        playback,
        speed: 1.25,
    }
}

#[test]
fn viewer_clip_selection_puts_the_showcase_first() {
    use super::clips::{ShowcaseState, default_clip};
    let mut select = clip("haze_select_pose", 80);
    select.showcase = Some(showcase(ShowcaseState::HeroSelect, Playback::Loop));
    let mut shop = clip("haze_shop_pose", 120);
    shop.showcase = Some(showcase(ShowcaseState::Shop, Playback::Hold));
    let mut epic = clip("haze_shop_scene", 2_401);
    epic.showcase = Some(showcase(ShowcaseState::Shop, Playback::Loop));
    let offered = [
        epic,
        select.clone(),
        shop,
        clip("shop_menu_base", 90),
        clip("primary_stand_idle", 45),
        clip("primary_stand_reload", 60),
    ];
    let haze = select_clips(&offered, "Haze");
    let names: Vec<&str> = haze.iter().map(|choice| choice.name.as_str()).collect();
    // The shop showcase (the long one is over the cap), one idle recipe hit as the standing
    // fallback, then the other recipes. No second idle.
    assert_eq!(
        names,
        ["haze_shop_pose", "shop_menu_base", "primary_stand_reload"]
    );
    assert_eq!(haze[0].playback, Playback::Hold);
    assert_eq!(haze[0].speed(), 1.25);
    assert_eq!(haze[0].action, None);
    assert_eq!(haze[2].playback, Playback::Hold);
    assert_eq!(haze[1].speed(), 1.0);
    assert_eq!(
        default_clip(&haze, "Haze").as_deref(),
        Some("haze_shop_pose")
    );

    // Without a shop state the hero-select one is the showcase.
    let only_select = select_clips(&[select, clip("shop_menu_base", 90)], "Haze");
    assert_eq!(only_select[0].name, "haze_select_pose");
    assert_eq!(only_select[0].playback, Playback::Loop);

    // Rem's recipes name no idle: a standing idle joins the showcase.
    let mut rem_shop = clip("familiar_shop", 50);
    rem_shop.showcase = Some(showcase(ShowcaseState::Shop, Playback::Loop));
    let rem = select_clips(
        &[
            rem_shop,
            clip("ui_shop", 40),
            clip("out_of_combat_stand_idle", 30),
            clip("primary_stand_idle", 30),
        ],
        "Rem",
    );
    let names: Vec<&str> = rem.iter().map(|choice| choice.name.as_str()).collect();
    assert_eq!(names, ["familiar_shop", "ui_shop", "primary_stand_idle"]);
}

#[test]
fn viewer_clip_selection_fills_and_rejects_like_grimoire() {
    use super::clips::default_clip;
    // Root motion, hidden, transition and non-standalone clips never qualify.
    let mut moving = clip("primary_stand_idle", 45);
    moving.root_motion = true;
    let mut relaxed = clip("primary_ooc_stand_idle", 45);
    relaxed.hidden = true;
    let mut flagged = clip("stand_idle", 45);
    flagged.standalone = Some(false);
    let mut wave = clip("emote_wave", 60);
    wave.standalone = Some(true);
    let mut bow = clip("emote_bow", 60);
    bow.standalone = Some(true);
    let mut huge = clip("emote_long", 9_990);
    huge.standalone = Some(true);
    let picked = select_clips(
        &[
            moving,
            relaxed,
            flagged,
            wave,
            bow,
            huge,
            clip("weapon_stand_idle", 40),
        ],
        "Somebody New",
    );
    let names: Vec<&str> = picked.iter().map(|choice| choice.name.as_str()).collect();
    // Neutral idle from the default recipes, then standalone clips by name within budget.
    assert_eq!(names, ["weapon_stand_idle", "emote_bow", "emote_wave"]);
    assert_eq!(
        default_clip(&picked, "Somebody New").as_deref(),
        Some("weapon_stand_idle")
    );

    // Fill stops at eight clips.
    let many: Vec<ClipCandidate> = (0..12)
        .map(|index| {
            let mut extra = clip(&format!("emote_{index:02}"), 10);
            extra.standalone = Some(true);
            extra
        })
        .collect();
    assert_eq!(select_clips(&many, "Somebody New").len(), 8);

    // No idle at all: the default clip is the first hero pose, then the first clip.
    let posed = select_clips(&[clip("hero_pose", 30)], "Somebody New");
    assert_eq!(
        default_clip(&posed, "Somebody New").as_deref(),
        Some("hero_pose")
    );
}

#[test]
fn viewer_clip_labels_and_groups() {
    use super::clips::{ClipGroup, clip_group, clip_label};
    assert_eq!(clip_label("shop_menu_base", "Haze"), "Idle");
    assert_eq!(clip_label("primary_stand_reload", "Haze"), "Reload");
    assert_eq!(
        clip_label("primary_ooc_stand_idle", "Mirage"),
        "Relaxed idle"
    );
    assert_eq!(clip_label("ui_main_menu", "Mirage"), "Hero pose");
    assert_eq!(clip_label("item_run_n", "Wraith"), "Run");
    assert_eq!(clip_label("primary_airdash", "Lash"), "Dash");
    assert_eq!(clip_label("melee_hit", "Lash"), "Heavy melee");
    assert_eq!(
        clip_label("models/x/clips/heroShop_pose-v2.vnmclip_c", "Haze"),
        "Hero Shop pose v2"
    );
    assert_eq!(clip_label("__", "Haze"), "__");

    assert_eq!(clip_group("shop_menu_base", "Haze"), ClipGroup::Idle);
    assert_eq!(
        clip_group("primary_stand_reload", "Haze"),
        ClipGroup::Combat
    );
    assert_eq!(clip_group("emote_menu_loop", "Haze"), ClipGroup::Idle);
    assert_eq!(clip_group("item_run_n", "Haze"), ClipGroup::Movement);
    assert_eq!(clip_group("jump2_land", "Haze"), ClipGroup::Movement);
    assert_eq!(clip_group("primary_airdash", "Lash"), ClipGroup::Other);
    assert_eq!(
        serde_json::to_value(ClipGroup::Movement).unwrap(),
        "movement"
    );
}

/// A small compiled UI graph as KV3 JSON: a root state machine over a shop state, a hero
/// select state for the same clip, a select state for another clip, a masked state and an
/// unrelated state.
fn ui_graph() -> serde_json::Value {
    serde_json::json!({
        "m_nRootNodeIdx": 0,
        "m_resources": [
            "models/heroes_staging/haze/clips/haze_shop_pose.vnmclip",
            "models/heroes_staging/haze/clips/haze_select_pose.vnmclip",
            "models/heroes_staging/haze/clips/missing.vnmclip",
            "models/heroes_staging/haze/clips/haze_masked.vnmclip"
        ],
        "m_nodePaths": [
            "root", "root/hero_shop", "root/Hero_Select", "root/select", "root/shop",
            "root/other", "a", "b", "c", "d", "e"
        ],
        "m_nodes": [
            { "_class": "CNmStateMachineNode::CDefinition", "m_nNodeIdx": 0,
              "m_stateDefinitions": [ { "m_nStateNodeIdx": 1 }, { "m_nStateNodeIdx": 2 },
                                      { "m_nStateNodeIdx": 3 }, { "m_nStateNodeIdx": 4 },
                                      { "m_nStateNodeIdx": 5 } ] },
            state(1, 6), state(2, 7), state(3, 8),
            { "_class": "CNmStateNode::CDefinition", "m_nNodeIdx": 4, "m_nChildNodeIdx": 9,
              "m_nLayerWeightNodeIdx": -1, "m_nLayerRootMotionWeightNodeIdx": -1,
              "m_nLayerBoneMaskNodeIdx": 10, "m_bIsOffState": false },
            state(5, 10),
            clip_node(6, 0, false, 1.25),
            clip_node(7, 0, true, 1.0),
            clip_node(8, 1, true, 1.0),
            clip_node(9, 3, true, 1.0),
            { "_class": "CNmClipNode::CDefinition", "m_nNodeIdx": 10, "m_nDataSlotIdx": 2,
              "m_bAllowLooping": true, "m_nPlayInReverseValueNodeIdx": 3,
              "m_nResetTimeValueNodeIdx": -1, "m_flSpeedMultiplier": 1.0 }
        ]
    })
}

fn state(index: i64, child: i64) -> serde_json::Value {
    serde_json::json!({
        "_class": "CNmStateNode::CDefinition", "m_nNodeIdx": index, "m_nChildNodeIdx": child,
        "m_nLayerWeightNodeIdx": -1, "m_nLayerRootMotionWeightNodeIdx": -1,
        "m_nLayerBoneMaskNodeIdx": -1, "m_bIsOffState": false
    })
}

fn clip_node(index: i64, slot: i64, looping: bool, speed: f64) -> serde_json::Value {
    serde_json::json!({
        "_class": "CNmClipNode::CDefinition", "m_nNodeIdx": index, "m_nDataSlotIdx": slot,
        "m_bAllowLooping": looping, "m_nPlayInReverseValueNodeIdx": -1,
        "m_nResetTimeValueNodeIdx": -1, "m_flSpeedMultiplier": speed
    })
}

#[test]
fn viewer_ui_graph_reader_finds_shop_and_select_states() {
    use super::clips::{ModelRefs, ShowcaseState, graph_showcases, read_graphs};
    let available = |path: &str| !path.contains("missing");
    let graph = ui_graph();
    let found = graph_showcases(&graph, "g/ui.vnmgraph_c", &available);
    assert_eq!(found.len(), 2, "{found:?}");
    let (shop_path, shop) = &found[0];
    assert_eq!(
        shop_path,
        "models/heroes_staging/haze/clips/haze_shop_pose.vnmclip_c"
    );
    // Shop wins over the hero select state that plays the same clip.
    assert_eq!(shop.state, ShowcaseState::Shop);
    assert_eq!(shop.playback, Playback::Hold);
    assert_eq!(shop.speed, 1.25);
    assert_eq!(shop.graph, "g/ui.vnmgraph_c");
    let (select_path, select) = &found[1];
    assert!(select_path.ends_with("haze_select_pose.vnmclip_c"));
    assert_eq!(select.state, ShowcaseState::HeroSelect);
    assert_eq!(select.playback, Playback::Loop);

    // Every graph lends its resources; only the `ui` graph is mined, and it goes first.
    let refs = ModelRefs {
        graphs: vec![
            ("default".into(), "g/default.vnmgraph".into()),
            ("ui".into(), "g/ui.vnmgraph".into()),
        ],
        skeletons: Vec::new(),
    };
    let mut decoded = Vec::new();
    let clips = read_graphs(
        &refs,
        |path| {
            decoded.push(path.to_string());
            match path {
                "g/ui.vnmgraph_c" => Some(ui_graph()),
                "g/default.vnmgraph_c" => Some(serde_json::json!({
                    "m_resources": ["models/heroes_staging/haze/clips/primary_stand_idle.vnmclip",
                                    "models/heroes_staging/haze/clips/haze_shop_pose.vnmclip"]
                })),
                _ => None,
            }
        },
        &available,
    );
    assert_eq!(decoded, ["g/ui.vnmgraph_c", "g/default.vnmgraph_c"]);
    let names: Vec<&str> = clips
        .resources
        .iter()
        .map(|path| path.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "haze_shop_pose.vnmclip_c",
            "haze_select_pose.vnmclip_c",
            "haze_masked.vnmclip_c",
            "primary_stand_idle.vnmclip_c"
        ]
    );
    assert_eq!(clips.showcases.len(), 2);

    // A graph whose root is not a state machine shows nothing.
    let mut rootless = ui_graph();
    rootless["m_nRootNodeIdx"] = serde_json::json!(4);
    assert!(graph_showcases(&rootless, "g", &available).is_empty());
}

#[test]
fn viewer_model_refs_and_clip_metadata_read_kv3() {
    use super::clips::{clip_file_meta, flag_embedded, metadata_paths, model_refs};
    use morphic::kv3::Value;
    let object = |pairs: Vec<(&str, Value)>| {
        Value::Object(
            pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    };
    let text = |value: &str| Value::String(value.into());
    let model = object(vec![
        (
            "m_animGraph2Refs",
            Value::Array(vec![object(vec![
                ("m_sIdentifier", text("ui")),
                (
                    "m_hGraph",
                    text("models/heroes_staging/haze/haze_ui.vnmgraph"),
                ),
            ])]),
        ),
        (
            "m_vecNmSkeletonRefs",
            Value::Array(vec![text("models/heroes_staging/haze/haze.vnmskel")]),
        ),
    ]);
    let refs = model_refs(&model);
    assert_eq!(refs.graphs[0].0, "ui");
    assert!(refs.accepts_skeleton(Some("models/heroes_staging/haze/haze.vnmskel_c")));
    assert!(!refs.accepts_skeleton(Some("models/heroes_wip/inferno/inferno.vnmskel")));
    assert!(!refs.accepts_skeleton(None));

    let clip_data = |frames: u64, duration: f64, linear: f64| {
        object(vec![
            (
                "m_skeleton",
                text("models/heroes_staging/haze/haze.vnmskel"),
            ),
            ("m_nNumFrames", Value::UInt(frames)),
            ("m_flDuration", Value::Double(duration)),
            ("m_bIsAdditive", Value::Bool(false)),
            (
                "m_rootMotion",
                object(vec![
                    ("m_flAverageLinearVelocity", Value::Double(linear)),
                    ("m_flAverageAngularVelocityRadians", Value::Double(0.0)),
                ]),
            ),
        ])
    };
    let still = clip_file_meta(&clip_data(91, 3.0, 0.0)).unwrap();
    assert!(!still.root_motion);
    assert_eq!(still.fps(), 30.0);
    assert!(
        clip_file_meta(&clip_data(91, 3.0, 0.5))
            .unwrap()
            .root_motion
    );
    assert!(clip_file_meta(&object(vec![("m_nNumFrames", Value::UInt(3))])).is_none());
    assert!(still.same_timing(91, 30.0));
    assert!(!still.same_timing(90, 30.0));
    assert!(!still.same_timing(91, 20.0));

    let entry = "models/heroes_wip/dynamo/dynamo.vmdl_c";
    assert_eq!(
        metadata_paths(entry, "primary_stand_reload"),
        [
            "models/heroes_wip/dynamo/clips/primary_stand_reload.vnmclip_c",
            "models/heroes_wip/dynamo/clips/reload_idle.vnmclip_c"
        ]
    );
    assert_eq!(
        metadata_paths("models/heroes_wip/inferno/inferno.vmdl_c", "shop_menu_base")[1],
        "models/heroes_wip/inferno/clips/ui_shop_idle.vnmclip_c"
    );

    // The alias file with the same timing flags the embedded idle; the mistimed one does not.
    let mut embedded = vec![
        ClipCandidate::new("primary_stand_idle", 91, 30.0),
        ClipCandidate::new("primary_stand_reload", 40, 30.0),
        ClipCandidate::new("some_layer", 91, 30.0),
    ];
    let mut asked = Vec::new();
    flag_embedded(entry, "Dynamo", &mut embedded, |path| {
        asked.push(path.to_string());
        if path.ends_with("weapon_stand_idle.vnmclip_c") || path.ends_with("reload_idle.vnmclip_c")
        {
            clip_file_meta(&clip_data(91, 3.0, 0.5))
        } else {
            None
        }
    });
    assert!(embedded[0].root_motion);
    assert!(!embedded[1].root_motion);
    assert!(!embedded[2].root_motion);
    assert!(!asked.iter().any(|path| path.contains("some_layer")));
}

#[test]
fn viewer_progress_only_moves_forward() {
    use super::hero::Progress;
    use std::sync::Mutex;
    let seen = Mutex::new(Vec::new());
    let report = |fraction: f32| seen.lock().unwrap().push(fraction);
    let progress = Progress::new(Some(&report));
    progress.at(0.25);
    progress.at(0.125);
    progress.step(0.25, 0.75, 1, 4);
    progress.step(0.25, 0.75, 4, 4);
    progress.step(0.25, 0.75, 9, 4);
    progress.at(f32::NAN);
    progress.at(3.0);
    progress.at(1.0);
    assert_eq!(*seen.lock().unwrap(), [0.25, 0.375, 0.75, 1.0]);
}

#[test]
fn viewer_cache_hit_reports_done() {
    use std::sync::Mutex;
    let root = game("progress-hit");
    let cache = root.join("cache");
    let plan = plan_hero(&HeroExportRequest::new(
        &root,
        &cache,
        "inferno",
        Variant::Modded,
    ))
    .unwrap();
    let entry = cache.join("heroes").join(&plan.key);
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("inferno.glb"), b"glTF").unwrap();
    fs::write(
        entry.join("inferno.json"),
        serde_json::to_vec(&sidecar("inferno", &plan.key)).unwrap(),
    )
    .unwrap();

    let seen = Mutex::new(Vec::new());
    let report = |fraction: f32| seen.lock().unwrap().push(fraction);
    let mut request = HeroExportRequest::new(&root, &cache, "inferno", Variant::Modded);
    request.progress = Some(&report);
    let found = export_hero_with(&request, &AtomicBool::new(false)).unwrap();
    assert!(found.cached);
    assert_eq!(*seen.lock().unwrap(), [1.0]);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn viewer_clip_meta_contract_field_names() {
    use super::clips::{ClipGroup, ClipShowcase, ShowcaseState};
    use super::hero::{ClipMeta, ClipOrigin};
    let meta = ClipMeta {
        name: "haze_shop_pose".into(),
        duration_seconds: 4.0,
        fps: 30.0,
        frame_count: 121,
        playback: Playback::Hold,
        action: None,
        origin: ClipOrigin::ClipFile,
        resource: Some("models/heroes_staging/haze/clips/haze_shop_pose.vnmclip_c".into()),
        label: "Haze shop pose".into(),
        group: ClipGroup::Other,
        speed: 1.25,
        showcase: Some(ClipShowcase {
            graph: "models/heroes_staging/haze/haze_ui.vnmgraph_c".into(),
            state: ShowcaseState::HeroSelect,
        }),
        floor: Some(-0.03),
    };
    let json = serde_json::to_value(&meta).unwrap();
    assert_eq!(json["floor"].as_f64().map(|f| (f * 1000.0).round()), Some(-30.0));
    assert_eq!(json["label"], "Haze shop pose");
    assert_eq!(json["group"], "other");
    assert_eq!(json["speed"], 1.25);
    assert_eq!(json["showcase"]["state"], "heroSelect");
    assert_eq!(
        json["showcase"]["graph"],
        "models/heroes_staging/haze/haze_ui.vnmgraph_c"
    );
    assert!(json["showcase"].get("playback").is_none());

    // A clip without the new fields still reads, at speed 1.
    let old: ClipMeta = serde_json::from_value(serde_json::json!({
        "name": "idle", "durationSeconds": 1.0, "fps": 30.0, "frameCount": 31,
        "playback": "loop", "action": "idle", "origin": "model", "resource": null
    }))
    .unwrap();
    assert_eq!(old.speed, 1.0);
    assert_eq!(old.group, ClipGroup::Other);
    assert!(old.showcase.is_none());
    assert!(old.floor.is_none());
}
