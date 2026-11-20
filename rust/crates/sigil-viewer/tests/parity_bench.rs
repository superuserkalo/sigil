//! What a frame costs with everything on: the stand-in hero at 1280 x 800 with its cloth,
//! its particle plume, shadows and bloom, and the stand-in Hideout with its baked lighting,
//! volumetric fog and map particles. Release only:
//! `cargo test --release -p sigil-viewer --test parity_bench -- --ignored --nocapture`.

use sigil_viewer::{Sidecar, Viewer, ViewerError, fixture, model};

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

fn time(v: &mut Viewer, label: &str) {
    let (w, h) = (1280, 800);
    for _ in 0..30 {
        let _ = v.render(w, h, 1.0 / 60.0);
    }
    let frames = 120;
    let started = std::time::Instant::now();
    for _ in 0..frames {
        let _ = v.render(w, h, 1.0 / 60.0);
    }
    let with_read = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let started = std::time::Instant::now();
    for _ in 0..frames {
        let _ = v.render_without_readback(w, h, 1.0 / 60.0);
    }
    let without = started.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("{label}: {with_read:.2} ms a frame with readback, {without:.2} ms without ({} particles)", v.particle_count());
}

#[test]
#[ignore]
fn frame_cost_with_cloth_particles_shadows() {
    let Some(mut v) = viewer() else { return };
    let side = Sidecar::parse(&fixture::sidecar_json());
    v.set_hero(model::load_slice(&fixture::hero_glb(fixture::Variant::Modded)).unwrap(), &side);
    v.set_hero_particles(Some(sigil_viewer::particles::fixture::hero_bundle()));
    let stats = v.set_hero_cloth(Some(&fixture::cloth_json()));
    assert!(stats.is_some(), "the stand-in's cloth builds");
    time(&mut v, "hero, cloth, particles, shadows, bloom");
    println!("cloth: {:?}", v.cloth_stats());
    v.set_hero_particles(None);
    v.look.shadows = false;
    time(&mut v, "hero without particles and shadows");

    let dir = std::env::temp_dir().join(format!("sigil-bench-hideout-{}", std::process::id()));
    let scene = fixture::write_hideout(&dir, fixture::Variant::Modded).unwrap();
    sigil_viewer::fixture_room::upgrade_hideout(&scene).unwrap();
    let Some(mut v) = viewer() else { return };
    let assets = sigil_viewer::load_scene(&scene).unwrap();
    let hero = assets.desc.hero.clone().unwrap();
    v.set_scene(assets);
    let side = Sidecar::read(hero.sidecar.as_ref().unwrap());
    v.set_hero(model::load_path(&hero.glb).unwrap(), &side);
    v.set_hero_particles(sigil_viewer::particles::Bundle::read(&dir.join("hero/particles")).ok());
    time(&mut v, "stand-in Hideout: lightmap, probes, fog volume, cookies, map post, particles");
    let _ = std::fs::remove_dir_all(&dir);
}
