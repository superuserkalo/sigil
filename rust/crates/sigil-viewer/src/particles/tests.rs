use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::bundle::{Sequence, Sheet, SheetFrame};
use super::fields::{self as fl, Particles};
use super::host::Effects;
use super::providers::{Curve, Mapping, NumIn, ParseCx};
use super::render::View;
use super::state::State;
use super::system::{Source, System};
use super::*;
use crate::math::{Mat4, Vec3, length, sub};

struct Defs(Vec<(String, Value, Option<String>)>);

impl Source for Defs {
    fn definition(&self, name: &str) -> Option<(&Value, Option<&str>)> {
        self.0.iter().find(|(n, _, _)| n == name).map(|(_, v, f)| (v, f.as_deref()))
    }
}

fn one(def: Value) -> System {
    let defs = Defs(vec![("test.vpcf".into(), def, Some("vpcf66".into()))]);
    let mut cx = ParseCx::default();
    System::load("test.vpcf", &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()).expect("system")
}

fn lit(x: f32) -> Value {
    json!({ "m_nType": "PF_TYPE_LITERAL", "m_flLiteralValue": x })
}

fn run(sys: &mut System, seconds: f32, dt: f32) {
    let frames = (seconds / dt).round() as usize;
    sys.start();
    for _ in 0..frames {
        sys.update(dt, [0.0; 3]);
    }
}

fn particles_with_ids(n: usize) -> Particles {
    let mut p = Particles::new([0.0; fl::COUNT], [[0.0; 3]; fl::COUNT], n);
    for i in 0..n {
        p.push(0.0);
        p.s[fl::PARTICLE_ID][i] = i as f32;
    }
    p
}

#[test]
fn particles_providers_literal_random_curve_and_mapping() {
    let mut cx = ParseCx::default();
    let mut st = State::new(16);
    st.rnd.start(1234, 0);
    assert_eq!(NumIn::parse(&json!(3.5), &mut cx).next(&mut st), 3.5);
    assert_eq!(NumIn::parse(&lit(-2.0), &mut cx).next(&mut st), -2.0);

    // Constant random: in range, stable per particle id across calls, varying between ids.
    let r = NumIn::parse(&json!({ "m_nType": "PF_TYPE_RANDOM_UNIFORM", "m_flRandomMin": 2.0, "m_flRandomMax": 6.0, "m_nRandomMode": "PF_RANDOM_MODE_CONSTANT" }), &mut cx);
    assert!(cx.random_inputs);
    let p = particles_with_ids(64);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    r.fill(&mut st, &p, 0, 64, &mut a);
    r.fill(&mut st, &p, 0, 64, &mut b);
    assert_eq!(a, b, "constant random inputs repeat per particle");
    assert!(a.iter().all(|x| (2.0..=6.0).contains(x)));
    let mean = a.iter().sum::<f32>() / 64.0;
    assert!((3.0..5.0).contains(&mean), "mean {mean}");
    assert!(a.windows(2).any(|w| w[0] != w[1]));

    // Curve: through (0, 0), (0.5, 1), (1, 0), held at the ends and clamped to the domain.
    let c = Curve::from_keys(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)], [0.0, 0.0], [1.0, 1.0]);
    assert!((c.evaluate(0.5) - 1.0).abs() < 1e-5);
    assert!(c.evaluate(0.25) > 0.4 && c.evaluate(0.25) <= 1.0);
    assert_eq!(c.evaluate(-1.0), 0.0);
    assert_eq!(c.evaluate(2.0), 0.0);

    // A curve driven by normalized particle age.
    let curve = NumIn::parse(
        &json!({ "m_nType": "PF_TYPE_PARTICLE_AGE_NORMALIZED", "m_nMapType": "PF_MAP_TYPE_CURVE",
                 "m_Curve": { "m_spline": [{ "x": 0.0, "y": 0.0 }, { "x": 1.0, "y": 1.0 }],
                              "m_tangents": [{ "m_nIncomingTangent": "CURVE_TANGENT_LINEAR", "m_nOutgoingTangent": "CURVE_TANGENT_LINEAR" },
                                             { "m_nIncomingTangent": "CURVE_TANGENT_LINEAR", "m_nOutgoingTangent": "CURVE_TANGENT_LINEAR" }],
                              "m_vDomainMins": [0.0, 0.0], "m_vDomainMaxs": [1.0, 1.0] } }),
        &mut cx,
    );
    let mut q = particles_with_ids(1);
    q.s[fl::LIFE][0] = 2.0;
    q.time = 0.5;
    let mut out = Vec::new();
    curve.fill(&mut st, &q, 0, 1, &mut out);
    assert!((out[0] - 0.25).abs() < 1e-5, "{}", out[0]);

    // Remap clamps to the output range; mult scales; a looped remap wraps.
    let remap = Mapping::parse(&json!({ "m_nMapType": "PF_MAP_TYPE_REMAP", "m_flInput0": 0.0, "m_flInput1": 10.0, "m_flOutput0": 100.0, "m_flOutput1": 200.0 }));
    assert_eq!(remap.apply(5.0), 150.0);
    assert_eq!(remap.apply(20.0), 200.0);
    assert_eq!(Mapping::parse(&json!({ "m_nMapType": "PF_MAP_TYPE_MULT", "m_flMultFactor": 3.0 })).apply(2.0), 6.0);
    let looped = Mapping::parse(&json!({ "m_nMapType": "PF_MAP_TYPE_REMAP", "m_nInputMode": "PF_INPUT_MODE_LOOPED", "m_flInput0": 0.0, "m_flInput1": 1.0, "m_flOutput0": 0.0, "m_flOutput1": 1.0 }));
    assert!((looped.apply(2.25) - 0.25).abs() < 1e-5);
    let col = NumIn::parse(&json!({ "m_nType": "PF_TYPE_COLLECTION_AGE", "m_nMapType": "PF_MAP_TYPE_REMAP", "m_flInput0": 0.0, "m_flInput1": 2.0, "m_flOutput0": 0.0, "m_flOutput1": 10.0 }), &mut cx);
    st.age = 1.0;
    assert_eq!(col.next(&mut st), 5.0);
}

fn emitter_system(emitter: Value, life: f32, decay: bool) -> Value {
    let mut ops = vec![];
    if decay {
        ops.push(json!({ "_class": "C_OP_Decay" }));
    }
    json!({
        "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12, "m_nMaxParticles": 500,
        "m_Emitters": [emitter],
        "m_Initializers": [{ "_class": "C_INIT_InitFloat", "m_InputValue": lit(life), "m_nOutputField": 1 }],
        "m_Operators": ops
    })
}

#[test]
fn particles_emitters_emit_the_right_counts_over_time() {
    let mut s = one(emitter_system(json!({ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(10.0) }), 100.0, false));
    run(&mut s, 1.0, 1.0 / 64.0);
    assert_eq!(s.p.len, 10);
    // Creation times spread evenly through the second.
    let mut t: Vec<f32> = s.p.s[fl::CREATION].clone();
    t.sort_by(f32::total_cmp);
    assert!((t[0] - 0.1).abs() < 0.02 && (t[9] - 1.0).abs() < 0.02, "{t:?}");
    run(&mut s, 2.0, 1.0 / 30.0);
    assert_eq!(s.p.len, 30);

    let mut s = one(emitter_system(json!({ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(20.0), "m_flEmissionDuration": lit(0.5) }), 100.0, false));
    run(&mut s, 2.0, 1.0 / 60.0);
    assert_eq!(s.p.len, 10, "a 0.5 s emission at 20/s");

    let mut s = one(emitter_system(json!({ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(25.0) }), 100.0, false));
    s.start();
    s.update(1.0 / 60.0, [0.0; 3]);
    assert_eq!(s.p.len, 25);
    for _ in 0..60 {
        s.update(1.0 / 60.0, [0.0; 3]);
    }
    assert_eq!(s.p.len, 25, "one burst only");

    let mut s = one(emitter_system(json!({ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(5.0), "m_flStartTime": lit(0.5) }), 100.0, false));
    run(&mut s, 0.4, 0.1);
    assert_eq!(s.p.len, 0);
    run(&mut s, 0.2, 0.1);
    assert_eq!(s.p.len, 5);
}

#[test]
fn particles_lifetime_decay_removes_particles() {
    let mut s = one(emitter_system(json!({ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(40.0) }), 0.5, true));
    run(&mut s, 3.0, 1.0 / 60.0);
    // Steady state: about rate x lifetime alive, all younger than their lifetime.
    assert!((18..=22).contains(&s.p.len), "{}", s.p.len);
    for i in 0..s.p.len {
        assert!(s.p.age(i) < 0.5);
    }
    // Without Decay nothing dies.
    let mut s = one(emitter_system(json!({ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(40.0) }), 0.5, false));
    run(&mut s, 1.0, 1.0 / 60.0);
    assert_eq!(s.p.len, 40);
}

#[test]
fn particles_fade_operators_shape_alpha() {
    let def = json!({
        "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12,
        "m_Emitters": [{ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(4.0) }],
        "m_Initializers": [{ "_class": "C_INIT_InitFloat", "m_InputValue": lit(1.0), "m_nOutputField": 1 }],
        "m_Operators": [{ "_class": "C_OP_FadeInSimple", "m_flFadeInTime": 0.25 }, { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.25 }, { "_class": "C_OP_Decay" }]
    });
    let mut s = one(def);
    s.start();
    let mut alphas = Vec::new();
    for _ in 0..20 {
        s.update(0.05, [0.0; 3]);
        if s.p.len > 0 {
            alphas.push((s.p.age(0), s.p.s[fl::ALPHA][0]));
        }
    }
    let at = |t: f32| alphas.iter().min_by(|a, b| (a.0 - t).abs().total_cmp(&(b.0 - t).abs())).unwrap().1;
    assert!(at(0.1) < 0.6, "fading in: {}", at(0.1));
    assert!((at(0.5) - 1.0).abs() < 1e-4, "full in the middle");
    assert!(at(0.9) < 0.4, "fading out: {}", at(0.9));
    assert_eq!(s.p.len, 0, "dead after its lifetime");

    // FadeAndKill fades and kills by itself.
    let def = json!({
        "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12,
        "m_Emitters": [{ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(3.0) }],
        "m_Initializers": [{ "_class": "C_INIT_InitFloat", "m_InputValue": lit(1.0), "m_nOutputField": 1 }],
        "m_Operators": [{ "_class": "C_OP_FadeAndKill" }]
    });
    let mut s = one(def);
    s.start();
    for _ in 0..15 {
        s.update(0.05, [0.0; 3]);
    }
    let a = s.p.s[fl::ALPHA][0];
    assert!(a > 0.0 && a < 1.0, "{a}");
    for _ in 0..10 {
        s.update(0.05, [0.0; 3]);
    }
    assert_eq!(s.p.len, 0);
}

fn four_frames(clamp: bool) -> Sheet {
    let rect = |k: f32| [k * 0.25, 0.0, k * 0.25 + 0.25, 1.0];
    Sheet { sequences: vec![Sequence { id: 0, clamp, duration: 1.0, frames: (0..4).map(|k| SheetFrame { duration: 0.25, rect: rect(k as f32) }).collect() }] }
}

#[test]
fn particles_sheet_frame_selection() {
    let sheet = four_frames(false);
    let (a, b, t) = sheet.frames(0, 0.0, false);
    assert_eq!((a[0], b[0], t), (0.0, 0.25, 0.0));
    let (a, b, t) = sheet.frames(0, 0.3, false);
    assert_eq!((a[0], b[0]), (0.25, 0.5));
    assert!((t - 0.2).abs() < 1e-5);
    // A looping sequence wraps from the last frame to the first.
    let (a, b, t) = sheet.frames(0, 1.9, false);
    assert_eq!((a[0], b[0]), (0.75, 0.0));
    assert!((t - 0.6).abs() < 1e-4, "{t}");
    // A clamped one holds its last frame.
    let (_, b, t) = four_frames(true).frames(0, 3.0, false);
    assert_eq!((b[0], t), (0.75, 1.0));
    // In sheet seconds: two frames' worth of time is half a pass.
    let (a, _, _) = sheet.frames(0, 0.5, true);
    assert_eq!(a[0], 0.5);
    assert_eq!(sheet.index_of(0), Some(0));
    assert_eq!(sheet.index_of(7), None);
}

fn grimoire_fixture(name: &str) -> Option<Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/grimoire/src/lib/particles/__fixtures__");
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    serde_json::from_str(&text).ok()
}

fn fixture_defs(v: &Value) -> Defs {
    let mut out = Vec::new();
    let formats = v.get("formats").and_then(Value::as_object);
    for (name, def) in v["definitions"].as_object().unwrap() {
        let format = formats.and_then(|f| f.get(&format!("{name}_c"))).and_then(Value::as_str).map(str::to_string);
        out.push((name.to_ascii_lowercase(), def.clone(), format));
    }
    Defs(out)
}

#[test]
fn particles_grimoire_fixture_systems_load_and_report_unknown_classes() {
    let Some(flame) = grimoire_fixture("batch38_inferno_hand_ambient_flame.json") else {
        eprintln!("Grimoire fixtures not vendored; skipped");
        return;
    };
    // Infernus' palm flame (vpcf61): a continuous 8/s emitter; its creation times land on
    // the emitter's 1/8 s lattice as in VRF's dump.
    let defs = fixture_defs(&flame);
    let mut cx = ParseCx::default();
    let name = flame["system"].as_str().unwrap();
    let mut s = System::load(name, &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()).expect("loads");
    s.start();
    for frame in 0..64 {
        let t = (frame + 1) as f32 / 64.0;
        s.set_cp(0, [7.0 * t, 3.0 * t, 2.0 * t], None);
        s.update(1.0 / 64.0, [0.0; 3]);
    }
    assert!(s.p.len > 0 && s.p.len <= 8, "{}", s.p.len);
    for &c in &s.p.s[fl::CREATION] {
        assert!(((c * 8.0).round() - c * 8.0).abs() < 1e-3, "creation {c}");
    }
    let expected: Vec<f64> = flame["expected"].as_array().unwrap().iter().find(|e| e["frame"] == 63).unwrap()["systems"][0]["CreationTime"].as_array().unwrap().iter().filter_map(Value::as_f64).collect();
    for e in expected {
        assert!(((e * 8.0).round() - e * 8.0).abs() < 1e-3);
    }

    // A system with classes this port lacks loads, runs, and names them.
    let Some(haze) = grimoire_fixture("haze_smokey.json") else { return };
    let defs = fixture_defs(&haze);
    let mut cx = ParseCx::default();
    let mut s = System::load(haze["system"].as_str().unwrap(), &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()).expect("loads");
    run(&mut s, 1.0, 1.0 / 60.0);
    let mut all_unknown = Vec::new();
    for f in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../vendor/grimoire/src/lib/particles/__fixtures__")).unwrap().flatten() {
        let Ok(v) = serde_json::from_str::<Value>(&std::fs::read_to_string(f.path()).unwrap_or_default()) else { continue };
        let Some(sys) = v.get("system").and_then(Value::as_str) else { continue };
        if v.get("definitions").is_none() {
            continue;
        }
        let defs = fixture_defs(&v);
        let mut cx = ParseCx::default();
        if let Some(mut s) = System::load(sys, &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()) {
            run(&mut s, 0.5, 1.0 / 30.0);
        }
        all_unknown.extend(cx.diagnostics);
    }
    assert!(all_unknown.iter().any(|d| d.contains("unsupported renderer C_OP_RenderRopes")), "{all_unknown:?}");
    assert!(all_unknown.iter().any(|d| d.contains("unsupported initializer C_INIT_NormalAlignToCP")));
    assert!(!all_unknown.iter().any(|d| d.contains("C_OP_RenderSprites") || d.contains("C_INIT_InitFloat") || d.contains("C_OP_Decay ")));
}

fn positions(s: &System) -> Vec<Vec3> {
    s.p.v[fl::POSITION].clone()
}

#[test]
fn particles_determinism_with_a_fixed_seed() {
    let def = || {
        json!({
            "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12,
            "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(30.0) }],
            "m_Initializers": [
                { "_class": "C_INIT_CreateWithinSphereTransform", "m_fRadiusMax": lit(10.0), "m_fSpeedMax": lit(20.0) },
                { "_class": "C_INIT_InitFloat", "m_InputValue": { "m_nType": "PF_TYPE_RANDOM_UNIFORM", "m_flRandomMin": 0.5, "m_flRandomMax": 1.5 }, "m_nOutputField": 1 }
            ],
            "m_Operators": [{ "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, -50.0] }, { "_class": "C_OP_Decay" }]
        })
    };
    let go = |seed: i32| {
        let mut s = one(def());
        s.seed(seed);
        run(&mut s, 1.0, 1.0 / 60.0);
        positions(&s)
    };
    assert_eq!(go(7), go(7));
    assert_ne!(go(7), go(8));
    // Replay kills everything and runs again from age 0; like VRF's (unpinned) replay its
    // draws continue, so the new run is a fresh one of the same shape.
    let mut s = one(def());
    s.seed(7);
    run(&mut s, 1.0, 1.0 / 60.0);
    let first = positions(&s);
    s.replay();
    assert_eq!(s.p.len, 0);
    run(&mut s, 1.0, 1.0 / 60.0);
    assert!(positions(&s).len().abs_diff(first.len()) <= 8);
    assert_ne!(first, positions(&s));
    assert_eq!(string_token("particles/x.vpcf"), string_token("PARTICLES/X.VPCF"));
}

fn scratch_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sigil-particles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// The stand-in hero's `hand_R` bone frame in Source model space (bind pose: identity
/// rotation at the bone head).
fn hand_bone(name: &str) -> Option<Mat4> {
    (name == "hand_R").then(|| Mat4::translation([1.5, -10.8, 31.0]))
}

#[test]
fn particles_standin_bundle_round_trips_and_plays() {
    let dir = scratch_dir("hero");
    let path = fixture::write_fixture(&dir).expect("write");
    let b = Bundle::read(&dir).expect("read");
    assert_eq!(Bundle::read(&path).expect("read json").systems.len(), 3);
    assert_eq!(b.systems.len(), 3);
    let sheet_tex = &b.textures[fixture::GLOW_TEXTURE];
    assert_eq!((sheet_tex.width, sheet_tex.height), (256, 256));
    assert_eq!(sheet_tex.sheet.as_ref().unwrap().sequences[0].frames.len(), 16);
    assert!(b.textures[fixture::EMBER_TEXTURE].sheet.is_none());
    assert_eq!(b.attachments[0].influences[0].bone, "hand_R");

    let hand = b.attachment_frame("attach_hand_r", &hand_bone).expect("attachment");
    assert_eq!([hand.0[3][0], hand.0[3][1], hand.0[3][2]], [1.5, -10.8, 28.0]);

    let mut fx = Effects::new(&b);
    assert!(fx.diagnostics().is_empty(), "{:?}", fx.diagnostics());
    let attach = |n: &str| b.attachment_frame(n, &hand_bone);
    for _ in 0..180 {
        fx.step(1.0 / 60.0, &attach, [100.0, 0.0, 50.0]);
    }
    let root = &fx.hosts[0].sys;
    assert!(root.p.len > 10, "embers {}", root.p.len);
    // Embers start at the hand and rise.
    for i in 0..root.p.len {
        let p = root.p.v[fl::POSITION][i];
        assert!(length(sub([p[0], p[1], 0.0], [1.5, -10.8, 0.0])) < 15.0 && p[2] > 26.0 && p[2] < 90.0, "{p:?}");
    }
    let glow = &root.children[0];
    assert!(glow.p.len >= 1, "glow child");
    assert!(length(sub(glow.p.v[fl::POSITION][0], [1.5, -10.8, 28.0])) < 1e-3, "locked to the hand");
    assert!(root.children[1].p.len > 0, "sparks from dying embers");

    // Cards: one per visible particle, sheet frames inside the 4 x 4 grid.
    let mut out = Vec::new();
    let (mut ranges, mut shading) = (Vec::new(), Vec::new());
    let sheets = b.textures.iter().filter_map(|(k, t)| t.sheet.clone().map(|s| (k.clone(), s))).collect();
    let view = View { camera: [100.0, 0.0, 50.0], to_world: crate::fixture::source_to_gltf(), world_scale: 0.0254 };
    fx.build(&view, &sheets, &mut out, &mut ranges, &mut shading);
    assert_eq!(ranges.len(), 3);
    assert!(!out.is_empty() && out.len() <= fx.particle_count());
    let glow_cards = &out[ranges[1].0 as usize..ranges[1].1 as usize];
    for c in glow_cards {
        assert!(c.uv_a[2] - c.uv_a[0] - 0.25 < 1e-5 && c.uv_a[0] >= 0.0 && c.uv_a[2] <= 1.0);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn particles_hideout_bundle_places_map_entities() {
    let dir = scratch_dir("hideout");
    fixture::write_hideout_fixture(&dir).expect("write");
    let b = Bundle::read(&dir).expect("read");
    let mut fx = Effects::new(&b);
    let none = |_: &str| None;
    for _ in 0..60 {
        fx.step(1.0 / 60.0, &none, [0.0, 200.0, 60.0]);
    }
    let candle = &fx.hosts[0].sys;
    assert_eq!(candle.st.cps[0].pos, fixture::CANDLE_ORIGIN);
    assert!(candle.p.len > 5);
    for i in 0..candle.p.len {
        let p = candle.p.v[fl::POSITION][i];
        assert!(length(sub(p, fixture::CANDLE_ORIGIN)) < 20.0, "{p:?}");
    }
    let dust = &fx.hosts[1].sys;
    assert!(dust.p.len >= 24);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn particles_gpu_pipelines_build_and_draw() {
    let gpu = match crate::gpu::Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("no GPU ({e:?}); skipped");
            return;
        }
    };
    let device = &gpu.device;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let b = fixture::hero_bundle();
    let mut pg = ParticleGpu::new(device, &gpu.queue, &frame_layout, gpu.msaa, crate::gpu::HDR, crate::gpu::DEPTH, &b);
    let attach = |n: &str| b.attachment_frame(n, &hand_bone);
    let to_world = crate::fixture::source_to_gltf();
    for _ in 0..60 {
        pg.step(1.0 / 60.0, &to_world, &attach, [0.0, 1.0, 3.0]);
    }
    pg.upload(&gpu.queue);
    assert!(pg.particle_count() > 0 && pg.instance_count() > 0);
    let mut frame = vec![0.0f32; 512];
    let vp = Mat4::perspective(0.8, 1.0, 0.05, 50.0).mul(&Mat4::look_at([0.0, 1.0, 3.0], [0.0, 0.8, 0.0], [0.0, 1.0, 0.0]));
    frame[..16].copy_from_slice(&vp.to_cols_array());
    frame[16..32].copy_from_slice(&Mat4::look_at([0.0, 1.0, 3.0], [0.0, 0.8, 0.0], [0.0, 1.0, 0.0]).to_cols_array());
    let buf = wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&frame), usage: wgpu::BufferUsages::UNIFORM });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &frame_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }] });
    let target = |format, samples| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let color = target(crate::gpu::HDR, gpu.msaa);
    let depth = target(crate::gpu::DEPTH, gpu.msaa);
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &color, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &group, &[]);
        pg.draw(&mut pass);
    }
    gpu.queue.submit([enc.finish()]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let err = pollster::block_on(scope.pop());
    assert!(err.is_none(), "{err:?}");
}

/// Release timing: `cargo test --release -p sigil-viewer particles_timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn particles_timing_2000() {
    let def = json!({
        "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12, "m_nMaxParticles": 2000,
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(1000.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinSphereTransform", "m_fRadiusMax": lit(10.0), "m_fSpeedMax": lit(20.0) },
            { "_class": "C_INIT_InitFloat", "m_InputValue": { "m_nType": "PF_TYPE_RANDOM_UNIFORM", "m_flRandomMin": 2.5, "m_flRandomMax": 3.0 }, "m_nOutputField": 1 },
            { "_class": "C_INIT_InitFloat", "m_InputValue": { "m_nType": "PF_TYPE_RANDOM_UNIFORM", "m_flRandomMin": 0.0, "m_flRandomMax": 360.0 }, "m_nOutputField": 4 },
            { "_class": "C_INIT_RandomColor", "m_ColorMin": [255, 100, 0], "m_ColorMax": [255, 255, 0] }
        ],
        "m_Operators": [
            { "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, 30.0], "m_fDrag": lit(0.05) },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeInSimple" },
            { "_class": "C_OP_FadeOutSimple" },
            { "_class": "C_OP_InterpolateRadius", "m_flEndScale": lit(0.2) },
            { "_class": "C_OP_ColorInterpolate", "m_ColorFade": [255, 0, 0] },
            { "_class": "C_OP_SpinUpdate" }
        ],
        "m_ForceGenerators": [{ "_class": "C_OP_AttractToControlPoint", "m_fForceAmount": lit(50.0) }],
        "m_Renderers": [{ "_class": "C_OP_RenderSprites", "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ADD" }]
    });
    let json = json!({ "schema": 1, "systems": { "t.vpcf": { "data": def, "format": "vpcf66" } }, "textures": {}, "attachments": [],
        "effects": [{ "id": "e", "system": "t.vpcf", "source": "map-entity", "transform": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1] }] });
    let b = Bundle::from_json(&json, |_| Err("none".into())).unwrap();
    let mut fx = Effects::new(&b);
    let none = |_: &str| None;
    for _ in 0..240 {
        fx.step(1.0 / 60.0, &none, [200.0, 0.0, 0.0]);
    }
    assert!(fx.particle_count() >= 1900, "{}", fx.particle_count());
    let view = View { camera: [200.0, 0.0, 0.0], to_world: crate::fixture::source_to_gltf(), world_scale: 0.0254 };
    let sheets = std::collections::HashMap::new();
    let (mut out, mut ranges, mut shading) = (Vec::new(), Vec::new(), Vec::new());
    let frames = 200;
    let t0 = std::time::Instant::now();
    let mut step_ms = 0.0;
    for _ in 0..frames {
        let s0 = std::time::Instant::now();
        fx.step(1.0 / 60.0, &none, [200.0, 0.0, 0.0]);
        step_ms += s0.elapsed().as_secs_f64() * 1000.0;
        fx.build(&view, &sheets, &mut out, &mut ranges, &mut shading);
    }
    let total = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("2000 particles: step {:.3} ms, step + build {:.3} ms per frame ({} cards)", step_ms / frames as f64, total, out.len());
}

fn count_systems(s: &System) -> usize {
    1 + s.get_children().iter().map(count_systems).sum::<usize>()
}

/// A wide tree (40 children a level, twelve levels deep) would load 40^11 systems; the budget
/// stops it at a few hundred and says so.
#[test]
fn particles_wide_child_trees_stop_at_the_system_budget() {
    let defs = Defs(
        (0..super::system::MAX_DEPTH + 1)
            .map(|d| {
                let children: Vec<Value> = (0..40).map(|_| json!({ "m_ChildRef": format!("lvl{}.vpcf", d + 1) })).collect();
                (format!("lvl{d}.vpcf"), json!({ "_class": "CParticleSystemDefinition", "m_Children": children }), Some("vpcf66".to_string()))
            })
            .collect(),
    );
    let mut cx = ParseCx::default();
    let started = std::time::Instant::now();
    let sys = System::load("lvl0.vpcf", &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()).expect("root");
    assert!(count_systems(&sys) <= super::system::MAX_EFFECT_SYSTEMS, "{}", count_systems(&sys));
    assert!(sys.get_children().len() <= super::system::MAX_CHILDREN);
    assert!(cx.diagnostics.iter().any(|d| d.contains("budget")), "{:?}", cx.diagnostics);
    assert!(cx.diagnostics.iter().any(|d| d.contains("children past the first")));
    assert!(started.elapsed().as_secs() < 5);
    // A second effect from the same bundle gets its own effect budget but shares the bundle's.
    cx.effect_loaded = 0;
    let _ = System::load("lvl0.vpcf", &defs, SYSTEM_CAP, &mut cx, &mut Vec::new());
    assert!(cx.loaded <= super::system::MAX_BUNDLE_SYSTEMS);
    cx.effect_loaded = 0;
    assert!(System::load("lvl0.vpcf", &defs, SYSTEM_CAP, &mut cx, &mut Vec::new()).is_none(), "the bundle budget is spent");
}

fn png_header(width: u32, height: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in bytes {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    png.extend_from_slice(&13u32.to_be_bytes());
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&crc(&ihdr).to_be_bytes());
    // An empty IDAT and the IEND, so the header reads through.
    for chunk in [&b"IDAT"[..], &b"IEND"[..]] {
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(chunk);
        png.extend_from_slice(&crc(chunk).to_be_bytes());
    }
    png
}

/// A bundle texture claiming 100000 x 100000 texels is refused from its header, unread.
#[test]
fn particles_huge_textures_are_refused_before_decoding() {
    let dir = scratch_dir("huge");
    std::fs::create_dir_all(dir.join("tex")).unwrap();
    std::fs::write(dir.join("tex/huge.png"), png_header(100_000, 100_000)).unwrap();
    let json = json!({ "schema": 1, "systems": {}, "attachments": [], "effects": [],
        "textures": { "materials/particle/huge.vtex": { "file": "tex/huge.png", "width": 100000, "height": 100000, "sheet": null } } });
    std::fs::write(dir.join("bundle.json"), json.to_string()).unwrap();
    let b = Bundle::read(&dir).expect("the bundle still reads");
    assert!(b.textures.is_empty(), "the huge texture is left out");
    assert!(b.issues.iter().any(|i| i.contains("cap")), "{:?}", b.issues);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A texture wider than the device allows is downscaled to the viewer's edge cap, not handed
/// to wgpu as is: no validation error on any device.
#[test]
fn particles_wide_textures_upload_within_the_limits() {
    let gpu = match crate::gpu::Gpu::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("no GPU ({e:?}); skipped");
            return;
        }
    };
    let device = &gpu.device;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let (w, h) = (device.limits().max_texture_dimension_2d.max(8192) * 2, 4);
    let mut b = fixture::hero_bundle();
    let ember = b.textures.get_mut(fixture::EMBER_TEXTURE).expect("ember texture");
    ember.width = w;
    ember.height = h;
    ember.rgba = vec![255; (w * h * 4) as usize];
    let pg = ParticleGpu::new(device, &gpu.queue, &frame_layout, gpu.msaa, crate::gpu::HDR, crate::gpu::DEPTH, &b);
    assert!(pollster::block_on(scope.pop()).is_none(), "no validation error");
    let _ = pg.diagnostics();
}
