//! End to end: the stand-in hero loads, poses, renders and compares. GPU tests skip (pass with
//! a note) on a machine without an adapter, which is the case `ViewerError::NoGpu` exists for.

use sigil_viewer::{Compare, Sidecar, Viewer, ViewerError, fixture, model};

fn viewer() -> Option<Viewer> {
    match Viewer::new() {
        Ok(v) => Some(v),
        Err(ViewerError::NoGpu(e)) => {
            eprintln!("skipped: no GPU adapter ({e})");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn hero(variant: fixture::Variant) -> model::Model {
    model::load_slice(&fixture::hero_glb(variant)).expect("the stand-in loads")
}

#[test]
fn the_stand_in_is_a_skinned_animated_npr_hero() {
    let m = hero(fixture::Variant::Stock);
    // The body, and the coat tails skinned to the cloth chains.
    assert_eq!(m.skins.len(), 2);
    assert_eq!(m.skins[0].joints.len(), 17);
    assert_eq!(m.skins[1].joints.len(), 15);
    assert_eq!(model::default_idle(&m).map(|c| c.name.as_str()), Some("hero_shop"));
    let body = m.materials[0].resolve();
    assert!(body.npr && body.tint_mask.is_some() && body.outline_thickness > 0.0);
    assert!(m.materials[1].resolve().self_illum.is_some());
    // The skeleton node converts inches to meters: one game unit is 0.0254 m.
    let locals = model::sample_pose(&m, None, 0.0);
    let globals = model::globals(&m, &locals);
    assert!((model::game_unit(&m, &globals) - 0.0254).abs() < 1e-4);
}

#[test]
fn the_pose_moves_with_the_clip() {
    let m = hero(fixture::Variant::Stock);
    let clip = model::default_idle(&m);
    let head = m.nodes.iter().position(|n| n.name == "head").unwrap();
    let a = model::globals(&m, &model::sample_pose(&m, clip, 0.0))[head];
    let b = model::globals(&m, &model::sample_pose(&m, clip, 1.2))[head];
    assert_ne!(a, b);
}

fn center_pixel(rgba: &[u8], w: u32, h: u32) -> [u8; 4] {
    let i = ((h / 2 * w + w / 2) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn renders_the_hero_over_the_backdrop() {
    let Some(mut v) = viewer() else { return };
    let side = Sidecar::parse(&fixture::sidecar_json());
    let info = v.set_hero(hero(fixture::Variant::Modded), &side);
    assert_eq!(info.clip.as_deref(), Some("hero_shop"));
    v.playback.playing = false;
    let (w, h) = (160, 200);
    let frame = v.render(w, h, 0.0).to_vec();
    assert_eq!(frame.len(), (w * h * 4) as usize);
    let corner = &frame[0..4];
    let middle = center_pixel(&frame, w, h);
    assert_eq!(corner[3], 255);
    assert_ne!(&middle[..3], &corner[..3], "the hero covers the middle");
    // The modded coat is blue: the middle of the frame is more blue than red.
    assert!(middle[2] > middle[0], "{middle:?}");

    // Playing moves the hero.
    v.playback.playing = true;
    let later = v.render(w, h, 1.1).to_vec();
    assert_ne!(frame, later);

    // The stock hero (beige coat) shows in "before".
    v.set_before(Some(hero(fixture::Variant::Stock)), &side);
    v.compare = Compare::Before;
    v.playback.playing = false;
    let before = v.render(w, h, 0.0).to_vec();
    let b = center_pixel(&before, w, h);
    assert!(b[0] >= b[2], "{b:?}");
    v.compare = Compare::Split(0.5);
    let split = v.render(w, h, 0.0).to_vec();
    assert_ne!(split, before);
}

#[test]
fn a_transparent_look_leaves_the_background_clear() {
    let Some(mut v) = viewer() else { return };
    v.set_hero(hero(fixture::Variant::Stock), &Sidecar::default());
    v.look.transparent = true;
    v.look.floor = false;
    let frame = v.render(64, 64, 0.0).to_vec();
    assert_eq!(frame[3], 0, "corner alpha");
    assert!(center_pixel(&frame, 64, 64)[3] > 200);
}
