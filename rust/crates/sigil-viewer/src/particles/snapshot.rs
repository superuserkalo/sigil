//! Particle snapshots: the bundle's decoded `.vsnap` tables bound to control points, and the
//! functions that read them, ported from Grimoire (MIT), itself a port of ValveResourceFormat
//! (MIT): `utils/snapshotBinding.ts`, `utils/skinnedSnapshot.ts`, `utils/hitBoxCache.ts`
//! (`computeSkinningMatrix`), `initializers/initSkinnedPositionFromCPSnapshot.ts` and
//! `operators/snapshotRigidSkinToBones.ts`.
//!
//! A skinned snapshot's points are deformed by the bones of control point 0's model, the hero:
//! each frame the host hands every system a palette of `posed bone * inverse bind` matrices
//! in Source model space (the space the simulation runs in), as Grimoire's bone cache reads
//! the hero's pose.

use std::sync::Arc;

use serde_json::Value;

use super::fields::{self as fl, Particles};
use super::kv;
use super::providers::{NumIn, ParseCx};
use super::smath::{safe_normalize, sat, table};
use super::state::State;
use crate::math::{Mat4, Vec3, add, length, scale, sub};

/// The attribute names VRF gives the fields a snapshot can hold (`SNAPSHOT_ATTRIBUTE_NAMES`).
pub fn attribute_name(field: usize) -> Option<&'static str> {
    Some(match field {
        0 => "position",
        1 => "lifespan",
        2 => "velocity",
        3 => "radius",
        4 => "rotation",
        5 => "rotation_speed",
        6 => "color",
        7 => "opacity",
        8 => "creation_time",
        9 => "sequence_number",
        10 => "trail_length",
        11 => "particle_id",
        12 => "yaw",
        13 => "sequence_number1",
        14 => "hitbox",
        15 => "hitbox_offset",
        16 => "alpha2",
        17 => "scratch_vec",
        18 => "scratch_float",
        20 => "pitch",
        21 => "normal",
        22 => "glow_rgb",
        23 => "glow_alpha",
        34 => "force_scale",
        38 => "manual_animation_frame",
        39 => "shader_extra_data_1",
        40 => "shader_extra_data_2",
        47 => "rope_segment_id",
        _ => return None,
    })
}

/// One snapshot table, its columns typed as VRF reads them.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub count: usize,
    /// The `float` and `int` columns by name, the first of each name.
    pub scalars: Vec<(String, Vec<f32>)>,
    pub vectors: Vec<(String, Vec<Vec3>)>,
    /// Per row: four bone slots (indices into the bundle's bones, [`fl::BONE_NONE`] for none)
    /// and four weights, from a `skinning` column or a `hitbox` bone name column.
    pub bones: Option<Vec<([f32; 4], [f32; 4])>>,
    /// Whether the rows came from hitbox names (one whole bone each).
    pub hitbox: bool,
}

impl Snapshot {
    /// Reads a bundle snapshot (`{ numParticles, strings, columns: [{ name, type, data, values }] }`),
    /// naming bones by their index in `bones`, the bundle's model bones.
    pub fn parse(v: &Value, bones: &[String]) -> Snapshot {
        // The exporter caps rows at 65536; a cache file edited by hand cannot ask for more.
        let count = kv::i(v, "numParticles", 0).clamp(0, 65_536) as usize;
        let mut s = Snapshot { count, ..Snapshot::default() };
        let bone = |name: &str| -> f32 {
            if bones.is_empty() {
                return fl::BONE_NONE;
            }
            // A name the model lacks reads bone 0, as VRF's `GetBoneIndex` then `Max(0)`.
            let i = bones.iter().position(|b| b == name).or_else(|| bones.iter().position(|b| b.eq_ignore_ascii_case(name))).unwrap_or(0);
            (i % bones.len()) as f32
        };
        let mut skinning = None;
        let mut hitboxes = None;
        // Joint and hitbox names are indices into the snapshot's string table (-1 for none).
        // Each name is looked up once, not once per row.
        let strings: Vec<f32> = kv::array(v, "strings").iter().map(|x| bone(x.as_str().unwrap_or(""))).collect();
        let unnamed = bone("");
        let named = |x: Option<&Value>| -> f32 {
            x.and_then(kv::number).and_then(|i| usize::try_from(i as i64).ok()).and_then(|i| strings.get(i)).copied().unwrap_or(unnamed)
        };
        for c in kv::array(v, "columns") {
            let name = kv::s(c, "name").unwrap_or("").to_string();
            let values = kv::array(c, "values");
            let num = |x: &Value| kv::number(x).unwrap_or(0.0) as f32;
            match kv::s(c, "data").unwrap_or("") {
                "float" | "int" if !s.scalars.iter().any(|(n, _)| *n == name) => s.scalars.push((name, values.iter().map(num).collect())),
                "vector" if !s.vectors.iter().any(|(n, _)| *n == name) => s.vectors.push((name, values.as_chunks::<3>().0.iter().map(|[x, y, z]| [num(x), num(y), num(z)]).collect())),
                "skinning" if skinning.is_none() => {
                    let table = c.get("values");
                    let joints = table.map_or(&[][..], |t| kv::array(t, "joints"));
                    let weights = table.map_or(&[][..], |t| kv::array(t, "weights"));
                    skinning = Some(
                        (0..count.min(joints.len() / 4))
                            .map(|row| {
                                let mut b = [fl::BONE_NONE; 4];
                                let mut w = [0.0; 4];
                                for k in 0..4 {
                                    b[k] = if bones.is_empty() { fl::BONE_NONE } else { named(joints.get(row * 4 + k)) };
                                    w[k] = weights.get(row * 4 + k).map_or(0.0, num);
                                }
                                (b, w)
                            })
                            .collect::<Vec<_>>(),
                    );
                }
                "string" if name == "hitbox" && hitboxes.is_none() => {
                    hitboxes = Some(values.iter().map(|x| ([named(Some(x)), fl::BONE_NONE, fl::BONE_NONE, fl::BONE_NONE], [1.0, 0.0, 0.0, 0.0])).collect::<Vec<_>>());
                }
                _ => {}
            }
        }
        s.hitbox = skinning.is_none() && hitboxes.is_some();
        s.bones = skinning.or(hitboxes);
        s
    }

    pub fn scalar(&self, field: usize) -> Option<&[f32]> {
        let name = attribute_name(field)?;
        self.scalars.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }

    pub fn vector(&self, field: usize) -> Option<&[Vec3]> {
        let name = attribute_name(field)?;
        self.vectors.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }

    /// How many points a skinned read may use: none without positions and bones.
    pub fn skinned_rows(&self) -> usize {
        match (self.vector(fl::POSITION), &self.bones) {
            (Some(p), Some(b)) => self.count.min(p.len()).min(b.len()),
            _ => 0,
        }
    }
}

/// A snapshot a function reads, bound to a control point (VRF `SnapshotBinding`): a function
/// that names a subset is unbound, as compiled snapshots carry none.
#[derive(Clone, Debug)]
pub struct Binding {
    pub cp: i32,
    subset: bool,
}

impl Binding {
    pub fn parse(v: &Value, key: &str, default: i32, clamp: bool) -> Binding {
        let cp = kv::i(v, key, default);
        Binding { cp: if clamp { cp.clamp(0, 63) } else { cp }, subset: kv::s(v, "m_strSnapshotSubset").is_some_and(|s| !s.is_empty()) }
    }

    pub fn is_bound(&self) -> bool {
        self.cp >= 0 && !self.subset
    }

    pub fn has_cp(&self) -> bool {
        self.cp >= 0
    }

    pub fn resolve<'a>(&self, st: &'a State) -> Option<&'a Arc<Snapshot>> {
        if self.cp < 0 || self.subset {
            return None;
        }
        st.snapshots.get(self.cp as usize).and_then(Option::as_ref)
    }

    pub fn count(&self, st: &State) -> usize {
        self.resolve(st).map_or(0, |s| s.count)
    }
}

/// The skin of a particle bound to up to four bones (`computeSkinningMatrix`): one valid bone
/// takes its matrix whole, several blend by weight (the fourth weight is what the first three
/// leave). `None` when no slot names a bone of the palette.
pub fn skin_matrix(palette: &[Mat4], bones: [f32; 4], weights: Vec3) -> Option<Mat4> {
    let last = sat(1.0 - weights[0] - weights[1] - weights[2]);
    let all = [weights[0], weights[1], weights[2], last];
    let mut valid = [(0usize, 0.0f32); 4];
    let mut n = 0;
    for k in 0..4 {
        let b = bones[k];
        if b == fl::BONE_NONE || b < 0.0 || b as usize >= palette.len() {
            continue;
        }
        valid[n] = (b as usize, all[k]);
        n += 1;
    }
    match n {
        0 => None,
        1 => Some(palette[valid[0].0]),
        _ => {
            let mut m = Mat4([[0.0; 4]; 4]);
            for &(b, w) in &valid[..n] {
                for c in 0..4 {
                    for r in 0..4 {
                        m.0[c][r] += palette[b].0[c][r] * w;
                    }
                }
            }
            m.0[0][3] = 0.0;
            m.0[1][3] = 0.0;
            m.0[2][3] = 0.0;
            m.0[3][3] = 1.0;
            Some(m)
        }
    }
}

fn max_axis_scale(m: &Mat4) -> f32 {
    (0..3).map(|c| length([m.0[c][0], m.0[c][1], m.0[c][2]])).fold(0.0, f32::max)
}

fn translation(m: &Mat4) -> Vec3 {
    [m.0[3][0], m.0[3][1], m.0[3][2]]
}

/// The palette of a function's model control point: only control point 0 carries the hero.
fn palette(st: &State, cp: i32) -> Option<&(Arc<Vec<Mat4>>, f32)> {
    if cp == 0 { st.skin.as_ref().filter(|(p, _)| !p.is_empty()) } else { None }
}

/// `C_INIT_InitSkinnedPositionFromCPSnapshot`: new particles on the points of the snapshot
/// bound to `m_nSnapshotControlPointNumber`, deformed by the bones of control point 0's model
/// (positions taken as they are without one), with their rest position and bones recorded
/// for [`RigidSkin`]. Normals, radii, colour and alpha can come along, and particles can leave
/// along their normal or with their bones' velocity.
#[derive(Clone, Debug)]
pub struct SkinnedInit {
    snapshot: Binding,
    cp: i32,
    random: bool,
    random_seed: i32,
    rigid: bool,
    set_normal: bool,
    ignore_dt: bool,
    min_normal_velocity: f32,
    max_normal_velocity: f32,
    direct: bool,
    read_index: NumIn,
    increment: f32,
    full_loop: i32,
    start: i32,
    bone_velocity: f32,
    bone_velocity_max: f32,
    copy_color: bool,
    copy_alpha: bool,
    set_radius: bool,
    next: i32,
    counter: i32,
}

impl SkinnedInit {
    pub fn parse(v: &Value, cx: &mut ParseCx) -> (SkinnedInit, u64) {
        let direct = kv::enumv(v, "m_nIndexType", &[("SNAPSHOT_INDEX_INCREMENT", 0), ("SNAPSHOT_INDEX_DIRECT", 1)], 0) == 1;
        let init = SkinnedInit {
            snapshot: Binding::parse(v, "m_nSnapshotControlPointNumber", 1, true),
            cp: kv::i(v, "m_nControlPointNumber", 0).clamp(0, 63),
            random: kv::b(v, "m_bRandom", false),
            random_seed: kv::i(v, "m_nRandomSeed", 0),
            rigid: kv::b(v, "m_bRigid", false),
            set_normal: kv::b(v, "m_bSetNormal", false),
            ignore_dt: kv::b(v, "m_bIgnoreDt", false),
            min_normal_velocity: kv::f(v, "m_flMinNormalVelocity", 0.0),
            max_normal_velocity: kv::f(v, "m_flMaxNormalVelocity", 0.0),
            direct,
            read_index: NumIn::parse_or(v, "m_flReadIndex", 0.0, cx),
            increment: kv::f(v, "m_flIncrement", 1.0),
            full_loop: kv::i(v, "m_nFullLoopIncrement", 0),
            start: kv::i(v, "m_nSnapShotStartPoint", 0),
            bone_velocity: kv::f(v, "m_flBoneVelocity", 0.0),
            bone_velocity_max: kv::f(v, "m_flBoneVelocityMax", 0.0),
            copy_color: kv::b(v, "m_bCopyColor", false),
            copy_alpha: kv::b(v, "m_bCopyAlpha", false),
            set_radius: kv::b(v, "m_bSetRadius", false),
            next: 0,
            counter: 0,
        };
        let m = |f: usize| fl::mask(f as i32);
        let mut written = m(fl::POSITION) | m(fl::PREV) | m(fl::HITBOX_OFFSET) | m(fl::BONE_INDICES) | m(fl::BONE_WEIGHTS) | m(fl::BONE_INDEX3);
        for (on, f) in [(init.set_normal, fl::NORMAL), (init.copy_color, fl::COLOR), (init.copy_alpha, fl::ALPHA), (init.set_radius, fl::RADIUS)] {
            if on {
                written |= m(f);
            }
        }
        (init, written)
    }

    pub fn reset(&mut self) {
        self.next = self.start;
        self.counter = 0;
    }

    /// The row the walk reads at `index`: past the last it wraps, moved on by the full loop
    /// increment per loop.
    pub fn walk_row(index: i32, count: i32, full_loop: i32) -> i32 {
        if index >= count { (index + full_loop * (index / count)) % count } else { index }
    }

    fn row(&mut self, direct: Option<f32>, st: &mut State, count: usize) -> i32 {
        if let Some(d) = direct {
            return ((d as i32 as u32) % (count as u32)) as i32;
        }
        if self.random {
            let r = if self.random_seed != 0 {
                let k = self.counter;
                self.counter = self.counter.wrapping_add(1);
                table(st.rnd.seed.wrapping_add(self.random_seed).wrapping_add(k))
            } else {
                st.rnd.next()
            };
            return (count as f32 * r) as i32;
        }
        let index = Self::walk_row(self.next, count as i32, self.full_loop);
        self.next = index.wrapping_add(self.increment as i32);
        index
    }

    pub fn run(&mut self, st: &mut State, p: &mut Particles, first: usize, n: usize, scratch: &mut Vec<f32>) {
        let Some(snap) = self.snapshot.resolve(st).cloned() else { return };
        let rows = snap.skinned_rows();
        let positions = snap.vector(fl::POSITION);
        let normals = snap.vector(fl::NORMAL);
        let colors = snap.vector(fl::COLOR);
        let radii = snap.scalar(fl::RADIUS);
        let alphas = snap.scalar(fl::ALPHA);
        let (Some(positions), Some(bones)) = (positions, snap.bones.as_ref()) else { return };
        if rows == 0 || (self.set_normal && normals.is_none()) || (self.copy_color && colors.is_none()) || (self.copy_alpha && alphas.is_none()) || (self.set_radius && radii.is_none()) {
            return;
        }
        if self.direct {
            self.read_index.fill(st, p, first, n, scratch);
        }
        let skin = palette(st, self.cp).cloned();
        let prev_skin = st.skin_prev.clone();
        for k in 0..n {
            let i = first + k;
            let direct = self.direct.then(|| scratch.get(k).copied().unwrap_or(0.0));
            let row = self.row(direct, st, rows);
            if row < 0 || row as usize >= rows {
                continue;
            }
            let row = row as usize;
            if let Some(c) = colors {
                p.v[fl::COLOR][i] = c[row];
            }
            if let Some(a) = alphas.filter(|_| self.copy_alpha) {
                p.s[fl::ALPHA][i] = a[row];
            }
            let local = positions[row];
            let Some((pal, now)) = &skin else {
                p.v[fl::POSITION][i] = local;
                p.v[fl::PREV][i] = local;
                if let Some(nr) = normals.filter(|_| self.set_normal) {
                    p.v[fl::NORMAL][i] = nr[row];
                }
                if let Some(r) = radii.filter(|_| self.set_radius) {
                    p.s[fl::RADIUS][i] = r[row];
                }
                continue;
            };
            let (b, w) = bones[row];
            p.v[fl::BONE_INDICES][i] = [b[0], b[1], b[2]];
            p.s[fl::BONE_INDEX3][i] = b[3];
            p.v[fl::HITBOX_OFFSET][i] = local;
            p.v[fl::BONE_WEIGHTS][i] = [w[0], w[1], w[2]];
            let skin_m = skin_matrix(pal, b, [w[0], w[1], w[2]]).unwrap_or(Mat4::IDENTITY);
            let mut normal_offset = [0.0; 3];
            if let Some(nr) = normals.filter(|_| self.set_normal) {
                let normal = if self.rigid { nr[row] } else { safe_normalize(skin_m.vector(nr[row])) };
                p.v[fl::NORMAL][i] = normal;
                let speed = st.rnd.next_between(self.min_normal_velocity, self.max_normal_velocity);
                normal_offset = scale(normal, if self.ignore_dt { speed } else { speed * st.previous_step });
            }
            if let Some(r) = radii.filter(|_| self.set_radius) {
                p.s[fl::RADIUS][i] = r[row] * if self.rigid { 1.0 } else { max_axis_scale(&skin_m) };
            }
            if self.rigid {
                p.v[fl::POSITION][i] = [0.0; 3];
                p.v[fl::PREV][i] = [0.0; 3];
            } else {
                p.v[fl::POSITION][i] = skin_m.point(local);
                p.v[fl::PREV][i] = skin_m.point(add(local, normal_offset));
            }
            // Leaving with the bones' velocity, from the palette a frame earlier.
            if self.bone_velocity != 0.0
                && let Some((prev_pal, then)) = &prev_skin
                && *now > *then
                && let Some(prev_m) = skin_matrix(prev_pal, b, [w[0], w[1], w[2]])
            {
                let mut v = scale(sub(translation(&skin_m), translation(&prev_m)), self.bone_velocity / (now - then));
                let l = length(v);
                if self.bone_velocity_max > 0.0 && l > self.bone_velocity_max {
                    v = scale(v, self.bone_velocity_max / l);
                }
                let step = if self.ignore_dt { v } else { scale(v, st.previous_step) };
                p.v[fl::PREV][i] = sub(p.v[fl::PREV][i], step);
            }
        }
    }
}

/// `C_OP_SnapshotRigidSkinToBones`: re-skins particles bound to bones to control point 0's
/// pose every step: the rest position, offset by the initial previous position, through the
/// particle's blend of bones; normals and radii on request.
#[derive(Clone, Debug)]
pub struct RigidSkin {
    normals: bool,
    radii: bool,
    cp: i32,
}

impl RigidSkin {
    pub fn parse(v: &Value) -> RigidSkin {
        RigidSkin { normals: kv::b(v, "m_bTransformNormals", false), radii: kv::b(v, "m_bTransformRadii", false), cp: kv::i(v, "m_nControlPointNumber", 0).clamp(0, 63) }
    }

    pub fn operate(&self, st: &State, p: &mut Particles) {
        let Some((pal, _)) = palette(st, self.cp) else { return };
        for i in 0..p.len {
            let b = p.v[fl::BONE_INDICES][i];
            let Some(skin) = skin_matrix(pal, [b[0], b[1], b[2], p.s[fl::BONE_INDEX3][i]], p.v[fl::BONE_WEIGHTS][i]) else { continue };
            let rest = p.v[fl::HITBOX_OFFSET][i];
            p.v[fl::POSITION][i] = skin.point(rest);
            p.v[fl::PREV][i] = skin.point(add(rest, p.iv[fl::PREV][i]));
            if self.normals {
                p.v[fl::NORMAL][i] = safe_normalize(skin.vector(p.iv[fl::NORMAL][i]));
            }
            if self.radii {
                p.s[fl::RADIUS][i] = max_axis_scale(&skin) * p.is[fl::RADIUS][i];
            }
        }
    }
}
