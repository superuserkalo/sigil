//! Whether a showcase pose is staged for the game's own menu scene, and how far each clip moves
//! the hero's floor contact.
//!
//! Ported from Grimoire (MIT), `electron/main/services/heroStagedPose.ts`, and its one caller,
//! `writeHeroAnimationPolicy` in `heroAnimationExport.ts`. Graves' shop state lays the spectral
//! hand on the shop floor and Rem's sits him on a seat the preview does not have: alone, these
//! read as a dropped hand or a floating hero, so the preview starts on the standing idle instead.
//! A clip that lowers the hero through the floor records how far, so the viewer can lower the
//! floor with it.
//!
//! Works on the exported glb alone (rest transforms, skin weights and the clips' node channels),
//! sampling bone origins only, no vertex skinning. All lengths are glTF world units (meters for
//! a Sigil hero export), and every threshold is a fraction of the hero's height: the rest-pose
//! height span of the bones that carry visible skin weight.

use std::borrow::Cow;

use super::clips::ClipChoice;

/// Growth of a bone's distance from its parent that detaches it.
const DETACH_FRACTION: f64 = 0.25;
/// Floor band above the model origin.
const FLOOR_FRACTION: f64 = 0.1;
/// Weights at or below this do not visibly attach geometry to a bone.
const MIN_WEIGHT: f32 = 0.05;
/// Bones resting within this band of the origin in the rest pose touch the floor.
const CONTACT_BELOW: f64 = 0.02;
const CONTACT_ABOVE: f64 = 0.05;
/// A clip that keeps every floor contact this far up holds the hero off the floor.
const LIFT_FRACTION: f64 = 0.08;
/// Bones moved this far below their rest height are hidden, not standing.
const HIDE_FRACTION: f64 = 0.25;
/// A clip that never stands the body this tall leaves it lying flat.
const FLAT_FRACTION: f64 = 0.5;
/// Floor offsets within this fraction of hero height are noise.
const FLOOR_TOLERANCE: f64 = 0.005;
/// Poses sampled per clip: `SAMPLES + 1`, from the start to the end.
const SAMPLES: usize = 8;

type Vec3 = [f64; 3];
/// Column-major 4x4, as glTF stores it.
type Mat4 = [f64; 16];

/// A node's local transform in the rest pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Local {
    Trs { t: Vec3, r: [f64; 4], s: Vec3 },
    Matrix(Mat4),
}

impl Local {
    fn matrix(&self) -> Mat4 {
        match *self {
            Local::Matrix(m) => m,
            Local::Trs { t, r, s } => compose(t, r, s),
        }
    }
}

/// The node tree of a glb as the test needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rig {
    /// Rest transform and children of every node.
    pub nodes: Vec<(Local, Vec<usize>)>,
    /// The scene's root nodes.
    pub roots: Vec<usize>,
    /// Every skin joint.
    pub joints: Vec<usize>,
    /// Joints some vertex weighs over 5%.
    pub weighted: Vec<usize>,
}

/// Which part of a node's transform a track animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interpolation {
    Linear,
    Step,
    CubicSpline,
}

/// One animation channel: key times and the flat output values (three per key for a cubic
/// spline: in tangent, value, out tangent).
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub node: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f64>,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Clip {
    pub name: String,
    pub tracks: Vec<Track>,
}

/// What the export does with the test's answers.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Policy {
    /// The clip to start on instead of the showcase: the standing idle, when the showcase is
    /// staged.
    pub start: Option<String>,
    /// The standing idle to drop, when the showcase is not staged.
    pub drop: Option<String>,
    /// Per clip, how far it lowers the floor contact (negative; clips that do not are absent).
    pub floors: Vec<(String, f32)>,
}

/// Grimoire's `writeHeroAnimationPolicy` over the exported glb and the clips it carries. When a
/// showcase and a standing fallback (the first other clip whose action is an idle or relaxed
/// idle, Grimoire's `showcaseFallback`) are both there, a staged showcase starts on the fallback
/// and an unstaged one drops it. Unreadable data counts as not staged, so the graph default is
/// kept.
pub(crate) fn policy(glb: &[u8], choices: &[ClipChoice]) -> Policy {
    let read = read_glb(glb);
    let mut out = Policy::default();
    let showcase = choices.iter().find(|choice| choice.showcase.is_some());
    let fallback = choices.iter().find(|choice| {
        choice.showcase.is_none() && choice.action.is_some_and(|action| action.is_idle())
    });
    let clip = |name: &str| {
        read.as_ref()
            .and_then(|(_, clips)| clips.iter().find(|clip| clip.name == name))
    };
    if let (Some(showcase), Some(fallback)) = (showcase, fallback) {
        let staged = match (&read, clip(&showcase.name)) {
            (Some((rig, _)), Some(shown)) => stages_pose(rig, shown, clip(&fallback.name)),
            _ => false,
        };
        if staged {
            out.start = Some(fallback.name.clone());
        } else {
            out.drop = Some(fallback.name.clone());
        }
    }
    if let Some((rig, clips)) = &read {
        for clip in clips {
            if out.drop.as_deref() == Some(clip.name.as_str()) {
                continue;
            }
            if let Some(floor) = clip_floor_offset(rig, clip)
                && floor < 0.0
            {
                out.floors.push((clip.name.clone(), floor as f32));
            }
        }
    }
    out
}

/// True when `showcase` stages the hero for the menu's own scene instead of standing it on the
/// floor: it pulls a weighted bone far from its parent and lays it on the floor in view, it
/// never stands the body at half its rest height, or it keeps every floor contact well above
/// where the `standing` idle has them (a hero that hovers in every pose is not staged by
/// hovering). Unreadable data returns false.
pub fn stages_pose(rig: &Rig, showcase: &Clip, standing: Option<&Clip>) -> bool {
    let Some(sampler) = Sampler::new(rig, showcase) else {
        return false;
    };
    if sampler.part_on_floor() || sampler.lies_flat() {
        return true;
    }
    let hover = standing
        .and_then(|clip| Sampler::new(rig, clip))
        .map_or(0.0, |reference| reference.floor_offset().unwrap_or(0.0).max(0.0));
    sampler
        .floor_offset()
        .is_some_and(|offset| offset - hover > LIFT_FRACTION * sampler.height)
}

/// How far `clip` moves the hero's floor contact from the rest pose's (negative lowers it),
/// snapped to 0 within half a percent of the hero's height. None when the clip or skeleton
/// cannot be read, or nothing rests on the floor.
pub fn clip_floor_offset(rig: &Rig, clip: &Clip) -> Option<f64> {
    let sampler = Sampler::new(rig, clip)?;
    let offset = sampler.floor_offset()?;
    Some(if offset.abs() < FLOOR_TOLERANCE * sampler.height {
        0.0
    } else {
        offset
    })
}

struct Sampler<'a> {
    rig: &'a Rig,
    parent: Vec<Option<usize>>,
    joint: Vec<bool>,
    rest_world: Vec<Option<Vec3>>,
    height: f64,
    tracks: Vec<&'a Track>,
    duration: f64,
}

impl<'a> Sampler<'a> {
    fn new(rig: &'a Rig, clip: &'a Clip) -> Option<Sampler<'a>> {
        let count = rig.nodes.len();
        if count == 0 {
            return None;
        }
        let mut parent = vec![None; count];
        for (index, (_, children)) in rig.nodes.iter().enumerate() {
            for &child in children {
                if let Some(slot) = parent.get_mut(child) {
                    *slot = Some(index);
                }
            }
        }
        let mut joint = vec![false; count];
        for &index in &rig.joints {
            if let Some(slot) = joint.get_mut(index) {
                *slot = true;
            }
        }
        let rest: Vec<Local> = rig.nodes.iter().map(|(local, _)| *local).collect();
        let rest_world = world_positions(rig, &rest);
        let heights = rig
            .weighted
            .iter()
            .filter_map(|&index| rest_world.get(index).copied().flatten().map(|at| at[1]));
        let (low, high) = heights.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), y| {
            (low.min(y), high.max(y))
        });
        let height = high - low;
        if height.is_nan() || height <= 0.0 {
            return None;
        }
        let tracks: Vec<&Track> = clip
            .tracks
            .iter()
            .filter(|track| track.node < count && !track.times.is_empty())
            .filter(|track| {
                let width = track.property.size()
                    * if track.interpolation == Interpolation::CubicSpline { 3 } else { 1 };
                track.values.len() >= track.times.len() * width
            })
            .collect();
        if tracks.is_empty() {
            return None;
        }
        let duration = tracks
            .iter()
            .filter_map(|track| track.times.last().copied())
            .fold(f64::NEG_INFINITY, f64::max);
        Some(Sampler {
            rig,
            parent,
            joint,
            rest_world,
            height,
            tracks,
            duration,
        })
    }

    /// World position of every node `k / SAMPLES` of the way through the clip.
    fn sample(&self, k: usize) -> Vec<Option<Vec3>> {
        let mut pose: Vec<Local> = self.rig.nodes.iter().map(|(local, _)| *local).collect();
        let time = self.duration * k as f64 / SAMPLES as f64;
        for track in &self.tracks {
            let value = sample_track(track, time);
            let node = &mut pose[track.node];
            // An animated node drops its matrix, as three.js does.
            let (mut t, mut r, mut s) = match *node {
                Local::Trs { t, r, s } => (t, r, s),
                Local::Matrix(_) => ([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
            };
            match track.property {
                Property::Translation => t = [value[0], value[1], value[2]],
                Property::Rotation => r = [value[0], value[1], value[2], value[3]],
                Property::Scale => s = [value[0], value[1], value[2]],
            }
            *node = Local::Trs { t, r, s };
        }
        world_positions(self.rig, &pose)
    }

    /// A weighted bone pulled far from its parent and left on the floor, within view.
    fn part_on_floor(&self) -> bool {
        let bones: Vec<(usize, usize, f64)> = self
            .rig
            .weighted
            .iter()
            .filter_map(|&index| {
                let parent = self.parent[index]?;
                self.joint[parent].then(|| {
                    let rest = match (self.rest_world[index], self.rest_world[parent]) {
                        (Some(a), Some(b)) => distance(a, b),
                        _ => 0.0,
                    };
                    (index, parent, rest)
                })
            })
            .collect();
        (0..=SAMPLES).any(|k| {
            let world = self.sample(k);
            bones.iter().any(|&(index, parent, rest)| {
                let (Some(at), Some(from)) = (world[index], world[parent]) else {
                    return false;
                };
                distance(at, from) - rest > DETACH_FRACTION * self.height
                    && at[1] < FLOOR_FRACTION * self.height
                    && at[1] > -self.height
            })
        })
    }

    /// The body never stands at even half its rest height over the whole clip. Viscous borrows
    /// Kelvin's skeleton, which has no pelvis: the clip's root turn goes uncorrected and he lies
    /// on his back.
    fn lies_flat(&self) -> bool {
        (0..=SAMPLES).all(|k| {
            let pose = self.sample(k);
            let mut heights = self
                .rig
                .weighted
                .iter()
                .filter_map(|&index| pose[index].map(|at| at[1]))
                .peekable();
            if heights.peek().is_none() {
                return true;
            }
            let (low, high) = heights.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), y| {
                (low.min(y), high.max(y))
            });
            high - low < FLAT_FRACTION * self.height
        })
    }

    /// The lowest the bones resting on the floor in the rest pose (soles, toes, a grounded prop)
    /// come relative to their rest heights over the clip. Bones sent far below are an authored
    /// hide and do not count. None when nothing rests on the floor.
    fn floor_offset(&self) -> Option<f64> {
        let contacts: Vec<usize> = self
            .rig
            .weighted
            .iter()
            .copied()
            .filter(|&index| {
                self.rest_world[index].is_some_and(|at| {
                    at[1] >= -CONTACT_BELOW * self.height && at[1] <= CONTACT_ABOVE * self.height
                })
            })
            .collect();
        if contacts.is_empty() {
            return None;
        }
        let mut lowest = f64::INFINITY;
        for k in 0..=SAMPLES {
            let world = self.sample(k);
            for &index in &contacts {
                let (Some(at), Some(from)) = (world[index], self.rest_world[index]) else {
                    continue;
                };
                let lift = at[1] - from[1];
                if lift > -HIDE_FRACTION * self.height {
                    lowest = lowest.min(lift);
                }
            }
        }
        lowest.is_finite().then_some(lowest)
    }
}

impl Property {
    fn size(self) -> usize {
        match self {
            Property::Rotation => 4,
            Property::Translation | Property::Scale => 3,
        }
    }
}

fn sample_track(track: &Track, time: f64) -> [f64; 4] {
    let size = track.property.size();
    let (width, offset) = match track.interpolation {
        Interpolation::CubicSpline => (size * 3, size),
        _ => (size, 0),
    };
    let key = |i: usize| {
        let mut out = [0.0; 4];
        out[..size].copy_from_slice(&track.values[i * width + offset..i * width + offset + size]);
        out
    };
    let times = &track.times;
    let last = times.len() - 1;
    if time <= times[0] {
        return key(0);
    }
    if time >= times[last] {
        return key(last);
    }
    let mut i = 0;
    while i < last && times[i + 1] < time {
        i += 1;
    }
    if track.interpolation == Interpolation::Step {
        return key(i);
    }
    let (a, b) = (key(i), key(i + 1));
    let f = (time - times[i]) / (times[i + 1] - times[i]).max(1e-9);
    let mut out = [0.0; 4];
    if size == 4 {
        // A normalized lerp along the short arc is enough to locate a bone.
        let dot: f64 = (0..4).map(|c| a[c] * b[c]).sum();
        let sign = if dot < 0.0 { -1.0 } else { 1.0 };
        for c in 0..4 {
            out[c] = a[c] + (b[c] * sign - a[c]) * f;
        }
        let length = out.iter().map(|v| v * v).sum::<f64>().sqrt();
        let length = if length > 0.0 { length } else { 1.0 };
        for value in &mut out {
            *value /= length;
        }
    } else {
        for c in 0..size {
            out[c] = a[c] + (b[c] - a[c]) * f;
        }
    }
    out
}

fn compose(t: Vec3, r: [f64; 4], s: Vec3) -> Mat4 {
    let [x, y, z, w] = r;
    let mut m = [0.0; 16];
    m[0] = (1.0 - 2.0 * (y * y + z * z)) * s[0];
    m[1] = 2.0 * (x * y + z * w) * s[0];
    m[2] = 2.0 * (x * z - y * w) * s[0];
    m[4] = 2.0 * (x * y - z * w) * s[1];
    m[5] = (1.0 - 2.0 * (x * x + z * z)) * s[1];
    m[6] = 2.0 * (y * z + x * w) * s[1];
    m[8] = 2.0 * (x * z + y * w) * s[2];
    m[9] = 2.0 * (y * z - x * w) * s[2];
    m[10] = (1.0 - 2.0 * (x * x + y * y)) * s[2];
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m[15] = 1.0;
    m
}

fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

/// World-space translation of every node reachable from the scene roots.
fn world_positions(rig: &Rig, pose: &[Local]) -> Vec<Option<Vec3>> {
    let mut out = vec![None; rig.nodes.len()];
    let mut stack: Vec<(usize, Option<Mat4>)> =
        rig.roots.iter().rev().map(|&root| (root, None)).collect();
    // A malformed tree (a cycle) cannot loop forever: every node is placed once.
    let mut seen = vec![false; rig.nodes.len()];
    while let Some((index, parent)) = stack.pop() {
        if index >= rig.nodes.len() || seen[index] {
            continue;
        }
        seen[index] = true;
        let local = pose[index].matrix();
        let world = match parent {
            Some(parent) => multiply(&parent, &local),
            None => local,
        };
        out[index] = Some([world[12], world[13], world[14]]);
        for &child in rig.nodes[index].1.iter().rev() {
            stack.push((child, Some(world)));
        }
    }
    out
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The node tree, skins, weights and clips of a binary glTF. None when it is not one.
pub fn read_glb(bytes: &[u8]) -> Option<(Rig, Vec<Clip>)> {
    let glb = gltf::Glb::from_slice(bytes).ok()?;
    let root = gltf::json::Root::from_slice(&glb.json).ok()?;
    let document = gltf::Document::from_json_without_validation(root);
    let bin: Option<Cow<'_, [u8]>> = glb.bin;
    let buffers = |buffer: gltf::Buffer<'_>| match buffer.source() {
        gltf::buffer::Source::Bin => bin.as_deref(),
        gltf::buffer::Source::Uri(_) => None,
    };

    let nodes: Vec<(Local, Vec<usize>)> = document
        .nodes()
        .map(|node| {
            let local = match node.transform() {
                gltf::scene::Transform::Matrix { matrix } => {
                    let mut m = [0.0; 16];
                    for (col, column) in matrix.iter().enumerate() {
                        for (row, value) in column.iter().enumerate() {
                            m[col * 4 + row] = f64::from(*value);
                        }
                    }
                    Local::Matrix(m)
                }
                gltf::scene::Transform::Decomposed {
                    translation,
                    rotation,
                    scale,
                } => Local::Trs {
                    t: translation.map(f64::from),
                    r: rotation.map(f64::from),
                    s: scale.map(f64::from),
                },
            };
            (local, node.children().map(|child| child.index()).collect())
        })
        .collect();
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next());
    let roots = scene.map_or_else(Vec::new, |scene| scene.nodes().map(|n| n.index()).collect());
    let mut joints: Vec<usize> = document
        .skins()
        .flat_map(|skin| skin.joints().map(|joint| joint.index()).collect::<Vec<_>>())
        .collect();
    joints.sort_unstable();
    joints.dedup();

    let mut weighted = vec![false; nodes.len()];
    for node in document.nodes() {
        let (Some(mesh), Some(skin)) = (node.mesh(), node.skin()) else {
            continue;
        };
        let skin_joints: Vec<usize> = skin.joints().map(|joint| joint.index()).collect();
        for primitive in mesh.primitives() {
            let reader = primitive.reader(buffers);
            let (Some(ids), Some(weights)) = (reader.read_joints(0), reader.read_weights(0)) else {
                continue;
            };
            for (ids, weights) in ids.into_u16().zip(weights.into_f32()) {
                for (id, weight) in ids.iter().zip(weights) {
                    if weight > MIN_WEIGHT
                        && let Some(&joint) = skin_joints.get(usize::from(*id))
                    {
                        weighted[joint] = true;
                    }
                }
            }
        }
    }
    let weighted = weighted
        .iter()
        .enumerate()
        .filter_map(|(index, on)| on.then_some(index))
        .collect();

    let clips = document
        .animations()
        .map(|animation| Clip {
            name: animation.name().unwrap_or_default().to_string(),
            tracks: animation
                .channels()
                .filter_map(|channel| {
                    use gltf::animation::util::ReadOutputs;
                    let reader = channel.reader(buffers);
                    let times: Vec<f64> = reader.read_inputs()?.map(f64::from).collect();
                    let (property, values): (Property, Vec<f64>) = match reader.read_outputs()? {
                        ReadOutputs::Translations(values) => (
                            Property::Translation,
                            values.flatten().map(f64::from).collect(),
                        ),
                        ReadOutputs::Rotations(values) => (
                            Property::Rotation,
                            values.into_f32().flatten().map(f64::from).collect(),
                        ),
                        ReadOutputs::Scales(values) => {
                            (Property::Scale, values.flatten().map(f64::from).collect())
                        }
                        ReadOutputs::MorphTargetWeights(_) => return None,
                    };
                    let interpolation = match channel.sampler().interpolation() {
                        gltf::animation::Interpolation::Linear => Interpolation::Linear,
                        gltf::animation::Interpolation::Step => Interpolation::Step,
                        gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
                    };
                    Some(Track {
                        node: channel.target().node().index(),
                        property,
                        interpolation,
                        times,
                        values,
                    })
                })
                .collect(),
        })
        .collect();

    Some((
        Rig {
            nodes,
            roots,
            joints,
            weighted,
        },
        clips,
    ))
}

/// The glb without the animation named `name`, as Grimoire splices it out of the JSON chunk:
/// its samplers' accessors stay in the binary chunk, unreferenced. None when the bytes are not a
/// glb or carry no such animation.
pub(crate) fn drop_animation(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    const MAGIC: u32 = 0x4654_6c67;
    const JSON: u32 = 0x4e4f_534a;
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if word(0)? != MAGIC || word(16)? != JSON {
        return None;
    }
    let end = 20 + word(12)? as usize;
    let mut json: serde_json::Value = serde_json::from_slice(bytes.get(20..end)?).ok()?;
    let animations = json.get_mut("animations")?.as_array_mut()?;
    let at = animations
        .iter()
        .position(|animation| animation.get("name").and_then(|n| n.as_str()) == Some(name))?;
    animations.remove(at);
    if animations.is_empty()
        && let Some(object) = json.as_object_mut()
    {
        object.remove("animations");
    }
    let mut chunk = serde_json::to_vec(&json).ok()?;
    while !chunk.len().is_multiple_of(4) {
        chunk.push(b' ');
    }
    let rest = &bytes[end..];
    let total = 20 + chunk.len() + rest.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&bytes[..8]);
    out.extend_from_slice(&u32::try_from(total).ok()?.to_le_bytes());
    out.extend_from_slice(&u32::try_from(chunk.len()).ok()?.to_le_bytes());
    out.extend_from_slice(&JSON.to_le_bytes());
    out.extend_from_slice(&chunk);
    out.extend_from_slice(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A biped in meters, Y up, 1.7 tall: a non-joint wrapper at the origin, the pelvis at 1.0,
    // the head at 1.7, knees at 0.5, feet on the floor, a left arm with the hand at 1.1.
    const WRAPPER: usize = 0;
    const PELVIS: usize = 1;
    const HEAD: usize = 2;
    const KNEE_L: usize = 3;
    const FOOT_L: usize = 4;
    const KNEE_R: usize = 5;
    const FOOT_R: usize = 6;
    const SHOULDER: usize = 7;
    const HAND: usize = 8;

    fn at(t: Vec3) -> Local {
        Local::Trs {
            t,
            r: [0.0, 0.0, 0.0, 1.0],
            s: [1.0; 3],
        }
    }

    fn biped() -> Rig {
        Rig {
            nodes: vec![
                (at([0.0; 3]), vec![PELVIS]),
                (at([0.0, 1.0, 0.0]), vec![HEAD, KNEE_L, KNEE_R, SHOULDER]),
                (at([0.0, 0.7, 0.0]), vec![]),
                (at([0.1, -0.5, 0.0]), vec![FOOT_L]),
                (at([0.0, -0.5, 0.0]), vec![]),
                (at([-0.1, -0.5, 0.0]), vec![FOOT_R]),
                (at([0.0, -0.5, 0.0]), vec![]),
                (at([0.2, 0.4, 0.0]), vec![HAND]),
                (at([0.3, -0.3, 0.0]), vec![]),
            ],
            roots: vec![WRAPPER],
            joints: (PELVIS..=HAND).collect(),
            weighted: (PELVIS..=HAND).collect(),
        }
    }

    fn track(node: usize, property: Property, keys: &[&[f64]]) -> Track {
        Track {
            node,
            property,
            interpolation: Interpolation::Linear,
            times: (0..keys.len()).map(|i| i as f64).collect(),
            values: keys.iter().flat_map(|key| key.iter().copied()).collect(),
        }
    }

    /// A translation that jumps at the last key, so no sample falls between the keys.
    fn stepped(node: usize, keys: &[&[f64]]) -> Track {
        Track {
            interpolation: Interpolation::Step,
            ..track(node, Property::Translation, keys)
        }
    }

    fn clip(name: &str, tracks: Vec<Track>) -> Clip {
        Clip {
            name: name.into(),
            tracks,
        }
    }

    /// The pelvis bobbing from `y` to `y + 0.01` and back.
    fn pelvis_at(name: &str, y: f64) -> Clip {
        clip(
            name,
            vec![track(
                PELVIS,
                Property::Translation,
                &[&[0.0, y, 0.0], &[0.0, y + 0.01, 0.0], &[0.0, y, 0.0]],
            )],
        )
    }

    fn standing() -> Clip {
        pelvis_at("primary_stand_idle", 1.0)
    }

    #[test]
    fn a_standing_biped_is_not_staged_and_keeps_its_floor() {
        let rig = biped();
        assert!(!stages_pose(&rig, &standing(), None));
        assert_eq!(clip_floor_offset(&rig, &standing()), Some(0.0));
    }

    #[test]
    fn a_hand_laid_on_the_floor_is_staged() {
        let rig = biped();
        // The hand leaves the arm and comes to rest 5 cm above the floor (Graves' shop).
        let hand = clip(
            "ui_shop",
            vec![track(
                HAND,
                Property::Translation,
                &[&[0.3, -0.3, 0.0], &[0.3, -0.8, 0.0], &[0.3, -1.35, 0.0]],
            )],
        );
        assert!(stages_pose(&rig, &hand, Some(&standing())));
        // A hand sent far below the hero is an authored hide, not a part on the floor.
        let hidden = clip("ui_shop", vec![stepped(HAND, &[&[0.3, -0.3, 0.0], &[0.3, -5.0, 0.0]])]);
        assert!(!stages_pose(&rig, &hidden, Some(&standing())));
    }

    #[test]
    fn a_body_lying_flat_is_staged() {
        let rig = biped();
        // A quarter turn about Z with the pelvis 20 cm up: the body lies on its side.
        let half = std::f64::consts::FRAC_1_SQRT_2;
        let lying = clip(
            "ui_shop",
            vec![
                track(
                    PELVIS,
                    Property::Rotation,
                    &[&[0.0, 0.0, half, half], &[0.0, 0.0, half, half]],
                ),
                track(PELVIS, Property::Translation, &[&[0.0, 0.2, 0.0], &[0.0, 0.2, 0.0]]),
            ],
        );
        let sampler = Sampler::new(&rig, &lying).unwrap();
        assert!(sampler.lies_flat());
        assert!(!sampler.part_on_floor());
        assert!(stages_pose(&rig, &lying, Some(&standing())));
    }

    #[test]
    fn a_seated_lifted_pose_is_staged() {
        let rig = biped();
        // Every floor contact held 40 cm up for the whole clip (Rem on his seat).
        let seated = pelvis_at("ui_shop", 1.4);
        assert!(stages_pose(&rig, &seated, Some(&standing())));
        assert!(stages_pose(&rig, &seated, None));
        // Lifts are not floor offsets the viewer applies.
        assert!(clip_floor_offset(&rig, &seated).unwrap() > 0.3);
    }

    #[test]
    fn a_hero_hovering_in_every_clip_is_not_staged() {
        let rig = biped();
        // Solomon: the standing idle hovers 40 cm up too, so the showcase is not staged.
        let idle = pelvis_at("primary_stand_idle", 1.4);
        let showcase = pelvis_at("ui_shop", 1.45);
        assert!(!stages_pose(&rig, &showcase, Some(&idle)));
        // Against a standing idle on the floor, the same hover is staged.
        assert!(stages_pose(&rig, &showcase, Some(&standing())));
    }

    #[test]
    fn floor_offsets_snap_noise_and_ignore_hidden_parts() {
        let rig = biped();
        let lowered = pelvis_at("ui_shop", 0.95);
        let offset = clip_floor_offset(&rig, &lowered).unwrap();
        assert!((offset + 0.05).abs() < 1e-9, "{offset}");
        // Under half a percent of 1.7 m is noise.
        let noise = pelvis_at("ui_shop", 0.995);
        assert_eq!(clip_floor_offset(&rig, &noise), Some(0.0));
        // A foot hidden far below does not lower the floor; the other foot still counts.
        let hidden = clip("ui_shop", vec![stepped(FOOT_L, &[&[0.0, -0.5, 0.0], &[0.0, -3.0, 0.0]])]);
        assert_eq!(clip_floor_offset(&rig, &hidden), Some(0.0));
        // Nothing on the floor in the rest pose: no offset.
        let mut floating = biped();
        floating.nodes[WRAPPER].0 = at([0.0, 0.5, 0.0]);
        assert_eq!(clip_floor_offset(&floating, &standing()), None);
    }

    #[test]
    fn unreadable_clips_are_not_staged() {
        let rig = biped();
        assert!(!stages_pose(&rig, &clip("ui_shop", vec![]), None));
        assert!(!stages_pose(&Rig::default(), &standing(), None));
        let mut unweighted = biped();
        unweighted.weighted.clear();
        assert!(!stages_pose(&unweighted, &pelvis_at("ui_shop", 1.4), None));
    }

    #[test]
    fn step_and_cubic_tracks_sample_their_keys() {
        let step = Track {
            interpolation: Interpolation::Step,
            ..track(PELVIS, Property::Translation, &[&[0.0, 1.0, 0.0], &[0.0, 2.0, 0.0]])
        };
        assert_eq!(sample_track(&step, 0.9)[1], 1.0);
        let cubic = Track {
            interpolation: Interpolation::CubicSpline,
            ..track(
                PELVIS,
                Property::Translation,
                &[&[9.0, 9.0, 9.0, 0.0, 1.0, 0.0, 9.0, 9.0, 9.0]],
            )
        };
        assert_eq!(sample_track(&cubic, 0.0)[1], 1.0);
    }

    // A glb of the biped: one skinned mesh weighting every joint, and the given clips as
    // translation tracks. Built by hand so `read_glb` and `drop_animation` meet a real file.
    fn biped_glb(clips: &[Clip]) -> Vec<u8> {
        use serde_json::json;
        let rig = biped();
        let mut bin: Vec<u8> = Vec::new();
        let mut views = Vec::new();
        let mut accessors = Vec::new();
        let mut push = |bin: &mut Vec<u8>, bytes: Vec<u8>, accessor: serde_json::Value| {
            views.push(json!({ "buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len() }));
            bin.extend_from_slice(&bytes);
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let mut accessor = accessor;
            accessor["bufferView"] = json!(views.len() - 1);
            accessors.push(accessor);
            accessors.len() - 1
        };
        let f32s = |values: &[f32]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let joints: Vec<usize> = rig.joints.clone();
        // One vertex per joint, fully weighted to it.
        let positions: Vec<f32> = vec![0.0; joints.len() * 3];
        let position = push(
            &mut bin,
            f32s(&positions),
            json!({ "componentType": 5126, "count": joints.len(), "type": "VEC3",
                    "min": [0.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0] }),
        );
        let ids: Vec<u8> = (0..joints.len())
            .flat_map(|i| [i as u16, 0, 0, 0])
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let joint_ids = push(
            &mut bin,
            ids,
            json!({ "componentType": 5123, "count": joints.len(), "type": "VEC4" }),
        );
        let weights: Vec<f32> = (0..joints.len()).flat_map(|_| [1.0, 0.0, 0.0, 0.0]).collect();
        let weight = push(
            &mut bin,
            f32s(&weights),
            json!({ "componentType": 5126, "count": joints.len(), "type": "VEC4" }),
        );
        let mut animations = Vec::new();
        for clip in clips {
            let mut samplers = Vec::new();
            let mut channels = Vec::new();
            for track in &clip.tracks {
                let times: Vec<f32> = track.times.iter().map(|t| *t as f32).collect();
                let input = push(
                    &mut bin,
                    f32s(&times),
                    json!({ "componentType": 5126, "count": times.len(), "type": "SCALAR",
                            "min": [times[0]], "max": [times[times.len() - 1]] }),
                );
                let values: Vec<f32> = track.values.iter().map(|v| *v as f32).collect();
                let output = push(
                    &mut bin,
                    f32s(&values),
                    json!({ "componentType": 5126, "count": times.len(), "type": "VEC3" }),
                );
                samplers.push(json!({ "input": input, "output": output }));
                channels.push(json!({ "sampler": samplers.len() - 1,
                                      "target": { "node": track.node, "path": "translation" } }));
            }
            animations.push(json!({ "name": clip.name, "samplers": samplers, "channels": channels }));
        }
        let mut nodes: Vec<serde_json::Value> = rig
            .nodes
            .iter()
            .map(|(local, children)| {
                let Local::Trs { t, .. } = local else { unreachable!() };
                let mut node = json!({ "translation": t });
                if !children.is_empty() {
                    node["children"] = json!(children);
                }
                node
            })
            .collect();
        nodes.push(json!({ "mesh": 0, "skin": 0 }));
        let root = json!({
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [WRAPPER, nodes.len() - 1] }],
            "nodes": nodes,
            "skins": [{ "joints": joints }],
            "meshes": [{ "primitives": [{ "attributes": {
                "POSITION": position, "JOINTS_0": joint_ids, "WEIGHTS_0": weight } }] }],
            "accessors": accessors,
            "bufferViews": views,
            "buffers": [{ "byteLength": bin.len() }],
            "animations": animations,
        });
        let mut chunk = serde_json::to_vec(&root).unwrap();
        while !chunk.len().is_multiple_of(4) {
            chunk.push(b' ');
        }
        let total = 12 + 8 + chunk.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend_from_slice(&0x4654_6c67u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4e4f_534au32.to_le_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004e_4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }

    fn choice(name: &str, action: Option<super::super::clips::Action>, showcase: bool) -> ClipChoice {
        use super::super::clips::{Playback, Showcase, ShowcaseState};
        ClipChoice {
            name: name.into(),
            playback: Playback::Loop,
            action,
            showcase: showcase.then(|| Showcase {
                graph: "models/heroes/x/x_ui.vnmgraph_c".into(),
                state: ShowcaseState::Shop,
                playback: Playback::Loop,
                speed: 1.0,
            }),
        }
    }

    #[test]
    fn the_glb_reader_sees_the_rig_and_clips() {
        let glb = biped_glb(&[standing(), pelvis_at("ui_shop", 1.4)]);
        let (rig, clips) = read_glb(&glb).expect("a glb");
        assert_eq!(rig.joints, biped().joints);
        assert_eq!(rig.weighted, biped().weighted);
        assert_eq!(clips.len(), 2);
        assert!(stages_pose(&rig, &clips[1], Some(&clips[0])));
        assert!(!stages_pose(&rig, &clips[0], None));
    }

    #[test]
    fn a_staged_showcase_starts_on_the_standing_idle() {
        use super::super::clips::Action;
        let glb = biped_glb(&[pelvis_at("ui_shop", 1.4), standing()]);
        let choices = [
            choice("ui_shop", Some(Action::Idle), true),
            choice("primary_stand_idle", Some(Action::Idle), false),
        ];
        let policy = policy(&glb, &choices);
        assert_eq!(policy.start.as_deref(), Some("primary_stand_idle"));
        assert_eq!(policy.drop, None);
        assert!(policy.floors.is_empty());
    }

    #[test]
    fn an_unstaged_showcase_drops_the_standing_idle_and_records_its_floor() {
        use super::super::clips::Action;
        let glb = biped_glb(&[pelvis_at("ui_shop", 0.95), standing(), pelvis_at("hero_pose", 0.9)]);
        let choices = [
            choice("ui_shop", Some(Action::Idle), true),
            choice("primary_stand_idle", Some(Action::Idle), false),
            choice("hero_pose", Some(Action::HeroPose), false),
        ];
        let policy = policy(&glb, &choices);
        assert_eq!(policy.start, None);
        assert_eq!(policy.drop.as_deref(), Some("primary_stand_idle"));
        assert_eq!(policy.floors.len(), 2);
        assert_eq!(policy.floors[0].0, "ui_shop");
        assert!((policy.floors[0].1 + 0.05).abs() < 1e-4);
        assert_eq!(policy.floors[1].0, "hero_pose");
        assert!((policy.floors[1].1 + 0.1).abs() < 1e-4);

        let dropped = drop_animation(&glb, "primary_stand_idle").expect("dropped");
        let (_, clips) = read_glb(&dropped).expect("still a glb");
        let names: Vec<&str> = clips.iter().map(|clip| clip.name.as_str()).collect();
        assert_eq!(names, ["ui_shop", "hero_pose"]);
        assert_eq!(drop_animation(&glb, "missing"), None);
    }

    #[test]
    fn without_a_showcase_nothing_is_dropped() {
        use super::super::clips::Action;
        let glb = biped_glb(&[standing()]);
        let policy = policy(&glb, &[choice("primary_stand_idle", Some(Action::Idle), false)]);
        assert_eq!(policy, Policy::default());
        // Not a glb: no floors, and with a showcase the idle is dropped (not staged).
        let policy = policy_for_garbage();
        assert_eq!(policy.drop.as_deref(), Some("primary_stand_idle"));
    }

    fn policy_for_garbage() -> Policy {
        use super::super::clips::Action;
        policy(
            b"not a glb",
            &[
                choice("ui_shop", Some(Action::Idle), true),
                choice("primary_stand_idle", Some(Action::Idle), false),
            ],
        )
    }
}
