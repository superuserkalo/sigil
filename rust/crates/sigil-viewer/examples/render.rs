//! Renders a hero to PNG and measures frames.
//!
//! ```text
//! cargo run -p sigil-viewer --example render -- out.png [--glb hero.glb] [--size 1280x800]
//!     [--time 1.0] [--split 0.5] [--yaw 0.4] [--bench 300]
//! ```
//!
//! Without `--glb` it draws the bundled stand-in (with `--split`, stock on the left and the
//! modded variant on the right). `--bench N` renders N frames at the size and prints the mean
//! encode, GPU wait and readback copy times.

use std::path::PathBuf;
use std::time::Instant;

use sigil_viewer::{Compare, Sidecar, Viewer, fixture, model};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = PathBuf::from("viewer.png");
    let (mut glb, mut size, mut time, mut split, mut yaw, mut bench, mut scene) = (None, (1280u32, 800u32), 1.0f32, None, None, 0usize, None);
    let mut i = 0;
    while i < args.len() {
        let next = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--glb" => glb = Some(PathBuf::from(&next)),
            "--scene" => scene = Some(PathBuf::from(&next)),
            "--hideout" => {
                let dir = PathBuf::from(&next);
                scene = Some(fixture::write_hideout(&dir, fixture::Variant::Modded).expect("write hideout"));
            }
            "--size" => {
                let (w, h) = next.split_once('x').expect("WxH");
                size = (w.parse().unwrap(), h.parse().unwrap());
            }
            "--time" => time = next.parse().unwrap(),
            "--split" => split = Some(next.parse::<f32>().unwrap()),
            "--yaw" => yaw = Some(next.parse::<f32>().unwrap()),
            "--bench" => bench = next.parse().unwrap(),
            other => {
                out = PathBuf::from(other);
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let started = Instant::now();
    let mut viewer = match Viewer::new() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    eprintln!("device {} in {:.0} ms", viewer.adapter(), started.elapsed().as_secs_f32() * 1000.0);
    let loaded = Instant::now();
    let info = match &glb {
        Some(path) => viewer.load_hero_file(path, None).expect("load"),
        None => {
            let side = Sidecar::parse(&fixture::sidecar_json());
            let after = model::load_slice(&fixture::hero_glb(fixture::Variant::Modded)).expect("fixture");
            let info = viewer.set_hero(after, &side);
            let before = model::load_slice(&fixture::hero_glb(fixture::Variant::Stock)).expect("fixture");
            viewer.set_before(Some(before), &side);
            info
        }
    };
    if let Some(path) = &scene {
        let assets = sigil_viewer::load_scene(path).expect("scene");
        for e in &assets.errors {
            eprintln!("scene: {e}");
        }
        let hero = assets.desc.hero.clone();
        viewer.set_scene(assets);
        if let (None, Some(h)) = (&glb, hero) {
            viewer.load_hero_file(&h.glb, h.sidecar.as_deref()).expect("scene hero");
            viewer.set_before(None, &Sidecar::default());
        }
    }
    eprintln!("loaded in {:.0} ms: {} triangles, clips {:?}, idle {:?}", loaded.elapsed().as_secs_f32() * 1000.0, info.triangles, info.clips, info.clip);
    if let Some(f) = split {
        viewer.compare = Compare::Split(f);
    } else if glb.is_none() {
        viewer.compare = Compare::After;
    }
    viewer.seek(time);
    viewer.playback.playing = false;
    let _ = viewer.render(size.0, size.1, 0.0);
    if let Some(y) = yaw {
        viewer.camera.yaw = y;
    }
    let png = viewer.capture_png(size.0, size.1).expect("png");
    std::fs::write(&out, png).expect("write");
    eprintln!("wrote {}", out.display());
    if bench > 0 {
        viewer.playback.playing = true;
        let mut sums = [0.0f32; 4];
        for _ in 0..10 {
            let _ = viewer.render(size.0, size.1, 1.0 / 60.0);
        }
        let t0 = Instant::now();
        for _ in 0..bench {
            let f = Instant::now();
            let _ = viewer.render(size.0, size.1, 1.0 / 60.0);
            let t = viewer.timings();
            sums[0] += t.encode_ms;
            sums[1] += t.gpu_wait_ms;
            sums[2] += t.unpad_ms;
            sums[3] += f.elapsed().as_secs_f32() * 1000.0;
        }
        let mut gpu_only = 0.0f32;
        for _ in 0..bench {
            gpu_only += viewer.render_without_readback(size.0, size.1, 1.0 / 60.0).gpu_wait_ms;
        }
        let n = bench as f32;
        println!("without readback: GPU wait {:.2} ms/frame, so the readback adds {:.2} ms", gpu_only / n, (sums[1] + sums[2] - gpu_only) / n);
        println!(
            "{}x{} over {bench} frames: total {:.2} ms/frame ({:.0} fps), encode+pose {:.2} ms, GPU+map wait {:.2} ms, readback copy {:.2} ms; wall {:.2} s",
            size.0,
            size.1,
            sums[3] / n,
            1000.0 / (sums[3] / n),
            sums[0] / n,
            sums[1] / n,
            sums[2] / n,
            t0.elapsed().as_secs_f32()
        );
    }
}
