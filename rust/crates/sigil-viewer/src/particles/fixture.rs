//! A stand-in particle bundle built in code, no Valve asset: an ember plume rising from the
//! stand-in hero's right hand (bone `hand_R` of `crate::fixture`), with a sheet-animated glow
//! child that follows the hand and a spark child born where embers die; and a Hideout variant
//! with a candle flame and dust motes at fixed Source-space transforms. The hero also carries
//! an animation-event effect: a rope wisp rising from the left hand that the idle clip starts
//! at its first frame and stops near its end. Written in the bundle contract's shape
//! (`bundle.json` plus `tex/*.png`), so the loader, the simulation and the renderer run end
//! to end on any machine.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::bundle::Bundle;

pub const EMBER_TEXTURE: &str = "materials/particle/sigil_ember.vtex";
pub const GLOW_TEXTURE: &str = "materials/particle/sigil_glow_sheet.vtex";
pub const WISP_TEXTURE: &str = "materials/particle/sigil_wisp_rope.vtex";
pub const WISP: &str = "particles/sigil/hand_wisp_rope.vpcf";
/// The stand-in hero's clip (`crate::fixture`), which starts and stops the wisp.
pub const WISP_CLIP: &str = "hero_shop";
/// Where in the clip the wisp stops (its start is the clip's first frame).
pub const WISP_STOP: f32 = 0.85;
pub const PLUME: &str = "particles/sigil/hand_ember_plume.vpcf";
pub const GLOW: &str = "particles/sigil/hand_glow.vpcf";
pub const SPARKS: &str = "particles/sigil/ember_sparks.vpcf";
pub const CANDLE: &str = "particles/sigil/candle_flame.vpcf";
pub const DUST: &str = "particles/sigil/dust_motes.vpcf";

/// The sheet's grid: 4 x 4 frames.
pub const SHEET_GRID: u32 = 4;

fn lit(x: f32) -> Value {
    json!({ "m_nType": "PF_TYPE_LITERAL", "m_flLiteralValue": x })
}

fn random(min: f32, max: f32) -> Value {
    json!({ "m_nType": "PF_TYPE_RANDOM_UNIFORM", "m_flRandomMin": min, "m_flRandomMax": max, "m_nRandomMode": "PF_RANDOM_MODE_CONSTANT" })
}

fn init_float(field: i32, value: Value) -> Value {
    json!({ "_class": "C_INIT_InitFloat", "m_InputValue": value, "m_nOutputField": field })
}

/// A 64 px ember: a white hot core in a soft falloff (tinted by the particle colour).
pub fn ember_rgba() -> (u32, u32, Vec<u8>) {
    let n = 64u32;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    let c = (n as f32 - 1.0) * 0.5;
    for y in 0..n {
        for x in 0..n {
            let d = (((x as f32 - c) / c).powi(2) + ((y as f32 - c) / c).powi(2)).sqrt();
            let a = (1.0 - d).clamp(0.0, 1.0);
            let core = (1.0 - d * 2.5).clamp(0.0, 1.0);
            let v = (180.0 + 75.0 * core) as u8;
            out.extend_from_slice(&[v, v, v, (a * a * 255.0) as u8]);
        }
    }
    (n, n, out)
}

/// A 4 x 4 sheet, 64 px cells: a glowing ring that widens and thins over the 16 frames.
pub fn glow_sheet_rgba() -> (u32, u32, Vec<u8>) {
    let cell = 64u32;
    let n = cell * SHEET_GRID;
    let mut out = vec![0u8; (n * n * 4) as usize];
    for frame in 0..SHEET_GRID * SHEET_GRID {
        let (cx0, cy0) = ((frame % SHEET_GRID) * cell, (frame / SHEET_GRID) * cell);
        let t = frame as f32 / 15.0;
        let ring = 0.2 + 0.6 * t;
        let width = 0.25 - 0.15 * t;
        let c = (cell as f32 - 1.0) * 0.5;
        for y in 0..cell {
            for x in 0..cell {
                let d = (((x as f32 - c) / c).powi(2) + ((y as f32 - c) / c).powi(2)).sqrt();
                let r = (1.0 - ((d - ring).abs() / width)).clamp(0.0, 1.0);
                let core = (1.0 - d / 0.35).clamp(0.0, 1.0) * (1.0 - t);
                let a = (r * r + core * 0.6).min(1.0);
                let i = (((cy0 + y) * n + cx0 + x) * 4) as usize;
                out[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
            }
        }
    }
    (n, n, out)
}

/// A 32 x 64 rope texture: soft across its width (u), faint streaks along its length (v).
pub fn wisp_rgba() -> (u32, u32, Vec<u8>) {
    let (w, h) = (32u32, 64u32);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let across = ((x as f32 + 0.5) / w as f32 - 0.5).abs() * 2.0;
            let edge = (1.0 - across).clamp(0.0, 1.0);
            let streak = 0.75 + 0.25 * (y as f32 / h as f32 * std::f32::consts::TAU * 2.0 + across * 3.0).sin();
            let a = edge * edge * streak;
            let v = (200.0 + 55.0 * (1.0 - across)) as u8;
            out.extend_from_slice(&[v, v, v, (a * 255.0) as u8]);
        }
    }
    (w, h, out)
}

/// A wisp of smoke threaded as a rope: particles stream up from the hand, swirl and widen,
/// and `C_OP_RenderRopes` splines a ribbon through them.
fn wisp() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 48,
        "m_ConstantColor": [140, 210, 255, 255],
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(24.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinSphereTransform", "m_fRadiusMax": lit(0.2), "m_fSpeedMin": lit(1.0), "m_fSpeedMax": lit(2.0) },
            init_float(1, lit(1.4)),
            init_float(3, lit(1.2))
        ],
        "m_Operators": [
            { "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, 70.0], "m_fDrag": lit(0.04) },
            { "_class": "C_OP_OscillateVectorSimple", "m_Rate": [2.0, 2.0, 0.0], "m_Frequency": [1.1, 0.8, 0.0] },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeInSimple", "m_flFadeInTime": 0.15 },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.6 },
            { "_class": "C_OP_InterpolateRadius", "m_flStartScale": lit(1.0), "m_flEndScale": lit(3.0) }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderRopes",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ADD",
            "m_flOverbrightFactor": lit(1.6),
            "m_flTextureVWorldSize": lit(12.0),
            "m_flTextureVScrollRate": lit(-6.0),
            "m_nMaxTesselation": 8,
            "m_vecTexturesInput": [{ "m_hTexture": WISP_TEXTURE }]
        }]
    })
}

fn sheet_json() -> Value {
    let k = 1.0 / SHEET_GRID as f32;
    let frames: Vec<Value> = (0..SHEET_GRID * SHEET_GRID)
        .map(|f| {
            let (u, v) = ((f % SHEET_GRID) as f32 * k, (f / SHEET_GRID) as f32 * k);
            json!({ "duration": 1.0 / 16.0, "images": [[u, v, u + k, v + k]] })
        })
        .collect();
    json!({ "sequences": [{ "id": 0, "clamp": false, "alphaCrop": false, "noColor": false, "noAlpha": false, "duration": 1.0, "frames": frames }] })
}

fn plume() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 96,
        "m_flConstantRadius": 1.0,
        "m_ConstantColor": [255, 180, 90, 255],
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(24.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinSphereTransform", "m_fRadiusMax": lit(1.5), "m_fSpeedMin": lit(4.0), "m_fSpeedMax": lit(10.0) },
            init_float(1, random(0.8, 1.6)),
            init_float(3, random(0.6, 1.3)),
            init_float(4, random(0.0, 360.0)),
            init_float(5, random(-90.0, 90.0)),
            { "_class": "C_INIT_RandomColor", "m_ColorMin": [255, 200, 110], "m_ColorMax": [255, 120, 50] }
        ],
        "m_Operators": [
            { "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, 70.0], "m_fDrag": lit(0.08) },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeInSimple", "m_flFadeInTime": 0.1 },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.45 },
            { "_class": "C_OP_InterpolateRadius", "m_flEndScale": lit(0.25), "m_bEaseInAndOut": true },
            { "_class": "C_OP_ColorInterpolate", "m_ColorFade": [200, 40, 20] },
            { "_class": "C_OP_SpinUpdate" }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderSprites",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ADD",
            "m_flOverbrightFactor": lit(2.0),
            "m_vecTexturesInput": [{ "m_hTexture": EMBER_TEXTURE }]
        }],
        "m_Children": [{ "m_ChildRef": GLOW }, { "m_ChildRef": SPARKS }]
    })
}

fn glow() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 8,
        "m_ConstantColor": [255, 150, 70, 255],
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(2.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinSphereTransform" },
            init_float(1, lit(1.5)),
            init_float(3, random(5.0, 7.0)),
            init_float(7, lit(0.7))
        ],
        "m_Operators": [
            { "_class": "C_OP_PositionLock" },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeInSimple", "m_flFadeInTime": 0.2 },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.5 }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderSprites",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ALPHA",
            "m_nAnimationType": "ANIMATION_TYPE_FIT_LIFETIME",
            "m_flAnimationRate": 1.0,
            "m_vecTexturesInput": [{ "m_hTexture": GLOW_TEXTURE }]
        }]
    })
}

fn sparks() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 64,
        "m_flConstantRadius": 0.35,
        "m_ConstantColor": [255, 230, 160, 255],
        "m_Emitters": [{ "_class": "C_OP_InstantaneousEmitter", "m_nParticlesToEmit": lit(2.0), "m_flInitFromKilledParentParticles": 1.0 }],
        "m_Initializers": [
            { "_class": "C_INIT_InitFromParentKilled", "m_nAttributeToCopy": 0 },
            { "_class": "C_INIT_InitFromParentKilled", "m_nAttributeToCopy": 2 },
            { "_class": "C_INIT_PositionOffset", "m_OffsetMin": [-0.5, -0.5, 0.0], "m_OffsetMax": [0.5, 0.5, 1.0] },
            init_float(1, random(0.25, 0.5))
        ],
        "m_Operators": [
            { "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, -120.0] },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.6 }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderSprites",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ADD",
            "m_flOverbrightFactor": lit(3.0),
            "m_vecTexturesInput": [{ "m_hTexture": EMBER_TEXTURE }]
        }]
    })
}

fn candle() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 40,
        "m_ConstantColor": [255, 160, 60, 255],
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(30.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinSphereTransform", "m_fRadiusMax": lit(0.2), "m_fSpeedMin": lit(2.0), "m_fSpeedMax": lit(4.0), "m_LocalCoordinateSystemSpeedMin": [0.0, 0.0, 6.0], "m_LocalCoordinateSystemSpeedMax": [0.0, 0.0, 10.0] },
            init_float(1, random(0.35, 0.55)),
            init_float(3, random(0.9, 1.2))
        ],
        "m_Operators": [
            { "_class": "C_OP_BasicMovement", "m_Gravity": [0.0, 0.0, 20.0] },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_InterpolateRadius", "m_flEndScale": lit(0.3) },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.5 }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderSprites",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ADD",
            "m_flOverbrightFactor": lit(3.0),
            "m_vecTexturesInput": [{ "m_hTexture": EMBER_TEXTURE }]
        }]
    })
}

fn dust() -> Value {
    json!({
        "_class": "CParticleSystemDefinition",
        "m_nBehaviorVersion": 12,
        "m_nMaxParticles": 80,
        "m_nInitialParticles": 24,
        "m_ConstantColor": [255, 236, 210, 255],
        "m_Emitters": [{ "_class": "C_OP_ContinuousEmitter", "m_flEmitRate": lit(5.0) }],
        "m_Initializers": [
            { "_class": "C_INIT_CreateWithinBox", "m_vecMin": [-60.0, -60.0, 0.0], "m_vecMax": [60.0, 60.0, 80.0], "m_bLocalSpace": true },
            init_float(1, random(5.0, 9.0)),
            init_float(3, random(0.25, 0.5)),
            init_float(7, random(0.25, 0.5))
        ],
        "m_Operators": [
            { "_class": "C_OP_OscillateVectorSimple", "m_Rate": [3.0, 3.0, 1.5], "m_Frequency": [0.3, 0.4, 0.25] },
            { "_class": "C_OP_Decay" },
            { "_class": "C_OP_FadeInSimple", "m_flFadeInTime": 0.25 },
            { "_class": "C_OP_FadeOutSimple", "m_flFadeOutTime": 0.3 }
        ],
        "m_Renderers": [{
            "_class": "C_OP_RenderSprites",
            "m_nOutputBlendMode": "PARTICLE_OUTPUT_BLEND_MODE_ALPHA",
            "m_vecTexturesInput": [{ "m_hTexture": EMBER_TEXTURE }]
        }]
    })
}

fn texture_entry(name: &str, w: u32, h: u32, sheet: Value) -> (String, Value) {
    let file = format!("tex/{}.png", name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>());
    (name.to_string(), json!({ "file": file, "width": w, "height": h, "sheet": sheet }))
}

fn textures_json() -> Value {
    let (ew, eh, _) = ember_rgba();
    let (gw, gh, _) = glow_sheet_rgba();
    let (ww, wh, _) = wisp_rgba();
    let mut t = serde_json::Map::new();
    for (k, v) in [texture_entry(EMBER_TEXTURE, ew, eh, Value::Null), texture_entry(GLOW_TEXTURE, gw, gh, sheet_json()), texture_entry(WISP_TEXTURE, ww, wh, Value::Null)] {
        t.insert(k, v);
    }
    Value::Object(t)
}

/// The hero bundle's JSON: the plume on `attach_hand_r` (bone `hand_R`, a little below the
/// wrist) with its two children.
pub fn hero_json() -> Value {
    json!({
        "schema": 1,
        "owner": "sigil_standin",
        "systems": {
            PLUME: { "data": plume(), "format": "vpcf66" },
            GLOW: { "data": glow(), "format": "vpcf66" },
            SPARKS: { "data": sparks(), "format": "vpcf66" },
            WISP: { "data": wisp(), "format": "vpcf66" }
        },
        "textures": textures_json(),
        "attachments": [{ "name": "attach_hand_r", "ignoreRotation": false,
            "influences": [{ "bone": "hand_R", "offset": [0.0, 0.0, -3.0], "rotation": [0.0, 0.0, 0.0, 1.0], "weight": 1.0, "rootTransform": false }] }],
        "effects": [{
            "id": "ambient_0", "system": PLUME, "source": "hero-ambient",
            "attachment": { "type": "point_follow", "point": "attach_hand_r", "offset": [0.0, 0.0, 0.0] },
            "attachmentCp1": null, "configuration": null, "transform": null, "controlPoints": [], "startActive": true, "seed": null
        }, {
            "id": "anim_1", "system": WISP, "source": "anim-event",
            "attachment": { "type": "point_follow", "point": "hand_L", "offset": [0.0, 0.0, -2.0] },
            "attachmentCp1": null, "configuration": null, "transform": null, "controlPoints": [], "startActive": false, "seed": null,
            "clip": { "name": WISP_CLIP, "start": 0.0, "stops": [{ "time": WISP_STOP, "immediately": false, "endCap": false }], "stopOnClipChange": true }
        }],
        "issues": []
    })
}

/// A Source entity matrix (column major) at `origin` turned `yaw` degrees.
fn entity(origin: [f32; 3], yaw: f32) -> Value {
    let (s, c) = yaw.to_radians().sin_cos();
    json!([c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, origin[0], origin[1], origin[2], 1.0])
}

/// Where the stand-in Hideout's candle stands and its dust hangs, in Source inches.
pub const CANDLE_ORIGIN: [f32; 3] = [-118.0, 98.4, 52.0];
pub const DUST_ORIGIN: [f32; 3] = [0.0, 0.0, 10.0];

/// The Hideout bundle's JSON: a candle flame and a box of dust motes, map entities.
pub fn hideout_json() -> Value {
    json!({
        "schema": 1,
        "owner": "dl_hideout",
        "systems": {
            CANDLE: { "data": candle(), "format": "vpcf66" },
            DUST: { "data": dust(), "format": "vpcf66" }
        },
        "textures": textures_json(),
        "attachments": [],
        "effects": [
            { "id": "info_particle_system_0", "system": CANDLE, "source": "map-entity", "attachment": null, "attachmentCp1": null, "configuration": null,
              "transform": entity(CANDLE_ORIGIN, 0.0), "controlPoints": [], "startActive": true, "seed": null },
            { "id": "info_particle_system_1", "system": DUST, "source": "map-entity", "attachment": null, "attachmentCp1": null, "configuration": null,
              "transform": entity(DUST_ORIGIN, 30.0), "controlPoints": [], "startActive": true, "seed": null }
        ],
        "issues": []
    })
}

fn texture_pixels(file: &str) -> (u32, u32, Vec<u8>) {
    if file.contains("glow_sheet") {
        glow_sheet_rgba()
    } else if file.contains("wisp_rope") {
        wisp_rgba()
    } else {
        ember_rgba()
    }
}

/// The hero bundle, in memory.
pub fn hero_bundle() -> Bundle {
    Bundle::from_json(&hero_json(), |f| Ok(texture_pixels(f))).expect("fixture bundle")
}

/// The Hideout bundle, in memory.
pub fn hideout_bundle() -> Bundle {
    Bundle::from_json(&hideout_json(), |f| Ok(texture_pixels(f))).expect("fixture bundle")
}

fn write(dir: &Path, json: &Value) -> std::io::Result<PathBuf> {
    use image::ImageEncoder;
    std::fs::create_dir_all(dir.join("tex"))?;
    if let Some(t) = json.get("textures").and_then(Value::as_object) {
        for entry in t.values() {
            let Some(file) = entry.get("file").and_then(Value::as_str) else { continue };
            let (w, h, rgba) = texture_pixels(file);
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png).write_image(&rgba, w, h, image::ExtendedColorType::Rgba8).map_err(std::io::Error::other)?;
            std::fs::write(dir.join(file), png)?;
        }
    }
    let path = dir.join("bundle.json");
    std::fs::write(&path, serde_json::to_vec_pretty(json).map_err(std::io::Error::other)?)?;
    Ok(path)
}

/// Writes the hero bundle (`bundle.json` and `tex/*.png`) into `dir`; returns the JSON path.
pub fn write_fixture(dir: &Path) -> std::io::Result<PathBuf> {
    write(dir, &hero_json())
}

/// Writes the Hideout bundle into `dir`; returns the JSON path.
pub fn write_hideout_fixture(dir: &Path) -> std::io::Result<PathBuf> {
    write(dir, &hideout_json())
}
