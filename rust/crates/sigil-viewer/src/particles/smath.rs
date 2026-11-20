//! Source space maths for the simulation: frames (forward, left, up axes plus an origin, the
//! rows of VRF's row-vector matrices, the columns of ours), Source Euler angles, the
//! deterministic random table and Valve's string token.

use crate::math::{Mat4, Quat, Vec3, add, cross, dot, length, scale, slerp};

pub const EPS: f32 = f32::EPSILON;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerpv(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

pub fn mulv(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

pub fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 == e0 {
        return if x >= e1 { 1.0 } else { 0.0 };
    }
    let t = sat((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

/// `value` clamped between `a` and `b` in either order.
pub fn clamp_either(value: f32, a: f32, b: f32) -> f32 {
    value.clamp(a.min(b), a.max(b))
}

pub fn safe_normalize(v: Vec3) -> Vec3 {
    let l = length(v);
    if l > 1e-12 { scale(v, 1.0 / l) } else { [0.0; 3] }
}

/// `x^exp` with the exponent truncated to quarter steps (VRF's `Pow` shortcut).
pub fn pow_quarter(x: f32, exp: f32) -> f32 {
    let e = (exp * 4.0).trunc() / 4.0;
    if e == 1.0 { x } else { x.powf(e) }
}

/// VRF `Bias`: `x / ((1 - x)(1/b - 2) + 1)`.
pub fn bias(x: f32, b: f32) -> f32 {
    let x = sat(x);
    let b = b.clamp(f32::MIN_POSITIVE, 1.0);
    x / ((1.0 - x) * (1.0 / b - 2.0) + 1.0)
}

pub const BIAS_GAIN: i32 = 1;
pub const BIAS_EXPONENTIAL: i32 = 2;

/// One of the `PF_BIAS_TYPE_*` curves for a parameter in [-1, 1], identity at 0.
pub fn bias_from_parameter(x: f32, param: f32, kind: i32) -> f32 {
    if kind == BIAS_EXPONENTIAL {
        let e = if param >= 0.0 { 1.0 - sat(param) } else { 20.0 - sat(param + 1.0) * 19.0 };
        if e <= 0.0 {
            return 1.0;
        }
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        return x.powf(e.min(20.0));
    }
    let b = sat((param + 1.0) * 0.5);
    if b <= 0.0 {
        return 0.0;
    }
    if b >= 1.0 {
        return 1.0;
    }
    if kind == BIAS_GAIN {
        return if x < 0.5 { bias(x + x, b) * 0.5 } else { 1.0 - bias(2.0 - x - x, b) * 0.5 };
    }
    bias(x, b)
}

/// A rigid Source frame: `point(v) = o + v.x * x + v.y * y + v.z * z`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub o: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
}

impl Xf {
    pub const IDENTITY: Xf = Xf { o: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] };

    pub fn point(&self, v: Vec3) -> Vec3 {
        add(self.o, self.vector(v))
    }

    pub fn vector(&self, v: Vec3) -> Vec3 {
        [
            self.x[0] * v[0] + self.y[0] * v[1] + self.z[0] * v[2],
            self.x[1] * v[0] + self.y[1] * v[1] + self.z[1] * v[2],
            self.x[2] * v[0] + self.y[2] * v[1] + self.z[2] * v[2],
        ]
    }

    /// The inverse of a rotation applied to a vector (its transpose).
    pub fn inv_vector(&self, v: Vec3) -> Vec3 {
        [dot(self.x, v), dot(self.y, v), dot(self.z, v)]
    }

    pub fn from_mat(m: &Mat4) -> Xf {
        let c = |k: usize| [m.0[k][0], m.0[k][1], m.0[k][2]];
        Xf { o: c(3), x: c(0), y: c(1), z: c(2) }
    }

    pub fn to_mat(self) -> Mat4 {
        Mat4([[self.x[0], self.x[1], self.x[2], 0.0], [self.y[0], self.y[1], self.y[2], 0.0], [self.z[0], self.z[1], self.z[2], 0.0], [self.o[0], self.o[1], self.o[2], 1.0]])
    }

    /// The axes normalized (VRF's `rigidTransform`).
    pub fn rigid(&self) -> Xf {
        Xf { o: self.o, x: safe_normalize(self.x), y: safe_normalize(self.y), z: safe_normalize(self.z) }
    }
}

/// The unit quaternion of an orthonormal basis (columns x, y, z).
pub fn quat_from_axes(x: Vec3, y: Vec3, z: Vec3) -> Quat {
    let (m00, m11, m22) = (x[0], y[1], z[2]);
    let tr = m00 + m11 + m22;
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [(y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s, 0.25 * s]
    } else if m00 > m11 && m00 > m22 {
        let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
        [0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s, (y[2] - z[1]) / s]
    } else if m11 > m22 {
        let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
        [(y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s, (z[0] - x[2]) / s]
    } else {
        let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
        [(z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s, (x[1] - y[0]) / s]
    };
    crate::math::quat_normalize(q)
}

pub fn axes_from_quat(q: Quat) -> [Vec3; 3] {
    let m = Mat4::from_trs([0.0; 3], q, [1.0; 3]);
    [[m.0[0][0], m.0[0][1], m.0[0][2]], [m.0[1][0], m.0[1][1], m.0[1][2]], [m.0[2][0], m.0[2][1], m.0[2][2]]]
}

/// Slerps two orientations given as axes.
pub fn slerp_axes(a: [Vec3; 3], b: [Vec3; 3], t: f32) -> [Vec3; 3] {
    if t >= 1.0 || a == b {
        return b;
    }
    if t <= 0.0 {
        return a;
    }
    axes_from_quat(slerp(quat_from_axes(a[0], a[1], a[2]), quat_from_axes(b[0], b[1], b[2]), t))
}

/// Source Euler angles in radians (pitch, yaw, roll) as the forward, left and up axes
/// (VRF's `EulerAnglesToRotationMatrix` rows).
pub fn euler_axes(pitch: f32, yaw: f32, roll: f32) -> [Vec3; 3] {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();
    [
        [cy * cp, sy * cp, -sp],
        [cy * sp * sr - sy * cr, sy * sp * sr + cy * cr, cp * sr],
        [cy * sp * cr + sy * sr, sy * sp * cr - cy * sr, cp * cr],
    ]
}

/// The degrees form, as the KV3 data stores angles.
pub fn euler_axes_deg(a: Vec3) -> [Vec3; 3] {
    euler_axes(a[0].to_radians(), a[1].to_radians(), a[2].to_radians())
}

/// The left and up axes completing a frame around `forward` (VRF `VectorVectors`).
pub fn vector_vectors(forward: Vec3) -> (Vec3, Vec3) {
    if forward[0].abs() < 0.001 && forward[1].abs() < 0.001 {
        return ([0.0, 1.0, 0.0], [-forward[2], 0.0, 0.0]);
    }
    let right = safe_normalize(cross(forward, [0.0, 0.0, 1.0]));
    let up = safe_normalize(cross(right, forward));
    (scale(right, -1.0), up)
}

/// Rotates `v` about the unit `axis` by `angle` radians.
pub fn rotate_about(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    add(add(scale(v, c), scale(cross(axis, v), s)), scale(axis, dot(axis, v) * (1.0 - c)))
}

/// The shared random table, as a hash: `table(i)` is a stable value in [0, 1) for any index,
/// standing in for VRF's 4096-entry table (Grimoire's research allows a hash; the indexing
/// scheme on top of it, seeds, operator offsets and particle ids, is VRF's).
pub fn table(index: i32) -> f32 {
    let mut x = index as u32;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / 16_777_216.0
}

/// One system's draws (VRF `ParticleRandom`): `next` runs a free counter, `for_particle` reads
/// a slot fixed by the particle id and the running operator.
#[derive(Clone, Debug, Default)]
pub struct Rand {
    pub seed: i32,
    pub operator_offset: i32,
    pub query: i32,
    pub generation: i32,
}

pub const OPERATOR_STRIDE: i32 = 17;

impl Rand {
    pub fn start(&mut self, seed: i32, counter: i32) {
        self.seed = seed & 0xfff;
        self.query = counter;
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn next(&mut self) -> f32 {
        let v = table(self.query.wrapping_add(self.seed));
        self.query = self.query.wrapping_add(1);
        v
    }

    pub fn next_between(&mut self, a: f32, b: f32) -> f32 {
        let r = self.next();
        lerp(a, b, r)
    }

    pub fn next_index(&mut self, count: i32) -> i32 {
        ((count as f32) * self.next()) as i32
    }

    pub fn for_particle(&self, id: i32, offset: i32) -> f32 {
        table(self.seed.wrapping_add(self.operator_offset).wrapping_add(offset).wrapping_add(id))
    }

    /// The per input seed a constant random input displaces its draws by (VRF `InputSeed`),
    /// redrawn when the system restarts.
    pub fn input_seed(&self, slot: i32) -> i32 {
        (table(slot.wrapping_mul(7919).wrapping_add(self.generation.wrapping_mul(104_729)).wrapping_add(self.seed)) * 2_147_483_648.0) as i32
    }

    fn interior(&mut self) -> f32 {
        self.next() * 0.9999 + 0.0001
    }

    /// A point uniform in the unit ball and its radius fraction (polar cosine, azimuth, radius).
    pub fn unit_ball(&mut self) -> (Vec3, f32) {
        let i = self.interior();
        let cos_polar = i + i - 1.0;
        let azimuth = self.interior() * std::f32::consts::TAU;
        let r = self.interior().powf(1.0 / 3.0);
        let sin_polar = (1.0 - cos_polar * cos_polar).max(0.0).sqrt();
        let ring = sin_polar * r;
        ([ring * azimuth.cos(), ring * azimuth.sin(), r * cos_polar], r)
    }

    pub fn between_per_component(&mut self, a: Vec3, b: Vec3) -> Vec3 {
        let x = self.next();
        let y = self.next();
        let z = self.next();
        [lerp(a[0], b[0], x), lerp(a[1], b[1], y), lerp(a[2], b[2], z)]
    }
}

/// Valve's string token: MurmurHash2 of the lowercase bytes, seed 0x31415926.
pub fn string_token(key: &str) -> u32 {
    if key.is_empty() {
        return 0;
    }
    const M: u32 = 0x5bd1_e995;
    let bytes: Vec<u8> = key.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let mut h: u32 = 0x3141_5926 ^ bytes.len() as u32;
    let mut chunks = bytes.chunks_exact(4);
    for c in &mut chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M) ^ k;
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        let mut tail = 0u32;
        for &b in rest.iter().rev() {
            tail = (tail << 8) | b as u32;
        }
        h = (h ^ tail).wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
