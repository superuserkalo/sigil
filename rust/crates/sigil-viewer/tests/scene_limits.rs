//! A room past the device's limits and the viewer's budgets opens with parts left out: never a
//! panic, never a load that reads one file per placement. The GPU tests skip (pass with a note)
//! on a machine without an adapter.

use std::sync::atomic::AtomicBool;

use sigil_viewer::{LoadStage, MAX_SCENE_PLACEMENTS, Viewer, ViewerError, fixture, load_scene, load_scene_with};

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sigil-viewer-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The stand-in Hideout with `count` placements of its room (fragments of one file, as an
/// aggregate is) and one placement of a file that is not there.
fn crowded_room(name: &str, count: usize) -> std::path::PathBuf {
    let dir = temp(name);
    let scene = fixture::write_hideout(&dir, fixture::Variant::Modded).expect("fixture");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&scene).unwrap()).unwrap();
    let one = json["models"][0].clone();
    let mut models: Vec<serde_json::Value> = (0..count)
        .map(|i| {
            let mut m = one.clone();
            m["primitive"] = serde_json::json!(i % 2);
            m
        })
        .collect();
    models.push(serde_json::json!({ "glb": "models/missing.glb", "transform": one["transform"], "tint": [1, 1, 1, 1] }));
    json["models"] = serde_json::Value::Array(models);
    std::fs::write(&scene, serde_json::to_vec(&json).unwrap()).unwrap();
    scene
}

#[test]
fn a_file_placed_many_times_is_read_once_and_placements_are_bounded() {
    let scene = crowded_room("crowded", MAX_SCENE_PLACEMENTS + 5);
    let assets = load_scene(&scene).expect("the room loads");
    assert_eq!(assets.files, 1, "one distinct file read");
    let loaded: Vec<_> = assets.models.iter().flatten().collect();
    assert_eq!(loaded.len(), MAX_SCENE_PLACEMENTS);
    assert!(loaded.windows(2).all(|w| std::sync::Arc::ptr_eq(w[0], w[1])), "placements share one model");
    assert!(assets.skipped.iter().any(|s| s.contains("placements past")), "{:?}", assets.skipped);
    let _ = std::fs::remove_dir_all(scene.parent().unwrap());
}

#[test]
fn a_missing_file_is_an_error_and_a_cancel_stops_the_read() {
    let scene = crowded_room("missing", 3);
    let mut seen = Vec::new();
    let assets = load_scene_with(&scene, &AtomicBool::new(false), &mut |p| seen.push(p)).expect("the room loads");
    assert_eq!(assets.files, 1);
    assert!(assets.errors.iter().any(|e| e.contains("missing.glb")), "{:?}", assets.errors);
    assert!(seen.iter().any(|p| p.stage == LoadStage::Reading && p.of == 2), "{seen:?}");
    let cancelled = load_scene_with(&scene, &AtomicBool::new(true), &mut |_| {});
    assert!(matches!(cancelled, Err(ViewerError::Cancelled)));
    let _ = std::fs::remove_dir_all(scene.parent().unwrap());
}

#[test]
fn a_room_past_the_budgets_opens_with_parts_left_out() {
    let viewer = match Viewer::new() {
        Ok(v) => Some(v),
        Err(ViewerError::NoGpu(e)) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
        Err(e) => panic!("{e}"),
    };
    let Some(mut viewer) = viewer else { return };
    let scene = crowded_room("budget", 40);
    let assets = load_scene(&scene).expect("the room loads");
    // Room for almost nothing: the second texture and the geometry go.
    viewer.set_budgets(64, 64);
    let report = viewer.set_scene(assets);
    assert_eq!(report.files, 1);
    assert_eq!(report.placements, 40);
    let usage = viewer.usage();
    assert!(usage.skipped_count > 0, "{usage:?}");
    assert!(usage.buffer_bytes <= 64 && usage.texture_bytes <= 64, "{usage:?}");
    let frame = viewer.render(64, 48, 0.0).to_vec();
    assert_eq!(frame.len(), 64 * 48 * 4);
    assert_eq!(viewer.gpu_error(), None);
    let _ = std::fs::remove_dir_all(scene.parent().unwrap());
}

#[test]
fn a_frame_larger_than_the_device_takes_is_clamped() {
    let Ok(mut viewer) = Viewer::new() else { return };
    let edge = viewer.max_edge();
    let frame = viewer.render(edge + 100, 8, 0.0).len();
    assert_eq!(frame, (edge * 8 * 4) as usize);
    assert_eq!(viewer.gpu_error(), None);
}

/// Grimoire's distant-instance culling: a prop far behind the stand-in room's back wall goes,
/// one as far out through its open side stays, and one near the spawn always stays.
#[test]
fn distant_props_hidden_by_the_room_are_left_out() {
    let Ok(mut viewer) = Viewer::new() else { return };
    let dir = temp("clearance");
    let scene = fixture::write_hideout(&dir, fixture::Variant::Modded).expect("fixture");
    let mut json: serde_json::Value = serde_json::from_slice(&std::fs::read(&scene).unwrap()).unwrap();
    let one = json["models"][0].clone();
    let placed = |t: [f32; 3]| {
        let mut m = one.clone();
        m["transform"] = serde_json::json!(sigil_viewer::math::Mat4::from_trs(t, [0.0, 0.0, 0.0, 1.0], [0.25; 3]).to_cols_array());
        m
    };
    // Behind the back wall (-Z), out through the open +X side, and near the spawn.
    let models = vec![one.clone(), placed([0.0, 0.0, -120.0]), placed([120.0, 0.5, -0.6]), placed([20.0, 0.0, 0.0])];
    json["models"] = serde_json::Value::Array(models);
    std::fs::write(&scene, serde_json::to_vec(&json).unwrap()).unwrap();
    let report = viewer.set_scene(load_scene(&scene).expect("the room loads"));
    assert_eq!(report.culled, 1, "{report:?}");
    assert_eq!(report.placements, 3);
    assert_eq!(viewer.gpu_error(), None);
    let _ = std::fs::remove_dir_all(&dir);
}
