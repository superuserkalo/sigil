//! Hero cloth: a port of Grimoire's Source 2 FeModel preview solver (MIT, Slush97/grimoire:
//! `src/lib/feModel.ts`, `clothConstraints.ts`, `clothMath.ts`, `useClothSim.ts`).
//!
//! The input is the JSON Sigil writes beside a hero (`<codename>.cloth.json`): the raw KV3
//! `PHYS.m_pFeModel` subtree with its key names verbatim, in Source units (inches, Z up, model
//! space, quaternions `[x, y, z, w]`). Nodes map to glb joints by exact name. A similarity fit
//! from the FeModel's rest pose to the glb's bind pose carries every solver quantity between
//! Source model space and the glb frame, so the solver itself runs in Source units like the
//! game and Grimoire do.
//!
//! The driver matches Grimoire's: fixed 1/120 s substeps, at most 12 a frame, an 8 ms wall
//! budget, the animation sampled once per substep through a caller closure, a 0.25 s resume
//! gap that drops the backlog and restarts the drape, and the same recovery guards. The math
//! runs in `f64` like Grimoire's JavaScript numbers; the engine's `Math.fround` thresholds keep
//! their `f32` values. Nothing allocates per substep after construction except the node locals
//! the caller's closure returns.

use std::time::Instant;

use crate::math::Mat4;
use crate::model::Model;
use serde_json::Value;

/// The solver's fixed substep, seconds.
pub const CLOTH_TIMESTEP: f64 = 1.0 / 120.0;
const MAX_CLOTH_STEPS_PER_FRAME: usize = 12;
/// Wall-clock time a frame gives the cloth before the animation catches up alone.
pub const CLOTH_FRAME_BUDGET_MS: f64 = 8.0;
/// The most animation time a frame over budget advances at once, four of the usual.
const MAX_CLOTH_CATCH_UP: f64 = 4.0 * CLOTH_TIMESTEP;
/// A frame longer than this (a suspended app, a seek) restarts the drape instead of catching up.
pub const CLOTH_RESUME_GAP: f64 = 0.25;
const MIN_TIMESTEP_HISTORY_RATIO: f64 = 0.25;
/// Legacy exports without integrator selectors keep the preview damping fallback.
const MIN_VELOCITY_DAMPING: f64 = 0.02;
/// `2 ** -23`, the engine's float epsilon.
const EPS23: f64 = 1.0 / 8_388_608.0;
/// `2 ** -30`.
const EPS30: f64 = 1.0 / 1_073_741_824.0;

// ---------------------------------------------------------------------------------------------
// f64 math with three.js semantics (normalize divides by `length || 1`, invert is conjugate).

type V3 = [f64; 3];
type Q4 = [f64; 4];
/// Column-major 4x4, three.js `Matrix4.elements` order.
type M4 = [f64; 16];

const Q_ID: Q4 = [0.0, 0.0, 0.0, 1.0];
const M4_ID: M4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

#[inline]
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn scl(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
/// `a + b * k` (three's `addScaledVector`).
#[inline]
fn madd(a: V3, b: V3, k: f64) -> V3 {
    [a[0] + b[0] * k, a[1] + b[1] * k, a[2] + b[2] * k]
}
#[inline]
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn len_sq(a: V3) -> f64 {
    dot(a, a)
}
#[inline]
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
#[inline]
fn dist_sq(a: V3, b: V3) -> f64 {
    len_sq(sub(a, b))
}
#[inline]
fn dist(a: V3, b: V3) -> f64 {
    len(sub(a, b))
}
/// three's `Vector3.normalize`: divides by the length, or by one for a zero vector.
#[inline]
fn norm(a: V3) -> V3 {
    let l = len(a);
    // `length || 1`: zero and NaN are both falsy.
    let l = if l == 0.0 || l.is_nan() { 1.0 } else { l };
    scl(a, 1.0 / l)
}
/// three's `lerp`: `a + (b - a) * t`.
#[inline]
fn lerp3(a: V3, b: V3, t: f64) -> V3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
#[inline]
fn finite3(a: V3) -> bool {
    a.iter().all(|v| v.is_finite())
}
fn finite_rot(q: Q4) -> bool {
    q.iter().all(|v| v.is_finite()) && q_dot(q, q) > 1e-12
}
#[inline]
fn q_dot(a: Q4, b: Q4) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}
/// three's `Quaternion.normalize`: identity for a zero quaternion.
fn q_norm(q: Q4) -> Q4 {
    let l = q_dot(q, q).sqrt();
    if l == 0.0 { Q_ID } else { [q[0] / l, q[1] / l, q[2] / l, q[3] / l] }
}
/// three's `multiplyQuaternions(a, b)`.
#[inline]
fn q_mul(a: Q4, b: Q4) -> Q4 {
    [
        a[0] * b[3] + a[3] * b[0] + a[1] * b[2] - a[2] * b[1],
        a[1] * b[3] + a[3] * b[1] + a[2] * b[0] - a[0] * b[2],
        a[2] * b[3] + a[3] * b[2] + a[0] * b[1] - a[1] * b[0],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}
/// three's `Quaternion.invert` (the conjugate; it does not renormalize).
#[inline]
fn q_conj(q: Q4) -> Q4 {
    [-q[0], -q[1], -q[2], q[3]]
}
/// three's `Vector3.applyQuaternion`.
#[inline]
fn q_rot(q: Q4, v: V3) -> V3 {
    let (qx, qy, qz, qw) = (q[0], q[1], q[2], q[3]);
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [v[0] + qw * tx + qy * tz - qz * ty, v[1] + qw * ty + qz * tx - qx * tz, v[2] + qw * tz + qx * ty - qy * tx]
}
/// three's `setFromUnitVectors`.
fn q_from_unit(from: V3, to: V3) -> Q4 {
    let mut r = dot(from, to) + 1.0;
    let q = if r < f64::EPSILON {
        r = 0.0;
        if from[0].abs() > from[2].abs() { [-from[1], from[0], 0.0, r] } else { [0.0, -from[2], from[1], r] }
    } else {
        [from[1] * to[2] - from[2] * to[1], from[2] * to[0] - from[0] * to[2], from[0] * to[1] - from[1] * to[0], r]
    };
    q_norm(q)
}
/// three's `setFromRotationMatrix` on a basis given as columns (no normalization).
fn q_from_cols(x: V3, y: V3, z: V3) -> Q4 {
    let (m11, m12, m13) = (x[0], y[0], z[0]);
    let (m21, m22, m23) = (x[1], y[1], z[1]);
    let (m31, m32, m33) = (x[2], y[2], z[2]);
    let trace = m11 + m22 + m33;
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        [(m32 - m23) * s, (m13 - m31) * s, (m21 - m12) * s, 0.25 / s]
    } else if m11 > m22 && m11 > m33 {
        let s = 2.0 * (1.0 + m11 - m22 - m33).sqrt();
        [0.25 * s, (m12 + m21) / s, (m13 + m31) / s, (m32 - m23) / s]
    } else if m22 > m33 {
        let s = 2.0 * (1.0 + m22 - m11 - m33).sqrt();
        [(m12 + m21) / s, 0.25 * s, (m23 + m32) / s, (m13 - m31) / s]
    } else {
        let s = 2.0 * (1.0 + m33 - m11 - m22).sqrt();
        [(m13 + m31) / s, (m23 + m32) / s, 0.25 * s, (m21 - m12) / s]
    }
}
fn q_from_v4(v: [f64; 4]) -> Q4 {
    q_norm(v)
}

/// three's `Matrix4.compose`.
fn m4_compose(p: V3, q: Q4, s: V3) -> M4 {
    let [x, y, z, w] = q;
    let (x2, y2, z2) = (x + x, y + y, z + z);
    let (xx, xy, xz) = (x * x2, x * y2, x * z2);
    let (yy, yz, zz) = (y * y2, y * z2, z * z2);
    let (wx, wy, wz) = (w * x2, w * y2, w * z2);
    [
        (1.0 - (yy + zz)) * s[0],
        (xy + wz) * s[0],
        (xz - wy) * s[0],
        0.0,
        (xy - wz) * s[1],
        (1.0 - (xx + zz)) * s[1],
        (yz + wx) * s[1],
        0.0,
        (xz + wy) * s[2],
        (yz - wx) * s[2],
        (1.0 - (xx + yy)) * s[2],
        0.0,
        p[0],
        p[1],
        p[2],
        1.0,
    ]
}
fn m4_mul(a: &M4, b: &M4) -> M4 {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
        }
    }
    out
}
/// three's `Matrix4.invert` (zero matrix when singular).
fn m4_invert(m: &M4) -> M4 {
    let te = m;
    let (n11, n21, n31, n41) = (te[0], te[1], te[2], te[3]);
    let (n12, n22, n32, n42) = (te[4], te[5], te[6], te[7]);
    let (n13, n23, n33, n43) = (te[8], te[9], te[10], te[11]);
    let (n14, n24, n34, n44) = (te[12], te[13], te[14], te[15]);
    let t11 = n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43 - n23 * n32 * n44 + n22 * n33 * n44;
    let t12 = n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43 + n13 * n32 * n44 - n12 * n33 * n44;
    let t13 = n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43 - n13 * n22 * n44 + n12 * n23 * n44;
    let t14 = n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34 - n12 * n23 * n34;
    let det = n11 * t11 + n21 * t12 + n31 * t13 + n41 * t14;
    if det == 0.0 {
        return [0.0; 16];
    }
    let d = 1.0 / det;
    [
        t11 * d,
        (n24 * n33 * n41 - n23 * n34 * n41 - n24 * n31 * n43 + n21 * n34 * n43 + n23 * n31 * n44 - n21 * n33 * n44) * d,
        (n22 * n34 * n41 - n24 * n32 * n41 + n24 * n31 * n42 - n21 * n34 * n42 - n22 * n31 * n44 + n21 * n32 * n44) * d,
        (n23 * n32 * n41 - n22 * n33 * n41 - n23 * n31 * n42 + n21 * n33 * n42 + n22 * n31 * n43 - n21 * n32 * n43) * d,
        t12 * d,
        (n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n43 - n11 * n34 * n43 - n13 * n31 * n44 + n11 * n33 * n44) * d,
        (n14 * n32 * n41 - n12 * n34 * n41 - n14 * n31 * n42 + n11 * n34 * n42 + n12 * n31 * n44 - n11 * n32 * n44) * d,
        (n12 * n33 * n41 - n13 * n32 * n41 + n13 * n31 * n42 - n11 * n33 * n42 - n12 * n31 * n43 + n11 * n32 * n43) * d,
        t13 * d,
        (n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n43 + n11 * n24 * n43 + n13 * n21 * n44 - n11 * n23 * n44) * d,
        (n12 * n24 * n41 - n14 * n22 * n41 + n14 * n21 * n42 - n11 * n24 * n42 - n12 * n21 * n44 + n11 * n22 * n44) * d,
        (n13 * n22 * n41 - n12 * n23 * n41 - n13 * n21 * n42 + n11 * n23 * n42 + n12 * n21 * n43 - n11 * n22 * n43) * d,
        t14 * d,
        (n13 * n24 * n31 - n14 * n23 * n31 + n14 * n21 * n33 - n11 * n24 * n33 - n13 * n21 * n34 + n11 * n23 * n34) * d,
        (n14 * n22 * n31 - n12 * n24 * n31 - n14 * n21 * n32 + n11 * n24 * n32 + n12 * n21 * n34 - n11 * n22 * n34) * d,
        (n12 * n23 * n31 - n13 * n22 * n31 + n13 * n21 * n32 - n11 * n23 * n32 - n12 * n21 * n33 + n11 * n22 * n33) * d,
    ]
}
/// three's `Vector3.applyMatrix4` (with the projective divide).
fn m4_point(e: &M4, v: V3) -> V3 {
    let w = 1.0 / (e[3] * v[0] + e[7] * v[1] + e[11] * v[2] + e[15]);
    [
        (e[0] * v[0] + e[4] * v[1] + e[8] * v[2] + e[12]) * w,
        (e[1] * v[0] + e[5] * v[1] + e[9] * v[2] + e[13]) * w,
        (e[2] * v[0] + e[6] * v[1] + e[10] * v[2] + e[14]) * w,
    ]
}
/// The rotation of three's `Matrix4.decompose`.
fn m4_rotation(te: &M4) -> Q4 {
    let mut sx = len([te[0], te[1], te[2]]);
    let sy = len([te[4], te[5], te[6]]);
    let sz = len([te[8], te[9], te[10]]);
    let det = te[0] * (te[5] * te[10] - te[9] * te[6]) - te[4] * (te[1] * te[10] - te[9] * te[2]) + te[8] * (te[1] * te[6] - te[5] * te[2]);
    if det < 0.0 {
        sx = -sx;
    }
    q_from_cols(scl([te[0], te[1], te[2]], 1.0 / sx), scl([te[4], te[5], te[6]], 1.0 / sy), scl([te[8], te[9], te[10]], 1.0 / sz))
}
fn m4_from(m: &Mat4) -> M4 {
    let a = m.to_cols_array();
    let mut out = [0.0; 16];
    for (o, v) in out.iter_mut().zip(a) {
        *o = v as f64;
    }
    out
}
#[inline]
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(v))
}
/// Grimoire's `unit`: clamp to `[0, 1]`, zero for a non-finite value.
#[inline]
fn unit(v: f64) -> f64 {
    if v.is_finite() { clamp(v, 0.0, 1.0) } else { 0.0 }
}
/// JavaScript's `Math.round` (half rounds up).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}
/// JavaScript's `ToInt32`, for collision masks combined with `&`.
fn to_i32(v: f64) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    let t = v.trunc().rem_euclid(4_294_967_296.0);
    (t as u64 as u32) as i32
}
/// `Number.isInteger(v) && v >= 0 && v < n`.
fn idx(v: f64, n: usize) -> Option<usize> {
    (v.is_finite() && v.fract() == 0.0 && v >= 0.0 && v < n as f64).then_some(v as usize)
}

// ---------------------------------------------------------------------------------------------
// The FeModel parser (`feModel.ts`).

#[derive(Clone, Debug, PartialEq)]
struct FeNode {
    name: String,
    inv_mass: f64,
    pinned: bool,
    gravity: f64,
    damping: f64,
    anim_force: f64,
    anim_vertex: f64,
    init_pos: V3,
    init_rot: Q4,
    collide_radius: f64,
    friction: f64,
    collision_mask: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rod {
    a: usize,
    b: usize,
    min: f64,
    max: f64,
    relax: f64,
    weight: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct ColliderFilter {
    mask: f64,
    priority: usize,
    /// Absent: layer filtering; empty: explicitly selects no nodes. Sorted.
    vertex_nodes: Option<Vec<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
struct CapsuleRigid {
    sphere0: [f64; 4],
    sphere1: [f64; 4],
    node: f64,
    filter: ColliderFilter,
}

#[derive(Clone, Debug, PartialEq)]
struct BoxRigid {
    pos: V3,
    rot: Q4,
    half_size: V3,
    node: f64,
    filter: ColliderFilter,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct NodeBase {
    node: usize,
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    q_adjust: Q4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Offset {
    offset: V3,
    parent: f64,
    child: f64,
    alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ReverseOffset {
    offset: V3,
    bone_ctrl: f64,
    target_node: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StrayRadius {
    /// Animation target, simulated particle.
    node: [usize; 2],
    max_dist: f64,
    relax: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Twist {
    node_orient: usize,
    node_end: usize,
    twist_relax: f64,
    swing_relax: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FitMatrix {
    bone: V3,
    bone_rot: Q4,
    center: V3,
    end_weight: f64,
    node: f64,
    begin_dynamic: f64,
    ctrl: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FitWeight {
    weight: f64,
    node: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CollisionPlane {
    priority: usize,
    ctrl_parent: f64,
    child_node: f64,
    normal: V3,
    offset: f64,
    strength: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct JiggleBone {
    node: f64,
    has_params: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct KelagerBend {
    height0: f64,
    /// Bent node, first end, second end.
    node: [usize; 3],
    /// Signed solver shares, not inverse masses or flags.
    weight: V3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HingeLimit {
    node: [usize; 6],
    weight4: f64,
    weight5: f64,
    center: f64,
    extents: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Triangle {
    node: [usize; 3],
    static_count: usize,
    weight1: f64,
    weight2: f64,
    x1: f64,
    x2: f64,
    y2: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Quad {
    node: [usize; 4],
    static_count: usize,
    shape: [[f64; 4]; 4],
    slack: f64,
}

/// A record the parser rejected, so coverage can report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeIssue {
    pub array: &'static str,
    pub record: usize,
    pub reason: &'static str,
}

/// A compiled feature the preview decodes but does not simulate (or only approximates).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureGap {
    pub field: String,
    pub label: &'static str,
    pub count: usize,
    pub approximate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    GoalDamped,
    Raw,
    Unknown,
}

#[derive(Clone, Debug, Default)]
struct FeModel {
    nodes: Vec<FeNode>,
    rods: Vec<Rod>,
    rod_batches: Vec<Vec<Rod>>,
    animated_rods: usize,
    animated_rod_batches: Vec<Vec<Rod>>,
    decode_issues: Vec<DecodeIssue>,
    feature_gaps: Vec<FeatureGap>,
    hinge_limits: Vec<HingeLimit>,
    triangles: usize,
    triangle_batches: Vec<Vec<Triangle>>,
    quads: usize,
    quad_batches: Vec<Vec<Quad>>,
    capsules: Vec<CapsuleRigid>,
    spheres: Vec<CapsuleRigid>,
    boxes: Vec<BoxRigid>,
    node_bases: Vec<NodeBase>,
    ctrl_offsets: Vec<Offset>,
    reverse_offsets: Vec<ReverseOffset>,
    soft_offsets: Vec<Offset>,
    stray_radius_batches: Vec<Vec<StrayRadius>>,
    skel_parents: Vec<f64>,
    static_node_count: f64,
    static_node_flags: Option<u32>,
    dynamic_node_flags: Option<u32>,
    goal_damped_spring_integrators: Vec<u32>,
    default_gravity_scale: f64,
    has_collision_friction: bool,
    extra_iterations: f64,
    extra_goal_iterations: f64,
    twists: Vec<Twist>,
    fit_matrices: Vec<FitMatrix>,
    fit_weights: Vec<FitWeight>,
    lock_to_goal: Vec<f64>,
    collision_planes: Vec<CollisionPlane>,
    rope_count: usize,
    rope_chains: Vec<Vec<usize>>,
    jiggle_bones: Vec<JiggleBone>,
    kelager_bends: Vec<KelagerBend>,
    first_position_driven_node: f64,
    rot_lock_static_node_count: f64,
}

/// Grimoire's `num`: a finite JSON number, or the default.
fn num(v: Option<&Value>, d: f64) -> f64 {
    v.and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(d)
}
fn at(v: Option<&Value>, i: usize) -> Option<&Value> {
    v.and_then(|v| v.get(i))
}
fn vec3(a: Option<&Value>) -> V3 {
    [num(at(a, 0), 0.0), num(at(a, 1), 0.0), num(at(a, 2), 0.0)]
}
fn vec4(a: Option<&Value>, w: f64) -> [f64; 4] {
    [num(at(a, 0), 0.0), num(at(a, 1), 0.0), num(at(a, 2), 0.0), num(at(a, 3), w)]
}
fn sphere4(a: Option<&Value>) -> [f64; 4] {
    vec4(a, 0.0)
}
/// Rotation of an `[x, y, z, 1, qx, qy, qz, qw]` CTransform.
fn transform_rot(a: Option<&Value>) -> Q4 {
    [num(at(a, 4), 0.0), num(at(a, 5), 0.0), num(at(a, 6), 0.0), num(at(a, 7), 1.0)]
}
fn finite_num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite())
}
fn is_uint32(v: Option<&Value>) -> Option<u32> {
    finite_num(v).filter(|x| x.fract() == 0.0 && *x >= 0.0 && *x <= 4_294_967_295.0).map(|x| x as u32)
}
fn node_index(v: Option<&Value>, n: usize) -> Option<usize> {
    finite_num(v).and_then(|x| idx(x, n))
}
/// `Value.get(key)` that treats a JSON null like an absent key (`??` semantics).
fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| !x.is_null())
}
/// The records of an array field (absent or null: none).
fn records<'a>(fe: &'a Value, key: &str) -> &'a [Value] {
    field(fe, key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}
/// `Array.isArray(n) ? n.flat() : []`.
fn flat(v: Option<&Value>) -> Vec<&Value> {
    let mut out = Vec::new();
    if let Some(items) = v.and_then(Value::as_array) {
        for item in items {
            match item.as_array() {
                Some(inner) => out.extend(inner.iter()),
                None => out.push(item),
            }
        }
    }
    out
}
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

const RECORD_ARRAY_FIELDS: [&str; 27] = [
    "m_NodeIntegrator",
    "m_Rods",
    "m_SimdRods",
    "m_SimdRodsAnim",
    "m_NodeBases",
    "m_CtrlOffsets",
    "m_ReverseOffsets",
    "m_CtrlSoftOffsets",
    "m_TaperedCapsuleRigids",
    "m_SphereRigids",
    "m_AnimStrayRadii",
    "m_SimdAnimStrayRadii",
    "m_BoxRigids",
    "m_Twists",
    "m_FitMatrices",
    "m_FitWeights",
    "m_LockToParent",
    "m_CollisionPlanes",
    "m_JiggleBones",
    "m_KelagerBends",
    "m_HingeLimits",
    "m_Tris",
    "m_SimdTris",
    "m_Quads",
    "m_SimdQuads",
    "m_RigidColliderPriorities",
    "m_VertexMaps",
];

const SCALAR_ARRAY_FIELDS: [&str; 8] =
    ["m_SkelParents", "m_NodeInvMasses", "m_NodeCollisionRadii", "m_DynNodeFriction", "m_TreeCollisionMasks", "m_FreeNodes", "m_LockToGoal", "m_VertexMapValues"];

const PRIORITY_FIELDS: [(&str, &str); 5] = [
    ("m_nTaperedCapsuleRigidIndex", "m_TaperedCapsuleRigids"),
    ("m_nSphereRigidIndex", "m_SphereRigids"),
    ("m_nBoxRigidIndex", "m_BoxRigids"),
    ("m_nSDFRigidIndex", "m_SDFRigids"),
    ("m_nCollisionPlaneIndex", "m_CollisionPlanes"),
];

struct Parser<'a> {
    fe: &'a Value,
    n: usize,
    issues: Vec<DecodeIssue>,
}

impl Parser<'_> {
    fn issue(&mut self, array: &'static str, record: usize, reason: &'static str) {
        self.issues.push(DecodeIssue { array, record, reason });
    }

    fn rope_chains(&mut self) -> Vec<Vec<usize>> {
        let packed = field(self.fe, "m_Ropes");
        let count_v = field(self.fe, "m_nRopeCount");
        let count = match count_v {
            None => Some(0),
            some => is_uint32(some),
        };
        let arr = match packed {
            None => Some(&[][..]),
            Some(v) => v.as_array().map(Vec::as_slice),
        };
        let (Some(arr), Some(count)) = (arr, count) else {
            self.issue("m_Ropes", 0, "invalid-count");
            return Vec::new();
        };
        let count = count as usize;
        if count > arr.len() || (count == 0 && !arr.is_empty()) {
            self.issue("m_Ropes", 0, "invalid-count");
            return Vec::new();
        }
        let mut chains = Vec::new();
        let mut begin = count;
        for record in 0..count {
            let end = is_uint32(arr.get(record)).map(|e| e as usize);
            let Some(end) = end.filter(|&e| e >= begin + 2 && e <= arr.len() && (record + 1 != count || e == arr.len())) else {
                self.issue("m_Ropes", record, "invalid-offsets");
                return Vec::new();
            };
            let chain: Option<Vec<usize>> = arr[begin..end].iter().map(|v| node_index(Some(v), self.n)).collect();
            match chain {
                Some(c) => chains.push(c),
                None => self.issue("m_Ropes", record, "invalid-nodes"),
            }
            begin = end;
        }
        chains
    }

    fn collider_priorities(&mut self) -> Vec<[f64; 5]> {
        let rows = records(self.fe, "m_RigidColliderPriorities");
        if rows.is_empty() {
            return Vec::new();
        }
        let counts: Vec<f64> = PRIORITY_FIELDS.iter().map(|(_, arr)| records(self.fe, arr).len() as f64).collect();
        let mut out: Vec<[f64; 5]> = Vec::with_capacity(rows.len());
        for (record, row) in rows.iter().enumerate() {
            let mut values = [0.0; 5];
            let mut valid = rows.len() >= 2;
            for (k, (key, _)) in PRIORITY_FIELDS.iter().enumerate() {
                if !valid {
                    break;
                }
                let value = finite_num(row.get(*key)).filter(|v| v.fract() == 0.0);
                valid = match value {
                    Some(v) => {
                        let prev_ok = if record > 0 { out.get(record - 1).is_some_and(|p| v >= p[k]) } else { v == 0.0 };
                        v >= 0.0 && v <= counts[k] && prev_ok && (record + 1 < rows.len() || v == counts[k])
                    }
                    None => false,
                };
                values[k] = value.unwrap_or(0.0);
            }
            if !valid {
                self.issue("m_RigidColliderPriorities", record, "invalid-offsets");
                return Vec::new();
            }
            out.push(values);
        }
        out
    }

    fn vertex_maps(&mut self) -> Vec<Vec<usize>> {
        let values = records(self.fe, "m_VertexMapValues");
        let maps = records(self.fe, "m_VertexMaps");
        let mut out = Vec::with_capacity(maps.len());
        for (record, map) in maps.iter().enumerate() {
            let int = |k: &str| finite_num(map.get(k)).filter(|v| v.fract() == 0.0);
            let (base, count, offset) = (int("nVertexBase"), int("nVertexCount"), int("nMapOffset"));
            let (Some(base), Some(count), Some(offset)) = (base, count, offset) else {
                self.issue("m_VertexMaps", record, "invalid-offsets");
                out.push(Vec::new());
                continue;
            };
            if base < 0.0 || count < 0.0 || offset < 0.0 || offset + count > values.len() as f64 || (count > 0.0 && base + count > self.n as f64) {
                self.issue("m_VertexMaps", record, "invalid-offsets");
                out.push(Vec::new());
                continue;
            }
            let mut nodes = Vec::new();
            let mut bad = false;
            for i in 0..count as usize {
                let weight = finite_num(values.get(offset as usize + i)).filter(|w| w.fract() == 0.0 && *w >= 0.0 && *w <= 255.0);
                match weight {
                    Some(w) => {
                        if w > 0.0 {
                            nodes.push(base as usize + i);
                        }
                    }
                    None => {
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                self.issue("m_VertexMaps", record, "invalid-weights");
                out.push(Vec::new());
            } else {
                out.push(nodes);
            }
        }
        out
    }

    fn animated_rods(&mut self) -> (usize, Vec<Vec<Rod>>) {
        let mut flat_rods: Vec<Rod> = Vec::new();
        let mut batches = Vec::new();
        for (record, entry) in records(self.fe, "m_SimdRodsAnim").iter().enumerate() {
            let indices = flat(entry.get("nNode"));
            if indices.len() != 8 {
                self.issue("m_SimdRodsAnim", record, "invalid-nodes");
                continue;
            }
            let mut batch = Vec::new();
            for lane in 0..4 {
                let (a, b) = (node_index(Some(indices[lane]), self.n), node_index(Some(indices[4 + lane]), self.n));
                let (Some(a), Some(b)) = (a, b) else {
                    self.issue("m_SimdRodsAnim", record, "invalid-nodes");
                    continue;
                };
                if a == b {
                    continue;
                }
                let lane_value = |key: &str, d: f64| -> Option<f64> {
                    match at(field(entry, key), lane).filter(|v| !v.is_null()) {
                        None => Some(d),
                        some => finite_num(some),
                    }
                };
                let (Some(weight), Some(relax)) = (lane_value("f4Weight0", 0.5), lane_value("f4RelaxationFactor", 1.0)) else {
                    self.issue("m_SimdRodsAnim", record, "invalid-weights");
                    continue;
                };
                let rod = Rod { a, b, min: 0.0, max: 0.0, relax, weight };
                batch.push(rod);
                // The flat list is for coverage; solving retains every lane and batch.
                if !flat_rods.contains(&rod) {
                    flat_rods.push(rod);
                }
            }
            if !batch.is_empty() {
                batches.push(batch);
            }
        }
        (flat_rods.len(), batches)
    }

    fn rod_batches(&mut self) -> Vec<Vec<Rod>> {
        let mut batches = Vec::new();
        let mut invalid = false;
        for (record, entry) in records(self.fe, "m_SimdRods").iter().enumerate() {
            let indices = flat(entry.get("nNode"));
            let nodes: Option<Vec<usize>> = indices.iter().map(|v| node_index(Some(v), self.n)).collect();
            let Some(nodes) = nodes.filter(|n| n.len() == 8) else {
                self.issue("m_SimdRods", record, "invalid-nodes");
                invalid = true;
                continue;
            };
            let mut batch = Vec::new();
            for lane in 0..4 {
                let get = |k: &str| finite_num(at(entry.get(k), lane));
                let (min, max) = (get("f4MinDist"), get("f4MaxDist"));
                let (Some(min), Some(max)) = (min, max) else {
                    self.issue("m_SimdRods", record, "invalid-limits");
                    invalid = true;
                    break;
                };
                if min < 0.0 || max < min {
                    self.issue("m_SimdRods", record, "invalid-limits");
                    invalid = true;
                    break;
                }
                let (Some(weight), Some(relax)) = (get("f4Weight0"), get("f4RelaxationFactor")) else {
                    self.issue("m_SimdRods", record, "invalid-weights");
                    invalid = true;
                    break;
                };
                batch.push(Rod { a: nodes[lane], b: nodes[lane + 4], min, max, weight, relax });
            }
            batches.push(batch);
        }
        // Keep a complete scalar fallback if any packed record is malformed. Never
        // deduplicate valid batches: repeated rods can encode authored extra passes.
        if invalid { Vec::new() } else { batches }
    }

    fn stray(&mut self, nodes: [Option<&Value>; 2], max_dist: Option<&Value>, relax: Option<&Value>, array: &'static str, record: usize) -> Option<StrayRadius> {
        let (Some(a), Some(b)) = (node_index(nodes[0], self.n), node_index(nodes[1], self.n)) else {
            self.issue(array, record, "invalid-nodes");
            return None;
        };
        match (finite_num(max_dist), finite_num(relax)) {
            (Some(m), Some(r)) if m >= 0.0 && (0.0..=1.0).contains(&r) => Some(StrayRadius { node: [a, b], max_dist: m, relax: r }),
            _ => {
                self.issue(array, record, "invalid-limits");
                None
            }
        }
    }

    fn stray_radii(&mut self) -> Vec<Vec<StrayRadius>> {
        let mut radii = Vec::new();
        for (record, entry) in records(self.fe, "m_AnimStrayRadii").iter().enumerate() {
            let nodes = entry.get("nNode").and_then(Value::as_array);
            let pair = match nodes {
                Some(n) if n.len() == 2 => [n.first(), n.get(1)],
                _ => [None, None],
            };
            if let Some(r) = self.stray(pair, entry.get("flMaxDist"), entry.get("flRelaxationFactor"), "m_AnimStrayRadii", record) {
                radii.push(r);
            }
        }
        let mut batches = Vec::new();
        let mut invalid = false;
        for (record, entry) in records(self.fe, "m_SimdAnimStrayRadii").iter().enumerate() {
            let indices = flat(entry.get("nNode"));
            if indices.len() != 8 {
                self.issue("m_SimdAnimStrayRadii", record, "invalid-nodes");
                invalid = true;
                continue;
            }
            let mut batch = Vec::new();
            for lane in 0..4 {
                let r = self.stray(
                    [Some(indices[lane]), Some(indices[lane + 4])],
                    at(entry.get("flMaxDist"), lane),
                    at(entry.get("flRelaxationFactor"), lane),
                    "m_SimdAnimStrayRadii",
                    record,
                );
                match r {
                    Some(r) => batch.push(r),
                    None => invalid = true,
                }
            }
            batches.push(batch);
        }
        if invalid || batches.is_empty() {
            return radii.into_iter().map(|r| vec![r]).collect();
        }
        batches
    }

    fn partitions(&mut self, count: usize, one: &str, two: &str, array: &'static str) -> Option<Vec<usize>> {
        if count == 0 {
            return Some(Vec::new());
        }
        let (Some(one), Some(two)) = (is_uint32(field(self.fe, one)), is_uint32(field(self.fe, two))) else {
            self.issue(array, 0, "invalid-count");
            return None;
        };
        let (one, two) = (one as usize, two as usize);
        if two > one || one > count {
            self.issue(array, 0, "invalid-count");
            return None;
        }
        Some((0..count).map(|i| if i < two { 2 } else if i < one { 1 } else { 0 }).collect())
    }

    fn distinct_nodes<const K: usize>(&self, n: &[Option<&Value>]) -> Option<[usize; K]> {
        if n.len() != K {
            return None;
        }
        let mut out = [0usize; K];
        for (i, v) in n.iter().enumerate() {
            out[i] = node_index(*v, self.n)?;
        }
        for i in 0..K {
            for j in i + 1..K {
                if out[i] == out[j] {
                    return None;
                }
            }
        }
        Some(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn triangle(&mut self, n: &[Option<&Value>], w1: Option<&Value>, w2: Option<&Value>, v1x: Option<&Value>, x2: Option<&Value>, y2: Option<&Value>, static_count: usize, array: &'static str, record: usize) -> Option<Triangle> {
        let Some(node) = self.distinct_nodes::<3>(n) else {
            self.issue(array, record, "invalid-nodes");
            return None;
        };
        let (w1, w2) = match (finite_num(w1), finite_num(w2)) {
            (Some(a), Some(b)) if a >= 0.0 && b >= 0.0 && a + b <= 1.0 + 1e-6 => (a, b),
            _ => {
                self.issue(array, record, "invalid-weights");
                return None;
            }
        };
        match (finite_num(v1x), finite_num(x2), finite_num(y2)) {
            (Some(x1), Some(x2), Some(y2)) if x1 >= 0.0 && y2 >= 0.0 => Some(Triangle { node, static_count, weight1: w1, weight2: w2, x1, x2, y2 }),
            _ => {
                self.issue(array, record, "invalid-limits");
                None
            }
        }
    }

    fn triangles(&mut self) -> (usize, Vec<Vec<Triangle>>) {
        let scalar = records(self.fe, "m_Tris");
        let mut triangles = Vec::new();
        if let Some(parts) = self.partitions(scalar.len(), "m_nTriCount1", "m_nTriCount2", "m_Tris") {
            for (record, fixed) in parts.into_iter().enumerate() {
                let e = &scalar[record];
                let n: Vec<Option<&Value>> = match e.get("nNode").and_then(Value::as_array) {
                    Some(a) => a.iter().map(Some).collect(),
                    None => Vec::new(),
                };
                let v2 = e.get("v2");
                if let Some(t) = self.triangle(&n, e.get("w1"), e.get("w2"), e.get("v1x"), at(v2, 0), at(v2, 1), fixed, "m_Tris", record) {
                    triangles.push(t);
                }
            }
        }
        let packed = records(self.fe, "m_SimdTris");
        let parts = self.partitions(packed.len(), "m_nSimdTriCount1", "m_nSimdTriCount2", "m_SimdTris");
        let mut invalid = parts.is_none();
        let mut batches = Vec::new();
        for (record, fixed) in parts.unwrap_or_default().into_iter().enumerate() {
            let e = &packed[record];
            let indices = flat(e.get("nNode"));
            if indices.len() != 12 {
                self.issue("m_SimdTris", record, "invalid-nodes");
                invalid = true;
                continue;
            }
            let v2 = e.get("v2");
            let has_v2 = truthy(v2.and_then(|v| v.get("x"))) && truthy(v2.and_then(|v| v.get("y")));
            let mut batch = Vec::new();
            for lane in 0..4 {
                let n = [Some(indices[lane]), Some(indices[lane + 4]), Some(indices[lane + 8])];
                let (x2, y2) = if has_v2 { (at(v2.and_then(|v| v.get("x")), lane), at(v2.and_then(|v| v.get("y")), lane)) } else { (None, None) };
                match self.triangle(&n, at(e.get("w1"), lane), at(e.get("w2"), lane), at(e.get("v1x"), lane), x2, y2, fixed, "m_SimdTris", record) {
                    Some(t) => batch.push(t),
                    None => invalid = true,
                }
            }
            batches.push(batch);
        }
        // A malformed packed block uses the validated scalar sequence. Valid blocks retain
        // padding and cross-batch repeats, just like compiled rod batches.
        if invalid || batches.is_empty() {
            return (triangles.len(), triangles.into_iter().map(|t| vec![t]).collect());
        }
        let count = if triangles.is_empty() { dedup_count(batches.iter().flatten()) } else { triangles.len() };
        (count, batches)
    }

    fn quad(&mut self, n: &[Option<&Value>], slack: Option<f64>, shape: Option<[[f64; 4]; 4]>, static_count: usize, array: &'static str, record: usize) -> Option<Quad> {
        let Some(node) = self.distinct_nodes::<4>(n) else {
            self.issue(array, record, "invalid-nodes");
            return None;
        };
        let (Some(shape), Some(slack)) = (shape, slack.filter(|s| *s >= 0.0)) else {
            self.issue(array, record, "invalid-limits");
            return None;
        };
        let bad = shape.iter().enumerate().any(|(i, v)| v[3] < 0.0 || (i < static_count && v[3] != 0.0)) || (shape.iter().map(|v| v[3]).sum::<f64>() - 1.0).abs() > 1e-5;
        if bad {
            self.issue(array, record, "invalid-weights");
            return None;
        }
        Some(Quad { node, static_count, shape, slack })
    }

    fn quads(&mut self) -> (usize, Vec<Vec<Quad>>) {
        let scalar = records(self.fe, "m_Quads");
        let mut quads = Vec::new();
        if let Some(parts) = self.partitions(scalar.len(), "m_nQuadCount1", "m_nQuadCount2", "m_Quads") {
            for (record, fixed) in parts.into_iter().enumerate() {
                let e = &scalar[record];
                let n: Vec<Option<&Value>> = match e.get("nNode").and_then(Value::as_array) {
                    Some(a) => a.iter().map(Some).collect(),
                    None => Vec::new(),
                };
                let shape = e.get("vShape").and_then(Value::as_array).filter(|s| s.len() == 4).and_then(|s| {
                    let mut out = [[0.0; 4]; 4];
                    for (i, row) in s.iter().enumerate() {
                        let row = row.as_array().filter(|r| r.len() == 4)?;
                        for (j, v) in row.iter().enumerate() {
                            out[i][j] = finite_num(Some(v))?;
                        }
                    }
                    Some(out)
                });
                if let Some(q) = self.quad(&n, finite_num(e.get("flSlack")), shape, fixed, "m_Quads", record) {
                    quads.push(q);
                }
            }
        }
        let packed = records(self.fe, "m_SimdQuads");
        let parts = self.partitions(packed.len(), "m_nSimdQuadCount1", "m_nSimdQuadCount2", "m_SimdQuads");
        let mut invalid = parts.is_none();
        let mut batches = Vec::new();
        for (record, fixed) in parts.unwrap_or_default().into_iter().enumerate() {
            let e = &packed[record];
            let indices = flat(e.get("nNode"));
            if indices.len() != 16 {
                self.issue("m_SimdQuads", record, "invalid-nodes");
                invalid = true;
                continue;
            }
            let mut batch = Vec::new();
            for lane in 0..4 {
                let n = [Some(indices[lane]), Some(indices[lane + 4]), Some(indices[lane + 8]), Some(indices[lane + 12])];
                let get = |v: Option<&Value>| -> Option<f64> {
                    match v.filter(|x| !x.is_null()) {
                        None => Some(f64::NAN),
                        some => finite_num(some),
                    }
                };
                let mut shape = Some([[0.0; 4]; 4]);
                for vertex in 0..4 {
                    let row = at(e.get("vShape"), vertex);
                    let vals = [get(at(row, lane)), get(at(row, lane + 4)), get(at(row, lane + 8)), get(at(at(e.get("f4Weights"), vertex), lane))];
                    for (k, v) in vals.into_iter().enumerate() {
                        match (v.filter(|x| x.is_finite()), shape.as_mut()) {
                            (Some(x), Some(s)) => s[vertex][k] = x,
                            _ => shape = None,
                        }
                    }
                }
                let slack = finite_num(at(e.get("f4Slack"), lane));
                match self.quad(&n, slack, shape, fixed, "m_SimdQuads", record) {
                    Some(q) => batch.push(q),
                    None => invalid = true,
                }
            }
            batches.push(batch);
        }
        if invalid || batches.is_empty() {
            return (quads.len(), quads.into_iter().map(|q| vec![q]).collect());
        }
        let count = if quads.is_empty() { dedup_count(batches.iter().flatten()) } else { quads.len() };
        (count, batches)
    }

    fn kelager_bends(&mut self) -> Vec<KelagerBend> {
        let mut bends = Vec::new();
        for (record, bend) in records(self.fe, "m_KelagerBends").iter().enumerate() {
            let nodes = bend.get("nNode").and_then(Value::as_array).filter(|a| a.len() == 3).and_then(|a| {
                let mut out = [0usize; 3];
                for (i, v) in a.iter().enumerate() {
                    out[i] = node_index(Some(v), self.n)?;
                }
                Some(out)
            });
            let Some(node) = nodes else {
                self.issue("m_KelagerBends", record, "invalid-nodes");
                continue;
            };
            let weights = bend.get("flWeight").and_then(Value::as_array).filter(|a| a.len() == 3).and_then(|a| {
                let mut out = [0.0; 3];
                for (i, v) in a.iter().enumerate() {
                    out[i] = finite_num(Some(v))?;
                }
                Some(out)
            });
            let Some(weight) = weights else {
                self.issue("m_KelagerBends", record, "invalid-weights");
                continue;
            };
            let Some(height0) = finite_num(bend.get("flHeight0")).filter(|h| *h >= 0.0) else {
                self.issue("m_KelagerBends", record, "invalid-height");
                continue;
            };
            bends.push(KelagerBend { height0, node, weight });
        }
        bends
    }

    fn hinge_limits(&mut self) -> Vec<HingeLimit> {
        let mut hinges = Vec::new();
        for (record, h) in records(self.fe, "m_HingeLimits").iter().enumerate() {
            let nodes = h.get("nNode").and_then(Value::as_array).filter(|a| a.len() == 6).and_then(|a| {
                let mut out = [0usize; 6];
                for (i, v) in a.iter().enumerate() {
                    out[i] = node_index(Some(v), self.n)?;
                }
                Some(out)
            });
            let Some(node) = nodes else {
                self.issue("m_HingeLimits", record, "invalid-nodes");
                continue;
            };
            let flags_zero = match field(h, "nFlags") {
                None => true,
                Some(v) => v.as_f64() == Some(0.0),
            };
            if !flags_zero {
                self.issue("m_HingeLimits", record, "unsupported-flags");
                continue;
            }
            let (w4, w5) = match (finite_num(h.get("flWeight4")), finite_num(h.get("flWeight5"))) {
                (Some(a), Some(b)) if (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b) => (a, b),
                _ => {
                    self.issue("m_HingeLimits", record, "invalid-weights");
                    continue;
                }
            };
            match (finite_num(h.get("flAngleCenter")), finite_num(h.get("flAngleExtents"))) {
                (Some(center), Some(extents)) if (0.0..=std::f64::consts::PI).contains(&extents) => {
                    hinges.push(HingeLimit { node, weight4: w4, weight5: w5, center, extents });
                }
                _ => self.issue("m_HingeLimits", record, "invalid-limits"),
            }
        }
        hinges
    }

    fn twists(&mut self) -> Vec<Twist> {
        let mut out = Vec::new();
        for (record, t) in records(self.fe, "m_Twists").iter().enumerate() {
            let (Some(o), Some(e)) = (node_index(t.get("nNodeOrient"), self.n), node_index(t.get("nNodeEnd"), self.n)) else {
                self.issue("m_Twists", record, "invalid-nodes");
                continue;
            };
            match (finite_num(t.get("flTwistRelax")), finite_num(t.get("flSwingRelax"))) {
                (Some(tw), Some(sw)) if (0.0..=1.0).contains(&tw) && (0.0..=1.0).contains(&sw) => {
                    out.push(Twist { node_orient: o, node_end: e, twist_relax: tw, swing_relax: sw });
                }
                _ => self.issue("m_Twists", record, "invalid-weights"),
            }
        }
        out
    }
}

fn dedup_count<'a, T: PartialEq + 'a>(items: impl Iterator<Item = &'a T>) -> usize {
    let mut seen: Vec<&T> = Vec::new();
    for item in items {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }
    seen.len()
}

fn feature_gaps(fe: &Value) -> Vec<FeatureGap> {
    let mut gaps = Vec::new();
    let mut add = |key: &str, label: &'static str, approximate: bool| {
        if let Some(a) = fe.get(key).and_then(Value::as_array).filter(|a| !a.is_empty()) {
            gaps.push(FeatureGap { field: key.to_string(), label, count: a.len(), approximate });
        }
    };
    add("m_AxialEdges", "Axial edges", false);
    add("m_FollowNodes", "Follow links", false);
    add("m_SDFRigids", "SDF colliders", false);
    add("m_JiggleBones", "Jiggle bones", false);
    add("m_FitMatrices", "Fit matrices", true);
    for key in ["m_TaperedCapsuleRigids", "m_SphereRigids", "m_BoxRigids"] {
        let flags = records(fe, key).iter().filter(|e| e.get("nFlags").is_some_and(|f| f.as_f64() != Some(0.0))).count();
        if flags > 0 {
            gaps.push(FeatureGap { field: format!("{key}.nFlags"), label: "Collider flags", count: flags, approximate: false });
        }
    }
    gaps
}

/// The structural guards: a container that is not what it claims rejects the whole sidecar.
fn structurally_valid(fe: &Value) -> bool {
    let Some(names) = fe.get("m_CtrlName").and_then(Value::as_array) else { return false };
    if names.len() > 16384 || !names.iter().all(Value::is_string) {
        return false;
    }
    for key in RECORD_ARRAY_FIELDS {
        if let Some(v) = field(fe, key) {
            match v.as_array() {
                Some(a) if a.len() <= 262_144 && a.iter().all(Value::is_object) => {}
                _ => return false,
            }
        }
    }
    for key in SCALAR_ARRAY_FIELDS {
        if let Some(v) = field(fe, key) {
            match v.as_array() {
                Some(a) if a.len() <= 262_144 => {}
                _ => return false,
            }
        }
    }
    if let Some(v) = field(fe, "m_InitPose") {
        match v.as_array() {
            Some(a) if a.len() <= 16384 && a.iter().all(Value::is_array) => {}
            _ => return false,
        }
    }
    true
}

/// Parses the raw FeModel JSON. `None` for a non-FeModel payload or a malformed container, so
/// the viewer keeps playing animation without cloth.
fn parse_fe_model(fe: &Value) -> Option<FeModel> {
    if !structurally_valid(fe) {
        return None;
    }
    let names = fe.get("m_CtrlName").and_then(Value::as_array)?;
    let n = names.len();
    let mut p = Parser { fe, n, issues: Vec::new() };
    let inv = records(fe, "m_NodeInvMasses");
    let integ = records(fe, "m_NodeIntegrator");
    let pose = records(fe, "m_InitPose");
    let radii = records(fe, "m_NodeCollisionRadii");
    let friction = records(fe, "m_DynNodeFriction");
    let tree_masks = records(fe, "m_TreeCollisionMasks");

    // Per-node masks live in the collision BVH leaves [0, D); fold only when the leaf layout
    // matches (len == 2 * D - 1), else collide with everything.
    let dynamic_count = inv.iter().filter(|v| num(Some(v), 0.0) > 0.0).count();
    let fold_masks = dynamic_count > 0 && tree_masks.len() == 2 * dynamic_count - 1;

    let mut nodes = Vec::with_capacity(n);
    let mut dyn_slot = 0usize;
    for (i, name) in names.iter().enumerate() {
        let inv_mass = num(inv.get(i), 0.0);
        let it = integ.get(i);
        let pose_i = pose.get(i);
        let (mut collide_radius, mut fric, mut mask) = (0.0, 0.0, 65535.0);
        if inv_mass > 0.0 {
            collide_radius = num(radii.get(dyn_slot), 0.0);
            fric = num(friction.get(dyn_slot), 0.0);
            if fold_masks {
                mask = num(tree_masks.get(dyn_slot), 65535.0);
            }
            dyn_slot += 1;
        }
        let get = |k: &str| num(it.and_then(|o| o.get(k)), 0.0);
        nodes.push(FeNode {
            name: name.as_str().unwrap_or_default().to_string(),
            inv_mass,
            pinned: inv_mass <= 0.0,
            gravity: get("flGravity"),
            damping: get("flPointDamping"),
            anim_force: get("flAnimationForceAttraction"),
            anim_vertex: get("flAnimationVertexAttraction"),
            init_pos: vec3(pose_i),
            init_rot: transform_rot(pose_i),
            collide_radius,
            friction: fric,
            collision_mask: mask,
        });
    }

    let mut rods = Vec::new();
    for (record, r) in records(fe, "m_Rods").iter().enumerate() {
        let pair = r.get("nNode").and_then(Value::as_array).filter(|a| a.len() == 2).and_then(|a| Some((node_index(a.first(), n)?, node_index(a.get(1), n)?)));
        let Some((a, b)) = pair else {
            p.issue("m_Rods", record, "invalid-nodes");
            continue;
        };
        rods.push(Rod {
            a,
            b,
            min: num(r.get("flMinDist"), 0.0),
            max: num(r.get("flMaxDist"), 0.0),
            relax: num(r.get("flRelaxationFactor"), 1.0),
            weight: num(r.get("flWeight0"), 0.0),
        });
    }

    let priorities = p.collider_priorities();
    let vertex_maps = p.vertex_maps();
    let priority = |k: usize, index: usize| -> usize {
        let mut rank = 0;
        let mut group = 1;
        while group < priorities.len() && index as f64 >= priorities[group][k] {
            rank = group;
            group += 1;
        }
        rank
    };
    let vertex_nodes = |v: Option<&Value>| -> Option<Vec<usize>> {
        let i = finite_num(v).and_then(|x| idx(x, vertex_maps.len()))?;
        let mut nodes = vertex_maps[i].clone();
        nodes.sort_unstable();
        Some(nodes)
    };
    let filter = |c: &Value, k: usize, index: usize| ColliderFilter { mask: num(c.get("nCollisionMask"), 0.0), priority: priority(k, index), vertex_nodes: vertex_nodes(c.get("nVertexMapIndex")) };
    let capsules = records(fe, "m_TaperedCapsuleRigids")
        .iter()
        .enumerate()
        .map(|(i, c)| CapsuleRigid {
            sphere0: sphere4(at(c.get("vSphere"), 0)),
            sphere1: sphere4(at(c.get("vSphere"), 1)),
            node: num(c.get("nNode"), 0.0),
            filter: filter(c, 0, i),
        })
        .collect();
    let spheres = records(fe, "m_SphereRigids")
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sphere = sphere4(s.get("vSphere"));
            CapsuleRigid { sphere0: sphere, sphere1: sphere, node: num(s.get("nNode"), 0.0), filter: filter(s, 1, i) }
        })
        .collect();
    let boxes = records(fe, "m_BoxRigids")
        .iter()
        .enumerate()
        .map(|(i, b)| BoxRigid {
            pos: vec3(b.get("tmFrame2")),
            rot: transform_rot(b.get("tmFrame2")),
            half_size: vec3(b.get("vSize")),
            node: num(b.get("nNode"), 0.0),
            filter: filter(b, 2, i),
        })
        .collect();

    let mut node_bases = Vec::new();
    for (record, b) in records(fe, "m_NodeBases").iter().enumerate() {
        let ids = ["nNode", "nNodeX0", "nNodeX1", "nNodeY0", "nNodeY1"].map(|k| node_index(b.get(k), n));
        let [Some(node), Some(x0), Some(x1), Some(y0), Some(y1)] = ids else {
            p.issue("m_NodeBases", record, "invalid-nodes");
            continue;
        };
        node_bases.push(NodeBase { node, x0, x1, y0, y1, q_adjust: vec4(b.get("qAdjust"), 1.0) });
    }
    let offsets = |key: &str| -> Vec<Offset> {
        records(fe, key)
            .iter()
            .map(|c| Offset { offset: vec3(c.get("vOffset")), parent: num(c.get("nCtrlParent"), 0.0), child: num(c.get("nCtrlChild"), 0.0), alpha: num(c.get("flAlpha"), 0.0) })
            .collect()
    };
    let ctrl_offsets = offsets("m_CtrlOffsets");
    let soft_offsets = offsets("m_CtrlSoftOffsets");
    let reverse_offsets = records(fe, "m_ReverseOffsets")
        .iter()
        .map(|r| ReverseOffset { offset: vec3(r.get("vOffset")), bone_ctrl: num(r.get("nBoneCtrl"), 0.0), target_node: num(r.get("nTargetNode"), 0.0) })
        .collect();
    let stray_radius_batches = p.stray_radii();
    let twists = p.twists();
    let fit_matrices = records(fe, "m_FitMatrices")
        .iter()
        .map(|m| FitMatrix {
            bone: vec3(m.get("bone")),
            bone_rot: transform_rot(m.get("bone")),
            center: vec3(m.get("vCenter")),
            end_weight: num(m.get("nEnd"), 0.0),
            node: num(m.get("nNode"), 0.0),
            begin_dynamic: num(m.get("nBeginDynamic"), 0.0),
            ctrl: num(m.get("nCtrl"), num(m.get("nNode"), 0.0)),
        })
        .collect();
    let fit_weights = records(fe, "m_FitWeights").iter().map(|w| FitWeight { weight: num(w.get("flWeight"), 0.0), node: num(w.get("nNode"), 0.0) }).collect();
    let collision_planes = records(fe, "m_CollisionPlanes")
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let plane = c.get("m_Plane");
            CollisionPlane {
                priority: priority(4, i),
                ctrl_parent: num(c.get("nCtrlParent"), 0.0),
                child_node: num(c.get("nChildNode"), 0.0),
                normal: vec3(plane.and_then(|p| p.get("m_vNormal"))),
                offset: num(plane.and_then(|p| p.get("m_flOffset")), 0.0),
                strength: num(c.get("flStrength"), 1.0),
            }
        })
        .collect();
    let jiggle_bones = records(fe, "m_JiggleBones").iter().map(|j| JiggleBone { node: num(j.get("m_nNode"), 0.0), has_params: j.get("m_jiggleBone").is_some_and(|p| p.is_object() || p.is_array()) }).collect();
    let (animated_rods, animated_rod_batches) = p.animated_rods();
    let rod_batches = p.rod_batches();
    let (triangles, triangle_batches) = p.triangles();
    let (quads, quad_batches) = p.quads();
    let kelager_bends = p.kelager_bends();
    let rope_chains = p.rope_chains();
    let bitset: Option<Vec<u32>> = match field(fe, "m_GoalDampedSpringIntegrators") {
        None => Some(Vec::new()),
        Some(v) => v.as_array().and_then(|a| a.iter().map(|x| is_uint32(Some(x))).collect()),
    };
    if bitset.is_none() {
        p.issue("m_GoalDampedSpringIntegrators", 0, "invalid-bitset");
    }
    let hinge_limits = p.hinge_limits();
    Some(FeModel {
        nodes,
        rods,
        rod_batches,
        animated_rods,
        animated_rod_batches,
        decode_issues: std::mem::take(&mut p.issues),
        feature_gaps: feature_gaps(fe),
        hinge_limits,
        triangles,
        triangle_batches,
        quads,
        quad_batches,
        capsules,
        spheres,
        boxes,
        node_bases,
        ctrl_offsets,
        reverse_offsets,
        soft_offsets,
        stray_radius_batches,
        skel_parents: records(fe, "m_SkelParents").iter().map(|v| num(Some(v), -1.0)).collect(),
        static_node_count: num(fe.get("m_nStaticNodes"), 0.0),
        static_node_flags: is_uint32(fe.get("m_nStaticNodeFlags")),
        dynamic_node_flags: is_uint32(fe.get("m_nDynamicNodeFlags")),
        goal_damped_spring_integrators: bitset.unwrap_or_default(),
        default_gravity_scale: num(fe.get("m_flDefaultGravityScale"), 1.0),
        has_collision_friction: !friction.is_empty(),
        extra_iterations: num(fe.get("m_nExtraIterations"), 0.0),
        extra_goal_iterations: num(fe.get("m_nExtraGoalIterations"), 0.0),
        twists,
        fit_matrices,
        fit_weights,
        lock_to_goal: records(fe, "m_LockToGoal").iter().map(|v| num(Some(v), 0.0)).collect(),
        collision_planes,
        rope_count: is_uint32(fe.get("m_nRopeCount")).unwrap_or(0) as usize,
        rope_chains,
        jiggle_bones,
        kelager_bends,
        first_position_driven_node: num(fe.get("m_nFirstPositionDrivenNode"), n as f64),
        rot_lock_static_node_count: num(fe.get("m_nRotLockStaticNodes"), 0.0),
    })
}

/// Grimoire's `clothIntegratorMode`.
fn integrator_mode(fe: &FeModel, node: usize) -> Mode {
    let dynamic_index = node as f64 - fe.static_node_count;
    if dynamic_index >= 0.0 && !fe.goal_damped_spring_integrators.is_empty() {
        let di = dynamic_index as u64;
        let Some(&word) = fe.goal_damped_spring_integrators.get((di >> 5) as usize) else { return Mode::Unknown };
        return if word & (1u32 << (di & 31)) != 0 { Mode::GoalDamped } else { Mode::Raw };
    }
    let flags = if dynamic_index >= 0.0 { fe.dynamic_node_flags } else { fe.static_node_flags };
    let Some(flags) = flags else { return Mode::Unknown };
    let raw = flags & 0x600 != 0;
    let goal = flags & 0x80 != 0;
    if !raw {
        Mode::GoalDamped
    } else if goal {
        // Mixed band flags without a node bitset cannot identify a node's mode.
        Mode::Unknown
    } else {
        Mode::Raw
    }
}

// ---------------------------------------------------------------------------------------------
// Solver nodes and constraint kernels (`clothConstraints.ts`, `useClothSim.ts`).

#[derive(Clone, Debug)]
struct Node {
    index: usize,
    /// The glb node this cloth node drives and reads, matched by exact joint name.
    bone: Option<usize>,
    inv_mass: f64,
    pinned: bool,
    position_driven: bool,
    lock_to_goal: bool,
    jiggle_driven: bool,
    kinematic: bool,
    generated_target: bool,
    mode: Mode,
    gravity: f64,
    damping: f64,
    anim_force: f64,
    anim_vertex: f64,
    collide_radius: f64,
    friction: f64,
    collision_mask: i32,
    init_pos: V3,
    /// As authored (not normalized), like Grimoire's `initRot`.
    init_rot: Q4,
    pos: V3,
    prev: V3,
    last_solved: V3,
    target: V3,
    solved_rot: Q4,
    target_rot: Q4,
}

impl Node {
    /// A free particle at `pos` (kernel tests).
    #[cfg(test)]
    fn free(index: usize, pos: V3) -> Node {
        Node {
            index,
            bone: None,
            inv_mass: 1.0,
            pinned: false,
            position_driven: false,
            lock_to_goal: false,
            jiggle_driven: false,
            kinematic: false,
            generated_target: false,
            mode: Mode::GoalDamped,
            gravity: 0.0,
            damping: 0.0,
            anim_force: 0.0,
            anim_vertex: 0.0,
            collide_radius: 0.0,
            friction: 0.0,
            collision_mask: 0xffff,
            init_pos: pos,
            init_rot: Q_ID,
            pos,
            prev: pos,
            last_solved: pos,
            target: pos,
            solved_rot: Q_ID,
            target_rot: Q_ID,
        }
    }

    /// Grimoire's `isKinematicNode`: driven by animation, never by the solver.
    fn flags_kinematic(&self) -> bool {
        self.pinned || self.position_driven || self.lock_to_goal || self.jiggle_driven
    }
}

fn goal_damped_attraction(pos: &mut V3, prev: &mut V3, goal: V3, force: f64, vertex: f64) {
    let attraction = unit(force);
    if attraction > 0.9999f32 as f64 {
        *pos = goal;
        *prev = goal;
        return;
    }
    if attraction >= EPS23 {
        *pos = lerp3(*pos, goal, attraction);
    }
    *prev = lerp3(*prev, *pos, unit(vertex));
}

fn raw_attraction(pos: &mut V3, prev: &mut V3, goal: V3, force: f64, vertex: f64, dt: f64) {
    let p = unit(vertex * dt);
    let f = if force.is_finite() { force.max(0.0) * dt * 2.0 } else { 0.0 };
    let delta = sub(goal, *pos);
    *prev = madd(*prev, delta, p * (1.0 - p));
    *pos = madd(*pos, delta, p + f);
}

/// The legacy preview attraction for nodes without a known integrator: a position blend in
/// both Verlet buffers and a velocity impulse in the current one.
fn animation_attraction(anim_vertex: f64, anim_force: f64, dt: f64) -> (f64, f64) {
    let v = if anim_vertex.is_finite() { anim_vertex.max(0.0) } else { 0.0 };
    let f = if anim_force.is_finite() { anim_force.max(0.0) } else { 0.0 };
    ((v * dt).min(1.0), f * dt * 2.0)
}

fn verlet_velocity_scale(dt: f64, last_dt: Option<f64>, damping: f64) -> f64 {
    if !dt.is_finite() || dt <= 0.0 {
        return 0.0;
    }
    let Some(last) = last_dt.filter(|l| l.is_finite() && *l > 0.0) else { return 0.0 };
    let damping_scale = (1.0 - if damping.is_finite() { damping } else { 0.0 }).max(0.0);
    damping_scale * (dt / (dt * MIN_TIMESTEP_HISTORY_RATIO).max(last))
}

/// Gathers every lane from the same pose, then scatters all first endpoints followed by all
/// second endpoints, as the compiled four-lane block does.
fn project_rod_batch(nodes: &mut [Node], rods: &[Rod]) {
    for chunk in rods.chunks(4) {
        let mut out_a = [[0.0; 3]; 4];
        let mut out_b = [[0.0; 3]; 4];
        for (lane, rod) in chunk.iter().enumerate() {
            let (a, b) = (nodes[rod.a].pos, nodes[rod.b].pos);
            let delta = sub(b, a);
            let distance = len_sq(delta).max(EPS30).sqrt();
            let wanted = clamp(distance, rod.min, rod.max);
            let delta = scl(delta, (distance - wanted) / distance * rod.relax);
            out_a[lane] = madd(a, delta, rod.weight);
            out_b[lane] = madd(b, delta, rod.weight - 1.0);
        }
        for (lane, rod) in chunk.iter().enumerate() {
            if !nodes[rod.a].kinematic {
                nodes[rod.a].pos = out_a[lane];
            }
        }
        for (lane, rod) in chunk.iter().enumerate() {
            if !nodes[rod.b].kinematic {
                nodes[rod.b].pos = out_b[lane];
            }
        }
    }
}

/// The scalar `m_Rods` fallback, used when no packed rod block decodes.
fn project_scalar_rods(nodes: &mut [Node], rods: &[Rod]) {
    for rod in rods {
        let delta = sub(nodes[rod.b].pos, nodes[rod.a].pos);
        let d = len(delta);
        if d < 1e-6 {
            continue;
        }
        let mut wanted = d;
        if rod.min > 0.0 && d < rod.min {
            wanted = rod.min;
        }
        if rod.max > 0.0 && d > rod.max {
            wanted = rod.max;
        }
        if wanted == d {
            continue;
        }
        let (share_a, share_b) = (rod.weight, 1.0 - rod.weight);
        if share_a <= 0.0 && share_b <= 0.0 {
            continue;
        }
        let correction = scl(delta, ((d - wanted) / d) * rod.relax);
        if share_a > 0.0 && !nodes[rod.a].kinematic {
            nodes[rod.a].pos = madd(nodes[rod.a].pos, correction, share_a);
        }
        if share_b > 0.0 && !nodes[rod.b].kinematic {
            nodes[rod.b].pos = madd(nodes[rod.b].pos, correction, -share_b);
        }
    }
}

fn triangle_projection(nodes: &[Node], t: &Triangle, scale: f64) -> [V3; 3] {
    let a = nodes[t.node[0]].pos;
    let b = nodes[t.node[1]].pos;
    let c = nodes[t.node[2]].pos;
    let mut tx = sub(b, a);
    let length = if len_sq(tx) > EPS23 { len(tx) } else { 1.0 };
    tx = if len_sq(tx) > EPS23 { scl(tx, 1.0 / length) } else { [1.0, 0.0, 0.0] };
    let mut ty = sub(c, a);
    let projection = dot(ty, tx);
    ty = madd(ty, tx, -projection);
    let height = if len_sq(ty) > EPS23 { len(ty) } else { 1.0 };
    ty = if len_sq(ty) > EPS23 { scl(ty, 1.0 / height) } else { [0.0, 1.0, 0.0] };
    let (x1, x2, y2, w1, w2) = (t.x1, t.x2, t.y2, t.weight1, t.weight2);
    if t.static_count == 2 {
        return [a, b, madd(madd(a, tx, ((length - x1) * 0.5 + x2) * scale), ty, y2 * scale)];
    }
    let s0 = t.static_count == 0;
    let center_x = if s0 { w1 * length + w2 * projection } else { 0.0 };
    let center_y = if s0 { w2 * height } else { 0.0 };
    let rest_x = if s0 { (w1 * x1 + w2 * x2) * scale } else { 0.0 };
    let rest_y = if s0 { w2 * y2 * scale } else { 0.0 };
    let (mut cosine, mut sine);
    if t.static_count == 1 {
        cosine = length * x1 * w1 + (height * y2 + projection * x2) * w2;
        sine = (height * x2 - projection * y2) * w2;
    } else {
        // Weighted 2D rigid fit in the current triangle plane, with compiled mass shares.
        let w0 = 1.0 - w1 - w2;
        let bx = x1 * scale - rest_x;
        let cx = x2 * scale - rest_x;
        let cy = y2 * scale - rest_y;
        cosine = w0 * (center_x * rest_x + center_y * rest_y) + w1 * ((length - center_x) * bx + center_y * rest_y) + w2 * ((projection - center_x) * cx + (height - center_y) * cy);
        sine = w0 * (center_y * rest_x - center_x * rest_y) + w1 * (-center_y * bx + (length - center_x) * rest_y) + w2 * ((height - center_y) * cx - (projection - center_x) * cy);
    }
    let squared = cosine * cosine + sine * sine;
    if squared > 1e-14 {
        let inverse = 1.0 / squared.sqrt();
        cosine *= inverse;
        sine *= inverse;
    } else {
        cosine = 1.0;
        sine = 0.0;
    }
    let origin = madd(madd(a, tx, center_x - (rest_x * cosine - rest_y * sine)), ty, center_y - (rest_x * sine + rest_y * cosine));
    [
        origin,
        madd(madd(origin, tx, x1 * scale * cosine), ty, x1 * scale * sine),
        madd(madd(origin, tx, (x2 * cosine - y2 * sine) * scale), ty, (x2 * sine + y2 * cosine) * scale),
    ]
}

fn project_triangle_batch(nodes: &mut [Node], triangles: &[Triangle], scale: f64) {
    for chunk in triangles.chunks(4) {
        let mut out = [[[0.0; 3]; 3]; 4];
        for (lane, t) in chunk.iter().enumerate() {
            out[lane] = triangle_projection(nodes, t, scale);
        }
        // Preserve compiled gather/scatter ordering even when lanes share a node.
        for vertex in 0..3 {
            for (lane, t) in chunk.iter().enumerate() {
                let node = &mut nodes[t.node[vertex]];
                if vertex >= t.static_count && !node.kinematic {
                    node.pos = out[lane][vertex];
                }
            }
        }
    }
}

/// Orthonormalizes the quad basis (three's `orthonormalizeQuadBasis`).
fn quad_basis(x: V3, y: V3) -> (V3, V3, V3) {
    let x = if len_sq(x) >= EPS23 { norm(x) } else { [1.0, 0.0, 0.0] };
    let mut y = madd(y, x, -dot(y, x));
    if len_sq(y) < EPS23 {
        y = if x[0].abs() > x[2].abs() { [-x[1], x[0], 0.0] } else { [0.0, -x[2], x[1]] };
    }
    let y = norm(y);
    (x, y, cross(x, y))
}

#[allow(clippy::needless_range_loop)] // Index loops mirror the compiled kernel.
fn anchored_quad_projection(nodes: &[Node], q: &Quad, scale: f64, out: &mut [V3; 4]) {
    let (a, b, c, d) = (nodes[q.node[0]].pos, nodes[q.node[1]].pos, nodes[q.node[2]].pos, nodes[q.node[3]].pos);
    let center = scl(add(a, b), 0.5);
    let (qx, qy, qz) = quad_basis(sub(b, a), madd(add(c, d), center, -2.0));
    // The two fixed corners define the axis and midpoint. Fit the movable corners' compiled
    // Y/Z shape about that axis using their mass shares.
    let (mut cosine, mut sine) = (0.0, 0.0);
    for vertex in 2..4 {
        let offset = sub(nodes[q.node[vertex]].pos, center);
        let (y, z) = (dot(offset, qy), dot(offset, qz));
        let [_, sy, sz, weight] = q.shape[vertex];
        cosine += weight * (sy * y + sz * z);
        sine += weight * (sz * y - sy * z);
    }
    let squared = cosine * cosine + sine * sine;
    if squared > EPS23 {
        let inverse = 1.0 / squared.sqrt();
        cosine *= inverse;
        sine *= inverse;
    } else {
        cosine = 1.0;
        sine = 0.0;
    }
    for vertex in 2..4 {
        let [x, y, z, _] = q.shape[vertex];
        out[vertex] = madd(madd(madd(center, qx, x * scale), qy, (y * cosine + z * sine) * scale), qz, (z * cosine - y * sine) * scale);
    }
}

fn movable_quad_projection(nodes: &[Node], q: &Quad, scale: f64, relaxation: f64, out: &mut [V3; 4]) {
    let fixed = q.static_count;
    let (qx, qy, qz) = quad_basis(sub(nodes[q.node[2]].pos, nodes[q.node[0]].pos), sub(nodes[q.node[3]].pos, nodes[q.node[1]].pos));
    let mut center = [0.0; 3];
    if fixed == 1 {
        center = nodes[q.node[0]].pos;
    } else {
        for i in 0..4 {
            center = madd(center, nodes[q.node[i]].pos, q.shape[i][3]);
        }
    }
    let (mut xx, mut yy, mut zz, mut xy, mut xz, mut yz) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let mut torque = [0.0; 3];
    let mut live = [[0.0; 3]; 4];
    let mut shape = [[0.0; 3]; 4];
    for i in fixed..4 {
        let [sx, sy, sz, weight] = q.shape[i];
        live[i] = sub(nodes[q.node[i]].pos, center);
        shape[i] = madd(madd(scl(qx, sx * scale), qy, sy * scale), qz, sz * scale);
        let [x, y, z] = live[i];
        xx += weight * (y * y + z * z);
        yy += weight * (x * x + z * z);
        zz += weight * (x * x + y * y);
        xy -= weight * x * y;
        xz -= weight * x * z;
        yz -= weight * y * z;
        torque = madd(torque, cross(live[i], shape[i]), weight);
    }
    // One linearized angular fit with the live inertia tensor. The compiled kernel masks a
    // singular Cholesky solve to zero angular correction.
    let inverse_x = 1.0 / xx.sqrt();
    let l10 = xy * inverse_x;
    let l20 = xz * inverse_x;
    let inverse_y = 1.0 / (yy - l10 * l10).sqrt();
    let l21 = (yz - l10 * l20) * inverse_y;
    let inverse_z = 1.0 / (zz - l20 * l20 - l21 * l21).sqrt();
    let mut rotation = [0.0; 3];
    if inverse_x + inverse_y + inverse_z < 1e7 {
        let mut x = torque[0] * inverse_x;
        let mut y = (torque[1] - l10 * x) * inverse_y;
        let z = (torque[2] - l20 * x - l21 * y) * inverse_z * inverse_z;
        y = (y - l21 * z) * inverse_y;
        x = (x - l20 * z - l10 * y) * inverse_x;
        rotation = [x, y, z];
    }
    for i in fixed..4 {
        let mut projected = sub(shape[i], cross(rotation, shape[i]));
        if fixed == 0 {
            projected = madd(scl(projected, relaxation), live[i], 1.0 - relaxation);
        }
        out[i] = add(projected, center);
    }
}

fn quad_projection(nodes: &[Node], q: &Quad, scale: f64, relaxation: f64) -> [V3; 4] {
    let mut out = [[0.0; 3]; 4];
    if q.static_count == 2 {
        anchored_quad_projection(nodes, q, scale, &mut out);
    } else {
        movable_quad_projection(nodes, q, scale, relaxation, &mut out);
    }
    out
}

fn project_quad_batch(nodes: &mut [Node], quads: &[Quad], scale: f64, relaxation: f64) {
    for chunk in quads.chunks(4) {
        let mut out = [[[0.0; 3]; 4]; 4];
        for (lane, q) in chunk.iter().enumerate() {
            out[lane] = quad_projection(nodes, q, scale, relaxation);
        }
        for vertex in 0..4 {
            for (lane, q) in chunk.iter().enumerate() {
                let node = &mut nodes[q.node[vertex]];
                if vertex >= q.static_count && !node.kinematic {
                    node.pos = out[lane][vertex];
                }
            }
        }
    }
}

fn project_kelager_bend(nodes: &mut [Node], bend: &KelagerBend) {
    let [a, b, c] = bend.node;
    let delta = scl(sub(sub(scl(nodes[a].pos, 2.0), nodes[b].pos), nodes[c].pos), 1.0 / 3.0);
    let height = len(delta);
    if height <= bend.height0 || height < 1e-12 {
        return;
    }
    let delta = scl(delta, 1.0 - bend.height0 / height);
    // Signed shares already include the compiler's mass weighting.
    for (k, &i) in bend.node.iter().enumerate() {
        if !nodes[i].kinematic {
            nodes[i].pos = madd(nodes[i].pos, delta, bend.weight[k]);
        }
    }
}

/// The hinge's angular error and its gradients for points `[a, b, reference, arm]`. Invalid
/// (collapsed) geometry gives `(0, false)`.
fn hinge_geometry(p: &[V3; 4], center: f64, g: &mut [V3; 4]) -> (f64, bool) {
    use std::f64::consts::PI;
    let axis_raw = sub(p[1], p[0]);
    let length = len(axis_raw);
    let axis = norm(axis_raw);
    let mut reference = sub(p[2], p[0]);
    let mut arm = sub(p[3], p[0]);
    let reference_height = dot(reference, axis);
    let arm_height = dot(arm, axis);
    reference = madd(reference, axis, -reference_height);
    arm = madd(arm, axis, -arm_height);
    let reference_radius = len(reference);
    let arm_radius = len(arm);
    if length < 0.01 || reference_radius < 0.01 || arm_radius < 0.01 {
        return (0.0, false);
    }
    reference = scl(reference, 1.0 / reference_radius);
    arm = scl(arm, 1.0 / arm_radius);
    let perpendicular = cross(axis, reference);
    let cosine = dot(reference, arm);
    let sine = dot(perpendicular, arm);
    let mut error = sine.atan2(cosine) - center;
    if error > PI {
        error -= PI * 2.0;
    } else if error < -PI {
        error += PI * 2.0;
    }
    g[2] = scl(perpendicular, -1.0 / reference_radius);
    g[3] = scl(madd(scl(perpendicular, cosine), reference, -sine), 1.0 / arm_radius);
    g[1] = madd(scl(g[2], -reference_height / length), g[3], -arm_height / length);
    g[0] = scl(add(add(g[1], g[2]), g[3]), -1.0);
    (error, true)
}

fn hinge_points(nodes: &[Node], h: &HingeLimit) -> [V3; 4] {
    let mut p = [nodes[h.node[0]].pos, nodes[h.node[1]].pos, nodes[h.node[2]].pos, nodes[h.node[3]].pos];
    p[2] = lerp3(p[2], nodes[h.node[4]].pos, h.weight4);
    p[3] = lerp3(p[3], nodes[h.node[5]].pos, h.weight5);
    p
}

fn project_hinge_limit(nodes: &mut [Node], h: &HingeLimit) {
    use std::f64::consts::PI;
    let mut points = hinge_points(nodes, h);
    let mut masses = [nodes[h.node[0]].inv_mass, nodes[h.node[1]].inv_mass, nodes[h.node[2]].inv_mass, nodes[h.node[3]].inv_mass];
    masses[2] *= 1.0 - h.weight4;
    masses[3] *= 1.0 - h.weight5;
    let mut grads = [[0.0; 3]; 4];
    let mut deltas = [[0.0; 3]; 4];
    let (mut error, mut valid) = hinge_geometry(&points, h.center, &mut grads);
    let tolerance = PI / 180.0;
    if !valid || error.abs() <= h.extents + tolerance {
        return;
    }
    let target = error.signum() * h.extents;
    for _ in 0..5 {
        if !valid {
            return;
        }
        let correction = target - error;
        let denominator: f64 = (0..4).map(|i| masses[i] * len_sq(grads[i])).sum();
        if denominator < EPS23 {
            return;
        }
        let scale = clamp(correction, -PI / 4.0, PI / 4.0) / denominator;
        for i in 0..4 {
            deltas[i] = scl(grads[i], masses[i] * scale);
            points[i] = add(points[i], deltas[i]);
        }
        if correction.abs() < PI / 45.0 {
            break;
        }
        (error, valid) = hinge_geometry(&points, h.center, &mut grads);
        if (error - target).abs() <= tolerance {
            break;
        }
    }
    for i in 0..4 {
        if masses[i] <= 0.0 {
            continue;
        }
        let weight = match i {
            2 => h.weight4,
            3 => h.weight5,
            _ => 0.0,
        };
        let n = h.node[i];
        if weight == 0.0 {
            if !nodes[n].kinematic {
                nodes[n].pos = points[i];
            }
        } else {
            // The compiled scatter uses the last correction for blended references.
            if !nodes[n].kinematic {
                nodes[n].pos = madd(nodes[n].pos, deltas[i], weight / (1.0 - weight));
            }
            let other = h.node[i + 2];
            if !nodes[other].kinematic {
                nodes[other].pos = madd(nodes[other].pos, deltas[i], (1.0 - weight) / weight);
            }
        }
    }
}

/// Clamps each particle within `max_dist` of its animation target. Every lane reads the
/// original positions; history is never changed.
fn project_stray_radius_batch(nodes: &mut [Node], radii: &[StrayRadius], scale: f64) -> bool {
    let mut changed = false;
    for chunk in radii.chunks(4) {
        let mut out = [[0.0; 3]; 4];
        let mut write = [false; 4];
        for (lane, s) in chunk.iter().enumerate() {
            let (Some(goal), Some(node)) = (nodes.get(s.node[0]), nodes.get(s.node[1])) else { continue };
            if node.flags_kinematic() || node.kinematic {
                continue;
            }
            if !s.max_dist.is_finite() || s.max_dist < 0.0 || !s.relax.is_finite() || s.relax <= 0.0 {
                continue;
            }
            let delta = sub(node.pos, goal.target);
            let distance = len_sq(delta).max(EPS30).sqrt();
            let correction = (distance.min(s.max_dist * scale) / distance - 1.0) * s.relax;
            out[lane] = madd(node.pos, delta, correction);
            write[lane] = true;
            changed |= correction != 0.0 && len_sq(delta) != 0.0;
        }
        for (lane, s) in chunk.iter().enumerate() {
            if write[lane] {
                nodes[s.node[1]].pos = out[lane];
            }
        }
    }
    changed
}

#[derive(Clone, Copy, Debug)]
struct PlaneRt {
    priority: usize,
    ctrl_parent: usize,
    child_node: usize,
    normal: V3,
    offset: f64,
    strength: f64,
}

fn project_collision_plane(nodes: &mut [Node], plane: &PlaneRt) -> bool {
    let (Some(parent), Some(child)) = (nodes.get(plane.ctrl_parent), nodes.get(plane.child_node)) else { return false };
    if child.flags_kinematic() || !finite3(plane.normal) || !plane.offset.is_finite() {
        return false;
    }
    let normal = q_rot(parent.solved_rot, plane.normal);
    let normal_length = len(normal);
    if !normal_length.is_finite() || normal_length < 1e-6 {
        return false;
    }
    let normal = scl(normal, 1.0 / normal_length);
    let strength = clamp(if plane.strength.is_finite() { plane.strength } else { 0.0 }, 0.0, 1.0);
    if strength <= 0.0 {
        return false;
    }
    let point = madd(parent.pos, normal, plane.offset);
    let signed = dot(sub(child.pos, point), normal);
    if signed >= 0.0 {
        return false;
    }
    let child = &mut nodes[plane.child_node];
    child.pos = madd(child.pos, normal, -signed * strength);
    true
}

/// Penetration depth of `p` (with particle radius `pr`) into a tapered capsule, and the push
/// normal. A sphere is a capsule with equal ends.
#[allow(clippy::too_many_arguments)]
fn capsule_depth(p: V3, a: V3, b: V3, ra: f64, rb: f64, pr: f64, friction: bool) -> (f64, V3) {
    let axis = sub(b, a);
    let length = len(axis);
    let t = if length < 1e-6 {
        if rb > ra { 1.0 } else { 0.0 }
    } else if length - (rb - ra) <= 0.5 {
        1.0
    } else {
        let axis = scl(axis, 1.0 / length);
        let radial = sub(p, a);
        let axial = dot(radial, axis);
        let radial = len(madd(radial, axis, -axial));
        // The tapered contact path shifts the sampled sphere toward the larger endpoint by
        // radius slope times radial distance before projecting out of it.
        clamp((axial + (rb - ra) / length * radial) / length, 0.0, 1.0)
    };
    let cp = lerp3(a, b, t);
    let r = ra + (rb - ra) * t + pr;
    let mut n = sub(p, cp);
    let distance_sq = len_sq(n);
    if r <= 0.0 || distance_sq >= r * r {
        return (0.0, n);
    }
    // Friction uses a full-radius correction from the current particle near the center. The
    // position-only kernel instead places it on the sphere's +Z pole.
    if friction && distance_sq < 0.01f32 as f64 {
        return (r, [0.0, 0.0, 1.0]);
    }
    if !friction && distance_sq <= EPS23 {
        n = scl(n, -1.0);
        n[2] += r;
        let correction = len(n);
        return (correction, scl(n, 1.0 / correction));
    }
    let d = distance_sq.sqrt();
    (r - d, scl(n, 1.0 / d))
}

#[allow(clippy::nonminimal_bool)] // Kept as the runtime writes the unique-face test.
fn box_depth(p: V3, center: V3, rotation: Q4, half: V3, pr: f64, friction: bool) -> (f64, V3) {
    let (hx, hy, hz) = (half[0].max(0.0), half[1].max(0.0), half[2].max(0.0));
    let local = q_rot(q_conj(rotation), sub(p, center));
    if friction {
        let closest = [clamp(local[0], -hx, hx), clamp(local[1], -hy, hy), clamp(local[2], -hz, hz)];
        let delta = sub(local, closest);
        let outside_sq = len_sq(delta);
        if outside_sq > 1e-5f32 as f64 {
            let outside = outside_sq.sqrt();
            if outside >= pr {
                return (0.0, [0.0; 3]);
            }
            return (pr - outside, q_rot(rotation, scl(delta, 1.0 / outside)));
        }
    }
    // The position-only kernel expands each face by the particle radius and requires a unique
    // nearest face. Friction uses rounded corners and breaks equal face distances Z, Y, X.
    let expansion = if friction { 0.0 } else { pr };
    let dx = hx + expansion - local[0].abs();
    let dy = hy + expansion - local[1].abs();
    let dz = hz + expansion - local[2].abs();
    let axis = if dx < dy && dx < dz {
        0
    } else if dy < dz {
        1
    } else {
        2
    };
    if !friction {
        if dx < 0.0 || dy < 0.0 || dz < 0.0 {
            return (0.0, [0.0; 3]);
        }
        if !(dx < dy && dx < dz) && !(dy < dx && dy < dz) && !(dz < dx && dz < dy) {
            return (0.0, [0.0; 3]);
        }
    }
    let depth = [dx, dy, dz][axis] + if friction { pr } else { 0.0 };
    if depth <= 0.0 {
        return (0.0, [0.0; 3]);
    }
    let mut n = [0.0; 3];
    n[axis] = if local[axis] >= 0.0 { 1.0 } else { -1.0 };
    (depth, q_rot(rotation, n))
}

/// Pushes a particle out along `normal` by `depth`, sliding it with the collider's motion up
/// to `friction * depth`.
fn project_contact(pos: &mut V3, prev: V3, normal: V3, depth: f64, friction: f64, motion: &M4) {
    if depth <= 0.0 {
        return;
    }
    let mut tangent = sub(m4_point(motion, prev), *pos);
    tangent = madd(tangent, normal, -dot(tangent, normal));
    let distance = len(tangent);
    let limit = friction.max(0.0) * depth;
    if distance > 0.0 {
        *pos = madd(*pos, tangent, (limit / distance).min(1.0));
    }
    *pos = madd(*pos, normal, depth);
}

/// Orients an ordered rope from its animated X axes and solved segment directions.
fn reconstruct_rope(nodes: &mut [Node], chain: &[usize], rotation_nodes: &[bool]) {
    if chain.len() < 2 {
        return;
    }
    let (first, second) = (&nodes[chain[0]], &nodes[chain[1]]);
    let axis = q_rot(q_norm(first.init_rot), [1.0, 0.0, 0.0]);
    let direction = sub(second.init_pos, first.init_pos);
    let sign = if dot(axis, direction) < 0.0 { -1.0 } else { 1.0 };
    for i in 0..chain.len() {
        if !rotation_nodes[chain[i]] {
            continue;
        }
        if i == chain.len() - 1 && chain.len() > 2 {
            nodes[chain[i]].solved_rot = nodes[chain[i - 1]].solved_rot;
            continue;
        }
        let start = i.min(chain.len() - 2);
        let direction = scl(sub(nodes[chain[start + 1]].pos, nodes[chain[start]].pos), sign);
        let node = &mut nodes[chain[i]];
        // Collapsed links keep their animated orientation, including two-node tips.
        node.solved_rot = node.target_rot;
        if len_sq(direction) <= 0.03 * 0.03 {
            continue;
        }
        let axis = norm(q_rot(node.target_rot, [1.0, 0.0, 0.0]));
        let align = q_from_unit(axis, norm(direction));
        node.solved_rot = q_norm(q_mul(align, node.solved_rot));
    }
}

fn blend_identity(q: Q4, amount: f64) -> Q4 {
    q_norm([q[0] * amount, q[1] * amount, q[2] * amount, 1.0 + (q[3] - 1.0) * amount])
}

/// Reconstructs one directed link, preserving the authored local axis and rest twist.
fn reconstruct_twist(nodes: &mut [Node], link: &Twist) {
    let (Some(node), Some(end)) = (nodes.get(link.node_orient), nodes.get(link.node_end)) else { return };
    let (axis, mut desired);
    if link.node_orient == link.node_end {
        axis = [0.0; 3];
        desired = node.target_rot;
    } else {
        let rest_axis = sub(end.init_pos, node.init_pos);
        let direction = sub(end.pos, node.pos);
        if len_sq(rest_axis) < 1e-12 || len_sq(direction) < 1e-12 {
            return;
        }
        let rest_orient = q_norm(node.init_rot);
        let rest_end = q_norm(end.init_rot);
        axis = q_rot(q_conj(rest_orient), norm(rest_axis));
        desired = q_norm(q_mul(q_mul(end.solved_rot, q_conj(rest_end)), rest_orient));
        let direction = q_rot(q_conj(desired), norm(direction));
        let align = q_from_unit(axis, direction);
        desired = q_norm(q_mul(desired, align));
    }
    let solved = node.solved_rot;
    if q_dot(solved, desired) < 0.0 {
        desired = [-desired[0], -desired[1], -desired[2], -desired[3]];
    }
    let twist = unit(link.twist_relax);
    let swing = unit(link.swing_relax);
    let node = &mut nodes[link.node_orient];
    if twist == swing {
        let l = |a: f64, b: f64| a + (b - a) * twist;
        node.solved_rot = q_norm([l(solved[0], desired[0]), l(solved[1], desired[1]), l(solved[2], desired[2]), l(solved[3], desired[3])]);
        return;
    }
    let error = q_mul(q_conj(solved), desired);
    let along = error[0] * axis[0] + error[1] * axis[1] + error[2] * axis[2];
    let twist_q = [axis[0] * along, axis[1] * along, axis[2] * along, error[3]];
    // The compiled relaxation blends the unnormalized twist projection.
    let weighted_twist = blend_identity(twist_q, twist);
    let swing_q = blend_identity(q_mul(error, q_conj(twist_q)), swing);
    node.solved_rot = q_norm(q_mul(solved, q_mul(swing_q, weighted_twist)));
}

/// The basis a `m_NodeBases` record reconstructs from four node positions.
fn node_base_quat(pos: impl Fn(usize) -> V3, base: &NodeBase) -> Q4 {
    let mut x = sub(pos(base.x1), pos(base.x0));
    let mut y = sub(pos(base.y1), pos(base.y0));
    y = if len_sq(y) > 0.0 { norm(y) } else { [0.0, 0.0, 1.0] };
    x = madd(x, y, -dot(x, y));
    // The runtime switches to a deterministic perpendicular when the projected span is at
    // most 0.05 Source units, before normalizing that span.
    if len(x) <= 0.05 {
        x = norm([y[2] + (1.0 - y[2]) * y[1] * y[1], 0.0, -y[0]]);
        x = madd(x, y, -dot(x, y));
    }
    x = norm(x);
    let z = cross(x, y);
    q_norm(q_mul(q_from_cols(x, y, z), q_norm(base.q_adjust)))
}

fn apply_offset(parent_pos: V3, parent_rot: Q4, offset: V3, sign: f64) -> V3 {
    add(parent_pos, q_rot(q_norm(parent_rot), scl(offset, sign)))
}

fn recover_offset_sign(parent_pos: V3, parent_rot: Q4, child_pos: V3, offset: V3) -> f64 {
    let plus = dist_sq(apply_offset(parent_pos, parent_rot, offset, 1.0), child_pos);
    let minus = dist_sq(apply_offset(parent_pos, parent_rot, offset, -1.0), child_pos);
    if plus <= minus { 1.0 } else { -1.0 }
}

/// The weighted rigid fit `m_FitMatrices` reconstructs: rest points moved to their current
/// positions with weights, about the rest center. `None` for invalid input.
#[allow(clippy::needless_range_loop)] // Index loops mirror the compiled kernel.
fn recover_cloth_fit(count: usize, sample: impl Fn(usize) -> (V3, V3, f64), rest_center: V3) -> Option<(V3, Q4)> {
    if count == 0 || !finite3(rest_center) {
        return None;
    }
    let mut position = [0.0; 3];
    let mut total = 0.0;
    for i in 0..count {
        let (s, t, w) = sample(i);
        if !finite3(s) || !finite3(t) || !w.is_finite() || w < 0.0 {
            return None;
        }
        position = madd(position, t, w);
        total += w;
    }
    if total <= 0.0 {
        return None;
    }
    position = scl(position, 1.0 / total);
    let mut cov = [[0.0f64; 3]; 3];
    for i in 0..count {
        let (s, t, w) = sample(i);
        for (row, cov_row) in cov.iter_mut().enumerate() {
            for (col, c) in cov_row.iter_mut().enumerate() {
                *c += w * (t[row] - position[row]) * (s[col] - rest_center[col]);
            }
        }
    }
    let mut normal = [[0.0f64; 3]; 3];
    for (row, normal_row) in normal.iter_mut().enumerate() {
        for (col, v) in normal_row.iter_mut().enumerate() {
            *v = cov.iter().map(|values| values[row] * values[col]).sum();
        }
    }
    let mut eig = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let epsilon = EPS23;
    // At most six cyclic sweeps with approximate half-angle rotations, as the runtime does;
    // this order also defines the rank-deficient fallback.
    for _ in 0..6 {
        let mut rotation_squared = 0.0;
        for (i, j) in [(0usize, 1usize), (1, 2), (2, 0)] {
            let mut sine = normal[i][j];
            let mut cosine = 2.0 * (normal[i][i] - normal[j][j]);
            if cosine * cosine > 5.828_427_314_758_301 * sine * sine {
                let inverse = 1.0 / sine.hypot(cosine);
                sine *= inverse;
                cosine *= inverse;
            } else {
                sine = (std::f64::consts::PI / 8.0).sin();
                cosine = (std::f64::consts::PI / 8.0).cos();
            }
            rotation_squared += sine * sine;
            let c = cosine * cosine - sine * sine;
            let s = 2.0 * cosine * sine;
            for row in 0..3 {
                let (a, b) = (normal[row][i], normal[row][j]);
                normal[row][i] = c * a + s * b;
                normal[row][j] = c * b - s * a;
                let (u, v) = (eig[row][i], eig[row][j]);
                eig[row][i] = c * u + s * v;
                eig[row][j] = c * v - s * u;
            }
            for col in 0..3 {
                let (a, b) = (normal[i][col], normal[j][col]);
                normal[i][col] = c * a + s * b;
                normal[j][col] = c * b - s * a;
            }
        }
        if rotation_squared <= epsilon {
            break;
        }
    }
    let mut axes = [[0.0f64; 3]; 3];
    for (col, axis) in axes.iter_mut().enumerate() {
        for (row, a) in axis.iter_mut().enumerate() {
            *a = (0..3).map(|k| cov[row][k] * eig[k][col]).sum();
        }
    }
    let (x, y, z) = (normal[0][0], normal[1][1], normal[2][2]);
    // Strict comparisons retain the engine's tie order for line-shaped sources.
    let order = if x > y {
        if y > z {
            [0, 1, 2]
        } else if z > x {
            [2, 0, 1]
        } else {
            [0, 2, 1]
        }
    } else if x > z {
        [1, 0, 2]
    } else if z > y {
        [2, 1, 0]
    } else {
        [1, 2, 0]
    };
    let [first, second, third] = order;
    if len(axes[first]) < epsilon {
        return Some(([0.0; 3], Q_ID));
    }
    axes[first] = norm(axes[first]);
    axes[second] = madd(axes[second], axes[first], -dot(axes[second], axes[first]));
    if len(axes[second]) <= epsilon {
        let [a, b, c] = axes[first];
        axes[second] = norm([c + (1.0 - c) * b * b, 0.0, -a]);
        axes[second] = madd(axes[second], axes[first], -dot(axes[second], axes[first]));
    }
    axes[second] = norm(axes[second]);
    let sign = if (second + 3 - first) % 3 == 1 { 1.0 } else { -1.0 };
    axes[third] = scl(cross(axes[first], axes[second]), sign);
    // The basis (columns = axes) times the transposed eigenvector matrix.
    let m = |r: usize, c: usize| (0..3).map(|k| axes[k][r] * eig[c][k]).sum::<f64>();
    let rotation = q_norm(q_from_cols([m(0, 0), m(1, 0), m(2, 0)], [m(0, 1), m(1, 1), m(2, 1)], [m(0, 2), m(1, 2), m(2, 2)]));
    Some((position, rotation))
}

#[allow(clippy::needless_range_loop)] // Jacobi sweeps read clearest by index.
fn largest_eigenvector4(m: [[f64; 4]; 4]) -> [f64; 4] {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    for _ in 0..64 {
        let (mut p, mut q, mut max) = (0, 1, 0.0);
        for r in 0..4 {
            for c in r + 1..4 {
                if a[r][c].abs() > max {
                    max = a[r][c].abs();
                    p = r;
                    q = c;
                }
            }
        }
        if max < 1e-12 {
            break;
        }
        let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
        // `Math.sign(theta || 1)`.
        let sgn = if theta == 0.0 || theta.is_nan() { 1.0 } else { theta.signum() };
        let t = sgn / (theta.abs() + (theta * theta + 1.0).sqrt());
        let cos = 1.0 / (t * t + 1.0).sqrt();
        let sin = t * cos;
        let (app, aqq, apq) = (a[p][p], a[q][q], a[p][q]);
        a[p][p] = app - t * apq;
        a[q][q] = aqq + t * apq;
        a[p][q] = 0.0;
        a[q][p] = 0.0;
        for r in 0..4 {
            if r == p || r == q {
                continue;
            }
            let (arp, arq) = (a[r][p], a[r][q]);
            a[r][p] = cos * arp - sin * arq;
            a[p][r] = a[r][p];
            a[r][q] = sin * arp + cos * arq;
            a[q][r] = a[r][q];
        }
        for row in v.iter_mut() {
            let (vrp, vrq) = (row[p], row[q]);
            row[p] = cos * vrp - sin * vrq;
            row[q] = sin * vrp + cos * vrq;
        }
    }
    let mut best = 0;
    for i in 1..4 {
        if a[i][i] > a[best][best] {
            best = i;
        }
    }
    let e = [v[0][best], v[1][best], v[2][best], v[3][best]];
    let l = q_dot(e, e).sqrt();
    if l < 1e-9 { [1.0, 0.0, 0.0, 0.0] } else { [e[0] / l, e[1] / l, e[2] / l, e[3] / l] }
}

/// A uniform-scale similarity from Source model space to the glb frame.
#[derive(Clone, Copy, Debug)]
struct Similarity {
    matrix: M4,
    inverse: M4,
    rotation: Q4,
    scale: f64,
    rmse: f64,
}

/// Horn's quaternion method with a least-squares scale. `None` for fewer than three pairs or
/// degenerate source points.
fn recover_similarity(source: &[V3], target: &[V3]) -> Option<Similarity> {
    if source.len() != target.len() || source.len() < 3 {
        return None;
    }
    let centroid = |pts: &[V3]| {
        let mut c = [0.0; 3];
        for p in pts {
            c = add(c, *p);
        }
        scl(c, 1.0 / pts.len() as f64)
    };
    let (sc, tc) = (centroid(source), centroid(target));
    let mut s = [[0.0f64; 3]; 3];
    for (p, q) in source.iter().zip(target) {
        let (x, y) = (sub(*p, sc), sub(*q, tc));
        for r in 0..3 {
            for c in 0..3 {
                s[r][c] += x[r] * y[c];
            }
        }
    }
    let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = s;
    let trace = sxx + syy + szz;
    let k = [
        [trace, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let [w, x, y, z] = largest_eigenvector4(k);
    let rotation = q_norm([x, y, z, w]);
    let (mut numerator, mut denominator) = (0.0, 0.0);
    for (p, q) in source.iter().zip(target) {
        let (x, y) = (sub(*p, sc), sub(*q, tc));
        numerator += dot(y, q_rot(rotation, x));
        denominator += len_sq(x);
    }
    if denominator < 1e-9 {
        return None;
    }
    let scale = numerator / denominator;
    let translation = sub(tc, scl(q_rot(rotation, sc), scale));
    let matrix = m4_compose(translation, rotation, [scale; 3]);
    let inverse = m4_invert(&matrix);
    let err2: f64 = source.iter().zip(target).map(|(p, q)| dist_sq(m4_point(&matrix, *p), *q)).sum();
    Some(Similarity { matrix, inverse, rotation, scale, rmse: (err2 / source.len() as f64).sqrt() })
}

/// The cloth node's bind offset from its anchor, rigidly carried by the anchor's current
/// transform (vpkmerge's ClothAnchors rest drape). At bind this is the node's own rest pose.
fn rigid_anchor_seed(node_init: V3, anchor_init: V3, anchor_init_rot: Q4, anchor_pos: V3, anchor_rot: Q4) -> V3 {
    let local = q_rot(q_conj(q_norm(anchor_init_rot)), sub(node_init, anchor_init));
    add(q_rot(anchor_rot, local), anchor_pos)
}

fn is_cloth_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("$cloth") || lower.starts_with("cloth")
}

/// Each `$cloth*` node's terminal (root) driver in `m_SkelParents`, which must be a real,
/// non-cloth bone (vpkmerge's `anchors_from_phys`).
fn cloth_anchor_map(fe: &FeModel) -> Vec<Option<usize>> {
    let n = fe.nodes.len();
    let parents = &fe.skel_parents;
    let mut out = vec![None; n];
    if parents.len() != n {
        return out;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        if !is_cloth_name(&fe.nodes[i].name) {
            continue;
        }
        let mut current = i;
        let mut seen = vec![false; n];
        let terminal = loop {
            if seen[current] {
                break None;
            }
            seen[current] = true;
            let parent = parents[current];
            if !parent.is_finite() || parent.fract() != 0.0 || parent < 0.0 {
                break Some(current);
            }
            if parent >= n as f64 {
                break None;
            }
            current = parent as usize;
        };
        if let Some(t) = terminal.filter(|&t| t != i && !is_cloth_name(&fe.nodes[t].name)) {
            *slot = Some(t);
        }
    }
    out
}

/// `m_FitMatrices` target: `nCtrl` when it differs from `nNode`, else `nNode`.
fn fit_target(fit: &FitMatrix, count: f64) -> Option<usize> {
    let valid = |v: f64| v.is_finite() && v.fract() == 0.0 && v >= 0.0 && v < count;
    if valid(fit.ctrl) && fit.ctrl != fit.node {
        return Some(fit.ctrl as usize);
    }
    valid(fit.node).then_some(fit.node as usize)
}

#[derive(Clone, Debug)]
struct FitRec {
    target: usize,
    bone: V3,
    bone_rot: Q4,
    center: V3,
    weights: Vec<(usize, f64)>,
}

fn build_fit_recs(fe: &FeModel) -> Vec<FitRec> {
    let n = fe.nodes.len();
    let mut out = Vec::new();
    let mut begin = 0usize;
    for fit in &fe.fit_matrices {
        let end = (fe.fit_weights.len() as f64).min(fit.end_weight.trunc());
        let end = begin.max(if end > 0.0 { end as usize } else { 0 });
        let mut weights = Vec::new();
        for w in fe.fit_weights.get(begin..end).unwrap_or(&[]) {
            let Some(node) = idx(w.node, n) else { continue };
            if !w.weight.is_finite() || w.weight <= 0.0 {
                continue;
            }
            weights.push((node, w.weight));
        }
        if let Some(target) = fit_target(fit, n as f64).filter(|_| !weights.is_empty()) {
            out.push(FitRec { target, bone: fit.bone, bone_rot: fit.bone_rot, center: fit.center, weights });
        }
        begin = end;
    }
    out
}

fn reconstruct_fit(fit: &FitRec, nodes: &[Node]) -> Option<(V3, Q4)> {
    nodes.get(fit.target)?;
    let (position, rotation) = recover_cloth_fit(
        fit.weights.len(),
        |i| {
            let (node, w) = fit.weights[i];
            (nodes[node].init_pos, nodes[node].pos, w)
        },
        fit.center,
    )?;
    // The compiled bone transform is already relative to the fit center; it is an output
    // transform, separate from the particle and its integration history.
    Some((add(position, q_rot(rotation, fit.bone)), q_norm(q_mul(rotation, q_norm(fit.bone_rot)))))
}

// ---------------------------------------------------------------------------------------------
// The runtime (`useClothSim.ts`).

#[derive(Clone, Debug)]
struct FilterRt {
    mask: i32,
    priority: usize,
    vertex_nodes: Option<Vec<usize>>,
}

impl FilterRt {
    fn new(f: &ColliderFilter) -> FilterRt {
        FilterRt { mask: to_i32(f.mask), priority: f.priority, vertex_nodes: f.vertex_nodes.clone() }
    }

    fn can_collide(&self, node: &Node) -> bool {
        match &self.vertex_nodes {
            Some(nodes) => nodes.binary_search(&node.index).is_ok(),
            None => node.collision_mask & self.mask != 0,
        }
    }
}

#[derive(Clone, Debug)]
enum Collider {
    Capsule { node: usize, sphere0: [f64; 4], sphere1: [f64; 4], filter: FilterRt },
    Box { node: usize, pos: V3, rot: Q4, half_size: V3, filter: FilterRt },
    Plane(PlaneRt),
}

impl Collider {
    fn priority(&self) -> usize {
        match self {
            Collider::Capsule { filter, .. } | Collider::Box { filter, .. } => filter.priority,
            Collider::Plane(p) => p.priority,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct OffsetRt {
    parent: usize,
    child: usize,
    offset: V3,
    sign: f64,
    alpha: f64,
}

#[derive(Clone, Copy, Debug)]
struct ReverseRt {
    bone_ctrl: usize,
    target_node: usize,
    offset: V3,
    sign: f64,
}

/// Debug and tuning knobs (Grimoire's `clothTuning`). The defaults are the authored behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothTuning {
    /// Constraint iterations per substep; 0 keeps the authored `m_nExtraIterations + 1`.
    pub iteration_override: u32,
    pub gravity_scale: f64,
    pub attraction_scale: f64,
    pub collision_scale: f64,
    /// Wall-clock milliseconds a frame gives the solver; `None` always runs every substep due.
    pub budget_ms: Option<f64>,
    /// Snap every node to its animated target instead of simulating (a debug view).
    pub targets_only: bool,
}

impl Default for ClothTuning {
    fn default() -> Self {
        ClothTuning { iteration_override: 0, gravity_scale: 1.0, attraction_scale: 1.0, collision_scale: 1.0, budget_ms: Some(CLOTH_FRAME_BUDGET_MS), targets_only: false }
    }
}

/// What the solver did, for a status line or a benchmark.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClothStats {
    /// FeModel nodes, and how many matched a glb joint.
    pub nodes: usize,
    pub matched: usize,
    /// Nodes the solver moves (not pinned, position-driven, locked or jiggle-driven).
    pub dynamic_nodes: usize,
    pub substeps_last_frame: u32,
    pub simulation_steps: u64,
    /// History resets: resume gaps, teleports and emergency recoveries.
    pub recoveries: u32,
    pub last_frame_ms: f32,
    /// Root-mean-square error of the rest-pose fit, in glb units.
    pub fit_rmse: f32,
}

/// Which compiled constraints the preview simulates (Grimoire's `clothSimulationCoverage`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothCoverage {
    pub rods: usize,
    pub animated_rods: usize,
    pub twists: usize,
    pub kelager_bends: usize,
    pub hinge_limits: usize,
    pub triangles: usize,
    pub quads: usize,
    pub rope_chains: usize,
    pub pending_rope_chains: usize,
    pub pending_jiggle_bones: usize,
    pub goal_damped: usize,
    pub raw: usize,
    pub unknown: usize,
    pub decode_issues: Vec<DecodeIssue>,
    pub feature_gaps: Vec<FeatureGap>,
}

/// Solver geometry in the glb frame, for a collider and node overlay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClothDebug {
    /// Node position and whether animation drives it.
    pub nodes: Vec<([f32; 3], bool)>,
    /// Tapered capsules (spheres have equal ends): end, radius, end, radius.
    pub capsules: Vec<([f32; 3], f32, [f32; 3], f32)>,
    /// Boxes as a transform of the unit cube `[-1, 1]^3`.
    pub boxes: Vec<Mat4>,
}

/// A hero's cloth runtime: one per loaded model and FeModel.
pub struct Cloth {
    fe: FeModel,
    nodes: Vec<Node>,
    initial: Vec<Node>,
    // The glb graph.
    parents: Vec<Option<usize>>,
    is_bone: Vec<bool>,
    order: Vec<usize>,
    needed: Vec<usize>,
    locals: Vec<Mat4>,
    world: Vec<M4>,
    fit: Similarity,
    root_to_model_rot: Q4,
    // Solver sets.
    twist_nodes: Vec<bool>,
    rotation_nodes: Vec<bool>,
    rotation_list: Vec<usize>,
    colliders: Vec<Collider>,
    collider_prev: Vec<Option<M4>>,
    collider_now: Vec<M4>,
    collider_motion: Vec<M4>,
    collider_stamp: Vec<u64>,
    stamp: u64,
    touched: Vec<usize>,
    ctrl_offsets: Vec<OffsetRt>,
    soft_offsets: Vec<OffsetRt>,
    reverse_offsets: Vec<ReverseRt>,
    fit_recs: Vec<FitRec>,
    animated_rod_batches: Vec<Vec<Rod>>,
    anchors: Vec<Option<usize>>,
    teleport_distance: f64,
    // State.
    last_substep_dt: Option<f64>,
    warm_started: bool,
    solved: bool,
    accumulator: f64,
    substep_ms: f64,
    simulation_steps: u64,
    recovery_count: u32,
    frame_steps: u32,
    frame_ms: f64,
    placement_rot: Q4,
    pub tuning: ClothTuning,
}

fn mat_finite(m: &Mat4) -> bool {
    m.0.iter().flatten().all(|v| v.is_finite())
}

impl Cloth {
    /// Parses a `.cloth.json` sidecar and fits it to `model`'s bind pose. `Err` when the JSON
    /// is not a FeModel; `Ok(None)` when fewer than three nodes match a joint or the fit is
    /// singular (the hero then plays without cloth).
    pub fn new(model: &Model, json: &str) -> Result<Option<Cloth>, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| format!("cloth sidecar: {e}"))?;
        let fe = parse_fe_model(&value).ok_or_else(|| "cloth sidecar: not an FeModel (m_CtrlName missing or a malformed container)".to_string())?;
        Ok(Cloth::build(model, fe))
    }

    fn build(model: &Model, fe: FeModel) -> Option<Cloth> {
        let count = model.nodes.len();
        let parents: Vec<Option<usize>> = model.nodes.iter().map(|n| n.parent.filter(|&p| p < count)).collect();
        let mut is_bone = vec![false; count];
        for skin in &model.skins {
            for &j in &skin.joints {
                if j < count {
                    is_bone[j] = true;
                }
            }
        }
        // Parent-first order (cycle-guarded like `model::globals`).
        let depth: Vec<usize> = (0..count)
            .map(|i| {
                let (mut d, mut at) = (0, i);
                while let Some(p) = parents[at] {
                    d += 1;
                    at = p;
                    if d > 512 {
                        break;
                    }
                }
                d
            })
            .collect();
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&i| depth[i]);
        let mut by_name = std::collections::HashMap::new();
        for (i, node) in model.nodes.iter().enumerate() {
            if is_bone[i] {
                by_name.insert(node.name.as_str(), i);
            }
        }
        let locals = crate::model::sample_pose(model, None, 0.0);
        let mut world = vec![M4_ID; count];
        for &i in &order {
            let local = m4_from(&locals[i]);
            world[i] = match parents[i] {
                Some(p) => m4_mul(&world[p], &local),
                None => local,
            };
        }
        let n = fe.nodes.len();
        let bones: Vec<Option<usize>> = fe.nodes.iter().map(|node| by_name.get(node.name.as_str()).copied()).collect();
        let (mut source_all, mut target_all, mut source_pinned, mut target_pinned) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (node, bone) in fe.nodes.iter().zip(&bones) {
            let Some(b) = *bone else { continue };
            let w = [world[b][12], world[b][13], world[b][14]];
            source_all.push(node.init_pos);
            target_all.push(w);
            if node.pinned {
                source_pinned.push(node.init_pos);
                target_pinned.push(w);
            }
        }
        let use_pinned = source_pinned.len() >= 3;
        let (source, target) = if use_pinned { (&source_pinned, &target_pinned) } else { (&source_all, &target_all) };
        let fit = recover_similarity(source, target)?;
        let fit_finite = fit.scale.is_finite() && fit.rmse.is_finite() && fit.matrix.iter().chain(fit.inverse.iter()).all(|v| v.is_finite());
        if !fit_finite || fit.scale.abs() < 1e-12 {
            return None;
        }

        let lock_to_goal: Vec<bool> = (0..n).map(|i| fe.lock_to_goal.contains(&(i as f64))).collect();
        let mut fit_driven = vec![false; n];
        for f in &fe.fit_matrices {
            if let Some(t) = fit_target(f, f64::INFINITY).filter(|&t| t < n) {
                fit_driven[t] = true;
            }
        }
        let mut jiggle = vec![false; n];
        for j in &fe.jiggle_bones {
            if let Some(i) = idx(j.node, n).filter(|_| j.has_params) {
                jiggle[i] = true;
            }
        }
        let mut generated = vec![false; n];
        for o in fe.ctrl_offsets.iter().chain(&fe.soft_offsets) {
            if let Some(c) = idx(o.child, n) {
                generated[c] = true;
            }
        }
        let gravity_scale = fe.default_gravity_scale;
        let mut nodes: Vec<Node> = fe
            .nodes
            .iter()
            .enumerate()
            .map(|(index, f)| {
                let rot = q_from_v4(f.init_rot);
                Node {
                    index,
                    bone: bones[index],
                    inv_mass: f.inv_mass,
                    // FreeNodes selects an orientation path; nodes with a reconstructed basis
                    // still simulate their positions.
                    pinned: f.pinned,
                    position_driven: (fe.first_position_driven_node.is_finite() && index as f64 >= fe.first_position_driven_node) || fit_driven[index],
                    lock_to_goal: lock_to_goal[index],
                    jiggle_driven: jiggle[index],
                    kinematic: false,
                    generated_target: generated[index],
                    mode: integrator_mode(&fe, index),
                    gravity: f.gravity * gravity_scale,
                    damping: f.damping,
                    anim_force: f.anim_force,
                    anim_vertex: f.anim_vertex,
                    collide_radius: f.collide_radius,
                    friction: f.friction,
                    collision_mask: to_i32(f.collision_mask),
                    init_pos: f.init_pos,
                    init_rot: f.init_rot,
                    pos: f.init_pos,
                    prev: f.init_pos,
                    last_solved: f.init_pos,
                    target: f.init_pos,
                    solved_rot: rot,
                    target_rot: rot,
                }
            })
            .collect();
        for node in &mut nodes {
            node.kinematic = node.flags_kinematic();
        }

        let pose_of = |i: f64| -> (V3, Q4) { idx(i, n).map_or(([0.0; 3], Q_ID), |i| (fe.nodes[i].init_pos, fe.nodes[i].init_rot)) };
        let offset_rt = |o: &Offset| -> Option<OffsetRt> {
            let (pp, pr) = pose_of(o.parent);
            let (cp, _) = pose_of(o.child);
            let sign = recover_offset_sign(pp, pr, cp, o.offset);
            Some(OffsetRt { parent: idx(o.parent, n)?, child: idx(o.child, n)?, offset: o.offset, sign, alpha: o.alpha })
        };
        let ctrl_offsets = fe.ctrl_offsets.iter().filter_map(offset_rt).collect();
        let soft_offsets = fe.soft_offsets.iter().filter_map(offset_rt).collect();
        let reverse_offsets = fe
            .reverse_offsets
            .iter()
            .filter_map(|o| {
                let (target_pos, _) = pose_of(o.target_node);
                let (bone_pos, bone_rot) = pose_of(o.bone_ctrl);
                let plus = dist_sq(apply_offset(target_pos, bone_rot, o.offset, 1.0), bone_pos);
                let minus = dist_sq(apply_offset(target_pos, bone_rot, o.offset, -1.0), bone_pos);
                Some(ReverseRt { bone_ctrl: idx(o.bone_ctrl, n)?, target_node: idx(o.target_node, n)?, offset: o.offset, sign: if plus <= minus { 1.0 } else { -1.0 } })
            })
            .collect();

        let mut colliders: Vec<Collider> = Vec::new();
        for c in fe.capsules.iter().rev().chain(fe.spheres.iter().rev()) {
            if let Some(node) = idx(c.node, n) {
                colliders.push(Collider::Capsule { node, sphere0: c.sphere0, sphere1: c.sphere1, filter: FilterRt::new(&c.filter) });
            }
        }
        for b in fe.boxes.iter().rev() {
            if let Some(node) = idx(b.node, n) {
                colliders.push(Collider::Box { node, pos: b.pos, rot: b.rot, half_size: b.half_size, filter: FilterRt::new(&b.filter) });
            }
        }
        for p in &fe.collision_planes {
            if let (Some(ctrl_parent), Some(child_node)) = (idx(p.ctrl_parent, n), idx(p.child_node, n)) {
                colliders.push(Collider::Plane(PlaneRt { priority: p.priority, ctrl_parent, child_node, normal: p.normal, offset: p.offset, strength: p.strength }));
            }
        }
        // Stable sorting keeps each type's compiled traversal within a priority.
        colliders.sort_by_key(|c| std::cmp::Reverse(c.priority()));

        let lock_static = fe.rot_lock_static_node_count;
        let rotatable = |i: usize| i as f64 >= lock_static && i < n && !nodes[i].jiggle_driven;
        let mut twist_nodes = vec![false; n];
        let mut rotation_nodes = vec![false; n];
        let mut rotation_list = Vec::new();
        for t in &fe.twists {
            if rotatable(t.node_orient) {
                twist_nodes[t.node_orient] = true;
            }
        }
        for i in fe.twists.iter().map(|t| t.node_orient).chain(fe.rope_chains.iter().flatten().copied()) {
            if rotatable(i) && !rotation_nodes[i] {
                rotation_nodes[i] = true;
                rotation_list.push(i);
            }
        }

        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for node in &nodes {
            for k in 0..3 {
                lo[k] = lo[k].min(node.init_pos[k]);
                hi[k] = hi[k].max(node.init_pos[k]);
            }
        }
        let size = if nodes.is_empty() { 0.0 } else { len(sub(hi, lo)) };
        // Source units. Ordinary animation stays untouched; discontinuous body motion spanning
        // several rest-pose bounds starts a new drape.
        let teleport_distance = 128f64.max(size * 4.0);

        let mut needed_flag = vec![false; count];
        for &b in bones.iter().flatten() {
            let mut at = Some(b);
            let mut guard = 0;
            while let Some(i) = at {
                if needed_flag[i] || guard > 512 {
                    break;
                }
                needed_flag[i] = true;
                at = parents[i];
                guard += 1;
            }
        }
        let needed = order.iter().copied().filter(|&i| needed_flag[i]).collect();
        let animated_rod_batches = fe.animated_rod_batches.clone();
        let fit_recs = build_fit_recs(&fe);
        let anchors = cloth_anchor_map(&fe);
        Some(Cloth {
            initial: nodes.clone(),
            nodes,
            parents,
            is_bone,
            order,
            needed,
            locals,
            world,
            root_to_model_rot: q_conj(fit.rotation),
            fit,
            twist_nodes,
            rotation_nodes,
            rotation_list,
            collider_prev: vec![None; n],
            collider_now: vec![M4_ID; n],
            collider_motion: vec![M4_ID; n],
            collider_stamp: vec![0; n],
            stamp: 0,
            touched: Vec::with_capacity(n),
            colliders,
            ctrl_offsets,
            soft_offsets,
            reverse_offsets,
            fit_recs,
            animated_rod_batches,
            anchors,
            teleport_distance,
            last_substep_dt: None,
            warm_started: false,
            solved: false,
            accumulator: 0.0,
            substep_ms: 0.0,
            simulation_steps: 0,
            recovery_count: 0,
            frame_steps: 0,
            frame_ms: 0.0,
            placement_rot: Q_ID,
            tuning: ClothTuning::default(),
            fe,
        })
    }

    /// The model's placement in the world (the hero turntable or Hideout anchor). Gravity is
    /// world down, so a turned model feels it along its own axes.
    pub fn set_placement(&mut self, placement: &Mat4) {
        let q = m4_rotation(&m4_from(placement));
        self.placement_rot = if finite_rot(q) { q_norm(q) } else { Q_ID };
    }

    /// Restarts the drape (a clip change or a seek), as Grimoire rebuilds its runtime.
    /// `locals` is the animated pose to show until the next substep (node locals, as
    /// `model::sample_pose` returns); a wrong-length slice keeps the current pose.
    pub fn reset(&mut self, locals: &[Mat4]) {
        self.nodes.clone_from(&self.initial);
        for (batch, fresh) in self.animated_rod_batches.iter_mut().zip(&self.fe.animated_rod_batches) {
            batch.copy_from_slice(fresh);
        }
        self.collider_prev.fill(None);
        self.last_substep_dt = None;
        self.warm_started = false;
        self.solved = false;
        self.accumulator = 0.0;
        self.substep_ms = 0.0;
        self.simulation_steps = 0;
        self.recovery_count = 0;
        self.frame_steps = 0;
        self.frame_ms = 0.0;
        if locals.len() == self.locals.len() && locals.iter().all(mat_finite) {
            self.locals.copy_from_slice(locals);
        }
    }

    /// Advances by `dt` seconds of playback. `animate(sub_dt)` advances the caller's animation
    /// clock by `sub_dt` and returns the node locals for that moment; it runs once per substep
    /// (the last one of a frame may take up to four substeps of leftover time). A frame that
    /// falls short of a substep calls nothing, and one longer than 0.25 s restarts the drape
    /// without calling it.
    pub fn step(&mut self, dt: f32, animate: &mut dyn FnMut(f32) -> Vec<Mat4>) {
        self.frame_steps = 0;
        let frame_started = Instant::now();
        let delta = dt as f64;
        if !delta.is_finite() || delta <= 0.0 {
            return;
        }
        if delta > CLOTH_RESUME_GAP {
            self.accumulator = 0.0;
            self.reset_history();
            return;
        }
        self.accumulator += delta.min(MAX_CLOTH_STEPS_PER_FRAME as f64 * CLOTH_TIMESTEP);
        let count = (((self.accumulator + 1e-10) / CLOTH_TIMESTEP).floor() as usize).min(MAX_CLOTH_STEPS_PER_FRAME);
        self.accumulator = (self.accumulator - count as f64 * CLOTH_TIMESTEP).max(0.0);
        let gravity = q_rot(self.root_to_model_rot, q_rot(q_conj(self.placement_rot), [0.0, -1.0, 0.0]));
        let (constraint_iterations, goal_iterations) = self.iteration_phases();
        let collide_after = self.fe.dynamic_node_flags.unwrap_or(0) & 0x2000 != 0;
        let budget = self.tuning.budget_ms;
        let started = Instant::now();
        for step in 0..count {
            // A substep that would run past the budget is the frame's last: the animation takes
            // the frame's remaining time (up to four substeps) and the cloth skips it. The solver
            // itself always steps by the fixed substep.
            let step_started = Instant::now();
            let last = step == count - 1 || budget.is_some_and(|b| ms(step_started - started) + self.substep_ms > b);
            let sub = if last { ((count - step) as f64 * CLOTH_TIMESTEP).min(MAX_CLOTH_CATCH_UP) } else { CLOTH_TIMESTEP };
            let locals = animate(sub as f32);
            self.refresh_targets(&locals);
            drop(locals);
            self.update_animated_rod_lengths();
            let dt = CLOTH_TIMESTEP;
            self.warm_start(dt);
            for node in &mut self.nodes {
                node.last_solved = node.pos;
            }
            if self.tuning.targets_only {
                for node in &mut self.nodes {
                    node.pos = node.target;
                    node.prev = node.target;
                    node.solved_rot = node.target_rot;
                }
            } else {
                self.integrate(gravity, dt);
                self.solve_goals(dt);
                if !collide_after {
                    self.solve_collisions();
                }
                for i in 0..constraint_iterations {
                    for bend in &self.fe.kelager_bends {
                        project_kelager_bend(&mut self.nodes, bend);
                    }
                    for hinge in &self.fe.hinge_limits {
                        project_hinge_limit(&mut self.nodes, hinge);
                    }
                    self.solve_rods();
                    for batch in &self.fe.stray_radius_batches {
                        project_stray_radius_batch(&mut self.nodes, batch, 1.0);
                    }
                    for batch in &self.fe.quad_batches {
                        project_quad_batch(&mut self.nodes, batch, 1.0, 1.0);
                    }
                    for batch in &self.fe.triangle_batches {
                        project_triangle_batch(&mut self.nodes, batch, 1.0);
                    }
                    if i >= constraint_iterations - goal_iterations {
                        self.solve_goal_damped_nodes();
                    }
                    self.restore_pinned();
                }
                if collide_after {
                    self.solve_collisions();
                }
                self.restore_pinned();
            }
            self.write_back_state();
            self.solved = true;
            self.simulation_steps += 1;
            self.frame_steps += 1;
            if budget.is_some() {
                self.substep_ms = ms(step_started.elapsed());
            }
            if last {
                break;
            }
        }
        self.frame_ms = ms(frame_started.elapsed());
    }

    /// The clean node locals of the last substep (or of the last `reset`).
    pub fn locals(&self) -> &[Mat4] {
        &self.locals
    }

    /// Overwrites the simulated bones in `globals` (node world matrices in the glb frame,
    /// computed from [`Cloth::locals`]) with the solved transforms. Descendants of a moved
    /// bone follow it through their own locals. Nothing changes before the first substep.
    pub fn apply(&self, globals: &mut [Mat4]) {
        let count = self.parents.len();
        if !self.solved || globals.len() != count {
            return;
        }
        let mut rot_w: Vec<Option<Q4>> = vec![None; count];
        let mut pos_w: Vec<Option<V3>> = vec![None; count];
        let mut fit_w: Vec<Option<(V3, Q4)>> = vec![None; count];
        for fit in &self.fit_recs {
            let node = &self.nodes[fit.target];
            let Some(bone) = node.bone.filter(|_| !node.jiggle_driven) else { continue };
            if let Some(t) = reconstruct_fit(fit, &self.nodes) {
                fit_w[bone] = Some(t);
            }
        }
        for &i in &self.rotation_list {
            if let Some(bone) = self.nodes[i].bone {
                rot_w[bone] = Some(self.nodes[i].solved_rot);
            }
        }
        for base in &self.fe.node_bases {
            let node = &self.nodes[base.node];
            if let Some(bone) = node.bone.filter(|_| !node.pinned && !node.jiggle_driven) {
                rot_w[bone] = Some(node.solved_rot);
            }
        }
        for (bone, t) in fit_w.iter().enumerate() {
            if let Some((_, rot)) = t {
                rot_w[bone] = Some(*rot);
            }
        }
        // A cloth bone under a rotated bone keeps its animated orientation. Ancestors are any
        // glb node; only joints count as rotated (three.js `Bone`).
        let mut rotated_above = vec![false; count];
        for &i in &self.order {
            if let Some(p) = self.parents[i] {
                rotated_above[i] = rotated_above[p] || (rot_w[p].is_some() && self.is_joint(p));
            }
        }
        for node in &self.nodes {
            if let Some(bone) = node.bone
                && rot_w[bone].is_none()
                && rotated_above[bone]
            {
                rot_w[bone] = Some(node.target_rot);
            }
        }
        // A simulated ancestor must not carry an animation-owned attachment away from its
        // sampled world position, even when that ancestor only translates.
        let mut moved = vec![false; count];
        for node in &self.nodes {
            if let Some(bone) = node.bone
                && (!node.kinematic || node.generated_target)
            {
                moved[bone] = true;
            }
        }
        let mut owned = vec![false; count];
        for &i in &self.order {
            let own = self.is_joint(i) && (rot_w[i].is_some() || moved[i]);
            owned[i] = own || self.parents[i].is_some_and(|p| owned[p]);
        }
        for node in &self.nodes {
            let Some(bone) = node.bone else { continue };
            if node.kinematic && !node.generated_target && !owned[bone] {
                continue;
            }
            pos_w[bone] = Some(node.pos);
        }
        for o in &self.reverse_offsets {
            let bone_node = &self.nodes[o.bone_ctrl];
            let Some(bone) = bone_node.bone.filter(|_| !bone_node.pinned && !bone_node.jiggle_driven) else { continue };
            // Reverse offsets place the rendered bone after orientation reconstruction.
            pos_w[bone] = Some(add(self.nodes[o.target_node].pos, q_rot(bone_node.solved_rot, scl(o.offset, o.sign))));
        }
        for (bone, t) in fit_w.iter().enumerate() {
            if let Some((pos, _)) = t {
                pos_w[bone] = Some(*pos);
            }
        }
        let mut affected = vec![false; count];
        for &i in &self.order {
            let parent_affected = self.parents[i].is_some_and(|p| affected[p]);
            affected[i] = parent_affected || rot_w[i].is_some() || pos_w[i].is_some();
            if !affected[i] {
                continue;
            }
            let base = match self.parents[i] {
                Some(p) => globals[p].mul(&self.locals[i]),
                None => self.locals[i],
            };
            let mut m = base;
            if let Some(q) = rot_w[i] {
                let world = q_norm(q_mul(self.fit.rotation, q));
                let s = |c: usize| (m.0[c][0] * m.0[c][0] + m.0[c][1] * m.0[c][1] + m.0[c][2] * m.0[c][2]).sqrt();
                let t = [m.0[3][0], m.0[3][1], m.0[3][2]];
                m = Mat4::from_trs(t, [world[0] as f32, world[1] as f32, world[2] as f32, world[3] as f32], [s(0), s(1), s(2)]);
            }
            if let Some(p) = pos_w[i] {
                let w = m4_point(&self.fit.matrix, p);
                m.0[3] = [w[0] as f32, w[1] as f32, w[2] as f32, 1.0];
            }
            globals[i] = m;
        }
    }

    /// World matrices of every glb node with the cloth applied: the convenience path for
    /// [`Cloth::locals`] then [`Cloth::apply`].
    pub fn pose(&self) -> Vec<Mat4> {
        let mut g = vec![Mat4::IDENTITY; self.parents.len()];
        for &i in &self.order {
            g[i] = match self.parents[i] {
                Some(p) => g[p].mul(&self.locals[i]),
                None => self.locals[i],
            };
        }
        self.apply(&mut g);
        g
    }

    pub fn stats(&self) -> ClothStats {
        ClothStats {
            nodes: self.nodes.len(),
            matched: self.nodes.iter().filter(|n| n.bone.is_some()).count(),
            dynamic_nodes: self.nodes.iter().filter(|n| !n.kinematic).count(),
            substeps_last_frame: self.frame_steps,
            simulation_steps: self.simulation_steps,
            recoveries: self.recovery_count,
            last_frame_ms: self.frame_ms as f32,
            fit_rmse: self.fit.rmse as f32,
        }
    }

    pub fn coverage(&self) -> ClothCoverage {
        let fe = &self.fe;
        let mut c = ClothCoverage {
            rods: fe.rods.len(),
            animated_rods: fe.animated_rods,
            twists: fe.twists.len(),
            kelager_bends: fe.kelager_bends.len(),
            hinge_limits: fe.hinge_limits.len(),
            triangles: fe.triangles,
            quads: fe.quads,
            rope_chains: fe.rope_chains.len(),
            pending_rope_chains: fe.rope_count.saturating_sub(fe.rope_chains.len()),
            pending_jiggle_bones: fe.jiggle_bones.len(),
            decode_issues: fe.decode_issues.clone(),
            feature_gaps: fe.feature_gaps.clone(),
            ..Default::default()
        };
        for (i, node) in fe.nodes.iter().enumerate() {
            if !node.pinned {
                match integrator_mode(fe, i) {
                    Mode::GoalDamped => c.goal_damped += 1,
                    Mode::Raw => c.raw += 1,
                    Mode::Unknown => c.unknown += 1,
                }
            }
        }
        c
    }

    /// Nodes and colliders in the glb frame (Grimoire's `showNodes` and `showColliders`).
    pub fn debug(&self) -> ClothDebug {
        let to = |p: V3| {
            let w = m4_point(&self.fit.matrix, p);
            [w[0] as f32, w[1] as f32, w[2] as f32]
        };
        let k = self.fit.scale.abs();
        let cs = self.tuning.collision_scale;
        let mut out = ClothDebug { nodes: self.nodes.iter().map(|n| (to(n.pos), n.kinematic)).collect(), ..Default::default() };
        for c in &self.colliders {
            match c {
                Collider::Capsule { node, sphere0, sphere1, .. } => {
                    let a = &self.nodes[*node];
                    let p0 = add(a.pos, q_rot(a.solved_rot, [sphere0[0], sphere0[1], sphere0[2]]));
                    let p1 = add(a.pos, q_rot(a.solved_rot, [sphere1[0], sphere1[1], sphere1[2]]));
                    out.capsules.push((to(p0), (sphere0[3] * cs * k) as f32, to(p1), (sphere1[3] * cs * k) as f32));
                }
                Collider::Box { node, pos, rot, half_size, .. } => {
                    let a = &self.nodes[*node];
                    let center = to(add(a.pos, q_rot(a.solved_rot, *pos)));
                    let q = q_norm(q_mul(self.fit.rotation, q_norm(q_mul(a.solved_rot, q_norm(*rot)))));
                    let h = half_size.map(|v| (v.abs() * cs * k) as f32);
                    out.boxes.push(Mat4::from_trs(center, [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32], h));
                }
                Collider::Plane(_) => {}
            }
        }
        out
    }

    /// Whether glb node `i` is a skin joint (a three.js `Bone`).
    fn is_joint(&self, i: usize) -> bool {
        self.is_bone[i]
    }

    fn iteration_phases(&self) -> (usize, usize) {
        let extra = self.fe.extra_iterations;
        let goal_extra = self.fe.extra_goal_iterations;
        let o = self.tuning.iteration_override as f64;
        let base = if o > 0.0 { o } else { extra + 1.0 };
        let constraint = clamp(js_round(base), 1.0, 256.0);
        let goal = clamp(js_round(goal_extra + 1.0), 1.0, constraint);
        (constraint as usize, goal as usize)
    }

    fn reset_history(&mut self) {
        self.warm_started = false;
        self.last_substep_dt = None;
        self.collider_prev.fill(None);
        self.recovery_count += 1;
    }

    fn refresh_targets(&mut self, locals: &[Mat4]) {
        // A malformed animation sample must not become the next clean pose.
        if locals.len() == self.locals.len() {
            for (clean, sample) in self.locals.iter_mut().zip(locals) {
                if mat_finite(sample) {
                    *clean = *sample;
                }
            }
        }
        for &i in &self.needed {
            let local = m4_from(&self.locals[i]);
            self.world[i] = match self.parents[i] {
                Some(p) => m4_mul(&self.world[p], &local),
                None => local,
            };
        }
        for i in 0..self.nodes.len() {
            let Some(bone) = self.nodes[i].bone else {
                let node = &mut self.nodes[i];
                node.target = node.init_pos;
                node.target_rot = q_from_v4(node.init_rot);
                continue;
            };
            let w = &self.world[bone];
            let target = m4_point(&self.fit.inverse, [w[12], w[13], w[14]]);
            let rotation = q_norm(q_mul(self.root_to_model_rot, m4_rotation(w)));
            if !finite3(target) || !finite_rot(rotation) {
                continue;
            }
            if self.warm_started && self.nodes[i].kinematic && dist(target, self.nodes[i].target) > self.teleport_distance {
                self.reset_history();
            }
            self.nodes[i].target = target;
            self.nodes[i].target_rot = rotation;
        }
        for o in &self.ctrl_offsets {
            let parent = &self.nodes[o.parent];
            let t = apply_offset(parent.target, parent.target_rot, o.offset, o.sign);
            self.nodes[o.child].target = t;
        }
        for o in &self.soft_offsets {
            let parent = &self.nodes[o.parent];
            let t = apply_offset(parent.target, parent.target_rot, o.offset, o.sign);
            // The serialized network is an ordered series of lerps: alpha keeps the previous
            // result, this parent's influence receives 1 - alpha.
            let child = &mut self.nodes[o.child];
            child.target = lerp3(child.target, t, 1.0 - clamp(o.alpha, 0.0, 1.0));
        }
        for base in &self.fe.node_bases {
            let q = node_base_quat(|i| self.nodes[i].target, base);
            self.nodes[base.node].target_rot = q;
        }
    }

    fn update_animated_rod_lengths(&mut self) {
        for batch in &mut self.animated_rod_batches {
            for rod in batch {
                let length = dist_sq(self.nodes[rod.a].target, self.nodes[rod.b].target).max(EPS30).sqrt();
                rod.min = length;
                rod.max = length;
            }
        }
    }

    fn warm_start(&mut self, dt: f64) {
        if self.warm_started {
            return;
        }
        for i in 0..self.nodes.len() {
            // Seed cloth at its rigid-anchor rest drape so it starts attached to the posed body.
            if let Some(a) = self.anchors[i] {
                let anchor = &self.nodes[a];
                let node = &self.nodes[i];
                let seed = rigid_anchor_seed(node.init_pos, anchor.init_pos, anchor.init_rot, anchor.target, anchor.target_rot);
                if finite3(seed) {
                    let node = &mut self.nodes[i];
                    node.pos = seed;
                    node.prev = seed;
                    node.solved_rot = node.target_rot;
                    continue;
                }
            }
            let node = &mut self.nodes[i];
            node.pos = node.target;
            node.prev = node.target;
            node.solved_rot = node.target_rot;
        }
        self.last_substep_dt = (dt.is_finite() && dt > 0.0).then_some(dt);
        self.warm_started = true;
    }

    fn integrate(&mut self, gravity: V3, dt: f64) {
        let dt2 = dt * dt;
        let last_dt = self.last_substep_dt;
        let gravity_scale = self.tuning.gravity_scale;
        for node in &mut self.nodes {
            if node.flags_kinematic() {
                node.pos = node.target;
                node.prev = node.target;
                if !self.twist_nodes[node.index] {
                    node.solved_rot = node.target_rot;
                }
                continue;
            }
            let damping = if node.mode == Mode::Unknown { node.damping.max(MIN_VELOCITY_DAMPING) } else { (node.damping * dt).max(0.0) };
            let velocity = scl(sub(node.pos, node.prev), verlet_velocity_scale(dt, last_dt, damping));
            node.prev = node.pos;
            node.pos = add(node.pos, velocity);
            node.pos = madd(node.pos, gravity, node.gravity * gravity_scale * dt2);
        }
        if dt.is_finite() && dt > 0.0 {
            self.last_substep_dt = Some(dt);
        }
    }

    fn solve_goals(&mut self, dt: f64) {
        let k = self.tuning.attraction_scale;
        for node in &mut self.nodes {
            if node.flags_kinematic() || node.mode == Mode::GoalDamped {
                continue;
            }
            if node.mode == Mode::Raw {
                raw_attraction(&mut node.pos, &mut node.prev, node.target, node.anim_force * k, node.anim_vertex * k, dt);
                continue;
            }
            let (blend, impulse) = animation_attraction(node.anim_vertex, node.anim_force, dt);
            let (pos, vel) = (blend * k, impulse * k);
            if pos <= 0.0 && vel <= 0.0 {
                continue;
            }
            // The position blend moves both Verlet buffers (no velocity change); the force term
            // moves only the current position (velocity toward the goal).
            let delta = sub(node.target, node.pos);
            node.prev = madd(node.prev, delta, pos);
            node.pos = madd(node.pos, delta, pos + vel);
        }
    }

    fn solve_goal_damped_nodes(&mut self) {
        let k = self.tuning.attraction_scale;
        for node in &mut self.nodes {
            if node.kinematic || node.mode != Mode::GoalDamped {
                continue;
            }
            goal_damped_attraction(&mut node.pos, &mut node.prev, node.target, node.anim_force * k, node.anim_vertex * k);
        }
    }

    fn solve_rods(&mut self) {
        if !self.fe.rod_batches.is_empty() {
            for batch in &self.fe.rod_batches {
                project_rod_batch(&mut self.nodes, batch);
            }
        } else {
            project_scalar_rods(&mut self.nodes, &self.fe.rods);
        }
        for batch in &self.animated_rod_batches {
            project_rod_batch(&mut self.nodes, batch);
        }
    }

    fn restore_pinned(&mut self) {
        for node in &mut self.nodes {
            if node.flags_kinematic() {
                node.pos = node.target;
                node.prev = node.target;
                node.solved_rot = node.target_rot;
            }
        }
    }

    fn collider_motion(&mut self, index: usize) -> M4 {
        if self.collider_stamp[index] == self.stamp {
            return self.collider_motion[index];
        }
        let parent = &self.nodes[index];
        let transform = m4_compose(parent.pos, parent.solved_rot, [1.0; 3]);
        let delta = match &self.collider_prev[index] {
            Some(previous) => m4_mul(&transform, &m4_invert(previous)),
            None => M4_ID,
        };
        self.collider_now[index] = transform;
        self.collider_motion[index] = delta;
        self.collider_stamp[index] = self.stamp;
        self.touched.push(index);
        delta
    }

    fn solve_collisions(&mut self) {
        self.stamp += 1;
        self.touched.clear();
        let friction = self.fe.has_collision_friction;
        let cs = self.tuning.collision_scale;
        for ci in 0..self.colliders.len() {
            match &self.colliders[ci] {
                Collider::Plane(plane) => {
                    let plane = *plane;
                    project_collision_plane(&mut self.nodes, &plane);
                }
                Collider::Capsule { node, sphere0, sphere1, .. } => {
                    let (node, s0, s1) = (*node, *sphere0, *sphere1);
                    let anchor = &self.nodes[node];
                    let a = add(anchor.pos, q_rot(anchor.solved_rot, [s0[0], s0[1], s0[2]]));
                    let b = add(anchor.pos, q_rot(anchor.solved_rot, [s1[0], s1[1], s1[2]]));
                    let (ra, rb) = (s0[3] * cs, s1[3] * cs);
                    let motion = self.collider_motion(node);
                    let Collider::Capsule { filter, .. } = &self.colliders[ci] else { unreachable!() };
                    for p in &mut self.nodes {
                        if p.kinematic || !filter.can_collide(p) {
                            continue;
                        }
                        let (depth, normal) = capsule_depth(p.pos, a, b, ra, rb, p.collide_radius.max(0.0), friction);
                        project_contact(&mut p.pos, p.prev, normal, depth, p.friction, &motion);
                    }
                }
                Collider::Box { node, pos, rot, half_size, .. } => {
                    let (node, pos, rot, half) = (*node, *pos, *rot, *half_size);
                    let anchor = &self.nodes[node];
                    let center = add(anchor.pos, q_rot(anchor.solved_rot, pos));
                    let rotation = q_norm(q_mul(anchor.solved_rot, q_norm(rot)));
                    let half = [half[0].abs() * cs, half[1].abs() * cs, half[2].abs() * cs];
                    let motion = self.collider_motion(node);
                    let Collider::Box { filter, .. } = &self.colliders[ci] else { unreachable!() };
                    for p in &mut self.nodes {
                        if p.kinematic || !filter.can_collide(p) {
                            continue;
                        }
                        let (depth, normal) = box_depth(p.pos, center, rotation, half, p.collide_radius.max(0.0), friction);
                        project_contact(&mut p.pos, p.prev, normal, depth, p.friction, &motion);
                    }
                }
            }
        }
        for &i in &self.touched {
            self.collider_prev[i] = Some(self.collider_now[i]);
        }
    }

    fn update_solved_rotations(&mut self) {
        for node in &mut self.nodes {
            if !self.twist_nodes[node.index] {
                node.solved_rot = node.target_rot;
            }
        }
        for base in &self.fe.node_bases {
            let q = node_base_quat(|i| self.nodes[i].pos, base);
            self.nodes[base.node].solved_rot = q;
        }
        for chain in &self.fe.rope_chains {
            reconstruct_rope(&mut self.nodes, chain, &self.rotation_nodes);
        }
        for link in &self.fe.twists {
            if self.twist_nodes[link.node_orient] {
                reconstruct_twist(&mut self.nodes, link);
            }
        }
    }

    /// The state half of Grimoire's `writeBack`: recovery, rotation reconstruction and the
    /// non-finite rotation fallback. The bone half runs in [`Cloth::apply`].
    fn write_back_state(&mut self) {
        let limit = 1e6f64.max(self.teleport_distance * 1024.0);
        // An emergency guard far beyond ordinary cloth motion, not a replacement for authored
        // constraints.
        let healthy = self.nodes.iter().all(|n| finite3(n.pos) && finite3(n.prev) && finite_rot(n.solved_rot) && dist_sq(n.pos, n.target) < limit * limit);
        if !healthy {
            self.reset_history();
            self.warm_start(CLOTH_TIMESTEP);
            for node in &mut self.nodes {
                node.last_solved = node.pos;
            }
        }
        self.update_solved_rotations();
        for node in &mut self.nodes {
            if !finite_rot(node.solved_rot) {
                node.solved_rot = node.target_rot;
            }
        }
    }
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Node as GNode, Skin};
    use serde_json::json;
    use std::cell::Cell;

    // An allocation counter for this thread only, so the per-substep guarantee is checkable.
    struct Counting;
    std::thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }
    unsafe impl std::alloc::GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            let _ = COUNTING.try_with(|on| {
                if on.get() {
                    let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
                }
            });
            unsafe { std::alloc::System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            unsafe { std::alloc::System.dealloc(ptr, layout) }
        }
    }
    #[global_allocator]
    static GLOBAL: Counting = Counting;

    const DT: f32 = CLOTH_TIMESTEP as f32;

    fn v(x: f64, y: f64, z: f64) -> V3 {
        [x, y, z]
    }
    fn a3(v: &Value) -> V3 {
        [v[0].as_f64().unwrap(), v[1].as_f64().unwrap(), v[2].as_f64().unwrap()]
    }
    fn a4(v: &Value, at: usize) -> Q4 {
        [v[at].as_f64().unwrap(), v[at + 1].as_f64().unwrap(), v[at + 2].as_f64().unwrap(), v[at + 3].as_f64().unwrap()]
    }
    fn pts(v: &Value) -> Vec<V3> {
        v.as_array().unwrap().iter().map(a3).collect()
    }
    fn free_nodes(positions: &[V3]) -> Vec<Node> {
        positions.iter().enumerate().map(|(i, p)| Node::free(i, *p)).collect()
    }
    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| i.to_string()).collect()
    }
    fn q_angle(a: Q4, b: Q4) -> f64 {
        2.0 * q_dot(q_norm(a), q_norm(b)).abs().min(1.0).acos()
    }
    fn axis_angle(axis: V3, angle: f64) -> Q4 {
        let a = norm(axis);
        let (s, c) = (angle / 2.0).sin_cos();
        [a[0] * s, a[1] * s, a[2] * s, c]
    }
    /// three's `setFromEuler` in XYZ order.
    fn euler(x: f64, y: f64, z: f64) -> Q4 {
        let (s1, c1) = (x / 2.0).sin_cos();
        let (s2, c2) = (y / 2.0).sin_cos();
        let (s3, c3) = (z / 2.0).sin_cos();
        [s1 * c2 * c3 + c1 * s2 * s3, c1 * s2 * c3 - s1 * c2 * s3, c1 * c2 * s3 + s1 * s2 * c3, c1 * c2 * c3 - s1 * s2 * s3]
    }
    fn fe(value: Value) -> FeModel {
        parse_fe_model(&value).expect("a FeModel")
    }
    fn reference() -> Value {
        serde_json::from_str(REFERENCE).unwrap()
    }

    // --- parser ---------------------------------------------------------------------------

    #[test]
    fn rejects_malformed_containers() {
        for field in ["m_NodeInvMasses", "m_Rods", "m_SimdRods", "m_NodeBases", "m_CtrlOffsets", "m_InitPose", "m_CollisionPlanes"] {
            let mut raw = json!({ "m_CtrlName": ["a"] });
            raw[field] = json!({});
            assert!(parse_fe_model(&raw).is_none(), "{field}");
        }
        for field in ["m_Rods", "m_SimdTris", "m_NodeBases", "m_CtrlOffsets", "m_SphereRigids", "m_FitMatrices"] {
            let mut raw = json!({ "m_CtrlName": ["a"] });
            raw[field] = json!([null]);
            assert!(parse_fe_model(&raw).is_none(), "{field}");
        }
        let many: Vec<&str> = vec!["a"; 16385];
        assert!(parse_fe_model(&json!({ "m_CtrlName": many })).is_none());
        let rods: Vec<Value> = (0..262_145).map(|_| json!({ "nNode": [0, 0] })).collect();
        assert!(parse_fe_model(&json!({ "m_CtrlName": ["a"], "m_Rods": rods })).is_none());
        assert!(parse_fe_model(&json!({ "m_CtrlName": ["a", 3] })).is_none());
        assert!(parse_fe_model(&json!({ "nodes": [] })).is_none());
        assert!(parse_fe_model(&json!([1, 2])).is_none());
        // Null containers count as absent.
        assert!(parse_fe_model(&json!({ "m_CtrlName": ["a"], "m_Rods": null })).is_some());
        let model = Model::default();
        assert!(Cloth::new(&model, "{ not json").is_err());
        assert!(Cloth::new(&model, "{\"m_Rods\": []}").is_err());
        assert!(matches!(Cloth::new(&model, "{\"m_CtrlName\": [\"a\"]}"), Ok(None)));
    }

    #[test]
    fn reports_invalid_records_instead_of_failing() {
        let m = fe(json!({
            "m_CtrlName": ["anchor", "spare_x", "spare_y"],
            "m_InitPose": [[0, 0, 0], [1, 0, 0], [0, 1, 0]],
            "m_Rods": [{ "nNode": [-1, 1] }, { "nNode": [0, 1.5] }],
            "m_NodeBases": [{ "nNode": 0, "nNodeX0": 0, "nNodeX1": 400, "nNodeY0": 0, "nNodeY1": 1 }]
        }));
        assert_eq!(m.decode_issues.len(), 3);
        assert!(m.rods.is_empty() && m.node_bases.is_empty());
        assert_eq!(m.nodes[1].init_rot, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_malformed_packed_rod_block_falls_back_to_scalar_rods() {
        let good = json!({ "nNode": [[0, 0, 0, 0], [1, 1, 1, 1]], "f4MinDist": [1, 1, 1, 1], "f4MaxDist": [2, 2, 2, 2], "f4Weight0": [0.5, 0.5, 0.5, 0.5], "f4RelaxationFactor": [1, 1, 1, 1] });
        let m = fe(json!({ "m_CtrlName": ["a", "b"], "m_SimdRods": [good.clone(), good.clone()] }));
        assert_eq!(m.rod_batches.len(), 2);
        assert_eq!(m.rod_batches[0].len(), 4);
        let mut bad = good.clone();
        bad["f4MaxDist"] = json!([0.5, 2, 2, 2]);
        let m = fe(json!({ "m_CtrlName": ["a", "b"], "m_SimdRods": [good, bad], "m_Rods": [{ "nNode": [0, 1] }] }));
        assert!(m.rod_batches.is_empty());
        assert_eq!(m.rods.len(), 1);
        assert_eq!(m.decode_issues, vec![DecodeIssue { array: "m_SimdRods", record: 1, reason: "invalid-limits" }]);
    }

    #[test]
    fn decodes_rope_chains_and_rejects_bad_offsets() {
        let m = fe(json!({ "m_CtrlName": names(5), "m_nRopeCount": 2, "m_Ropes": [4, 7, 0, 1, 2, 3, 4] }));
        assert_eq!(m.rope_chains, vec![vec![0, 1], vec![2, 3, 4]]);
        let m = fe(json!({ "m_CtrlName": names(5), "m_nRopeCount": 2, "m_Ropes": [3, 7, 0, 1, 2, 3, 4] }));
        assert!(m.rope_chains.is_empty());
        assert_eq!(m.decode_issues[0].reason, "invalid-offsets");
        let m = fe(json!({ "m_CtrlName": names(5), "m_nRopeCount": 0, "m_Ropes": [1] }));
        assert_eq!(m.decode_issues[0].reason, "invalid-count");
    }

    #[test]
    fn selects_integrator_modes() {
        let mut m = fe(json!({ "m_CtrlName": names(4), "m_nStaticNodes": 1, "m_GoalDampedSpringIntegrators": [5] }));
        assert_eq!([1, 2, 3].map(|i| integrator_mode(&m, i)), [Mode::GoalDamped, Mode::Raw, Mode::GoalDamped]);
        m.goal_damped_spring_integrators.clear();
        m.dynamic_node_flags = Some(0x200);
        assert_eq!(integrator_mode(&m, 2), Mode::Raw);
        m.dynamic_node_flags = Some(0x280);
        assert_eq!(integrator_mode(&m, 2), Mode::Unknown);
        m.dynamic_node_flags = Some(0);
        assert_eq!(integrator_mode(&m, 2), Mode::GoalDamped);
        m.dynamic_node_flags = None;
        assert_eq!(integrator_mode(&m, 2), Mode::Unknown);
        let bad = fe(json!({ "m_CtrlName": names(2), "m_GoalDampedSpringIntegrators": [-1] }));
        assert_eq!(bad.decode_issues[0].reason, "invalid-bitset");
    }

    #[test]
    fn folds_per_node_masks_and_dynamic_slot_radii() {
        let m = fe(json!({
            "m_CtrlName": names(3),
            "m_NodeInvMasses": [0, 1, 1],
            "m_NodeCollisionRadii": [0.5, 0.75],
            "m_DynNodeFriction": [0.1, 0.2],
            "m_TreeCollisionMasks": [2, 4, 6]
        }));
        assert_eq!((m.nodes[0].collision_mask, m.nodes[1].collision_mask, m.nodes[2].collision_mask), (65535.0, 2.0, 4.0));
        assert_eq!((m.nodes[1].collide_radius, m.nodes[2].friction), (0.5, 0.2));
        assert!(m.has_collision_friction);
        // A leaf layout that does not match collides with everything.
        let m = fe(json!({ "m_CtrlName": names(2), "m_NodeInvMasses": [1, 1], "m_TreeCollisionMasks": [2, 4] }));
        assert_eq!(m.nodes[1].collision_mask, 65535.0);
    }

    // --- kernels --------------------------------------------------------------------------

    #[test]
    fn rod_batches_ignore_padding_and_gather_before_scatter() {
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0)]);
        nodes[0].kinematic = true;
        let rod = Rod { a: 0, b: 1, min: 2.0, max: 2.0, relax: 0.5, weight: 0.0 };
        project_rod_batch(&mut nodes, &[rod; 4]);
        assert_eq!(nodes[1].pos[0], 6.0);
        project_rod_batch(&mut nodes, &[rod; 4]);
        assert_eq!(nodes[1].pos[0], 4.0);
        assert_eq!(nodes[0].pos[0], 0.0);

        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(8.0, 0.0, 0.0)]);
        let first = Rod { a: 0, b: 1, min: 2.0, max: 2.0, relax: 1.0, weight: 0.5 };
        let second = Rod { a: 1, b: 2, ..first };
        project_rod_batch(&mut nodes, &[first, second]);
        assert_eq!(nodes.iter().map(|n| n.pos[0]).collect::<Vec<_>>(), vec![1.0, 3.0, 7.0]);
    }

    #[test]
    fn scalar_rods_share_by_weight() {
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0)]);
        project_scalar_rods(&mut nodes, &[Rod { a: 0, b: 1, min: 0.0, max: 2.0, relax: 1.0, weight: 0.0 }]);
        assert_eq!((nodes[0].pos[0], nodes[1].pos[0]), (0.0, 2.0));
        project_scalar_rods(&mut nodes, &[Rod { a: 0, b: 1, min: 3.0, max: 0.0, relax: 1.0, weight: 0.5 }]);
        assert_eq!((nodes[0].pos[0], nodes[1].pos[0]), (-0.5, 2.5));
    }

    #[test]
    fn goal_attraction_matches_the_runtime_reference() {
        for case in reference()["goal"].as_array().unwrap() {
            let (mut p, mut h) = (a3(&case["position"]), a3(&case["previous"]));
            let (f, vtx) = (case["force"].as_f64().unwrap(), case["vertex"].as_f64().unwrap());
            goal_damped_attraction(&mut p, &mut h, a3(&case["goal"]), f, vtx);
            assert!(dist(p, a3(&case["expected"]["position"])) < 2e-6, "{case}");
            assert!(dist(h, a3(&case["expected"]["previous"])) < 2e-6, "{case}");
        }
        let (mut p, mut h) = (v(8.0, 0.0, 0.0), v(6.0, 0.0, 0.0));
        goal_damped_attraction(&mut p, &mut h, [0.0; 3], 0.25, 0.5);
        assert_eq!((p[0], h[0]), (6.0, 6.0));
        let (mut p, mut h) = (v(8.0, 0.0, 0.0), v(6.0, 0.0, 0.0));
        goal_damped_attraction(&mut p, &mut h, [0.0; 3], 0.0, 1.0);
        assert_eq!((p[0], h[0]), (8.0, 8.0));
        goal_damped_attraction(&mut p, &mut h, v(2.0, 0.0, 0.0), 1.0, 0.0);
        assert_eq!((p[0], h[0]), (2.0, 2.0));
        // An undriven displaced particle settles without the preview damping floor.
        let (mut p, mut h) = (v(8.0, 0.0, 0.0), v(8.0, 0.0, 0.0));
        for _ in 0..1200 {
            let velocity = sub(p, h);
            h = p;
            p = add(p, velocity);
            goal_damped_attraction(&mut p, &mut h, [0.0; 3], 0.05, 0.3);
        }
        assert!(len(p) < 1e-8 && dist(p, h) < 1e-8);
    }

    #[test]
    fn raw_attraction_blends_both_buffers() {
        let (mut p, mut h) = ([0.0; 3], [0.0; 3]);
        raw_attraction(&mut p, &mut h, v(10.0, 0.0, 0.0), 0.0, 30.0, 1.0 / 60.0);
        assert_eq!((p[0], h[0]), (5.0, 2.5));
        let (mut d, mut dh) = ([0.0; 3], [0.0; 3]);
        goal_damped_attraction(&mut d, &mut dh, v(10.0, 0.0, 0.0), 0.0, 30.0);
        assert_eq!(d[0], 0.0);
        let (mut p, mut h) = ([0.0; 3], [0.0; 3]);
        raw_attraction(&mut p, &mut h, v(10.0, 0.0, 0.0), 3.0, 0.0, 1.0 / 60.0);
        assert!((p[0] - 1.0).abs() < 1e-12 && h[0] == 0.0);
    }

    #[test]
    fn verlet_scale_and_legacy_attraction() {
        assert_eq!(verlet_velocity_scale(0.0, Some(1.0), 0.0), 0.0);
        assert_eq!(verlet_velocity_scale(1.0, None, 0.0), 0.0);
        assert_eq!(verlet_velocity_scale(0.02, Some(0.01), 0.0), 2.0);
        assert_eq!(verlet_velocity_scale(0.04, Some(0.001), 0.0), 4.0);
        assert_eq!(verlet_velocity_scale(0.01, Some(0.01), 0.25), 0.75);
        assert_eq!(animation_attraction(4.0, 3.0, 0.5), (1.0, 3.0));
        assert_eq!(animation_attraction(-1.0, -1.0, 0.5), (0.0, 0.0));
    }

    #[test]
    fn kelager_bends_straighten_and_respect_anchors() {
        let bend = KelagerBend { node: [0, 1, 2], weight: [-1.0, 0.5, 0.5], height0: 0.0 };
        let make = || free_nodes(&[v(0.0, 1.0, 0.0), v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0)]);
        let mut p = make();
        project_kelager_bend(&mut p, &bend);
        for n in &p {
            assert!((n.pos[1] - 1.0 / 3.0).abs() < 1e-12);
        }
        assert!((dist(p[1].pos, p[2].pos) - 2.0).abs() < 1e-12);
        let mut p = make();
        project_kelager_bend(&mut p, &KelagerBend { height0: 1.0, ..bend });
        assert_eq!(p.iter().map(|n| n.pos).collect::<Vec<_>>(), vec![v(0.0, 1.0, 0.0), v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0)]);
        let mut p = free_nodes(&[v(0.0, 0.0, 0.0), v(-1.0, 1.0, 0.0), v(1.0, 0.0, 0.0)]);
        p[0].kinematic = true;
        p[2].kinematic = true;
        project_kelager_bend(&mut p, &KelagerBend { weight: [0.0, 3.0, 0.0], ..bend });
        assert_eq!(p.iter().map(|n| n.pos).collect::<Vec<_>>(), vec![v(0.0, 0.0, 0.0), v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0)]);
        let mut collapsed = free_nodes(&[[0.0; 3]; 3]);
        project_kelager_bend(&mut collapsed, &bend);
        assert!(collapsed.iter().all(|n| len(n.pos) == 0.0));
    }

    #[test]
    fn hinge_limits_match_the_runtime_reference() {
        for case in reference()["hinge"].as_array().unwrap() {
            let positions = pts(&case["positions"]);
            let model = fe(json!({ "m_CtrlName": names(positions.len()), "m_HingeLimits": [case["hinge"].clone()] }));
            let mut nodes = free_nodes(&positions);
            for (i, m) in case["invMasses"].as_array().unwrap().iter().enumerate() {
                nodes[i].inv_mass = m.as_f64().unwrap();
                nodes[i].kinematic = nodes[i].inv_mass == 0.0;
            }
            project_hinge_limit(&mut nodes, &model.hinge_limits[0]);
            for (i, e) in pts(&case["expected"]).iter().enumerate() {
                assert!(dist(nodes[i].pos, *e) < 1e-6, "{case}: node {i} {:?}", nodes[i].pos);
                if nodes[i].kinematic {
                    assert_eq!(nodes[i].pos, positions[i]);
                }
            }
        }
    }

    #[test]
    fn hinge_records_reject_flags_and_bad_limits() {
        let hinge = |extra: Value| {
            let mut h = json!({ "nNode": [0, 1, 2, 3, 4, 5], "flWeight4": 0, "flWeight5": 0, "flAngleCenter": 0, "flAngleExtents": 0.5 });
            for (k, val) in extra.as_object().unwrap() {
                h[k] = val.clone();
            }
            fe(json!({ "m_CtrlName": names(6), "m_HingeLimits": [h] })).decode_issues.first().map(|i| i.reason)
        };
        assert_eq!(hinge(json!({})), None);
        assert_eq!(hinge(json!({ "nFlags": 1 })), Some("unsupported-flags"));
        assert_eq!(hinge(json!({ "flWeight4": 2 })), Some("invalid-weights"));
        assert_eq!(hinge(json!({ "flAngleExtents": 4 })), Some("invalid-limits"));
        assert_eq!(hinge(json!({ "nNode": [0, 1, 2, 3, 4] })), Some("invalid-nodes"));
    }

    #[test]
    fn triangles_match_the_runtime_reference() {
        for case in reference()["triangle"].as_array().unwrap() {
            let positions = pts(&case["positions"]);
            let batches = case["batches"].as_array().unwrap();
            let lanes = |b: &Value, f: &dyn Fn(&Value) -> Value| Value::Array(b.as_array().unwrap().iter().map(f).collect());
            let packed: Vec<Value> = batches
                .iter()
                .map(|b| {
                    json!({
                        "nNode": (0..3).map(|axis| lanes(b, &|t| t["nNode"][axis].clone())).collect::<Vec<_>>(),
                        "w1": lanes(b, &|t| t["w1"].clone()),
                        "w2": lanes(b, &|t| t["w2"].clone()),
                        "v1x": lanes(b, &|t| t["v1x"].clone()),
                        "v2": { "x": lanes(b, &|t| t["v2"][0].clone()), "y": lanes(b, &|t| t["v2"][1].clone()) }
                    })
                })
                .collect();
            let counts: Vec<u64> = case["staticCounts"].as_array().unwrap().iter().map(|c| c.as_u64().unwrap()).collect();
            let model = fe(json!({
                "m_CtrlName": names(positions.len()), "m_SimdTris": packed,
                "m_nSimdTriCount1": counts.iter().filter(|&&c| c >= 1).count(),
                "m_nSimdTriCount2": counts.iter().filter(|&&c| c == 2).count()
            }));
            assert!(model.decode_issues.is_empty(), "{:?}", model.decode_issues);
            let mut nodes = free_nodes(&positions);
            let scale = case["scale"].as_f64().unwrap();
            for batch in &model.triangle_batches {
                project_triangle_batch(&mut nodes, batch, scale);
            }
            for (i, e) in pts(&case["expected"]).iter().enumerate() {
                assert!(dist(nodes[i].pos, *e) < 1e-5, "{case}: node {i} {:?}", nodes[i].pos);
            }
        }
        // Animation-owned nodes stay put even in a dynamic partition.
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(1.0, 3.0, 0.0)]);
        nodes[0].kinematic = true;
        let t = Triangle { node: [0, 1, 2], static_count: 0, weight1: 0.3, weight2: 0.4, x1: 2.0, x2: 1.0, y2: 1.0 };
        let projected = triangle_projection(&nodes, &t, 1.0);
        assert!((1..3).map(|i| dist(nodes[i].pos, projected[i])).fold(0.0, f64::max) > 0.5);
        project_triangle_batch(&mut nodes, &[t], 1.0);
        assert_eq!(nodes[0].pos, [0.0; 3]);
    }

    #[test]
    fn quads_match_the_runtime_reference() {
        for case in reference()["quad"].as_array().unwrap() {
            let positions = pts(&case["positions"]);
            let lanes = |b: &Value, f: &dyn Fn(&Value) -> Value| -> Vec<Value> { b.as_array().unwrap().iter().map(f).collect() };
            let packed: Vec<Value> = case["batches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| {
                    json!({
                        "nNode": (0..4).map(|vtx| lanes(b, &|q| q["nNode"][vtx].clone())).collect::<Vec<_>>(),
                        "f4Slack": lanes(b, &|q| q["flSlack"].clone()),
                        "vShape": (0..4).map(|vtx| (0..3).flat_map(|axis| lanes(b, &|q| q["vShape"][vtx][axis].clone())).collect::<Vec<_>>()).collect::<Vec<_>>(),
                        "f4Weights": (0..4).map(|vtx| lanes(b, &|q| q["vShape"][vtx][3].clone())).collect::<Vec<_>>()
                    })
                })
                .collect();
            let counts: Vec<u64> = case["staticCounts"].as_array().unwrap().iter().map(|c| c.as_u64().unwrap()).collect();
            let model = fe(json!({
                "m_CtrlName": names(positions.len()), "m_SimdQuads": packed,
                "m_nSimdQuadCount1": counts.iter().filter(|&&c| c >= 1).count(),
                "m_nSimdQuadCount2": counts.iter().filter(|&&c| c == 2).count()
            }));
            assert!(model.decode_issues.is_empty() && model.feature_gaps.is_empty(), "{:?}", model.decode_issues);
            let mut nodes = free_nodes(&positions);
            let (scale, relaxation) = (case["scale"].as_f64().unwrap(), case["relaxation"].as_f64().unwrap());
            for batch in &model.quad_batches {
                project_quad_batch(&mut nodes, batch, scale, relaxation);
            }
            for (i, e) in pts(&case["expected"]).iter().enumerate() {
                assert!(dist(nodes[i].pos, *e) < 1e-5, "{case}: node {i} {:?}", nodes[i].pos);
            }
        }
        // Fixed and animation-owned corners stay put.
        let mut nodes = free_nodes(&[v(-1.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(-1.0, 4.0, 0.0), v(1.0, 3.0, 0.0)]);
        nodes[2].kinematic = true;
        let q = Quad { node: [0, 1, 2, 3], static_count: 2, slack: 0.0, shape: [[-1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [-1.0, 2.0, 0.0, 0.5], [1.0, 2.0, 0.0, 0.5]] };
        let projected = quad_projection(&nodes, &q, 1.0, 1.0);
        assert!((dist(nodes[3].pos, projected[3]) - 1.0).abs() < 1e-9);
        project_quad_batch(&mut nodes, &[q], 1.0, 1.0);
        assert_eq!(nodes[2].pos, v(-1.0, 4.0, 0.0));
        assert_eq!(nodes[3].pos, v(1.0, 2.0, 0.0));
    }

    #[test]
    fn stray_radii_match_the_runtime_reference() {
        for case in reference()["stray"].as_array().unwrap() {
            let positions = pts(&case["positions"]);
            let mut nodes = free_nodes(&positions);
            for (n, t) in nodes.iter_mut().zip(pts(&case["targets"])) {
                n.target = t;
            }
            let scale = case["scale"].as_f64().unwrap();
            for batch in case["batches"].as_array().unwrap() {
                let radii: Vec<StrayRadius> = batch
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|l| StrayRadius {
                        node: [l["nNode"][0].as_u64().unwrap() as usize, l["nNode"][1].as_u64().unwrap() as usize],
                        max_dist: l["flMaxDist"].as_f64().unwrap(),
                        relax: l["flRelaxationFactor"].as_f64().unwrap(),
                    })
                    .collect();
                project_stray_radius_batch(&mut nodes, &radii, scale);
            }
            for (i, e) in pts(&case["expected"]).iter().enumerate() {
                assert!(dist(nodes[i].pos, *e) < 2e-6, "{case}");
                assert_eq!(nodes[i].prev, positions[i]);
            }
        }
        let one = |pos: V3, max: f64, relax: f64, pinned: bool| {
            let mut nodes = free_nodes(&[pos]);
            nodes[0].target = [0.0; 3];
            nodes[0].pinned = pinned;
            let changed = project_stray_radius_batch(&mut nodes, &[StrayRadius { node: [0, 0], max_dist: max, relax }], 1.0);
            (changed, nodes[0].pos[0])
        };
        assert_eq!(one(v(3.0, 0.0, 0.0), 4.0, 1.0, false), (false, 3.0));
        let (changed, x) = one(v(10.0, 0.0, 0.0), 4.0, 1.0, false);
        assert!(changed && (x - 4.0).abs() < 1e-9);
        assert!((one(v(10.0, 0.0, 0.0), 4.0, 0.25, false).1 - 8.5).abs() < 1e-9);
        assert_eq!(one(v(10.0, 0.0, 0.0), 4.0, 1.0, true), (false, 10.0));
        let mut nodes = free_nodes(&[v(10.0, 0.0, 0.0)]);
        assert!(!project_stray_radius_batch(&mut nodes, &[StrayRadius { node: [5, 5], max_dist: 4.0, relax: 1.0 }], 1.0));
    }

    #[test]
    fn collision_planes_push_along_the_rotated_normal() {
        let plane = |parent: usize, normal: V3, offset: f64, strength: f64| PlaneRt { priority: 0, ctrl_parent: parent, child_node: 1, normal, offset, strength };
        let mut nodes = free_nodes(&[v(10.0, 0.0, 0.0), v(10.0, 1.75, 0.0)]);
        nodes[0].solved_rot = euler(0.0, 0.0, std::f64::consts::FRAC_PI_2);
        nodes[1].prev = v(9.5, 1.5, 0.0);
        assert!(project_collision_plane(&mut nodes, &plane(0, v(1.0, 0.0, 0.0), 2.0, 1.0)));
        assert!(dist(nodes[1].pos, v(10.0, 2.0, 0.0)) < 1e-9);
        assert_eq!(nodes[1].prev, v(9.5, 1.5, 0.0));
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(0.0, -1.0, 0.0)]);
        assert!(project_collision_plane(&mut nodes, &plane(0, v(0.0, 1.0, 0.0), 0.0, 0.25)));
        assert!((nodes[1].pos[1] + 0.75).abs() < 1e-9);
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(0.0, -1.0, 0.0)]);
        project_collision_plane(&mut nodes, &plane(0, v(0.0, 1.0, 0.0), 0.0, 5.0));
        assert!(nodes[1].pos[1].abs() < 1e-9);
        let mut nodes = free_nodes(&[v(0.0, 0.0, 0.0), v(0.0, -1.0, 0.0)]);
        assert!(!project_collision_plane(&mut nodes, &plane(9, v(0.0, 1.0, 0.0), 0.0, 1.0)));
        assert!(!project_collision_plane(&mut nodes, &plane(0, [0.0; 3], 0.0, 1.0)));
        nodes[1].pinned = true;
        assert!(!project_collision_plane(&mut nodes, &plane(0, v(0.0, 1.0, 0.0), 0.0, 1.0)));
        assert_eq!(nodes[1].pos, v(0.0, -1.0, 0.0));
    }

    fn push_capsule(p: V3, a: V3, b: V3, ra: f64, rb: f64, pr: f64) -> (bool, V3) {
        let (depth, n) = capsule_depth(p, a, b, ra, rb, pr, false);
        if depth <= 0.0 { (false, p) } else { (true, madd(p, n, depth)) }
    }

    #[test]
    fn capsules_push_points_out() {
        let o = [0.0; 3];
        let (hit, p) = push_capsule(v(1.0, 0.0, 0.0), o, o, 5.0, 5.0, 0.0);
        assert!(hit && (len(p) - 5.0).abs() < 1e-4);
        assert_eq!(push_capsule(v(20.0, 0.0, 0.0), o, o, 5.0, 5.0, 0.0), (false, v(20.0, 0.0, 0.0)));
        assert!((len(push_capsule(v(5.0, 0.0, 0.0), o, o, 5.0, 5.0, 1.0).1) - 6.0).abs() < 1e-4);
        let (_, p) = push_capsule(v(5.0, 1.0, 0.0), o, v(10.0, 0.0, 0.0), 1.0, 5.0, 0.0);
        assert!((p[0] - 4.226_405_462_721_032).abs() < 1e-8 && (p[1] - 2.933_986_343_197_419).abs() < 1e-8 && p[2] == 0.0);
        assert_eq!(push_capsule(v(5.0, 0.0, 0.0), o, v(10.0, 0.0, 0.0), 1.0, 5.0, 0.0), (true, v(5.0, 0.0, 3.0)));
        let (_, p) = push_capsule(v(0.0, 1.0, 0.0), o, v(1.0, 0.0, 0.0), 1.0, 3.0, 0.0);
        assert!((dist(p, v(1.0, 0.0, 0.0)) - 3.0).abs() < 1e-8);
        assert_eq!(push_capsule(v(5.0, 1.0, 0.0), o, v(10.0, 0.0, 0.0), 3.0, 3.0, 0.0).1, v(5.0, 3.0, 0.0));
    }

    fn push_box(p: V3, rotation: Q4, half: V3, pr: f64) -> (bool, V3) {
        let (depth, n) = box_depth(p, [0.0; 3], rotation, half, pr, false);
        if depth <= 0.0 { (false, p) } else { (true, madd(p, n, depth)) }
    }

    #[test]
    fn boxes_push_points_out() {
        assert_eq!(push_box(v(1.5, 0.0, 0.0), Q_ID, v(2.0, 3.0, 4.0), 0.0), (true, v(2.0, 0.0, 0.0)));
        assert_eq!(push_box(v(5.0, 0.0, 0.0), Q_ID, v(2.0, 3.0, 4.0), 1.0), (false, v(5.0, 0.0, 0.0)));
        assert_eq!(push_box(v(2.5, 0.0, 0.0), Q_ID, v(2.0, 3.0, 4.0), 1.0), (true, v(3.0, 0.0, 0.0)));
        let (hit, p) = push_box(v(0.0, 0.5, 0.0), euler(0.0, 0.0, std::f64::consts::FRAC_PI_2), v(1.0, 3.0, 4.0), 0.0);
        assert!(hit && dist(p, v(0.0, 1.0, 0.0)) < 1e-4);
    }

    fn transform(values: &Value) -> M4 {
        m4_compose(a3(values), a4(values, 4), [1.0; 3])
    }

    #[test]
    fn contacts_match_the_runtime_reference() {
        let r = reference();
        for case in r["friction"].as_array().unwrap() {
            let now = transform(&case["current"]);
            let motion = if case["old"].is_null() { M4_ID } else { m4_mul(&now, &m4_invert(&transform(&case["old"]))) };
            let cap = &case["capsule"];
            let (a, b) = (m4_point(&now, a3(&cap[0])), m4_point(&now, a3(&cap[1])));
            let (mut p, h) = (a3(&case["position"]), a3(&case["previous"]));
            let fr = case["frictionEnabled"].as_bool().unwrap();
            let (depth, n) = capsule_depth(p, a, b, cap[0][3].as_f64().unwrap(), cap[1][3].as_f64().unwrap(), case["radius"].as_f64().unwrap(), fr);
            project_contact(&mut p, h, n, depth, case["friction"].as_f64().unwrap(), &motion);
            assert!(dist(p, a3(&case["expected"])) < 2e-6, "{case}: {p:?}");
        }
        for case in r["box_sphere"].as_array().unwrap() {
            let now = transform(&case["current"]);
            let motion = if case["old"].is_null() { M4_ID } else { m4_mul(&now, &m4_invert(&transform(&case["old"]))) };
            let (mut p, h) = (a3(&case["position"]), a3(&case["previous"]));
            let fr = case["frictionEnabled"].as_bool().unwrap();
            let radius = case["radius"].as_f64().unwrap();
            let (depth, n) = if case["kind"] == "sphere" {
                let c = m4_point(&now, a3(&case["sphere"]));
                let r = case["sphere"][3].as_f64().unwrap();
                capsule_depth(p, c, c, r, r, radius, fr)
            } else {
                let frame = m4_mul(&now, &transform(&case["frame"]));
                box_depth(p, [frame[12], frame[13], frame[14]], m4_rotation(&frame), a3(&case["size"]), radius, fr)
            };
            let friction = if fr { case["friction"].as_f64().unwrap() } else { 0.0 };
            project_contact(&mut p, h, n, depth, friction, &motion);
            assert!(dist(p, a3(&case["expected"])) < 2e-6, "{case}: {p:?}");
        }
        // Sliding is limited by friction times depth and follows the collider's motion.
        for (friction, x) in [(0.0, 0.0), (0.25, 0.5), (10.0, 10.0)] {
            let mut p = [0.0; 3];
            project_contact(&mut p, v(10.0, 2.0, 0.0), v(0.0, 1.0, 0.0), 2.0, friction, &M4_ID);
            assert_eq!(p, v(x, 2.0, 0.0));
        }
        let mut p = [0.0; 3];
        project_contact(&mut p, [0.0; 3], v(0.0, 1.0, 0.0), 2.0, 0.5, &m4_compose(v(4.0, 0.0, 0.0), Q_ID, [1.0; 3]));
        assert_eq!(p, v(1.0, 2.0, 0.0));
        let mut p = [0.0; 3];
        project_contact(&mut p, v(2.0, 0.0, 0.0), v(0.0, 0.0, 1.0), 1.0, 0.5, &m4_compose([0.0; 3], axis_angle(v(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2), [1.0; 3]));
        assert!(p[0].abs() < 1e-8 && (p[1] - 0.5).abs() < 1e-8 && p[2] == 1.0);
        let mut p = [0.0; 3];
        project_contact(&mut p, v(10.0, 2.0, 0.0), v(0.0, 1.0, 0.0), 0.0, 1.0, &M4_ID);
        assert_eq!(p, [0.0; 3]);
    }

    fn twist_node(i: usize, pos: V3, init: V3) -> Node {
        let mut n = Node::free(i, pos);
        n.init_pos = init;
        n
    }

    #[test]
    fn ropes_align_segments() {
        let make = || (0..3).map(|x| twist_node(x, v(0.0, x as f64, 0.0), v(x as f64, 0.0, 0.0))).collect::<Vec<_>>();
        let mut p = make();
        p[2].pos = v(1.0, 1.0, 0.0);
        p[1].target_rot = axis_angle(v(1.0, 0.0, 0.0), 0.8);
        reconstruct_rope(&mut p, &[0, 1, 2], &[true, true, true]);
        assert!(dist(q_rot(p[0].solved_rot, v(1.0, 0.0, 0.0)), v(0.0, 1.0, 0.0)) < 1e-7);
        assert!(q_angle(p[1].solved_rot, p[1].target_rot) < 1e-7);
        assert!(q_angle(p[2].solved_rot, p[1].solved_rot) < 1e-7);
        let mut p = make();
        p[1].target_rot = axis_angle(v(1.0, 0.0, 0.0), 0.8);
        reconstruct_rope(&mut p, &[0, 1], &[false, true, false]);
        assert_eq!(p[0].solved_rot, Q_ID);
        assert!(dist(q_rot(p[1].solved_rot, v(1.0, 0.0, 0.0)), v(0.0, 1.0, 0.0)) < 1e-7);
        assert!((q_rot(p[1].solved_rot, v(0.0, 0.0, 1.0))[2] - 0.8f64.cos()).abs() < 1e-7);
        let mut p = make();
        p[1].init_pos = v(-1.0, 0.0, 0.0);
        reconstruct_rope(&mut p, &[0, 1], &[true, true, false]);
        assert!(dist(q_rot(p[0].solved_rot, v(-1.0, 0.0, 0.0)), v(0.0, 1.0, 0.0)) < 1e-7);
        p[1].pos = p[0].pos;
        reconstruct_rope(&mut p, &[0, 1], &[true, true, false]);
        assert_eq!(p[0].solved_rot, Q_ID);
    }

    #[test]
    fn twists_transfer_swing_and_twist() {
        let link = Twist { node_orient: 0, node_end: 1, twist_relax: 1.0, swing_relax: 1.0 };
        let z = v(0.0, 0.0, 1.0);
        let make = || vec![twist_node(0, [0.0; 3], [0.0; 3]), twist_node(1, z, z)];
        let mut p = make();
        p[1].pos = v(0.0, -1.0, 0.0);
        reconstruct_twist(&mut p, &Twist { twist_relax: 0.0, ..link });
        assert!(dist(q_rot(p[0].solved_rot, z), v(0.0, -1.0, 0.0)) < 1e-7);
        assert_eq!((p[0].pos, p[1].pos), ([0.0; 3], v(0.0, -1.0, 0.0)));
        let mut p = make();
        p[1].solved_rot = axis_angle(z, std::f64::consts::FRAC_PI_2);
        reconstruct_twist(&mut p, &Twist { swing_relax: 0.0, ..link });
        assert!(q_angle(p[0].solved_rot, p[1].solved_rot) < 1e-7);
        assert!(dist(q_rot(p[0].solved_rot, z), z) < 1e-7);
        let mut p = make();
        p[1].pos = v(0.0, -1.0, 0.0);
        reconstruct_twist(&mut p, &Twist { twist_relax: 0.0, swing_relax: 0.0, ..link });
        assert_eq!(p[0].solved_rot, Q_ID);
        p[1].pos = p[0].pos;
        reconstruct_twist(&mut p, &link);
        assert_eq!(p[0].solved_rot, Q_ID);
        let mut p = make();
        p[1].solved_rot = q_mul(axis_angle(v(1.0, 0.0, 0.0), 1.2), axis_angle(z, 0.8));
        p[1].pos = q_rot(p[1].solved_rot, z);
        reconstruct_twist(&mut p, &Twist { twist_relax: 0.618, swing_relax: 0.5, ..link });
        let expected = [0.260_146_057_163_821_7, -0.060_662_198_251_375_664, 0.218_840_621_348_455_42, 0.938_484_368_060_145_3];
        assert!(q_angle(p[0].solved_rot, expected) < 1e-7);
        let mut p = make();
        p[0].target_rot = axis_angle(z, 0.7);
        reconstruct_twist(&mut p, &Twist { node_end: 0, ..link });
        assert!(q_angle(p[0].solved_rot, p[0].target_rot) < 1e-7);
        // Authored relative rotations survive a rigidly transformed pose.
        let mut p = make();
        let rotation = euler(0.4, 0.8, -0.6);
        let (rest0, rest1) = (euler(0.2, -0.5, 0.3), euler(-0.8, 0.1, 1.1));
        p[0].init_rot = rest0;
        p[1].init_rot = rest1;
        p[0].solved_rot = q_mul(rotation, rest0);
        p[1].solved_rot = q_mul(rotation, rest1);
        for n in &mut p {
            n.pos = add(q_rot(rotation, n.pos), v(7.0, 5.0, -9.0));
        }
        let expected = p[0].solved_rot;
        reconstruct_twist(&mut p, &Twist { twist_relax: 0.618, swing_relax: 0.5, ..link });
        assert!(q_angle(p[0].solved_rot, expected) < 1e-7);
    }

    #[test]
    fn similarity_and_fits_recover_transforms() {
        let source = [v(1.0, 2.0, 3.0), v(-1.0, 5.0, 2.0), v(4.0, 0.0, 2.0), v(3.0, 2.0, -2.0), v(0.0, 1.0, 0.0), v(2.0, -3.0, 1.0)];
        let m = m4_compose(v(0.5, -1.25, 2.0), euler(0.35, -0.2, 0.6), [0.0254; 3]);
        let target: Vec<V3> = source.iter().map(|p| m4_point(&m, *p)).collect();
        let fit = recover_similarity(&source, &target).unwrap();
        assert!(fit.rmse < 1e-9 && (fit.scale - 0.0254).abs() < 1e-10);
        for (s, t) in source.iter().zip(&target) {
            assert!(dist(m4_point(&fit.matrix, *s), *t) < 1e-9);
            assert!(dist(m4_point(&fit.inverse, *t), *s) < 1e-7);
        }
        assert!(recover_similarity(&source[..2], &target[..2]).is_none());
        assert!(recover_similarity(&[[1.0; 3]; 3], &target[..3]).is_none());

        let source = [v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0), v(0.0, 3.0, 0.0), v(0.0, 0.0, 4.0)];
        let weights = [2.0, 1.0, 3.0, 4.0];
        let center = v(0.2, 0.9, 1.6);
        let (p, q) = recover_cloth_fit(4, |i| (source[i], source[i], weights[i]), center).unwrap();
        assert!(q_angle(q, Q_ID) < 1e-6 && dist(p, center) < 1e-10);
        let source = [v(-1.0, 0.0, 0.0), v(2.0, 0.5, 0.0), v(0.0, 3.0, 1.0), v(0.5, -1.0, 2.0), v(1.0, 1.0, -1.0)];
        let weights = [4.0, 1.0, 2.0, 3.0, 5.0];
        let rotation = euler(0.2, -0.5, 0.9);
        let shift = v(3.0, -2.0, 5.0);
        let moved: Vec<V3> = source.iter().map(|p| add(q_rot(rotation, *p), shift)).collect();
        let mut c = [0.0; 3];
        for (p, w) in source.iter().zip(weights) {
            c = madd(c, *p, w);
        }
        let c = scl(c, 1.0 / 15.0);
        let (p, q) = recover_cloth_fit(5, |i| (source[i], moved[i], weights[i]), c).unwrap();
        assert!(q_angle(q, rotation) < 1e-6 && dist(add(q_rot(rotation, c), shift), p) < 1e-10);
        assert!(recover_cloth_fit(0, |_| ([0.0; 3], [0.0; 3], 1.0), [0.0; 3]).is_none());
        assert!(recover_cloth_fit(1, |_| ([0.0; 3], v(1.0, 2.0, 3.0), 0.0), [0.0; 3]).is_none());
        assert!(recover_cloth_fit(1, |_| ([0.0; 3], v(f64::NAN, 2.0, 3.0), 1.0), [0.0; 3]).is_none());
    }

    #[test]
    fn fit_matrices_match_the_runtime_reference() {
        for case in reference()["fit"].as_array().unwrap() {
            let initial = pts(&case["initial"]);
            let pose: Vec<Value> = initial.iter().map(|p| json!([p[0], p[1], p[2], 1, 0, 0, 0, 1])).collect();
            let model = fe(json!({ "m_CtrlName": names(initial.len()), "m_InitPose": pose, "m_FitMatrices": [case["fit"].clone()], "m_FitWeights": case["weights"].clone() }));
            let mut nodes = free_nodes(&pts(&case["positions"]));
            for (n, init) in nodes.iter_mut().zip(&initial) {
                n.init_pos = *init;
            }
            let before: Vec<V3> = nodes.iter().map(|n| n.pos).collect();
            let (p, q) = reconstruct_fit(&build_fit_recs(&model)[0], &nodes).unwrap();
            let e = &case["expected"];
            assert!(dist(p, a3(e)) < 5e-5, "{case}: {p:?}");
            assert!(q_angle(q, a4(e, 4)) < 1e-4, "{case}: {q:?}");
            assert_eq!(nodes.iter().map(|n| n.pos).collect::<Vec<_>>(), before);
        }
        // Spans start where the previous fit ended and ctrl wins over node.
        let model = fe(json!({
            "m_CtrlName": names(4),
            "m_FitMatrices": [{ "nEnd": 2, "nNode": 1, "nCtrl": 3 }, { "nEnd": 4, "nNode": 2 }],
            "m_FitWeights": [{ "nNode": 0, "flWeight": 1 }, { "nNode": 1, "flWeight": 0 }, { "nNode": 2, "flWeight": 2 }, { "nNode": 7, "flWeight": 1 }]
        }));
        let recs = build_fit_recs(&model);
        assert_eq!(recs.iter().map(|r| (r.target, r.weights.clone())).collect::<Vec<_>>(), vec![(3, vec![(0, 1.0)]), (2, vec![(2, 2.0)])]);
    }

    #[test]
    fn node_bases_follow_the_runtime_convention() {
        let base = NodeBase { node: 2, x0: 0, x1: 1, y0: 0, y1: 2, q_adjust: Q_ID };
        let mut positions = [v(0.0, 0.0, 0.0), v(0.0, 8.0, -0.049), v(0.0, 2.0, 0.0)];
        let collapsed = node_base_quat(|i| positions[i], &base);
        assert!(q_angle(collapsed, Q_ID) < 1e-10);
        positions[1] = v(0.0, 8.0, -0.051);
        let separated = node_base_quat(|i| positions[i], &base);
        assert!(dist(q_rot(separated, v(1.0, 0.0, 0.0)), v(0.0, 0.0, -1.0)) < 1e-10);
        let q = node_base_quat(|_| [0.0; 3], &base);
        assert!(dist(q_rot(q, v(0.0, 1.0, 0.0)), v(0.0, 0.0, 1.0)) < 1e-10);
        assert!(dist(q_rot(q, v(1.0, 0.0, 0.0)), v(1.0, 0.0, 0.0)) < 1e-10);
        let adjust = euler(0.2, 0.4, -0.1);
        let positions = [v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0), v(0.0, 1.0, 0.0)];
        assert!(q_angle(node_base_quat(|i| positions[i], &NodeBase { q_adjust: adjust, ..base }), adjust) < 1e-10);
        let positions = [v(0.0, 0.0, 0.0), v(0.0, 0.0, 1.0), v(0.0, 1.0, 0.0)];
        let expected = q_from_cols(v(0.0, 0.0, 1.0), v(0.0, 1.0, 0.0), v(-1.0, 0.0, 0.0));
        assert!(q_angle(node_base_quat(|i| positions[i], &base), expected) < 1e-10);
        let (pp, pr, off) = (v(10.0, -3.0, 2.0), axis_angle(v(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2), v(2.0, 0.0, 0.0));
        assert_eq!(recover_offset_sign(pp, pr, apply_offset(pp, pr, off, 1.0), off), 1.0);
        assert_eq!(recover_offset_sign(pp, pr, apply_offset(pp, pr, off, -1.0), off), -1.0);
    }

    #[test]
    fn anchors_walk_to_a_real_driver_bone() {
        let m = fe(json!({ "m_CtrlName": ["pelvis", "$cloth_a", "$cloth_b", "cloth_orphan", "spine"], "m_SkelParents": [-1, 0, 1, -1, 0] }));
        assert_eq!(cloth_anchor_map(&m), vec![None, Some(0), Some(0), None, None]);
        let cycle = fe(json!({ "m_CtrlName": ["$cloth_a", "$cloth_b"], "m_SkelParents": [1, 0] }));
        assert_eq!(cloth_anchor_map(&cycle), vec![None, None]);
        let short = fe(json!({ "m_CtrlName": ["a", "$cloth_b"], "m_SkelParents": [-1] }));
        assert_eq!(cloth_anchor_map(&short), vec![None, None]);
        assert_eq!(rigid_anchor_seed(v(1.0, 2.0, 3.0), v(1.0, 0.0, 0.0), Q_ID, v(1.0, 0.0, 0.0), Q_ID), v(1.0, 2.0, 3.0));
        assert_eq!(rigid_anchor_seed(v(1.0, 2.0, 3.0), v(1.0, 0.0, 0.0), Q_ID, v(4.0, 0.0, 0.0), Q_ID), v(4.0, 2.0, 3.0));
        let carried = rigid_anchor_seed(v(1.0, 0.0, 0.0), [0.0; 3], Q_ID, [0.0; 3], axis_angle(v(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2));
        assert!(dist(carried, v(0.0, 1.0, 0.0)) < 1e-12);
    }

    // --- runtime safety (Grimoire's rig: three pinned nodes and a raw-integrated tip) -------

    fn graph(names: &[&str], parents: &[Option<usize>], heads: &[V3]) -> Model {
        let nodes = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let local = match parents[i] {
                    Some(p) => sub(heads[i], heads[p]),
                    None => heads[i],
                };
                GNode { name: name.to_string(), parent: parents[i], translation: local.map(|x| x as f32), rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], matrix: None }
            })
            .collect();
        let n = names.len();
        Model { nodes, skins: vec![Skin { joints: (0..n).collect(), inverse_bind: vec![Mat4::IDENTITY; n] }], ..Default::default() }
    }

    fn rig_json(extra: Value) -> String {
        let mut raw = json!({
            "m_CtrlName": ["anchor", "spare_x", "spare_y", "$cloth_tip"],
            "m_NodeInvMasses": [0, 0, 0, 1],
            "m_nStaticNodes": 3,
            "m_nFirstPositionDrivenNode": 4,
            "m_nDynamicNodeFlags": 0x200,
            "m_InitPose": [[0, 0, 0, 1, 0, 0, 0, 1], [1, 0, 0, 1, 0, 0, 0, 1], [0, 1, 0, 1, 0, 0, 0, 1], [0, 0, -1, 1, 0, 0, 0, 1]],
            "m_SkelParents": [-1, -1, -1, 0],
            "m_NodeIntegrator": [{}, {}, {}, { "flGravity": 24, "flPointDamping": 1, "flAnimationForceAttraction": 20, "flAnimationVertexAttraction": 4 }],
            "m_Rods": [{ "nNode": [0, 3], "flMinDist": 1, "flMaxDist": 1, "flWeight0": 0, "flRelaxationFactor": 1 }]
        });
        for (k, val) in extra.as_object().unwrap() {
            raw[k] = val.clone();
        }
        raw.to_string()
    }

    fn rig_model() -> Model {
        graph(&["anchor", "spare_x", "spare_y", "$cloth_tip"], &[None, None, None, Some(0)], &[v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0), v(0.0, 0.0, -1.0)])
    }

    /// The rig's animation: the anchor's translation and rotation, the tip's rotation.
    #[derive(Clone, Copy)]
    struct RigPose {
        anchor: [f32; 3],
        anchor_rot: [f32; 4],
        tip_rot: [f32; 4],
    }

    impl RigPose {
        fn rest() -> RigPose {
            RigPose { anchor: [0.0; 3], anchor_rot: [0.0, 0.0, 0.0, 1.0], tip_rot: [0.0, 0.0, 0.0, 1.0] }
        }
        fn locals(&self) -> Vec<Mat4> {
            vec![
                Mat4::from_trs(self.anchor, self.anchor_rot, [1.0; 3]),
                Mat4::translation([1.0, 0.0, 0.0]),
                Mat4::translation([0.0, 1.0, 0.0]),
                Mat4::from_trs([0.0, 0.0, -1.0], self.tip_rot, [1.0; 3]),
            ]
        }
    }

    fn rig(extra: Value) -> Cloth {
        let mut cloth = Cloth::new(&rig_model(), &rig_json(extra)).unwrap().expect("a runtime");
        cloth.tuning.budget_ms = None;
        cloth
    }

    /// Grimoire's harness metrics: everything finite, and kinematic nodes on their targets.
    fn healthy(c: &Cloth) -> (bool, f64) {
        let mut finite = true;
        let mut anchor_error: f64 = 0.0;
        for n in &c.nodes {
            finite &= finite3(n.pos) && finite3(n.prev) && finite3(n.target) && n.solved_rot.iter().all(|x| x.is_finite());
            if n.kinematic && !n.position_driven {
                anchor_error = anchor_error.max(dist(n.pos, n.target));
            }
        }
        (finite, anchor_error)
    }

    #[test]
    fn catch_up_is_bounded_and_a_suspension_drops_the_backlog() {
        let mut cloth = rig(json!({}));
        let mut pose = RigPose::rest();
        let calls = Cell::new(0);
        let mut animate = |dt: f32| {
            calls.set(calls.get() + 1);
            pose.anchor[0] += dt;
            pose.locals()
        };
        for _ in 0..20 {
            cloth.step(0.25, &mut animate);
            assert_eq!(cloth.stats().substeps_last_frame, 12);
        }
        assert_eq!(cloth.stats().simulation_steps, 240);
        assert_eq!(cloth.stats().recoveries, 0);
        assert!(healthy(&cloth).0);
        cloth.step(DT / 2.0, &mut animate);
        cloth.step(60.0, &mut animate);
        cloth.step(DT / 2.0, &mut animate);
        cloth.step(f32::NAN, &mut animate);
        cloth.step(-1.0, &mut animate);
        assert_eq!(cloth.stats().simulation_steps, 240);
        cloth.step(DT / 2.0, &mut animate);
        assert_eq!(cloth.stats().recoveries, 1);
        assert_eq!(calls.get(), 241);
    }

    #[test]
    fn a_slow_frame_hands_the_animation_its_leftover_time() {
        let mut cloth = rig(json!({}));
        let mut seen = Vec::new();
        cloth.step(5.0 * DT, &mut |dt| {
            seen.push(dt);
            RigPose::rest().locals()
        });
        assert_eq!(seen, vec![DT; 5]);
        // A budget already spent by the previous substep ends the frame after one substep,
        // which carries the rest of the frame's time (at most four substeps) to the animation.
        cloth.tuning.budget_ms = Some(0.0);
        cloth.substep_ms = 1.0;
        seen.clear();
        cloth.step(6.0 * DT, &mut |dt| {
            seen.push(dt);
            RigPose::rest().locals()
        });
        assert_eq!(seen.len(), 1);
        assert!((seen[0] - 4.0 * DT).abs() < 1e-7);
    }

    #[test]
    fn a_teleport_restarts_the_drape_without_stale_motion() {
        let mut cloth = rig(json!({ "m_SphereRigids": [{ "nNode": 0, "vSphere": [0, 0, 0, 0.25], "nCollisionMask": 65535 }] }));
        for _ in 0..30 {
            cloth.step(DT, &mut |_| RigPose::rest().locals());
        }
        let mut moved = RigPose::rest();
        moved.anchor[0] = 1000.0;
        cloth.step(DT, &mut |_| moved.locals());
        assert_eq!(cloth.stats().recoveries, 1);
        assert!(healthy(&cloth).0);
        let tip = cloth.nodes[3].pos;
        assert!((tip[0] - 1000.0).abs() < 5e-9, "{tip:?}");
        assert!((dist(tip, v(1000.0, 0.0, 0.0)) - 1.0).abs() < 5e-9);
        for _ in 0..120 {
            cloth.step(DT, &mut |_| moved.locals());
            assert!(healthy(&cloth).0);
        }
        assert_eq!(cloth.stats().recoveries, 1);
        assert_eq!(cloth.locals()[0].0[3][0], 1000.0);
    }

    #[test]
    fn a_non_finite_sample_never_becomes_the_clean_pose() {
        let mut cloth = rig(json!({}));
        let mut pose = RigPose::rest();
        pose.anchor[0] = 3.0;
        cloth.step(DT, &mut |_| pose.locals());
        let mut bad = pose;
        bad.anchor[0] = f32::NAN;
        bad.tip_rot = [f32::NAN, 0.0, 0.0, 1.0];
        cloth.step(DT, &mut |_| bad.locals());
        assert!(healthy(&cloth).0);
        assert_eq!(cloth.locals()[0].0[3][0], 3.0);
        pose.anchor[0] += 1.0;
        cloth.step(DT, &mut |_| pose.locals());
        assert!(healthy(&cloth).0);
        assert_eq!(cloth.locals()[0].0[3][0], 4.0);
        assert_eq!(cloth.locals()[3], Mat4::translation([0.0, 0.0, -1.0]));
        assert!(cloth.pose().iter().all(mat_finite));
    }

    #[test]
    fn an_overflowed_solver_recovers_before_writeback() {
        let mut cloth = rig(json!({}));
        cloth.step(DT, &mut |_| RigPose::rest().locals());
        cloth.tuning.gravity_scale = f64::MAX;
        let mut pose = RigPose::rest();
        pose.anchor[0] = 1.0;
        cloth.step(DT, &mut |_| pose.locals());
        assert_eq!(cloth.stats().recoveries, 1);
        assert!(healthy(&cloth).0);
        assert!(cloth.pose().iter().all(mat_finite));
        cloth.tuning = ClothTuning { budget_ms: None, ..ClothTuning::default() };
        for _ in 0..120 {
            cloth.step(DT, &mut |_| pose.locals());
            assert!(healthy(&cloth).0);
        }
        assert_eq!(cloth.stats().recoveries, 1);
    }

    #[test]
    fn anchors_track_sustained_turning_and_running() {
        let mut cloth = rig(json!({}));
        let mut time = 0.0f32;
        let mut pose = RigPose::rest();
        for _ in 0..600 {
            cloth.step(1.0 / 60.0, &mut |dt| {
                time += dt;
                pose.anchor = [3.0 * (time * 3.0).sin(), 0.2 * (time * 8.0).sin(), 2.0 * (time * 3.0).cos()];
                pose.anchor_rot = crate::math::quat_axis_angle([0.0, 1.0, 0.0], time * 2.0);
                pose.locals()
            });
            let (finite, anchor_error) = healthy(&cloth);
            assert!(finite && anchor_error < 1e-8, "{anchor_error}");
        }
        assert_eq!((cloth.stats().simulation_steps, cloth.stats().recoveries), (1200, 0));
        let posed = cloth.pose();
        let animated = crate::model::globals(&rig_model(), &pose.locals());
        for i in 0..3 {
            assert!(posed[i].0.iter().flatten().zip(animated[i].0.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-5));
        }
        // The tip hangs one unit from the anchor.
        let tip = [posed[3].0[3][0], posed[3].0[3][1], posed[3].0[3][2]];
        let anchor = [posed[0].0[3][0], posed[0].0[3][1], posed[0].0[3][2]];
        assert!((crate::math::length(crate::math::sub(tip, anchor)) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn iteration_budgets_stay_finite() {
        let mut cloth = rig(json!({ "m_nExtraIterations": 1e9, "m_nExtraGoalIterations": 1e9 }));
        assert_eq!(cloth.iteration_phases(), (256, 256));
        cloth.tuning.iteration_override = u32::MAX;
        assert_eq!(cloth.iteration_phases(), (256, 256));
        let cloth = rig(json!({ "m_nExtraIterations": 2, "m_nExtraGoalIterations": 0 }));
        assert_eq!(cloth.iteration_phases(), (3, 1));
    }

    #[test]
    fn a_singular_fit_gives_no_runtime() {
        let model = graph(&["anchor", "spare_x", "spare_y", "$cloth_tip"], &[None, None, None, Some(0)], &[[0.0; 3]; 4]);
        assert!(Cloth::new(&model, &rig_json(json!({}))).unwrap().is_none());
        let unmatched = graph(&["a", "b", "c"], &[None, None, None], &[[0.0; 3], v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)]);
        assert!(Cloth::new(&unmatched, &rig_json(json!({}))).unwrap().is_none());
    }

    #[test]
    fn invalid_references_are_skipped_before_stepping() {
        let mut cloth = rig(json!({
            "m_Rods": [{ "nNode": [-1, 1] }, { "nNode": [0, 1.5] }],
            "m_NodeBases": [{ "nNode": 0, "nNodeX0": 0, "nNodeX1": 400, "nNodeY0": 0, "nNodeY1": 1 }],
            "m_SphereRigids": [{ "nNode": 0.5, "vSphere": [0, 0, 0, 1], "nCollisionMask": 65535 }],
            "m_BoxRigids": [{ "nNode": 1.5, "tmFrame2": [0, 0, 0, 1, 0, 0, 0, 1], "vSize": [1, 1, 1], "nCollisionMask": 65535 }],
            "m_CollisionPlanes": [{ "nCtrlParent": 0.5, "nChildNode": 3, "m_Plane": { "m_vNormal": [0, 1, 0], "m_flOffset": 0 } }]
        }));
        assert!(cloth.colliders.is_empty());
        assert_eq!(cloth.coverage().decode_issues.len(), 3);
        cloth.step(DT, &mut |_| RigPose::rest().locals());
        assert!(healthy(&cloth).0);
    }

    #[test]
    fn colliders_sort_by_priority_and_keep_compiled_order() {
        let cloth = rig(json!({
            "m_TaperedCapsuleRigids": [{ "nNode": 0, "vSphere": [[0, 0, 0, 1], [0, 0, 1, 1]] }, { "nNode": 1, "vSphere": [[0, 0, 0, 1], [0, 0, 1, 1]] }],
            "m_SphereRigids": [{ "nNode": 2, "vSphere": [0, 0, 0, 1] }],
            "m_BoxRigids": [{ "nNode": 0, "tmFrame2": [0, 0, 0, 1, 0, 0, 0, 1], "vSize": [1, 1, 1] }],
            "m_CollisionPlanes": [{ "nCtrlParent": 0, "nChildNode": 3, "m_Plane": { "m_vNormal": [0, 1, 0], "m_flOffset": 0 } }],
            "m_RigidColliderPriorities": [
                { "m_nTaperedCapsuleRigidIndex": 0, "m_nSphereRigidIndex": 0, "m_nBoxRigidIndex": 0, "m_nSDFRigidIndex": 0, "m_nCollisionPlaneIndex": 0 },
                { "m_nTaperedCapsuleRigidIndex": 1, "m_nSphereRigidIndex": 1, "m_nBoxRigidIndex": 1, "m_nSDFRigidIndex": 0, "m_nCollisionPlaneIndex": 0 },
                { "m_nTaperedCapsuleRigidIndex": 2, "m_nSphereRigidIndex": 1, "m_nBoxRigidIndex": 1, "m_nSDFRigidIndex": 0, "m_nCollisionPlaneIndex": 1 }
            ]
        }));
        let order: Vec<(char, usize, usize)> = cloth
            .colliders
            .iter()
            .map(|c| match c {
                Collider::Capsule { node, filter, .. } => ('c', *node, filter.priority),
                Collider::Box { node, filter, .. } => ('b', *node, filter.priority),
                Collider::Plane(p) => ('p', p.ctrl_parent, p.priority),
            })
            .collect();
        assert_eq!(order, vec![('c', 1, 1), ('p', 0, 1), ('c', 0, 0), ('c', 2, 0), ('b', 0, 0)]);
    }

    /// A pinned bar with `chains` chains of `links` nodes hanging from it under a Source-to-glTF
    /// wrapper, the way an exported hero is laid out: the glb model and its FeModel JSON.
    fn hanging(chains: usize, links: usize, sideways: bool, force: f64) -> (Model, String) {
        let mut names = vec!["skeleton".to_string(), "bar".to_string()];
        let mut parents = vec![None, Some(0)];
        let mut heads = vec![[0.0; 3], v(0.0, 3.0, 64.0)];
        let mut fe_names = vec!["bar".to_string()];
        let mut pose = vec![json!([0, 3, 64, 1, 0, 0, 0, 1])];
        let mut inv = vec![0.0];
        let mut skel = vec![-1i64];
        let mut rods = Vec::new();
        let mut bends = Vec::new();
        let node_of = |c: usize, k: usize| 1 + c * links + k;
        for c in 0..chains {
            for k in 0..links {
                let y = (c as f64 - (chains as f64 - 1.0) / 2.0) * 2.0;
                let head = if sideways { v(-10.0 - 3.0 * k as f64, y, 60.0) } else { v(-10.0, y, 60.0 - 3.0 * k as f64) };
                names.push(format!("$cloth_{c}_{k}"));
                parents.push(Some(if k == 0 { 1 } else { names.len() - 2 }));
                heads.push(head);
                fe_names.push(format!("$cloth_{c}_{k}"));
                pose.push(json!([head[0], head[1], head[2], 1, 0, 0, 0, 1]));
                inv.push(if k == 0 { 0.0 } else { 1.0 });
                skel.push(if k == 0 { 0 } else { node_of(c, k - 1) as i64 });
                if k > 0 {
                    rods.push(json!({ "nNode": [node_of(c, k - 1), node_of(c, k)], "flMinDist": 2.7, "flMaxDist": 3.0, "flWeight0": if k == 1 { 0.0 } else { 0.5 }, "flRelaxationFactor": 1 }));
                    if c > 0 {
                        rods.push(json!({ "nNode": [node_of(c - 1, k), node_of(c, k)], "flMinDist": 1.7, "flMaxDist": 2.1, "flWeight0": 0.5, "flRelaxationFactor": 0.5 }));
                    }
                }
                if k > 0 && k + 1 < links {
                    bends.push(json!({ "nNode": [node_of(c, k), node_of(c, k - 1), node_of(c, k + 1)], "flWeight": [-1, 0.5, 0.5], "flHeight0": 0.5 }));
                }
            }
        }
        let n = fe_names.len();
        let mut model = graph(&names.iter().map(String::as_str).collect::<Vec<_>>(), &parents, &heads);
        model.nodes[0].matrix = Some(crate::fixture::source_to_gltf());
        model.nodes[0].translation = [0.0; 3];
        model.skins[0].joints.remove(0);
        let dynamic = inv.iter().filter(|&&m| m > 0.0).count();
        let raw = json!({
            "m_CtrlName": fe_names,
            "m_NodeInvMasses": inv,
            "m_InitPose": pose,
            "m_SkelParents": skel,
            "m_NodeIntegrator": (0..n).map(|i| if i == 0 { json!({}) } else { json!({ "flGravity": 360, "flPointDamping": 0.5, "flAnimationForceAttraction": force, "flAnimationVertexAttraction": 0.005 }) }).collect::<Vec<_>>(),
            "m_nDynamicNodeFlags": 0,
            "m_nExtraIterations": 3,
            "m_Rods": rods,
            "m_KelagerBends": bends,
            "m_NodeCollisionRadii": vec![0.5; dynamic],
            "m_TaperedCapsuleRigids": [{ "nNode": 0, "vSphere": [[0, 0, -10, 8], [0, 0, -50, 9]], "nCollisionMask": 65535 }],
            "m_SphereRigids": [{ "nNode": 0, "vSphere": [0, 0, -30, 6], "nCollisionMask": 65535 }]
        });
        (model, raw.to_string())
    }

    #[test]
    fn a_gravity_chain_hangs_and_stays_finite() {
        let (model, json) = hanging(1, 8, true, 0.0);
        let mut cloth = Cloth::new(&model, &json).unwrap().expect("a runtime");
        cloth.tuning.budget_ms = None;
        let rest = crate::model::sample_pose(&model, None, 0.0);
        for _ in 0..600 {
            cloth.step(1.0 / 60.0, &mut |_| rest.clone());
            assert!(healthy(&cloth).0);
        }
        let tip = cloth.nodes[8].pos;
        let root = cloth.nodes[1].pos;
        // Sideways at rest, the chain now hangs under its root (Source Z is up).
        assert!(root[2] - tip[2] > 15.0, "{tip:?}");
        assert!((tip[0] - root[0]).abs() < 4.0, "{tip:?}");
        for k in 2..=8 {
            let d = dist(cloth.nodes[k].pos, cloth.nodes[k - 1].pos);
            assert!((2.6..3.05).contains(&d), "link {k}: {d}");
        }
        assert_eq!(cloth.stats().recoveries, 0);
    }

    #[test]
    fn substeps_do_not_allocate() {
        let (model, json) = hanging(4, 6, false, 0.02);
        let mut cloth = Cloth::new(&model, &json).unwrap().expect("a runtime");
        cloth.tuning.budget_ms = None;
        let rest = crate::model::sample_pose(&model, None, 0.0);
        cloth.step(0.05, &mut |_| rest.clone());
        ALLOCATIONS.with(|n| n.set(0));
        COUNTING.with(|on| on.set(true));
        for _ in 0..60 {
            cloth.step(1.0 / 60.0, &mut |_| Vec::new());
        }
        COUNTING.with(|on| on.set(false));
        assert_eq!(ALLOCATIONS.with(Cell::get), 0);
        assert_eq!(cloth.stats().simulation_steps, 126);
    }

    // --- the stand-in hero -------------------------------------------------------------------

    fn stand_in() -> (Model, Cloth) {
        let model = crate::model::load_slice(&crate::fixture::hero_glb(crate::fixture::Variant::Stock)).expect("the stand-in loads");
        let mut cloth = Cloth::new(&model, &crate::fixture::cloth_json()).expect("a FeModel").expect("a runtime");
        cloth.tuning.budget_ms = None;
        (model, cloth)
    }

    #[test]
    fn the_stand_in_cloth_builds() {
        let (_, cloth) = stand_in();
        let stats = cloth.stats();
        assert_eq!((stats.nodes, stats.matched, stats.dynamic_nodes), (21, 21, 12));
        assert!(stats.fit_rmse < 1e-5, "{}", stats.fit_rmse);
        assert!((cloth.fit.scale - 0.0254).abs() < 1e-7);
        let c = cloth.coverage();
        assert_eq!((c.rods, c.kelager_bends, c.rope_chains, c.goal_damped, c.raw, c.unknown), (20, 9, 3, 12, 0, 0));
        assert!(c.decode_issues.is_empty() && c.feature_gaps.is_empty(), "{c:?}");
        assert!(!cloth.fe.rod_batches.is_empty());
        assert_eq!(cloth.colliders.len(), 5);
        assert_eq!(cloth.debug().capsules.len(), 5);
        // At bind, before any substep, the pose is the animation's.
        let animated = crate::model::globals(&stand_in().0, cloth.locals());
        assert_eq!(cloth.pose(), animated);
    }

    #[test]
    fn the_stand_in_tails_swing_while_the_roots_track_their_bones() {
        let (model, mut cloth) = stand_in();
        let clip = crate::model::clip_by_name(&model, "hero_shop").expect("idle");
        let duration = clip.duration;
        let names = ["$cloth_cape_l_0", "$cloth_cape_c_0", "$cloth_cape_r_0"];
        let roots: Vec<usize> = names.iter().map(|n| model.nodes.iter().position(|m| m.name == *n).unwrap()).collect();
        let tip = model.nodes.iter().position(|m| m.name == "$cloth_cape_c_4").unwrap();
        let mut time = 0.0f32;
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let mut lag: f32 = 0.0;
        for frame in 0..(2.0 * duration * 60.0) as usize {
            cloth.step(1.0 / 60.0, &mut |dt| {
                time += dt;
                crate::model::sample_pose(&model, Some(clip), time.rem_euclid(duration))
            });
            let posed = cloth.pose();
            let animated = crate::model::globals(&model, cloth.locals());
            assert!(posed.iter().all(mat_finite));
            for &r in &roots {
                let (a, b) = (posed[r].0[3], animated[r].0[3]);
                assert!((0..3).all(|k| (a[k] - b[k]).abs() < 1e-5), "root {r} drifted: {a:?} vs {b:?}");
            }
            if frame >= 60 {
                let p = posed[tip].0[3];
                let q = animated[tip].0[3];
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
                lag = lag.max(crate::math::length(crate::math::sub([p[0], p[1], p[2]], [q[0], q[1], q[2]])));
            }
        }
        let swing = crate::math::length(crate::math::sub(hi, lo));
        eprintln!("coat tail tip: swing {:.1} mm, largest lag behind its bone {:.1} mm", swing * 1000.0, lag * 1000.0);
        assert!(swing > 0.02, "the tails barely move: {swing} m");
        assert!(lag > 0.015, "the tails follow rigidly: {lag} m");
        assert!(lag < 0.25, "the tails fly off: {lag} m");
        assert_eq!(cloth.stats().recoveries, 0);
        // A seek restarts the drape from the pose it names.
        let seek = crate::model::sample_pose(&model, Some(clip), 1.0);
        cloth.reset(&seek);
        assert_eq!(cloth.pose(), crate::model::globals(&model, &seek));
        assert_eq!(cloth.stats().simulation_steps, 0);
    }

    #[test]
    fn placement_turns_gravity_into_the_model() {
        let (model, json) = hanging(1, 4, true, 0.0);
        let mut cloth = Cloth::new(&model, &json).unwrap().unwrap();
        cloth.tuning.budget_ms = None;
        // Tip the model over: world down is now the model's glb -Z (Source -X), along which the
        // chain already lies, so it stays level instead of drooping.
        cloth.set_placement(&Mat4::from_trs([0.0; 3], crate::math::quat_axis_angle([1.0, 0.0, 0.0], -std::f32::consts::FRAC_PI_2), [1.0; 3]));
        let rest = crate::model::sample_pose(&model, None, 0.0);
        for _ in 0..240 {
            cloth.step(1.0 / 60.0, &mut |_| rest.clone());
        }
        let (root, tip) = (cloth.nodes[1].pos, cloth.nodes[4].pos);
        assert!(root[0] - tip[0] > 7.0 && (tip[2] - root[2]).abs() < 2.0, "{tip:?}");
    }

    #[test]
    #[ignore = "timing: cargo test --release -p sigil-viewer cloth_timing -- --ignored --nocapture"]
    fn cloth_timing() {
        let run = |label: &str, model: &Model, mut cloth: Cloth, clip: Option<&crate::model::Clip>| {
            cloth.tuning.budget_ms = None;
            let mut time = 0.0f32;
            let frames = 600;
            let (mut solve, mut total) = (0.0f64, 0.0f64);
            for _ in 0..frames {
                let start = Instant::now();
                let mut sample = 0.0f64;
                cloth.step(1.0 / 60.0, &mut |dt| {
                    time += dt;
                    let t = Instant::now();
                    let locals = crate::model::sample_pose(model, clip, clip.map_or(0.0, |c| time.rem_euclid(c.duration)));
                    sample += ms(t.elapsed());
                    locals
                });
                let posed = cloth.pose();
                std::hint::black_box(posed);
                let frame = ms(start.elapsed());
                total += frame;
                solve += frame - sample;
            }
            let s = cloth.stats();
            println!(
                "{label}: {} nodes ({} dynamic), {:.3} ms/frame cloth (solve + pose), {:.3} ms/frame with clip sampling, 60 fps, {} substeps",
                s.nodes,
                s.dynamic_nodes,
                solve / frames as f64,
                total / frames as f64,
                s.simulation_steps
            );
        };
        let (model, cloth) = stand_in();
        let clip = crate::model::clip_by_name(&model, "hero_shop");
        run("stand-in", &model, cloth, clip);
        let (model, json) = hanging(20, 20, false, 0.02);
        let cloth = Cloth::new(&model, &json).unwrap().unwrap();
        run("synthetic", &model, cloth, None);
    }

    const REFERENCE: &str = r#"
{
"hinge": [
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,1,1],[1,0,1],[0,1,1]],"invMasses":[0,0,0,1,0,0],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0,"flWeight5":0,"flAngleCenter":0,"flAngleExtents":0.5235987755982988},"expected":[[0.0,0.0,0.0],[0.0,0.0,2.0],[1.0,0.0,1.0],[1.1668219566345215,0.7004305124282837,1.0],[1.0,0.0,1.0],[0.0,1.0,1.0]]},
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,-1,1],[1,0,1],[0,1,1]],"invMasses":[0,0,0,1,0,0],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0,"flWeight5":0,"flAngleCenter":0,"flAngleExtents":0.5235987755982988},"expected":[[0.0,0.0,0.0],[0.0,0.0,2.0],[1.0,0.0,1.0],[1.1668219566345215,-0.7004305124282837,1.0],[1.0,0.0,1.0],[0.0,1.0,1.0]]},
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,1,1],[1,0,1],[0,1,1]],"invMasses":[0,0,0,1,0,0],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0,"flWeight5":0,"flAngleCenter":-3,"flAngleExtents":0.5235987755982988},"expected":[[0.0,0.0,0.0],[0.0,0.0,2.0],[1.0,0.0,1.0],[-1.3326467275619507,0.5354145765304565,1.0],[1.0,0.0,1.0],[0.0,1.0,1.0]]},
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,1,1],[1,0,1],[0,1,1]],"invMasses":[1,2,3,4,0,0],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0,"flWeight5":0,"flAngleCenter":0,"flAngleExtents":0.5235987755982988},"expected":[[-0.06247750297188759,-0.06214771792292595,-0.0007444048533216119],[-0.12269773334264755,-0.12235833704471588,1.998608112335205],[0.9508591890335083,0.4288482666015625,1.0023679733276367],[0.5608265399932861,0.921509861946106,1.0026042461395264],[1.0,0.0,1.0],[0.0,1.0,1.0]]},
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,1,1],[2,0.2,1],[0.1,2,0.8]],"invMasses":[0,0,1,1,1,1],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0.3,"flWeight5":0.4,"flAngleCenter":0,"flAngleExtents":0.5235987755982988},"expected":[[0.0,0.0,0.0],[0.0,0.0,2.0],[0.9652017951011658,0.06848587840795517,1.0],[0.09300244599580765,0.9659522771835327,1.0],[1.810543179512024,0.5728675127029419,1.0],[0.3092555105686188,1.923392653465271,0.800000011920929]]},
{"positions":[[0,0,0],[0,0,2],[1,0,1],[0,1,1],[1,0,1],[0,1,1]],"invMasses":[0,0,0,1,0,1],"hinge":{"nNode":[0,1,2,3,4,5],"nFlags":0,"flWeight4":0,"flWeight5":1,"flAngleCenter":0,"flAngleExtents":0.5235987755982988},"expected":[[0.0,0.0,0.0],[0.0,0.0,2.0],[1.0,0.0,1.0],[0.0,1.0,1.0],[1.0,0.0,1.0],[0.0,1.0,1.0]]}
],
"triangle": [
{"positions":[[-3.7215419417587494,-1.8986006603219678,-3.3755476846434034],[2.597489458263989,-0.33640822808224335,1.0706613886687508],[1.926269787340523,-1.442150647804679,-0.7565075321307235]],"batches":[[{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]}]],"staticCounts":[2],"scale":1.5,"expected":[[-3.7215418815612793,-1.898600697517395,-3.3755476474761963],[2.597489356994629,-0.33640822768211365,1.0706614255905151],[2.1836962699890137,-2.321887493133545,-1.9354145526885986]]},
{"positions":[[3.1513732757785515,-1.8615180718731104,2.2931082909135228],[2.158875188689648,0.5179762830822057,-1.5489978738065977],[3.4459608666771038,1.5466809119737208,-0.31765197268088396]],"batches":[[{"nNode":[0,1,2],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]}]],"staticCounts":[1],"scale":1,"expected":[[3.1513733863830566,-1.8615180253982544,2.2931082248687744],[2.2052359580993652,-1.687307596206665,0.5396902561187744],[3.5584118366241455,-0.17606055736541748,1.4424898624420166]]},
{"positions":[[2.5275924576002886,-2.039900123983565,1.912124353682966],[1.0845265579855488,-0.8687921378268806,1.5175078427189614],[-0.7904593840684537,-1.9813806488778578,-3.320636919353297]],"batches":[[{"nNode":[0,1,2],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]}]],"staticCounts":[0],"scale":1.5,"expected":[[2.295607328414917,-2.4430112838745117,0.8776389360427856],[0.36291587352752686,-0.2160816192626953,1.4303460121154785],[0.4930468797683716,-2.0390334129333496,-1.3539689779281616]]},
{"positions":[[0,0,0],[2,1,0],[4,2,1],[1,3,2],[3,1,3],[2,-1,1]],"batches":[[{"nNode":[0,1,2],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,2,3],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]}]],"staticCounts":[0],"scale":1,"expected":[[0.6519293785095215,0.5775379538536072,0.7870149612426758],[2.0198330879211426,1.0099165439605713,-0.9677027463912964],[2.4635868072509766,1.2317934036254883,1.1965184211730957],[0.8814035058021545,2.327524423599243,1.5411267280578613],[2.0854923725128174,0.6740822792053223,2.0775868892669678],[2.0,-1.0,1.0]]},
{"positions":[[0,0,0],[2,1,0],[4,2,1],[1,3,2],[3,1,3],[2,-1,1]],"batches":[[{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,1,2],"w1":0,"w2":1,"v1x":2,"v2":[0.7,1.8]}],[{"nNode":[0,2,3],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,2,3],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,2,3],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]},{"nNode":[0,2,3],"w1":0.3,"w2":0.7,"v1x":2,"v2":[0.7,1.8]}],[{"nNode":[2,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[2,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[2,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]},{"nNode":[2,3,4],"w1":0.3,"w2":0.25,"v1x":2,"v2":[0.7,1.8]}]],"staticCounts":[2,1,0],"scale":1,"expected":[[0.0,0.0,0.0],[2.0,1.0,0.0],[0.8847969770431519,-0.2531565725803375,1.937288761138916],[0.7535931468009949,1.574216604232788,1.1351090669631958],[2.3211207389831543,0.8842769861221313,2.5482048988342285],[2.0,-1.0,1.0]]}
],
"quad": [
{"positions":[[-1,0,0],[1,0,0],[-1,3,1],[2,2,-2]],"batches":[[{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]}]],"staticCounts":[2],"scale":1,"relaxation":1,"expected":[[-1.0,0.0,0.0],[1.0,0.0,0.0],[-0.699999988079071,2.0101261138916016,-0.2222457379102707],[1.2000000476837158,1.495408296585083,-0.6031204462051392]]},
{"positions":[[-1.2959567719478975,-1.005280319248104,1.418329464113215],[-1.8314701040173702,0.6555877509347159,1.4879043453802958],[1.327424308794852,0.8487481960887253,-0.2912940349472315],[-1.100788060310748,0.9350243053044069,1.585950279544646]],"batches":[[{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]}]],"staticCounts":[0],"scale":1,"relaxation":1,"expected":[[-1.8955085277557373,-1.3311810493469238,1.1401472091674805],[-1.5008968114852905,-0.030169308185577393,-1.1456910371780396],[0.49932968616485596,1.4495099782943726,0.3910675644874573],[-0.49511581659317017,0.9088069796562195,2.460522413253784]]},
{"positions":[[-1,0,0],[1,0,0],[-1,3,1],[2,2,-2]],"batches":[[{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]}]],"staticCounts":[0],"scale":1,"relaxation":0.35,"expected":[[-0.6866563558578491,-0.05983734130859375,-0.17297086119651794],[0.6910380721092224,0.4354987144470215,0.48810410499572754],[-0.4589744806289673,3.017705202102661,0.6127512454986572],[1.670375943183899,1.7839311361312866,-1.9103728532791138]]},
{"positions":[[1.983280561772038,-0.570416347087018,1.535785038788175],[-0.6732868476585425,-0.5542030781811897,0.8908108366802616],[0.9308128386563705,0.8792921695148102,1.7003252094278793],[-0.33070894900304815,-1.2865647284723476,-1.3765028675676385]],"batches":[[{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]}]],"staticCounts":[1],"scale":1,"relaxation":1,"expected":[[1.9832805395126343,-0.5704163312911987,1.5357850790023804],[1.9747236967086792,-0.4273774027824402,-0.7678862810134888],[0.9091304540634155,1.0084306001663208,0.549575924873352],[1.2597732543945312,-1.1127500534057617,1.766203761100769]]},
{"positions":[[-1,0,0],[1,0,0],[-1,3,1],[2,2,-2],[0,2,1],[1,-3,2],[2,4,-1],[0,2,-3]],"batches":[[{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]},{"nNode":[0,1,2,3],"flSlack":0,"vShape":[[-1,0,0,0],[1,0,0,0],[-0.7,2,0.3,0.4],[1.2,1.6,-0.2,0.6]]}],[{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[0,0,0,0],[-0.7,2,0.3,0.2],[1.2,1.6,-0.2,0.3],[0.3,-0.2,0.8,0.5]]}],[{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]},{"nNode":[0,1,2,3],"flSlack":2,"vShape":[[-1.0,-1.4,0.19000000000000003,0.1],[1.0,-1.4,-0.10999999999999999,0.2],[1.0,0.6000000000000001,0.29000000000000004,0.3],[-1.0,0.6000000000000001,-0.21,0.4]]}]],"staticCounts":[2,1,0],"scale":1,"relaxation":0.4,"expected":[[-1.1467043161392212,-0.5057735443115234,-0.12766879796981812],[-1.301835298538208,-0.4093334674835205,-2.218663215637207],[-0.48239725828170776,1.3790863752365112,-1.4701740741729736],[-1.6628270149230957,0.6058942079544067,0.2798078656196594],[0.0,2.0,1.0],[1.0,-3.0,2.0],[2.0,4.0,-1.0],[0.0,2.0,-3.0]]}
],
"goal": [
{"position":[8.0,-3.0,2.0],"previous":[6.0,1.0,4.0],"goal":[0.0,2.0,-1.0],"force":0.0,"vertex":0.0,"expected":{"position":[8.0,-3.0,2.0],"previous":[6.0,1.0,4.0]}},
{"position":[8.0,-3.0,2.0],"previous":[6.0,1.0,4.0],"goal":[0.0,2.0,-1.0],"force":0.10000000149011612,"vertex":1.0,"expected":{"position":[7.199999809265137,-2.5,1.6999998092651367],"previous":[7.199999809265137,-2.5,1.6999998092651367]}},
{"position":[8.0,-3.0,2.0],"previous":[6.0,1.0,4.0],"goal":[0.0,2.0,-1.0],"force":2.0,"vertex":0.10000000149011612,"expected":{"position":[0.0,2.0,-1.0],"previous":[0.0,2.0,-1.0]}},
{"position":[0.0,0.0,0.0],"previous":[-2.0,3.0,-1.0],"goal":[16.0,-8.0,4.0],"force":2.384185791015625e-07,"vertex":0.30000001192092896,"expected":{"position":[3.814697265625e-06,-1.9073486328125e-06,9.5367431640625e-07],"previous":[-1.3999987840652466,2.09999942779541,-0.6999996900558472]}},
{"position":[-2.5377867221832275,-6.4066572189331055,-6.5465006828308105],"previous":[0.9869021773338318,1.3696011304855347,1.985405683517456],"goal":[5.103391647338867,-0.542006254196167,4.230568885803223],"force":0.31353941559791565,"vertex":0.884516716003418,"expected":{"position":[-0.14197635650634766,-4.56785774230957,-3.1674647331237793],"previous":[-0.011609748005867004,-3.8821804523468018,-2.572394371032715]}},
{"position":[-6.522793769836426,4.219630718231201,0.7396376132965088],"previous":[8.574593544006348,-0.6638140678405762,1.6769980192184448],"goal":[-7.739219665527344,5.285033226013184,-5.683429718017578],"force":0.30832231044769287,"vertex":0.4112124443054199,"expected":{"position":[-6.897845268249512,4.5481181144714355,-1.2407374382019043],"previous":[2.21213436126709,1.4793972969055176,0.47718894481658936]}}
],
"stray": [
{"positions":[[1,2,3],[7,-2,1],[0,5,3]],"targets":[[0,0,0],[1,0,0],[-1,3,2]],"batches":[[{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[1,2],"flMaxDist":1,"flRelaxationFactor":1},{"nNode":[2,1],"flMaxDist":1,"flRelaxationFactor":0.2},{"nNode":[2,1],"flMaxDist":1,"flRelaxationFactor":0.2}]],"scale":1,"expected":[[1.0,2.0,3.0],[5.568655014038086,-1.1054092645645142,1.1789181232452393],[0.8309691548347473,0.8451542854309082,0.5070924758911133]]},
{"positions":[[1,2,3],[7,-2,1],[0,5,3]],"targets":[[0,0,0],[1,0,0],[-1,3,2]],"batches":[[{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5}],[{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.5}]],"scale":1,"expected":[[1.0,2.0,3.0],[3.1788692474365234,-0.9082483053207397,0.4541241526603699],[0.0,5.0,3.0]]},
{"positions":[[1,2,3],[7,-2,1],[0,5,3]],"targets":[[0,0,0],[1,0,0],[-1,3,2]],"batches":[[{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.7},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.7},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.7},{"nNode":[0,1],"flMaxDist":2,"flRelaxationFactor":0.7}]],"scale":1.5,"expected":[[1.0,2.0,3.0],[4.10041618347168,-1.171547532081604,0.585773766040802],[0.0,5.0,3.0]]},
{"positions":[[2.4481717600810953,-0.8843190261782947,0.47585317917984415],[2.8111488087646563,-0.4868770839376815,3.9297407998689406],[-1.542157745233907,-2.6246693428516386,0.33152212726211516],[-2.0282599154279266,-1.160286642558452,1.6191393681144293]],"targets":[[1.722984219288346,-3.352471368328384,-0.5846741687326773],[3.3709899135919263,-0.394433519497416,2.243648386843117],[-3.3177752741770377,-2.403506490999776,-0.3769991412679907],[2.5116261772735786,1.36525893061994,3.0669781236364653]],"batches":[[{"nNode":[0,1],"flMaxDist":3.890495013049322,"flRelaxationFactor":0.5738351996398582},{"nNode":[1,2],"flMaxDist":2.460079188457947,"flRelaxationFactor":0.8445769856558247},{"nNode":[2,3],"flMaxDist":2.581621504307001,"flRelaxationFactor":0.5186133713208729},{"nNode":[3,0],"flMaxDist":1.0584232747106652,"flRelaxationFactor":0.9377189594644552}]],"scale":1,"expected":[[2.489323854446411,0.5745933651924133,2.1562681198120117],[2.631922483444214,-0.9588556885719299,3.186192750930786],[0.8241111040115356,-1.5505436658859253,1.2524399757385254],[-2.0532846450805664,-1.1844128370285034,1.5804017782211304]]}
],
"fit": [
{"initial":[[20,-10,50],[22,-10,50],[20,-7,50],[20,-10,54],[22.4,-9.6,51.85]],"positions":[[20.0,-10.0,50.0],[22.0,-10.0,50.0],[20.0,-7.0,50.0],[20.0,-10.0,54.0],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0,0,0,1],"vCenter":[20.4,-9.1,51.6],"nEnd":4,"nNode":4,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.1},{"nNode":1,"flWeight":0.2},{"nNode":2,"flWeight":0.3},{"nNode":3,"flWeight":0.4}],"expected":[22.399999618530273,-9.600000381469727,51.849998474121094,1.0,-7.450580596923828e-09,1.4901161193847656e-08,-1.043081283569336e-07,1.0]},
{"initial":[[20,-10,50],[22,-10,50],[20,-7,50],[20,-10,54],[22.4,-9.6,51.85]],"positions":[[-2.812194910759235,-34.41880827281186,-33.64981145638294],[-3.5893216128922774,-33.91682334808091,-35.422968309054106],[-0.2326202552959984,-33.085084294091686,-34.40278884347936],[-1.4875825671233471,-37.858241146762836,-35.20406486064901],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0.14048043101898117,0.14048043101898117,0.0,0.9800665778412416],"vCenter":[20.4,-9.1,51.6],"nEnd":4,"nNode":4,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.1},{"nNode":1,"flWeight":0.2},{"nNode":2,"flWeight":0.3},{"nNode":3,"flWeight":0.4}],"expected":[-2.78817081451416,-35.22933578491211,-36.596839904785156,1.0,0.47493860125541687,0.735449492931366,-0.4175545275211334,0.24330154061317444]},
{"initial":[[20,-10,50],[22,-10,50],[20,-7,50],[22.666666666666664,-9.0,50.25]],"positions":[[39.007264561517225,-32.97777261653272,31.0517193284518],[40.615334873658036,-33.34700228039178,29.921330312874478],[40.03142468273685,-30.21293042924822,31.605563824240406],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0,0,0,1],"vCenter":[20.666666666666664,-8.5,50.0],"nEnd":3,"nNode":3,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.16666666666666666},{"nNode":1,"flWeight":0.3333333333333333},{"nNode":2,"flWeight":0.5}],"expected":[41.614444732666016,-32.63381576538086,29.930156707763672,1.0,0.13998757302761078,0.27997493743896484,-0.13998781144618988,0.9393725991249084]},
{"initial":[[-4.397985071427623,-7.543412104258513,-0.6587527448917818],[-7.532788850111711,5.26488770849064,4.302259014617334],[2.110161753494056,0.12725910192429524,7.1270594057988585],[-4.499833610656978,3.7137903954144615,-7.048945824122098],[-5.039780406033749,0.8714036125570619,0.9427420747608934],[-5.74325359413376,7.328413302622145,-6.080462382600247],[-3.4256541449852316,2.896446437283709,0.5108530657504244]],"positions":[[-0.2596043424556995,-11.409286846443706,1.7436568485293473],[-3.27402593152569,1.3326375372087411,7.31456724177073],[6.151831267054808,-2.2878918150528778,9.04108838851253],[-1.7008861168898144,0.07014714669366601,-4.846469814023193],[-1.5811440740766631,-2.1151010116074564,2.683387713723536],[-2.4043265089047847,3.761255279773698,-3.0930860782591183],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0,0,0,1],"vCenter":[-5.425654144985232,3.396446437283709,0.26085306575042444],"nEnd":6,"nNode":6,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.07090447863326611},{"nNode":1,"flWeight":0.33375878506041057},{"nNode":2,"flWeight":0.07502367028935011},{"nNode":3,"flWeight":0.05966722025633437},{"nNode":4,"flWeight":0.22190403801018485},{"nNode":5,"flWeight":0.23874180775045414}],"expected":[0.3577650785446167,-0.5177225470542908,2.877739191055298,1.0,0.012155725620687008,0.042634688317775726,0.022077146917581558,0.9987728595733643]},
{"initial":[[-2,0,0],[0,0,0],[2,0,0],[2.0,-0.5,0.25]],"positions":[[2.0,-3.0,2.0],[4.0,-3.0,2.0],[6.0,-3.0,2.0],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0.14048043101898117,0.14048043101898117,0.0,0.9800665778412416],"vCenter":[0.0,0.0,0.0],"nEnd":3,"nNode":3,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.3333333333333333},{"nNode":1,"flWeight":0.3333333333333333},{"nNode":2,"flWeight":0.3333333333333333}],"expected":[6.0,-2.75,2.5,1.0,-0.5936770439147949,0.09933467209339142,-0.09933466464281082,0.7923464179039001]},
{"initial":[[0.0008457279333832408,0,0],[-0.0008457279333832408,0,0],[0,0.0008457279333832408,0],[0,-0.0008457279333832408,0],[0,0,0.0008457279333832408],[0,0,-0.0008457279333832408],[2.0,-0.5,0.25]],"positions":[[0.0006799949909108968,-0.00015613392027965893,-0.0004780007830316619],[-0.0006799949909108968,0.00015613392027965893,0.0004780007830316619],[0.0002887202742575341,0.0007794347563943032,0.00015613392027965893],[-0.0002887202742575341,-0.0007794347563943032,-0.00015613392027965893],[0.00041170760604272423,-0.0002887202742575341,0.0006799949909108968],[-0.00041170760604272423,0.0002887202742575341,-0.0006799949909108968],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0,0,0,1],"vCenter":[0.0,0.0,0.0],"nEnd":6,"nNode":6,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.16666666666666666},{"nNode":1,"flWeight":0.16666666666666666},{"nNode":2,"flWeight":0.16666666666666666},{"nNode":3,"flWeight":0.16666666666666666},{"nNode":4,"flWeight":0.16666666666666666},{"nNode":5,"flWeight":0.16666666666666666}],"expected":[1.5590789318084717,-0.9153833985328674,-1.0216878652572632,1.0,0.13998748362064362,0.27997490763664246,-0.13998748362064362,0.939372718334198]},
{"initial":[[2,0,0],[-2,0,0],[0,3,0],[0,-3,0],[0,0,4],[0,0,-4],[2.0,-0.5,0.25]],"positions":[[2,0,0],[-2,0,0],[0,-3,0],[0,3,0],[0,0,4],[0,0,-4],[100,-50,20]],"fit":{"bone":[2,-0.5,0.25,1,0,0,0,1],"vCenter":[0.0,0.0,0.0],"nEnd":6,"nNode":6,"nBeginDynamic":0},"weights":[{"nNode":0,"flWeight":0.16666666666666666},{"nNode":1,"flWeight":0.16666666666666666},{"nNode":2,"flWeight":0.16666666666666666},{"nNode":3,"flWeight":0.16666666666666666},{"nNode":4,"flWeight":0.16666666666666666},{"nNode":5,"flWeight":0.16666666666666666}],"expected":[-2.0,0.5,0.25,1.0,0.0,0.0,1.0,0.0]}
],
"friction": [
{"position":[0.5,0,2],"previous":[0.5,1,2],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":true,"expected":[1.0,0.0,2.0]},
{"position":[0.5,0,2],"previous":[0.5,1,2],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":2,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":true,"expected":[1.0,1.0,2.0]},
{"position":[0.001,0,-0.001],"previous":[0.001,1,-0.001],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0.5,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":true,"expected":[0.0010000000474974513,0.5,0.9990000128746033]},
{"position":[0.099,0,4.001],"previous":[0.099,1,4.001],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0.5,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":true,"expected":[0.0989999994635582,0.5,5.000999927520752]},
{"position":[0.5,0,2],"previous":[0.5,1,2],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0.5,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":true,"expected":[1.0,0.25,2.0]},
{"position":[0,0,0],"previous":[0,1,0],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":false,"expected":[0.0,0.0,1.0]},
{"position":[0.05,0,0],"previous":[0.05,1,0],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0,"current":[0,0,0,1,0,0,0,1],"old":null,"radius":0,"frictionEnabled":false,"expected":[1.0,0.0,0.0]},
{"position":[3.0001,2,5],"previous":[3.0001,2,5],"capsule":[[0,0,0,1],[0,0,4,1]],"friction":0,"current":[3,4,5,1,0.7071067811865476,0,0,0.7071067811865476],"old":null,"radius":0,"frictionEnabled":false,"expected":[3.0,1.9999998807907104,6.0]}
],
"box_sphere": [
{"kind":"sphere","position":[0.5,0.0,1.0],"previous":[0.5,0.20000000298023224,1.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":[0.0,0.0,0.0,1.0,0.0,0.0,0.34289780259132385,0.939372718334198],"radius":0.20000000298023224,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[0.9928815960884094,-0.16914039850234985,1.9632339477539062]},
{"kind":"sphere","position":[0.0,0.0,0.0],"previous":[0.0,1.0,0.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":null,"radius":0.0,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":false,"scoped":true,"expected":[0.0,0.0,2.0]},
{"kind":"box","position":[0.5,0.0,1.0],"previous":[0.5,0.20000000298023224,1.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":[0.0,-1.0,0.0,1.0,0.0,0.0,0.0,1.0],"radius":0.20000000298023224,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[2.200000047683716,0.8500000238418579,1.0]},
{"kind":"box","position":[9.999999747378752e-05,0.0,0.0],"previous":[9.999999747378752e-05,1.0,0.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":null,"radius":0.0,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[2.0,0.9999499917030334,0.0]},
{"kind":"box","position":[2.200000047683716,3.200000047683716,4.199999809265137],"previous":[2.200000047683716,3.200000047683716,4.199999809265137],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":null,"radius":0.25,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[2.200000047683716,3.200000047683716,4.199999809265137]},
{"kind":"box","position":[1.0,1.0,1.0],"previous":[1.0,1.0,1.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":null,"radius":0.25,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":false,"scoped":true,"expected":[2.25,1.0,1.0]},
{"kind":"box","position":[3.5,2.0,5.0],"previous":[3.5,2.0,5.199999809265137],"friction":0.5,"current":[3.0,4.0,5.0,1.0,0.7071067690849304,0.0,0.0,0.7071067690849304],"old":[1.0,2.0,5.0,1.0,0.0,0.0,0.0,1.0],"radius":0.5,"size":[2.0,3.0,4.0],"frame":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[5.5,3.0,5.0]},
{"kind":"box","position":[1.0,0.0,1.0],"previous":[0.0,0.0,1.0],"friction":0.5,"current":[0.0,0.0,0.0,1.0,0.0,0.0,0.0,1.0],"old":null,"radius":0.25,"size":[2.0,3.0,4.0],"frame":[0.5,0.5,0.5,1.0,0.0,0.0,0.3894183337688446,0.9210609793663025],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":true,"expected":[-1.0749964714050293,-1.1068580150604248,1.0]},
{"kind":"box","position":[0.3743686378002167,-0.7137623429298401,0.06422501802444458],"previous":[-0.9949660897254944,2.8022549152374268,-2.602639675140381],"friction":0.6267806887626648,"current":[-0.7853038907051086,-0.06882232427597046,2.7593255043029785,1.0,0.0,0.0,-0.7847374081611633,0.6198284029960632],"old":[0.22503376007080078,-2.2191882133483887,1.800200343132019,1.0,0.0,0.0,-0.07989060878753662,0.9968036413192749],"radius":0.49484530091285706,"size":[2.0,3.0,4.0],"frame":[0.20000000298023224,-0.30000001192092896,0.4000000059604645,1.0,0.364696204662323,0.0,0.0,0.9311265349388123],"sphere":[0.0,0.0,0.0,2.0],"frictionEnabled":true,"scoped":false,"expected":[1.541674256324768,-0.353127658367157,-0.5314027667045593]}
]
}"#;
}
