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
    assert!(all_unknown.iter().any(|d| d.contains("unsupported renderer C_OP_RenderModels")), "{all_unknown:?}");
    assert!(!all_unknown.iter().any(|d| d.contains("unsupported renderer C_OP_RenderRopes")), "ropes are drawn");
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
    assert_eq!(Bundle::read(&path).expect("read json").systems.len(), 4);
    assert_eq!(b.systems.len(), 4);
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
    let view = View { camera: [100.0, 0.0, 50.0], to_world: crate::fixture::source_to_gltf(), world_scale: 0.0254, projection: 2.0 };
    fx.build(&view, &sheets, &mut out, &mut ranges, &mut shading);
    // The plume, its two children, and the wisp's two idle event instances.
    assert_eq!(ranges.len(), 5);
    assert_eq!(ranges[3].0, ranges[3].1, "an idle event effect draws nothing");
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
    let mut pg = ParticleGpu::new(device, &gpu.queue, &frame_layout, gpu.msaa, crate::gpu::HDR, crate::gpu::DEPTH, &b, u64::MAX);
    let attach = |n: &str| b.attachment_frame(n, &hand_bone);
    let to_world = crate::fixture::source_to_gltf();
    // Fire the wisp so its rope strip quads go through the pipeline too.
    pg.set_clip(Some((fixture::WISP_CLIP, 0.0)));
    pg.set_fov(0.8);
    for _ in 0..60 {
        pg.step(1.0 / 60.0, &to_world, &attach, [0.0, 1.0, 3.0]);
    }
    assert_eq!(pg.active_effects(), 2);
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
    let view = View { camera: [200.0, 0.0, 0.0], to_world: crate::fixture::source_to_gltf(), world_scale: 0.0254, projection: 2.0 };
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
    let pg = ParticleGpu::new(device, &gpu.queue, &frame_layout, gpu.msaa, crate::gpu::HDR, crate::gpu::DEPTH, &b, u64::MAX);
    assert!(pollster::block_on(scope.pop()).is_none(), "no validation error");
    let _ = pg.diagnostics();
}

/// The stand-in bundle's wisp: an animation-event effect with two idle instances.
fn wisp_hosts(fx: &Effects) -> Vec<usize> {
    (0..fx.hosts.len()).filter(|&k| fx.hosts[k].effect.system == fixture::WISP).collect()
}

#[test]
fn particles_animation_events_follow_the_clip() {
    let b = fixture::hero_bundle();
    let wisp = b.effects.iter().find(|e| e.system == fixture::WISP).expect("wisp effect");
    assert_eq!(wisp.source, "anim-event");
    let clip = wisp.clip.as_ref().expect("clip");
    assert_eq!((clip.name.as_str(), clip.start, clip.stop_on_clip_change), (fixture::WISP_CLIP, 0.0, true));
    assert_eq!(clip.stops, vec![ClipStop { time: fixture::WISP_STOP, immediately: false, end_cap: false }]);

    let mut fx = Effects::new(&b);
    let hosts = wisp_hosts(&fx);
    assert_eq!(hosts.len(), super::host::EVENT_INSTANCES);
    // Only the ambient plays until the clip passes an event.
    assert_eq!(fx.active_count(), 1);
    let attach = |n: &str| b.attachment_frame(n, &hand_bone);
    let run = |fx: &mut Effects, frames: usize| {
        for _ in 0..frames {
            fx.step(1.0 / 60.0, &attach, [100.0, 0.0, 50.0]);
        }
    };
    // A clip that names no event fires nothing.
    fx.set_clip(Some(("other_clip", 0.0)));
    assert_eq!(fx.active_count(), 1);

    // Switching to the clip passes its events from the start: the wisp at 0 fires.
    fx.set_clip(Some((fixture::WISP_CLIP, 0.0)));
    assert_eq!(fx.active_count(), 2);
    let first = hosts.iter().copied().find(|&k| fx.hosts[k].active).unwrap();
    run(&mut fx, 30);
    let wisp_p = &fx.hosts[first].sys.p;
    assert!(wisp_p.len > 5, "wisp particles {}", wisp_p.len);
    // The wisp rises from the hand (here the model origin, 2 inches down).
    let top = (0..wisp_p.len).map(|i| wisp_p.v[fl::POSITION][i][2]).fold(f32::MIN, f32::max);
    assert!(top > 3.0, "{:?}", (0..wisp_p.len).map(|i| wisp_p.v[fl::POSITION][i]).collect::<Vec<_>>());
    // Moving on inside the clip passes nothing new.
    fx.set_clip(Some((fixture::WISP_CLIP, 0.5)));
    assert!(!fx.hosts[first].stopped);
    // Its stop at 0.85 stops emission; the particles live out their life, then it idles.
    fx.set_clip(Some((fixture::WISP_CLIP, 0.9)));
    assert!(fx.hosts[first].stopped && fx.hosts[first].active);
    run(&mut fx, 150);
    assert!(!fx.hosts[first].active, "a finished event effect is removed");
    assert_eq!(fx.active_count(), 1);

    // The clip loops: the start fires again (on an idle instance).
    fx.set_clip(Some((fixture::WISP_CLIP, 0.1)));
    assert_eq!(fx.active_count(), 2);
    let second = hosts.iter().copied().find(|&k| fx.hosts[k].active).unwrap();
    run(&mut fx, 10);
    // Another loop passes the stop (the playing one fades out) and the start, which takes
    // the other instance while the first still plays.
    fx.set_clip(Some((fixture::WISP_CLIP, 0.05)));
    assert!(fx.hosts[second].stopped);
    assert_eq!(fx.active_count(), 3);
    run(&mut fx, 10);
    let order: Vec<bool> = hosts.iter().map(|&k| fx.hosts[k].active).collect();
    assert_eq!(order, vec![true, true]);
    // A third with none idle reuses the oldest.
    fx.set_clip(Some((fixture::WISP_CLIP, 0.01)));
    assert_eq!(fx.active_count(), 3);
    assert!(!fx.hosts[second].stopped, "refired");

    // A clip change stops the effects that end with their clip (not at once: they fade out).
    fx.set_clip(Some(("other_clip", 0.0)));
    assert!(hosts.iter().all(|&k| fx.hosts[k].stopped));
    run(&mut fx, 150);
    assert_eq!(fx.active_count(), 1);
    // No clip at all stops nothing more and fires nothing.
    fx.set_clip(None);
    assert_eq!(fx.active_count(), 1);
}

#[test]
fn particles_animation_event_immediate_stop_drops_the_particles() {
    let mut json = fixture::hero_json();
    json["effects"][1]["clip"]["stops"][0]["immediately"] = json!(true);
    let b = Bundle::from_json(&json, |_| Ok((1, 1, vec![255; 4]))).unwrap();
    let mut fx = Effects::new(&b);
    let k = wisp_hosts(&fx)[0];
    let none = |_: &str| None;
    fx.set_clip(Some((fixture::WISP_CLIP, 0.0)));
    for _ in 0..30 {
        fx.step(1.0 / 60.0, &none, [100.0, 0.0, 50.0]);
    }
    assert!(fx.hosts[k].sys.p.len > 0);
    fx.set_clip(Some((fixture::WISP_CLIP, 0.9)));
    assert_eq!(fx.hosts[k].sys.p.len, 0, "an immediate stop removes every particle");
    fx.step(1.0 / 60.0, &none, [100.0, 0.0, 50.0]);
    assert!(!fx.hosts[k].active);
}

fn rope_renderer(extra: Value) -> super::render::Renderer {
    let mut v = json!({
        "_class": "C_OP_RenderRopes",
        "m_flTextureVWorldSize": lit(10.0),
        "m_nMinTesselation": 1,
        "m_nMaxTesselation": 1,
        "m_bGammaCorrectVertexColors": false
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    let mut cx = ParseCx::default();
    let r = super::render::Renderer::parse("C_OP_RenderRopes", &v, 12, &mut cx).expect("rope renderer");
    assert!(cx.diagnostics.is_empty(), "{:?}", cx.diagnostics);
    r
}

/// Four particles on a 10 inch square in the XY plane, oldest first.
fn square(segments: [f32; 4]) -> Particles {
    let mut consts_s = [0.0; fl::COUNT];
    consts_s[fl::ALPHA] = 1.0;
    consts_s[fl::ALPHA2] = 1.0;
    consts_s[fl::RADIUS] = 2.0;
    let mut consts_v = [[0.0; 3]; fl::COUNT];
    consts_v[fl::COLOR] = [1.0; 3];
    let mut p = Particles::new(consts_s, consts_v, 8);
    for (i, pos) in [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0]].into_iter().enumerate() {
        p.push(0.0);
        p.v[fl::POSITION][i] = pos;
        p.s[fl::PARTICLE_ID][i] = i as f32;
        p.s[47][i] = segments[i];
    }
    p
}

fn build_rope(r: &mut super::render::Renderer, p: &Particles) -> Vec<Instance> {
    let mut st = State::new(8);
    let view = View { camera: [5.0, 5.0, 200.0], to_world: Mat4::IDENTITY, world_scale: 1.0, projection: 2.0 };
    let (mut out, mut order, mut dist) = (Vec::new(), Vec::new(), Vec::new());
    r.build(&mut st, p, &view, None, &mut out, &mut order, &mut dist);
    out
}

#[test]
fn particles_ropes_thread_strip_quads_through_the_particles() {
    let p = square([0.0; 4]);
    let mut open = rope_renderer(json!({}));
    let quads = build_rope(&mut open, &p);
    assert_eq!(quads.len(), 3, "an open rope of 4 particles has 3 segments");
    for q in &quads {
        assert_eq!(q.axis_u[3], 3.0, "strip quad mode");
        assert_eq!((q.pos_radius[3], q.axis_v[3]), (2.0, 2.0));
        // A unit width axis across the spline, nearly flat under the camera above.
        let across = [q.axis_u[0], q.axis_u[1], q.axis_u[2]];
        assert!((length(across) - 1.0).abs() < 1e-4 && across[2].abs() < 0.1, "{across:?}");
    }
    // The first quad starts along +X, so its width runs along Y.
    assert!(quads[0].axis_u[1].abs() > 0.99, "{:?}", quads[0].axis_u);
    // Consecutive quads share their joint, V runs on with the arc length over the world size.
    assert_eq!([quads[0].axis_v[0], quads[0].axis_v[1]], [quads[1].pos_radius[0], quads[1].pos_radius[1]]);
    assert_eq!((quads[0].uv_a[1], quads[0].uv_a[3]), (0.0, 1.0));
    assert_eq!((quads[1].uv_a[1], quads[1].uv_a[3]), (1.0, 2.0));
    // u runs against the width axis: 1 on the minus side.
    assert_eq!((quads[0].uv_a[0], quads[0].uv_a[2]), (1.0, 0.0));
    // The width axis keeps one hemisphere along the strip.
    for w in quads.windows(2) {
        assert!(crate::math::dot([w[0].misc[0], w[0].misc[1], w[0].misc[2]], [w[1].axis_u[0], w[1].axis_u[1], w[1].axis_u[2]]) >= 0.0);
    }

    // A closed loop adds the segment back to the first particle.
    let mut closed = rope_renderer(json!({ "m_bClosedLoop": true }));
    let quads = build_rope(&mut closed, &p);
    assert_eq!(quads.len(), 4);
    assert_eq!([quads[3].axis_v[0], quads[3].axis_v[1]], [0.0, 0.0]);

    // The split field breaks the rope where the segment id changes.
    let split = square([0.0, 0.0, 1.0, 1.0]);
    let quads = build_rope(&mut rope_renderer(json!({})), &split);
    assert_eq!(quads.len(), 2);

    // Tessellation subdivides each segment into a power of two of pieces along the spline.
    let mut fine = rope_renderer(json!({ "m_nMinTesselation": 3, "m_nMaxTesselation": 3 }));
    let quads = build_rope(&mut fine, &p);
    assert_eq!(quads.len(), 3 * 4);
    // The spline passes through the particles.
    assert_eq!([quads[3].axis_v[0], quads[3].axis_v[1]], [10.0, 0.0]);

    // V offsets by its input, and clamps on request.
    let mut offset = rope_renderer(json!({ "m_flTextureVOffset": lit(5.0) }));
    let quads = build_rope(&mut offset, &p);
    assert_eq!((quads[0].uv_a[1], quads[0].uv_a[3]), (0.5, 1.5));
    let mut clamped = rope_renderer(json!({ "m_bClampV": true }));
    let quads = build_rope(&mut clamped, &p);
    assert_eq!((quads[2].uv_a[1], quads[2].uv_a[3]), (1.0, 1.0));
}

#[test]
fn particles_ropes_carry_v_when_the_first_particle_dies() {
    let mut r = rope_renderer(json!({ "m_flTextureVWorldSize": lit(25.0) }));
    let p = square([0.0; 4]);
    let _ = build_rope(&mut r, &p);
    // The oldest particle goes: the next frame's rope carries the last measured segment's
    // length (10), wrapped to the world size, into its V.
    let mut q = square([0.0; 4]);
    q.s[fl::PARTICLE_ID][0] = 9.0;
    let quads = build_rope(&mut r, &q);
    assert!((quads[0].uv_a[1] - 0.4).abs() < 1e-6, "{}", quads[0].uv_a[1]);
    // Another frame with the same first particle carries nothing more.
    let quads = build_rope(&mut r, &q);
    assert!((quads[0].uv_a[1] - 0.4).abs() < 1e-6);
}

/// A bundle whose system emits one particle per row of a skinned snapshot and keeps them on
/// the bones: three points, two on `hand` (bound 10 inches along X), one on `root`.
fn snapshot_bundle(emitter: Value, extra_ops: Vec<Value>) -> Bundle {
    let mut ops = vec![json!({ "_class": "C_OP_SnapshotRigidSkinToBones", "m_nControlPointNumber": 0 })];
    ops.extend(extra_ops);
    let def = json!({
        "_class": "CParticleSystemDefinition", "m_nBehaviorVersion": 12, "m_nMaxParticles": 16,
        "m_hSnapshot": "particles/sigil/points.vsnap", "m_nSnapshotControlPoint": 1,
        "m_flConstantLifespan": 100.0,
        "m_Emitters": [emitter],
        "m_Initializers": [{ "_class": "C_INIT_InitSkinnedPositionFromCPSnapshot", "m_nSnapshotControlPointNumber": 1, "m_nControlPointNumber": 0 }],
        "m_Operators": ops,
        "m_Renderers": [{ "_class": "C_OP_RenderSprites" }]
    });
    let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let mut hand = identity;
    hand[12] = 10.0;
    let json = json!({
        "schema": 1,
        "systems": { "particles/sigil/points.vpcf": { "data": def, "format": "vpcf66" } },
        "textures": {}, "attachments": [],
        "effects": [{ "id": "e", "system": "particles/sigil/points.vpcf", "source": "model-particles-list",
                      "attachment": { "type": "absorigin_follow", "point": "", "offset": [0, 0, 0] } }],
        "bones": [{ "name": "root", "bind": identity }, { "name": "hand", "bind": hand }],
        "snapshots": { "particles/sigil/points.vsnap": { "numParticles": 3, "strings": ["root", "hand"], "columns": [
            { "name": "position", "type": "float3", "data": "vector", "values": [10.0, 0.0, 1.0, 10.0, 0.0, 2.0, 0.0, 0.0, 0.0] },
            { "name": "skinning", "type": "skinning", "data": "skinning", "values": {
                "joints": [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
                "weights": [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0] } },
            { "name": "creation_time", "type": "float", "data": "float", "values": [0.0, 0.0, 0.5] }
        ] } }
    });
    Bundle::from_json(&json, |_| Err("none".into())).unwrap()
}

#[test]
fn particles_snapshots_skin_points_to_the_bones() {
    let b = snapshot_bundle(json!({ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(1.0), "m_nSnapshotControlPoint": 1 }), vec![]);
    assert_eq!(b.bones.len(), 2);
    let snap = &b.snapshots["particles/sigil/points.vsnap"];
    assert_eq!((snap.count, snap.skinned_rows()), (3, 3));
    let mut fx = Effects::new(&b);
    assert!(fx.diagnostics().is_empty(), "{:?}", fx.diagnostics());
    // The hand is raised 5 inches from its bind pose.
    let lift = std::cell::Cell::new(5.0f32);
    let posed = |n: &str| match n {
        "hand" => Some(Mat4::translation([10.0, 0.0, lift.get()])),
        "root" => Some(Mat4::IDENTITY),
        _ => None,
    };
    fx.step(1.0 / 30.0, &posed, [100.0, 0.0, 0.0]);
    let p = &fx.hosts[0].sys.p;
    // The instantaneous emitter releases a row as its creation time passes, not its count.
    assert_eq!(p.len, 2, "rows at 0 s; the third waits for 0.5 s");
    assert_eq!(p.v[fl::POSITION][0], [10.0, 0.0, 6.0]);
    assert_eq!(p.v[fl::POSITION][1], [10.0, 0.0, 7.0]);
    for _ in 0..20 {
        fx.step(1.0 / 30.0, &posed, [100.0, 0.0, 0.0]);
    }
    let p = &fx.hosts[0].sys.p;
    assert_eq!(p.len, 3);
    assert_eq!(p.v[fl::POSITION][2], [0.0, 0.0, 0.0]);
    // The rigid skin operator keeps them on the moving bones.
    lift.set(9.0);
    fx.step(1.0 / 30.0, &posed, [100.0, 0.0, 0.0]);
    let p = &fx.hosts[0].sys.p;
    assert_eq!(p.v[fl::POSITION][0], [10.0, 0.0, 10.0]);
    assert_eq!(p.v[fl::POSITION][2], [0.0, 0.0, 0.0]);
}

#[test]
fn particles_snapshot_counts_scale_emitters_and_inputs() {
    // A continuous rate scales by the bound snapshot's rows: 2 a second times 3 rows.
    let b = snapshot_bundle(json!({ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(2.0), "m_nSnapshotControlPoint": 1 }), vec![]);
    let mut fx = Effects::new(&b);
    let none = |_: &str| None;
    for _ in 0..30 {
        fx.step(1.0 / 30.0, &none, [100.0, 0.0, 0.0]);
    }
    assert_eq!(fx.hosts[0].sys.p.len, 6);
    // Without a model on control point 0 the points are taken as they are.
    assert_eq!(fx.hosts[0].sys.p.v[fl::POSITION][0], [10.0, 0.0, 1.0]);

    // PF_TYPE_SNAPSHOT_COUNT reads the row count of its control point's snapshot.
    let count = json!({ "m_nType": "PF_TYPE_SNAPSHOT_COUNT", "m_nControlPoint": 1, "m_nMapType": "PF_MAP_TYPE_DIRECT" });
    let b = snapshot_bundle(json!({ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": count }), vec![]);
    let mut fx = Effects::new(&b);
    fx.step(1.0 / 30.0, &none, [100.0, 0.0, 0.0]);
    assert_eq!(fx.hosts[0].sys.p.len, 3);

    // A snapshot the bundle lacks is a diagnostic, not a guess.
    let mut b = snapshot_bundle(json!({ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(1.0), "m_nSnapshotControlPoint": 1 }), vec![]);
    b.snapshots.clear();
    let mut fx = Effects::new(&b);
    assert!(fx.diagnostics().iter().any(|d| d.contains("snapshot particles/sigil/points.vsnap is not in the bundle")), "{:?}", fx.diagnostics());
    fx.step(1.0 / 30.0, &none, [100.0, 0.0, 0.0]);
    assert_eq!(fx.hosts[0].sys.p.len, 0, "a bound emitter with no snapshot emits nothing");
}

#[test]
fn particles_snapshot_skin_blends_weights() {
    let a = Mat4::translation([2.0, 0.0, 0.0]);
    let b = Mat4::translation([0.0, 4.0, 0.0]);
    let palette = [a, b];
    // One valid bone takes its matrix whole, whatever the weight.
    let m = snapshot::skin_matrix(&palette, [1.0, fl::BONE_NONE, fl::BONE_NONE, fl::BONE_NONE], [0.25, 0.0, 0.0]).unwrap();
    assert_eq!(m.point([0.0; 3]), [0.0, 4.0, 0.0]);
    // Two blend: the last weight is what the first three leave.
    let m = snapshot::skin_matrix(&palette, [0.0, 1.0, fl::BONE_NONE, fl::BONE_NONE], [0.25, 0.0, 0.0]).unwrap();
    assert_eq!(m.point([0.0; 3]), [0.5, 0.0, 0.0]);
    let m = snapshot::skin_matrix(&palette, [0.0, fl::BONE_NONE, fl::BONE_NONE, 1.0], [0.25, 0.0, 0.0]).unwrap();
    assert_eq!(m.point([0.0; 3]), [0.5, 3.0, 0.0]);
    assert!(snapshot::skin_matrix(&palette, [fl::BONE_NONE, 7.0, fl::BONE_NONE, fl::BONE_NONE], [1.0, 0.0, 0.0]).is_none());
    // The walk wraps past the last row, moved on by the full loop increment.
    assert_eq!(snapshot::SkinnedInit::walk_row(5, 4, 0), 1);
    assert_eq!(snapshot::SkinnedInit::walk_row(5, 4, 1), 2);
}
