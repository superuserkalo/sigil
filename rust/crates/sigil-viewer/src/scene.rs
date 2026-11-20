//! The files `deadlock_core::viewer` writes beside the models, per `docs/design/3d-viewer.md`
//! (schema 1): a hero's JSON sidecar (clips, the idle to start on, per-clip loop or hold) and a
//! Hideout `scene.json` (placed models, lights, fog, sky and reflection cubes, a camera and
//! where the hero stands). Everything is glTF space: Y up, meters, column-major 4x4 matrices;
//! colours are sRGB unless a field says linear. Unknown fields are ignored, and a field the
//! pipeline lists in `omissions` is simply absent, so the renderer never guesses it.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::material::srgb_to_linear;
use crate::math::{Mat4, Vec3, quat_axis_angle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playmode {
    #[default]
    Loop,
    /// Play once and hold the last frame.
    Hold,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    pub duration: Option<f32>,
    pub playback: Playmode,
    pub action: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidecar {
    pub clips: Vec<String>,
    pub idle: Option<String>,
    pub clip_info: Vec<ClipInfo>,
    pub hero: Option<String>,
    pub display_name: Option<String>,
    /// `modded` or `original`.
    pub variant: Option<String>,
    pub issues: Vec<String>,
}

impl Sidecar {
    pub fn parse(text: &str) -> Sidecar {
        let Ok(v) = serde_json::from_str::<Value>(text) else { return Sidecar::default() };
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        let clips = v
            .get("clips")
            .and_then(|c| c.as_array())
            .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string).or_else(|| c.get("name").and_then(|n| n.as_str()).map(str::to_string))).collect())
            .unwrap_or_default();
        let clip_info = v
            .get("clipInfo")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|c| {
                        Some(ClipInfo {
                            name: c.get("name")?.as_str()?.to_string(),
                            duration: c.get("durationSeconds").and_then(|d| d.as_f64()).map(|d| d as f32),
                            playback: if c.get("playback").and_then(|p| p.as_str()) == Some("hold") { Playmode::Hold } else { Playmode::Loop },
                            action: c.get("action").and_then(|a| a.as_str()).map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Sidecar {
            clips,
            idle: s("idleClip").or_else(|| s("idle_clip")),
            clip_info,
            hero: s("hero"),
            display_name: s("displayName"),
            variant: s("variant"),
            issues: v.get("issues").and_then(|i| i.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        }
    }

    pub fn read(path: &Path) -> Sidecar {
        std::fs::read_to_string(path).map(|t| Sidecar::parse(&t)).unwrap_or_default()
    }

    pub fn playmode(&self, clip: &str) -> Playmode {
        self.clip_info.iter().find(|c| c.name == clip).map(|c| c.playback).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLight {
    pub kind: LightKind,
    /// Position for point and spot lights.
    pub position: Vec3,
    /// Toward the light for a directional light; the way a spot shines.
    pub direction: Vec3,
    /// Linear colour times intensity.
    pub color: Vec3,
    pub range: f32,
    /// Spot cone, inner and outer half-angles in radians.
    pub cone: Option<(f32, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    /// Linear.
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
    pub strength: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneModel {
    pub glb: PathBuf,
    pub transform: Mat4,
    /// Linear multiplier and opacity.
    pub tint: [f32; 4],
    /// Draw only this primitive of the glb's mesh (an aggregate fragment).
    pub primitive: Option<usize>,
    pub animation: Option<String>,
}

/// Six Radiance faces in glTF cube order (+X, -X, +Y, -Y, +Z, -Z) and their scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    pub faces: [PathBuf; 6],
    /// Linear `exp2(exposureBias) * tint`.
    pub scale: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SceneHero {
    pub glb: PathBuf,
    pub sidecar: Option<PathBuf>,
    pub transform: Mat4,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneDesc {
    pub models: Vec<SceneModel>,
    pub lights: Vec<SceneLight>,
    pub fog: Option<Fog>,
    pub sky: Option<Cube>,
    pub reflection: Option<Cube>,
    /// Eye, target, vertical field of view in degrees.
    pub camera: Option<(Vec3, Vec3, f32)>,
    pub hero_anchor: Option<Mat4>,
    pub hero: Option<SceneHero>,
    pub omissions: Vec<String>,
    pub failures: Vec<String>,
    pub exposure: Option<f32>,
}

fn num(v: &Value) -> Option<f32> {
    v.as_f64().map(|f| f as f32)
}

fn vec3(v: Option<&Value>) -> Option<Vec3> {
    let a = v?.as_array()?;
    (a.len() >= 3).then(|| [num(&a[0]).unwrap_or(0.0), num(&a[1]).unwrap_or(0.0), num(&a[2]).unwrap_or(0.0)])
}

/// An sRGB colour (0 to 1) as linear.
fn srgb(v: Option<&Value>) -> Option<Vec3> {
    vec3(v).map(|c| c.map(|x| srgb_to_linear(x.clamp(0.0, 1.0))))
}

fn matrix(v: Option<&Value>) -> Option<Mat4> {
    let a = v?.as_array().filter(|a| a.len() == 16)?;
    let mut m = [0.0f32; 16];
    for (o, x) in m.iter_mut().zip(a) {
        *o = num(x)?;
    }
    Some(Mat4::from_cols_slice(&m))
}

fn path(base: &Path, v: Option<&Value>) -> Option<PathBuf> {
    let p = PathBuf::from(v?.as_str()?);
    Some(if p.is_absolute() { p } else { base.join(p) })
}

fn cube(base: &Path, v: Option<&Value>) -> Option<Cube> {
    let v = v?;
    let names = v.get("cubemap")?.as_array().filter(|a| a.len() == 6)?;
    let mut faces: Vec<PathBuf> = names.iter().filter_map(|n| path(base, Some(n))).collect();
    if faces.len() != 6 {
        return None;
    }
    // The files are px, nx, py, ny, pz, nz in Source axes: Source +X is glTF +Z, Source +Y is
    // glTF +X and Source +Z is glTF +Y.
    let source = std::mem::take(&mut faces);
    let order = [2, 3, 4, 5, 0, 1];
    let faces: [PathBuf; 6] = std::array::from_fn(|i| source[order[i]].clone());
    let bias = v.get("exposureBias").and_then(num).unwrap_or(0.0);
    let tint = vec3(v.get("tint")).unwrap_or([1.0; 3]).map(|x| srgb_to_linear(x.clamp(0.0, 1.0)));
    let k = 2f32.powf(bias);
    Some(Cube { faces, scale: tint.map(|t| t * k) })
}

impl SceneDesc {
    pub fn parse(text: &str, base: &Path) -> Result<SceneDesc, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("scene.json: {e}"))?;
        let mut s = SceneDesc::default();
        for m in v.get("models").and_then(|m| m.as_array()).into_iter().flatten() {
            let Some(glb) = path(base, m.get("glb")) else { continue };
            let tint = match m.get("tint").and_then(|t| t.as_array()) {
                Some(t) if t.len() >= 3 => {
                    let c = |i: usize| t.get(i).and_then(num).unwrap_or(1.0);
                    [srgb_to_linear(c(0)), srgb_to_linear(c(1)), srgb_to_linear(c(2)), c(3)]
                }
                _ => [1.0; 4],
            };
            s.models.push(SceneModel {
                glb,
                transform: matrix(m.get("transform")).unwrap_or_default(),
                tint,
                primitive: m.get("primitive").and_then(|p| p.as_u64()).map(|p| p as usize),
                animation: m.get("animation").and_then(|a| a.as_str()).map(str::to_string),
            });
        }
        for l in v.get("lights").and_then(|l| l.as_array()).into_iter().flatten() {
            let kind = match l.get("kind").or_else(|| l.get("type")).and_then(|k| k.as_str()).unwrap_or("point") {
                "directional" | "sun" | "env" => LightKind::Directional,
                "spot" => LightKind::Spot,
                _ => LightKind::Point,
            };
            let intensity = l.get("intensity").and_then(num).unwrap_or(1.0);
            let c = srgb(l.get("color")).unwrap_or([1.0; 3]);
            let shine = vec3(l.get("direction")).unwrap_or([0.0, -1.0, 0.0]);
            let cone = l.get("cone").and_then(|c| c.as_array()).filter(|c| c.len() == 2).map(|c| {
                let (inner, outer) = (num(&c[0]).unwrap_or(0.0).to_radians(), num(&c[1]).unwrap_or(45.0).to_radians());
                (inner.min(outer), outer)
            });
            s.lights.push(SceneLight {
                kind,
                position: vec3(l.get("position")).unwrap_or([0.0; 3]),
                direction: if kind == LightKind::Directional { shine.map(|x| -x) } else { shine },
                color: c.map(|x| x * intensity),
                range: l.get("range").and_then(num).unwrap_or(0.0),
                cone: if kind == LightKind::Spot { cone } else { None },
            });
        }
        if let Some(f) = v.get("fog").filter(|f| f.is_object()) {
            let fade = f.get("fadeIn").and_then(|a| a.as_array());
            let start = fade.and_then(|a| a.first()).and_then(num).unwrap_or(0.0);
            let draw = f.get("drawDistance").and_then(num).unwrap_or(60.0);
            s.fog = Some(Fog {
                color: srgb(f.get("color")).unwrap_or([0.5; 3]),
                start,
                end: draw.max(start + 0.1),
                strength: f.get("strength").and_then(num).unwrap_or(1.0).clamp(0.0, 1.0),
            });
        }
        s.sky = cube(base, v.get("sky"));
        s.reflection = cube(base, v.get("reflection"));
        if let Some(c) = v.get("camera")
            && let (Some(pos), Some(target)) = (vec3(c.get("position")), vec3(c.get("target")))
        {
            s.camera = Some((pos, target, c.get("fov").and_then(num).unwrap_or(60.0)));
        }
        s.hero_anchor = matrix(v.get("heroAnchor")).or_else(|| {
            let spawn = v.get("spawn")?;
            let p = vec3(spawn.get("position"))?;
            let yaw = spawn.get("yawDegrees").and_then(num).unwrap_or(0.0).to_radians() + std::f32::consts::PI;
            Some(Mat4::from_trs(p, quat_axis_angle([0.0, 1.0, 0.0], yaw), [1.0; 3]))
        });
        if let Some(h) = v.get("hero")
            && let Some(glb) = path(base, h.get("glb"))
        {
            s.hero = Some(SceneHero { glb, sidecar: path(base, h.get("sidecar")), transform: matrix(h.get("transform")).or(s.hero_anchor).unwrap_or_default() });
        }
        let strings = |k: &str| v.get(k).and_then(|o| o.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        s.omissions = strings("omissions");
        s.failures = strings("failures");
        s.exposure = v.get("exposure").and_then(num);
        Ok(s)
    }

    pub fn read(path: &Path) -> Result<SceneDesc, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        SceneDesc::parse(&text, path.parent().unwrap_or(Path::new(".")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_reads_the_contract() {
        let text = r#"{"schema":1,"hero":"inferno","displayName":"Infernus","variant":"modded",
            "clips":["shop_menu_base","primary_stand_reload"],"idleClip":"shop_menu_base",
            "clipInfo":[{"name":"shop_menu_base","durationSeconds":2.97,"fps":30.0,"playback":"hold","action":"idle"}],
            "issues":["changesNotShown:addons/pak05_dir.vpk"]}"#;
        let s = Sidecar::parse(text);
        assert_eq!(s.idle.as_deref(), Some("shop_menu_base"));
        assert_eq!(s.clips.len(), 2);
        assert_eq!(s.playmode("shop_menu_base"), Playmode::Hold);
        assert_eq!(s.playmode("primary_stand_reload"), Playmode::Loop);
        assert_eq!(s.display_name.as_deref(), Some("Infernus"));
        assert_eq!(s.issues.len(), 1);
        assert_eq!(Sidecar::parse(r#"{"idleClip":null}"#).idle, None);
        assert_eq!(Sidecar::parse("not json"), Sidecar::default());
    }

    #[test]
    fn scene_reads_the_contract() {
        let id: Vec<f32> = Mat4::translation([1.0, 0.0, 2.0]).to_cols_array().to_vec();
        let text = serde_json::json!({
            "schema": 1,
            "models": [{ "glb": "models/a.glb", "transform": id, "tint": [0.5, 0.5, 0.5, 1.0], "primitive": 3, "animation": null }],
            "lights": [
                { "kind": "spot", "color": [1, 1, 1], "intensity": 80.0, "position": [0, 2, 0], "direction": [0, -1, 0], "range": 12.7, "cone": [20, 45] },
                { "kind": "directional", "color": [1, 1, 1], "intensity": 2.0, "direction": [0, -1, 0] }
            ],
            "fog": { "color": [0.5, 0.5, 0.5], "strength": 0.6, "drawDistance": 60.0, "fadeIn": [2, 4] },
            "sky": { "cubemap": ["sky/px.hdr", "sky/nx.hdr", "sky/py.hdr", "sky/ny.hdr", "sky/pz.hdr", "sky/nz.hdr"], "exposureBias": 1.0, "tint": [1, 1, 1] },
            "camera": { "position": [0, 1.6, -4], "target": [0, 1, 0], "fov": 60.0 },
            "heroAnchor": id,
            "hero": { "glb": "../../heroes/k/inferno.glb", "sidecar": "../../heroes/k/inferno.json", "transform": id },
            "omissions": ["tonemap", "particles"]
        })
        .to_string();
        let s = SceneDesc::parse(&text, Path::new("/cache/hideout/key")).unwrap();
        let m = &s.models[0];
        assert_eq!(m.glb, PathBuf::from("/cache/hideout/key/models/a.glb"));
        assert_eq!(m.transform.point([0.0; 3]), [1.0, 0.0, 2.0]);
        assert_eq!(m.primitive, Some(3));
        assert!((m.tint[0] - srgb_to_linear(0.5)).abs() < 1e-6);
        assert_eq!(s.lights[0].kind, LightKind::Spot);
        assert_eq!(s.lights[0].direction, [0.0, -1.0, 0.0]);
        assert!(s.lights[0].cone.is_some());
        assert_eq!(s.lights[1].direction, [0.0, 1.0, 0.0], "a directional light points toward the light");
        let sky = s.sky.unwrap();
        assert!(sky.faces[0].ends_with("sky/py.hdr"), "glTF +X is Source +Y");
        assert!(sky.faces[4].ends_with("sky/px.hdr"), "glTF +Z is Source +X");
        assert_eq!(sky.scale, [2.0; 3]);
        assert!(s.hero.is_some() && s.hero_anchor.is_some());
        assert_eq!(s.omissions, vec!["tonemap", "particles"]);
    }

    #[test]
    fn an_empty_scene_is_still_a_scene() {
        let s = SceneDesc::parse("{}", Path::new(".")).unwrap();
        assert!(s.models.is_empty() && s.lights.is_empty() && s.sky.is_none());
    }
}
