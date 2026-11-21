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
    /// The friendly name the clip picker shows ("Idle", "Hero pose", or the humanised name).
    pub label: Option<String>,
    /// `idle`, `movement`, `combat` or `other`: the picker's groups.
    pub group: Option<String>,
    /// The animation graph's playback speed for the clip (Nurse Harrow's shop plays at 0.69).
    pub speed: f32,
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
    /// The cloth file beside the glb (`<codename>.cloth.json`), relative to the sidecar.
    pub cloth: Option<String>,
    /// The particle bundle, relative to the sidecar (`particles/bundle.json`).
    pub particles: Option<String>,
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
                            label: c.get("label").and_then(|a| a.as_str()).map(str::to_string),
                            group: c.get("group").and_then(|a| a.as_str()).map(str::to_string),
                            speed: c.get("speed").and_then(|d| d.as_f64()).map(|d| d as f32).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0),
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
            cloth: s("cloth"),
            particles: s("particles"),
        }
    }

    pub fn read(path: &Path) -> Sidecar {
        std::fs::read_to_string(path).map(|t| Sidecar::parse(&t)).unwrap_or_default()
    }

    pub fn playmode(&self, clip: &str) -> Playmode {
        self.clip_info.iter().find(|c| c.name == clip).map(|c| c.playback).unwrap_or_default()
    }

    /// The animation graph's speed for a clip (1 when the sidecar gives none).
    pub fn speed(&self, clip: &str) -> f32 {
        self.clip_info.iter().find(|c| c.name == clip).map(|c| c.speed).filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0)
    }

    /// The clip picker's name for a clip: the sidecar's label, else the clip name made readable.
    pub fn label(&self, clip: &str) -> String {
        self.clip_info.iter().find(|c| c.name == clip).and_then(|c| c.label.clone()).unwrap_or_else(|| humanize_clip(clip))
    }

    /// The clip picker's group: `idle`, `movement`, `combat` or `other`.
    pub fn group(&self, clip: &str) -> String {
        self.clip_info.iter().find(|c| c.name == clip).and_then(|c| c.group.clone()).unwrap_or_else(|| clip_group(clip).to_string())
    }
}

/// A clip name as Grimoire's picker shows it when no recipe names it (heroViewerPlayback.ts):
/// folder and extension dropped, camelCase split, underscores and dashes as spaces, the first
/// letter upper case.
pub fn humanize_clip(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    let stem = [".vnmclip_c", ".vnmclip", ".vanim_c", ".vanim"].iter().find_map(|ext| lower.ends_with(ext).then(|| &base[..base.len() - ext.len()])).unwrap_or(base);
    let mut out = String::new();
    let mut prev_lower = false;
    for c in stem.chars() {
        if c == '_' || c == '-' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = c.is_ascii_lowercase();
        out.push(c);
    }
    let out = out.trim().to_string();
    let mut chars = out.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().chain(chars).collect(),
        None => name.to_string(),
    }
}

/// Grimoire's picker groups by name tokens (groupHeroClips).
pub fn clip_group(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect();
    let any = |set: &[&str]| tokens.iter().any(|t| set.contains(t));
    let movement = tokens.iter().any(|t| {
        let word = t.trim_end_matches(|c: char| c.is_ascii_digit());
        ["walk", "run", "sprint", "jump", "land", "crouch", "dash", "slide", "climb", "move"].contains(&word)
    });
    if any(&["attack", "fire", "shoot", "reload", "melee", "punch", "cast", "ability", "ult", "ultimate"]) {
        "combat"
    } else if any(&["idle", "menu", "loadout", "select", "shop", "pose"]) {
        "idle"
    } else if movement {
        "movement"
    } else {
        "other"
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[derive(Clone, Debug, PartialEq)]
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
    /// Lit in real time (the game draws its shadow), not only kept for fog.
    pub realtime: bool,
    /// Grimoire's light units: `color` is linear colour times the linear `brightness`, and a
    /// point or spot falls off as 10000 / d^2 in inches with a skirt window, no 1/PI.
    pub source_units: bool,
    /// Fraction of `range` over which the light fades out (Source `skirt`).
    pub skirt: f32,
    /// Drawn into fog only, never onto surfaces.
    pub fog_only: bool,
    pub fog_strength: Option<f32>,
    /// The baked shadow mask channel for a stationary light (`direct_light_shadows`).
    pub shadow_channel: Option<u32>,
    pub cookie: Option<PathBuf>,
    /// The light's own axes in glTF space: Source forward, left, up.
    pub basis: Option<[Vec3; 3]>,
    /// Its tiles in the fog shadow atlas (`fog.shadows`), for the fog march.
    pub fog_shadow: Option<FogShadow>,
}

/// A fog light's precomputed shadow: one barn tile or six omni faces in the atlas, `[w, h, x,
/// y]` in atlas uv, and its frustum (`size_params`, `shear`).
#[derive(Clone, Debug, PartialEq)]
pub struct FogShadow {
    pub tiles: Vec<[f32; 4]>,
    pub size: Vec3,
    pub shear: [f32; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fog {
    /// Linear.
    pub color: Vec3,
    pub start: f32,
    pub end: f32,
    pub strength: f32,
    pub indirect_strength: f32,
    /// The fog irradiance volume and the map from world to its [0,1] box.
    pub irradiance: Option<(PathBuf, Mat4)>,
    /// Density volumes. Without any, the controller alone draws no volumetric fog (Grimoire's
    /// march); the renderer then keeps plain distance fog.
    pub volumes: Vec<FogVolume>,
    /// The fog lights' precomputed shadow atlas (VTV, R16), when exported.
    pub shadows: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FogVolume {
    pub sphere: bool,
    pub world_to_local: Mat4,
    pub strength: f32,
    pub falloff: f32,
    pub indirect_strength: Option<f32>,
    /// Linear.
    pub tint: Option<Vec3>,
}

/// The map's tone curve (`m_toneMapParams` of its post volume; white point in stops).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tonemap {
    pub exposure_bias: f32,
    pub shoulder: f32,
    pub linear_strength: f32,
    pub linear_angle: f32,
    pub toe: f32,
    pub toe_num: f32,
    pub toe_denom: f32,
    pub white_point: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BloomMode {
    Add,
    Screen,
    Blur,
}

/// The map's bloom (`m_bloomParams`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    pub mode: BloomMode,
    pub add: f32,
    pub screen: f32,
    pub blur: f32,
    pub threshold: f32,
    pub width: f32,
    pub weights: [f32; 5],
    pub tints: [Vec3; 5],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Post {
    pub tonemap: Option<Tonemap>,
    pub bloom: Option<Bloom>,
    /// The colour correction volume: raw RGBA8, `dim`^3 texels, x fastest.
    pub lut: Option<(PathBuf, u32)>,
}

/// The baked lightmap: DC irradiance, and for v8/5 maps the SH2 direction and the stationary
/// lights' shadow mask.
#[derive(Clone, Debug, PartialEq)]
pub struct Lightmap {
    pub file: PathBuf,
    pub uv_scale: [f32; 2],
    pub direction: Option<PathBuf>,
    pub shadows: Option<PathBuf>,
}

/// A light probe volume (`env_combined_light_probe_volume`): six ambient cube faces per voxel
/// in one atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeVolume {
    pub file: PathBuf,
    pub shadows: Option<PathBuf>,
    pub world_to_local: Mat4,
    pub size: Vec3,
    pub offset: Vec3,
    pub atlas: Vec3,
    pub fade: Vec3,
    pub priority: i32,
    pub rgbm: bool,
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
    pub post: Post,
    pub lightmap: Option<Lightmap>,
    pub probes: Vec<ProbeVolume>,
    /// The map's particle bundle (`particles/bundle.json`).
    pub particles: Option<PathBuf>,
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
            let brightness = l.get("brightness").and_then(num);
            let intensity = brightness.or_else(|| l.get("intensity").and_then(num)).unwrap_or(1.0);
            let c = srgb(l.get("color")).unwrap_or([1.0; 3]);
            let basis = l.get("basis").and_then(|b| b.as_array()).filter(|b| b.len() == 3).and_then(|b| Some([vec3(b.first())?, vec3(b.get(1))?, vec3(b.get(2))?]));
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
                realtime: l.get("realtime").and_then(|r| r.as_bool()).unwrap_or(true),
                source_units: brightness.is_some(),
                skirt: l.get("skirt").and_then(num).unwrap_or(0.1).clamp(0.0, 1.0),
                fog_only: l.get("fogOnly").and_then(|r| r.as_bool()).unwrap_or(!l.get("realtime").and_then(|r| r.as_bool()).unwrap_or(true)),
                fog_strength: l.get("fogStrength").and_then(num),
                shadow_channel: l.get("shadowChannel").and_then(|c| c.as_u64()).map(|c| c as u32),
                cookie: path(base, l.get("cookie")),
                basis,
                fog_shadow: l.get("fogShadow").filter(|f| f.is_object()).and_then(|f| {
                    let tiles: Vec<[f32; 4]> = f.get("tiles")?.as_array()?.iter().filter_map(|t| {
                        let t = t.as_array().filter(|t| t.len() == 4)?;
                        Some([num(&t[0])?, num(&t[1])?, num(&t[2])?, num(&t[3])?])
                    }).collect();
                    let shear = f.get("shear").and_then(|v| v.as_array()).filter(|v| v.len() == 2).and_then(|v| Some([num(&v[0])?, num(&v[1])?])).unwrap_or([0.0; 2]);
                    (tiles.len() == 1 || tiles.len() == 6).then(|| FogShadow { tiles, size: vec3(f.get("size")).unwrap_or([0.0; 3]), shear })
                }),
            });
        }
        if let Some(f) = v.get("fog").filter(|f| f.is_object()) {
            let fade = f.get("fadeIn").and_then(|a| a.as_array());
            let start = fade.and_then(|a| a.first()).and_then(num).unwrap_or(0.0);
            let draw = f.get("drawDistance").and_then(num).unwrap_or(60.0);
            let volumes = f
                .get("volumes")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .take(16)
                .filter_map(|v| {
                    Some(FogVolume {
                        sphere: v.get("shape").and_then(|x| x.as_str()) == Some("sphere"),
                        world_to_local: matrix(v.get("worldToLocal"))?,
                        strength: v.get("strength").and_then(num).unwrap_or(1.0),
                        falloff: v.get("falloff").and_then(num).unwrap_or(1.0),
                        indirect_strength: v.get("indirectStrength").and_then(num),
                        tint: srgb(v.get("tint")),
                    })
                })
                .collect();
            let irradiance = f.get("irradiance").and_then(|i| Some((path(base, i.get("file"))?, matrix(i.get("worldToLocal"))?)));
            s.fog = Some(Fog {
                color: srgb(f.get("color")).unwrap_or([0.5; 3]),
                start,
                end: draw.max(start + 0.1),
                strength: f.get("strength").and_then(num).unwrap_or(1.0).clamp(0.0, 1.0),
                indirect_strength: f.get("indirectStrength").and_then(num).unwrap_or(1.0),
                irradiance,
                volumes,
                shadows: path(base, f.get("shadows")),
            });
        }
        if let Some(p) = v.get("post").filter(|p| p.is_object()) {
            let f = |o: &Value, k: &str, d: f32| o.get(k).and_then(num).unwrap_or(d);
            s.post.tonemap = p.get("tonemap").filter(|t| t.is_object()).map(|t| Tonemap {
                exposure_bias: f(t, "exposureBias", 0.0),
                shoulder: f(t, "shoulderStrength", 0.3538),
                linear_strength: f(t, "linearStrength", 0.3258),
                linear_angle: f(t, "linearAngle", 0.2528),
                toe: f(t, "toeStrength", 0.6966),
                toe_num: f(t, "toeNum", 0.0),
                toe_denom: f(t, "toeDenom", 0.7819),
                white_point: f(t, "whitePoint", 3.9996),
            });
            s.post.bloom = p.get("bloom").filter(|b| b.is_object()).map(|b| {
                let list = |k: &str| b.get(k).and_then(|x| x.as_array()).cloned().unwrap_or_default();
                let weights = list("blurWeights");
                let tints = list("blurTints");
                Bloom {
                    mode: match b.get("blendMode").and_then(|m| m.as_str()) {
                        Some("screen") => BloomMode::Screen,
                        Some("blur") => BloomMode::Blur,
                        _ => BloomMode::Add,
                    },
                    add: f(b, "bloomStrength", 0.0),
                    screen: f(b, "screenBloomStrength", 0.0),
                    blur: f(b, "blurBloomStrength", 0.0),
                    threshold: f(b, "threshold", 1.0),
                    width: f(b, "thresholdWidth", 1.0).max(1e-3),
                    weights: std::array::from_fn(|i| weights.get(i).and_then(num).unwrap_or(0.2)),
                    tints: std::array::from_fn(|i| vec3(tints.get(i)).unwrap_or([1.0; 3])),
                }
            });
            s.post.lut = p.get("colorCorrection").and_then(|c| Some((path(base, c.get("file"))?, c.get("dimension")?.as_u64()? as u32))).filter(|(_, d)| (2..=64).contains(d));
        }
        if let Some(l) = v.get("lightmap").filter(|l| l.is_object())
            && let Some(file) = path(base, l.get("file"))
        {
            let scale = l.get("uvScale").and_then(|a| a.as_array()).map(|a| [a.first().and_then(num).unwrap_or(1.0), a.get(1).and_then(num).unwrap_or(1.0)]).unwrap_or([1.0; 2]);
            s.lightmap = Some(Lightmap { file, uv_scale: scale, direction: path(base, l.get("direction")), shadows: path(base, l.get("shadows")) });
        }
        for p in v.get("probes").and_then(|p| p.as_array()).into_iter().flatten() {
            let (Some(file), Some(m)) = (path(base, p.get("file")), matrix(p.get("worldToLocal"))) else { continue };
            s.probes.push(ProbeVolume {
                file,
                shadows: path(base, p.get("shadows")),
                world_to_local: m,
                size: vec3(p.get("size")).unwrap_or([1.0; 3]),
                offset: vec3(p.get("offset")).unwrap_or([0.0; 3]),
                atlas: vec3(p.get("atlasSize")).unwrap_or([1.0; 3]),
                fade: vec3(p.get("fade")).unwrap_or([0.0; 3]),
                priority: p.get("priority").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                rgbm: p.get("encoding").and_then(|e| e.as_str()) == Some("rgbm"),
            });
        }
        // Grimoire's order: higher priority first, then the smaller volume.
        s.probes.sort_by(|a, b| b.priority.cmp(&a.priority).then((a.size[0] * a.size[1] * a.size[2]).total_cmp(&(b.size[0] * b.size[1] * b.size[2]))));
        s.particles = path(base, v.get("particles"));
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
    fn clip_names_read_as_grimoire_shows_them() {
        assert_eq!(humanize_clip("models/heroes/haze/clips/primary_stand_reload.vnmclip_c"), "Primary stand reload");
        assert_eq!(humanize_clip("heroSelectIdle"), "Hero Select Idle");
        assert_eq!(clip_group("primary_stand_reload"), "combat");
        assert_eq!(clip_group("ui_shop_idle"), "idle");
        assert_eq!(clip_group("run_n2"), "movement");
        assert_eq!(clip_group("emote_wave"), "other");
        let s = Sidecar::parse(r#"{"clipInfo":[{"name":"shop_menu_base","label":"Idle","group":"idle","speed":0.69}]}"#);
        assert_eq!(s.label("shop_menu_base"), "Idle");
        assert_eq!(s.speed("shop_menu_base"), 0.69);
        assert_eq!(s.speed("other"), 1.0);
        assert_eq!(s.label("dash_forward"), "Dash forward");
    }

    #[test]
    fn an_empty_scene_is_still_a_scene() {
        let s = SceneDesc::parse("{}", Path::new(".")).unwrap();
        assert!(s.models.is_empty() && s.lights.is_empty() && s.sky.is_none());
    }
}
