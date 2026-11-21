//! The particle bundle deadlock-core writes (`particles/bundle.json` plus `tex/*.png`, schema 1,
//! everything in Source space): the decoded `.vpcf` definitions, their textures with sprite
//! sheets, the model attachments effects bind to and the effects to play.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::kv;
use super::smath::Xf;
use crate::math::{Mat4, Quat, Vec3};

#[derive(Clone, Debug)]
pub struct SheetFrame {
    pub duration: f32,
    /// The first image's uv rect (u0, v0, u1, v1), top-left origin.
    pub rect: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct Sequence {
    pub id: i32,
    pub clamp: bool,
    pub duration: f32,
    pub frames: Vec<SheetFrame>,
}

/// A vtex sprite sheet.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub sequences: Vec<Sequence>,
}

impl Sheet {
    /// The index of the sequence with `id`.
    pub fn index_of(&self, id: i32) -> Option<usize> {
        self.sequences.iter().position(|s| s.id == id)
    }

    /// The two frames and their blend for `passes` through sequence `seq` (VRF's sheet
    /// sampling, unquantized): a looping sequence wraps from its last frame to its first, a
    /// clamped one holds its last frame. `in_fps` measures `passes` in sheet seconds.
    pub fn frames(&self, seq: usize, passes: f32, in_fps: bool) -> ([f32; 4], [f32; 4], f32) {
        let full = [0.0, 0.0, 1.0, 1.0];
        let Some(s) = self.sequences.get(seq.min(self.sequences.len().saturating_sub(1))) else { return (full, full, 0.0) };
        let n = s.frames.len();
        if n == 0 {
            return (full, full, 0.0);
        }
        if n == 1 {
            return (s.frames[0].rect, s.frames[0].rect, 0.0);
        }
        let total = if s.duration > 0.0 { s.duration } else { 1.0 };
        let length: f32 = s.frames.iter().map(|f| f.duration).sum();
        let mut t = passes;
        if in_fps && length > 0.0 {
            t /= length;
        }
        let u = if s.clamp { t.clamp(0.0, 1.0) } else { t.rem_euclid(1.0) };
        // Frame k starts at the sum of the earlier display times over the total.
        let mut start_a = 0.0;
        let mut elapsed = 0.0;
        let mut a = None;
        for (k, f) in s.frames.iter().enumerate() {
            let start = elapsed / total;
            if start > u {
                break;
            }
            a = Some(k);
            start_a = start;
            elapsed += f.duration;
        }
        let Some(a) = a else { return (s.frames[0].rect, s.frames[1].rect, 0.0) };
        if a + 1 < n {
            let next = (start_a * total + s.frames[a].duration) / total;
            let blend = if next > start_a { ((u - start_a) / (next - start_a)).clamp(0.0, 1.0) } else { 0.0 };
            return (s.frames[a].rect, s.frames[a + 1].rect, blend);
        }
        if s.clamp {
            return (s.frames[n - 2].rect, s.frames[n - 1].rect, 1.0);
        }
        let first = 0.0;
        let span = 1.0 - start_a + first;
        let blend = if span > 0.0 { ((u - start_a) / span).clamp(0.0, 1.0) } else { 0.0 };
        (s.frames[n - 1].rect, s.frames[0].rect, blend)
    }

    fn parse(v: &Value) -> Sheet {
        let sequences = kv::array(v, "sequences")
            .iter()
            .map(|s| Sequence {
                id: kv::i(s, "id", 0),
                clamp: kv::b(s, "clamp", false),
                duration: kv::f(s, "duration", 0.0),
                frames: kv::array(s, "frames")
                    .iter()
                    .map(|f| {
                        let rect = kv::array(f, "images").first().and_then(|r| r.as_array()).map_or([0.0, 0.0, 1.0, 1.0], |r| {
                            let c = |k: usize| r.get(k).and_then(kv::number).unwrap_or(0.0) as f32;
                            [c(0), c(1), c(2), c(3)]
                        });
                        SheetFrame { duration: kv::f(f, "duration", 0.0), rect }
                    })
                    .collect(),
            })
            .collect();
        Sheet { sequences }
    }
}

/// A decoded particle texture: RGBA8, straight alpha, top row first.
#[derive(Clone, Debug)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sheet: Option<Sheet>,
}

#[derive(Clone, Debug)]
pub struct Influence {
    pub bone: String,
    pub offset: Vec3,
    pub rotation: Quat,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: String,
    pub ignore_rotation: bool,
    pub influences: Vec<Influence>,
}

/// How an effect binds to the model (`PATTACH_*` lowercased without the prefix).
#[derive(Clone, Debug)]
pub struct EffectAttach {
    pub kind: String,
    pub point: String,
    pub offset: Vec3,
}

#[derive(Clone, Debug)]
pub struct EffectCp {
    pub index: usize,
    /// A Source-space frame (column major), for map `cpointN` targets.
    pub transform: Option<Mat4>,
    pub value: Option<Vec3>,
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub id: String,
    pub system: String,
    pub source: String,
    pub attachment: Option<EffectAttach>,
    pub attachment_cp1: Option<EffectAttach>,
    pub configuration: Option<String>,
    pub transform: Option<Mat4>,
    pub control_points: Vec<EffectCp>,
    pub start_active: bool,
    pub seed: Option<u32>,
    /// Animation-event effects only: when the playing clip starts and stops this one.
    pub clip: Option<EffectClip>,
}

/// An `AE_CL_STOP_PARTICLE_EFFECT` of an animation-event effect's clip.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipStop {
    /// Normalized clip time, 0 to 1.
    pub time: f32,
    /// The particles go at once.
    pub immediately: bool,
    pub end_cap: bool,
}

/// The clip (as the glb names it) and normalized event times of an animation-event effect.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectClip {
    pub name: String,
    pub start: f32,
    pub stops: Vec<ClipStop>,
    /// A clip change stops the effect.
    pub stop_on_clip_change: bool,
}

#[derive(Clone, Debug)]
pub struct SystemDef {
    pub data: Value,
    pub format: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Bundle {
    pub owner: String,
    pub systems: HashMap<String, SystemDef>,
    pub textures: HashMap<String, Texture>,
    pub attachments: Vec<Attachment>,
    pub effects: Vec<Effect>,
    pub issues: Vec<String>,
    /// The hero model's bones: name and Source model-space bind pose.
    pub bones: Vec<(String, Mat4)>,
    /// Decoded snapshots by `particles/x/y.vsnap` path (lowercase).
    pub snapshots: HashMap<String, std::sync::Arc<super::snapshot::Snapshot>>,
}

fn mat_of(v: &Value) -> Option<Mat4> {
    let a = v.as_array()?;
    if a.len() < 16 {
        return None;
    }
    let mut m = [0.0f32; 16];
    for (k, x) in a.iter().take(16).enumerate() {
        m[k] = kv::number(x).unwrap_or(0.0) as f32;
    }
    Some(Mat4::from_cols_slice(&m))
}

fn attach_of(v: Option<&Value>) -> Option<EffectAttach> {
    let v = v?;
    if !v.is_object() {
        return None;
    }
    let kind = kv::s(v, "type").unwrap_or("point_follow").to_ascii_lowercase();
    Some(EffectAttach { kind: kind.trim_start_matches("pattach_").to_string(), point: kv::s(v, "point").unwrap_or("").to_string(), offset: kv::v3(v, "offset", [0.0; 3]) })
}

/// The most texels a particle texture may have when read (64 MB as RGBA8).
pub const MAX_TEXTURE_TEXELS: u64 = 4096 * 4096;

impl Bundle {
    /// Reads `bundle.json` (or the directory holding it) and decodes its textures.
    pub fn read(path: &Path) -> Result<Bundle, String> {
        // (Textures past `MAX_TEXTURE_TEXELS` are refused before they are decoded.)
        let file: PathBuf = if path.is_dir() { path.join("bundle.json") } else { path.to_path_buf() };
        let text = std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", file.display()))?;
        let dir = file.parent().unwrap_or(Path::new("."));
        Bundle::from_json(&json, |rel| {
            let p = dir.join(rel);
            // A bundle's picture comes from a mod: read its size first and refuse a huge one
            // before decoding it.
            let (w, h) = image::image_dimensions(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            if u64::from(w) * u64::from(h) > MAX_TEXTURE_TEXELS {
                return Err(format!("{}: {w}x{h} is past the particle texture cap", p.display()));
            }
            let bytes = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            let img = image::load_from_memory(&bytes).map_err(|e| format!("decode {}: {e}", p.display()))?.to_rgba8();
            Ok((img.width(), img.height(), img.into_raw()))
        })
    }

    /// Builds a bundle from its JSON, loading each texture file through `load`. A texture
    /// that fails to load becomes an issue, not an error.
    pub fn from_json(json: &Value, mut load: impl FnMut(&str) -> Result<(u32, u32, Vec<u8>), String>) -> Result<Bundle, String> {
        let schema = kv::i(json, "schema", 1);
        if schema != 1 {
            return Err(format!("particle bundle schema {schema} is not supported"));
        }
        let mut b = Bundle { owner: kv::s(json, "owner").unwrap_or("").to_string(), ..Bundle::default() };
        if let Some(systems) = kv::get(json, "systems").and_then(Value::as_object) {
            for (name, s) in systems {
                let Some(data) = kv::get(s, "data") else { continue };
                b.systems.insert(name.to_ascii_lowercase(), SystemDef { data: data.clone(), format: kv::s(s, "format").map(str::to_string) });
            }
        }
        if let Some(textures) = kv::get(json, "textures").and_then(Value::as_object) {
            for (name, t) in textures {
                let Some(file) = kv::s(t, "file") else { continue };
                match load(file) {
                    Ok((width, height, rgba)) => {
                        let sheet = kv::get(t, "sheet").filter(|s| s.is_object()).map(Sheet::parse);
                        b.textures.insert(name.to_ascii_lowercase(), Texture { width, height, rgba, sheet });
                    }
                    Err(e) => b.issues.push(format!("texture:{name}: {e}")),
                }
            }
        }
        for a in kv::array(json, "attachments") {
            b.attachments.push(Attachment {
                name: kv::s(a, "name").unwrap_or("").to_string(),
                ignore_rotation: kv::b(a, "ignoreRotation", false),
                influences: kv::array(a, "influences")
                    .iter()
                    .map(|i| {
                        let r = kv::get(i, "rotation").and_then(Value::as_array);
                        let q = r.map_or([0.0, 0.0, 0.0, 1.0], |r| {
                            let c = |k: usize, d: f64| r.get(k).and_then(kv::number).unwrap_or(d) as f32;
                            [c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 1.0)]
                        });
                        Influence { bone: kv::s(i, "bone").unwrap_or("").to_string(), offset: kv::v3(i, "offset", [0.0; 3]), rotation: q, weight: kv::f(i, "weight", 1.0) }
                    })
                    .collect(),
            });
        }
        for e in kv::array(json, "effects") {
            let Some(system) = kv::s(e, "system") else { continue };
            b.effects.push(Effect {
                id: kv::s(e, "id").unwrap_or("").to_string(),
                system: system.to_ascii_lowercase(),
                source: kv::s(e, "source").unwrap_or("").to_string(),
                attachment: attach_of(kv::get(e, "attachment")),
                attachment_cp1: attach_of(kv::get(e, "attachmentCp1")),
                configuration: kv::s(e, "configuration").map(str::to_string),
                transform: kv::get(e, "transform").and_then(mat_of),
                control_points: kv::array(e, "controlPoints")
                    .iter()
                    .map(|c| EffectCp { index: kv::i(c, "index", 0).clamp(0, 63) as usize, transform: kv::get(c, "transform").and_then(mat_of), value: kv::get(c, "value").and_then(kv::vec3_of) })
                    .collect(),
                start_active: kv::b(e, "startActive", true),
                seed: kv::get(e, "seed").and_then(kv::number).map(|n| n as i64 as u32),
                clip: kv::get(e, "clip").filter(|c| c.is_object()).and_then(|c| {
                    Some(EffectClip {
                        name: kv::s(c, "name")?.to_string(),
                        start: kv::f(c, "start", 0.0),
                        stops: kv::array(c, "stops")
                            .iter()
                            .map(|s| ClipStop { time: kv::f(s, "time", 0.0), immediately: kv::b(s, "immediately", false), end_cap: kv::b(s, "endCap", false) })
                            .collect(),
                        stop_on_clip_change: kv::b(c, "stopOnClipChange", false),
                    })
                }),
            });
        }
        b.issues.extend(kv::array(json, "issues").iter().filter_map(|i| i.as_str().map(str::to_string)));
        for bone in kv::array(json, "bones") {
            if let (Some(name), Some(bind)) = (kv::s(bone, "name"), kv::get(bone, "bind").and_then(mat_of)) {
                b.bones.push((name.to_string(), bind));
            }
        }
        let names: Vec<String> = b.bones.iter().map(|(n, _)| n.clone()).collect();
        if let Some(snapshots) = kv::get(json, "snapshots").and_then(Value::as_object) {
            for (path, s) in snapshots {
                b.snapshots.insert(path.to_ascii_lowercase(), std::sync::Arc::new(super::snapshot::Snapshot::parse(s, &names)));
            }
        }
        Ok(b)
    }

    /// The Source-space frame of attachment `name` (VRF's attachment transform: each
    /// influence's offset and rotation applied in its bone's frame, weighted), from the posed
    /// bones `bone` returns as Source model-space matrices. A bone name works as an
    /// attachment too; `None` when neither resolves.
    pub fn attachment_frame(&self, name: &str, bone: &dyn Fn(&str) -> Option<Mat4>) -> Option<Mat4> {
        let found = self.attachments.iter().find(|a| a.name == name).or_else(|| self.attachments.iter().find(|a| a.name.eq_ignore_ascii_case(name)));
        let Some(a) = found else { return bone(name).map(|m| Xf::from_mat(&m).rigid().to_mat()) };
        let mut m = Mat4::IDENTITY;
        let mut any = false;
        for inf in &a.influences {
            let Some(b) = bone(&inf.bone) else { continue };
            any = true;
            let local = Mat4::from_trs([0.0; 3], inf.rotation, [1.0; 3]);
            // Row-vector `R(q) * T(offset) * bone` in VRF; column form `bone * T(offset) * R(q)`.
            let x = b.mul(&Mat4::translation(inf.offset)).mul(&local);
            let w = if inf.weight >= 1.0 {
                x
            } else {
                let mut out = Mat4::IDENTITY;
                for c in 0..4 {
                    for r in 0..4 {
                        let id = Mat4::IDENTITY.0[c][r];
                        out.0[c][r] = id + (x.0[c][r] - id) * inf.weight;
                    }
                }
                out
            };
            m = w.mul(&m);
        }
        if !any {
            return None;
        }
        if a.ignore_rotation {
            let s = m.0[1][1];
            let t = [m.0[3][0], m.0[3][1], m.0[3][2]];
            m = Mat4([[s, 0.0, 0.0, 0.0], [0.0, s, 0.0, 0.0], [0.0, 0.0, s, 0.0], [t[0], t[1], t[2], 1.0]]);
        }
        Some(Xf::from_mat(&m).rigid().to_mat())
    }
}
