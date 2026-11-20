//! Particle attributes (VRF `ParticleField`) and the column store a system keeps them in.

use crate::math::Vec3;

pub const POSITION: usize = 0;
pub const LIFE: usize = 1;
pub const PREV: usize = 2;
pub const RADIUS: usize = 3;
pub const ROLL: usize = 4;
pub const ROLL_SPEED: usize = 5;
pub const COLOR: usize = 6;
pub const ALPHA: usize = 7;
pub const CREATION: usize = 8;
pub const SEQUENCE: usize = 9;
pub const TRAIL_LENGTH: usize = 10;
pub const PARTICLE_ID: usize = 11;
pub const YAW: usize = 12;
pub const SEQUENCE2: usize = 13;
pub const ALPHA2: usize = 16;
pub const NONE: usize = 19;
pub const PITCH: usize = 20;
pub const NORMAL: usize = 21;
pub const GLOW_RGB: usize = 22;
pub const GLOW_ALPHA: usize = 23;
pub const FORCE_SCALE: usize = 34;
pub const MANUAL_FRAME: usize = 38;
pub const EXTRA1: usize = 39;
pub const EXTRA2: usize = 40;
pub const COUNT: usize = 49;

const SCALARS: &[usize] = &[1, 3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 16, 18, 20, 23, 26, 27, 33, 34, 38, 39, 40, 44, 45, 46, 47];
const VECTORS: &[usize] = &[0, 2, 6, 15, 17, 21, 22, 30, 32, 41, 42, 43, 48];

pub fn is_scalar(f: i32) -> bool {
    f >= 0 && SCALARS.contains(&(f as usize))
}

pub fn is_vector(f: i32) -> bool {
    f >= 0 && VECTORS.contains(&(f as usize))
}

/// Angles are stored in radians and authored in degrees; roll speed counts.
pub fn is_angle(f: i32) -> bool {
    f == ROLL as i32 || f == YAW as i32 || f == PITCH as i32 || f == ROLL_SPEED as i32
}

/// The range the float set operators hold a written value inside.
pub fn float_range(f: i32) -> (f32, f32) {
    if f == ALPHA as i32 || f == ALPHA2 as i32 {
        (0.0, 1.0)
    } else if f == RADIUS as i32 || f == TRAIL_LENGTH as i32 {
        (0.0, f32::MAX)
    } else {
        (f32::MIN, f32::MAX)
    }
}

pub fn mask(f: i32) -> u64 {
    if (0..COUNT as i32).contains(&f) { 1u64 << f } else { 0 }
}

/// `m_nSetMethod`.
pub const SET_SCALE_INITIAL: i32 = 1;
pub const SET_ADD_INITIAL: i32 = 2;
pub const SET_RAMP_CURRENT: i32 = 3;
pub const SET_SCALE_CURRENT: i32 = 4;
pub const SET_ADD_CURRENT: i32 = 5;

pub const SET_METHODS: &[(&str, i32)] = &[
    ("PARTICLE_SET_REPLACE_VALUE", 0),
    ("PARTICLE_SET_SCALE_INITIAL_VALUE", 1),
    ("PARTICLE_SET_ADD_TO_INITIAL_VALUE", 2),
    ("PARTICLE_SET_RAMP_CURRENT_VALUE", 3),
    ("PARTICLE_SET_SCALE_CURRENT_VALUE", 4),
    ("PARTICLE_SET_ADD_TO_CURRENT_VALUE", 5),
];

pub fn apply_set(v: f32, initial: f32, current: f32, dt: f32, method: i32) -> f32 {
    match method {
        SET_SCALE_INITIAL => v * initial,
        SET_ADD_INITIAL => v + initial,
        SET_SCALE_CURRENT => v * current,
        SET_ADD_CURRENT => v + current,
        SET_RAMP_CURRENT => current + v * dt,
        _ => v,
    }
}

pub fn apply_set_v(v: Vec3, initial: Vec3, current: Vec3, dt: f32, method: i32) -> Vec3 {
    [apply_set(v[0], initial[0], current[0], dt, method), apply_set(v[1], initial[1], current[1], dt, method), apply_set(v[2], initial[2], current[2], dt, method)]
}

pub fn reads_initial(method: i32) -> bool {
    method == SET_SCALE_INITIAL || method == SET_ADD_INITIAL
}

/// The particles of one system as columns, every scalar and vector field, plus the copy of
/// each taken when the particle was initialized.
#[derive(Clone, Debug)]
pub struct Particles {
    pub len: usize,
    pub s: Vec<Vec<f32>>,
    pub v: Vec<Vec<Vec3>>,
    pub is: Vec<Vec<f32>>,
    pub iv: Vec<Vec<Vec3>>,
    pub killed: Vec<bool>,
    pub kills: usize,
    /// The system age at the end of the step: a particle's age is `time - CreationTime`.
    pub time: f32,
    pub consts_s: [f32; COUNT],
    pub consts_v: [Vec3; COUNT],
}

impl Particles {
    pub fn new(consts_s: [f32; COUNT], consts_v: [Vec3; COUNT], capacity: usize) -> Particles {
        let cap = capacity.min(4096);
        let sc = |f: usize| if SCALARS.contains(&f) { Vec::with_capacity(cap) } else { Vec::new() };
        let vc = |f: usize| if VECTORS.contains(&f) { Vec::with_capacity(cap) } else { Vec::new() };
        Particles {
            len: 0,
            s: (0..COUNT).map(sc).collect(),
            v: (0..COUNT).map(vc).collect(),
            is: (0..COUNT).map(sc).collect(),
            iv: (0..COUNT).map(vc).collect(),
            killed: Vec::with_capacity(cap),
            kills: 0,
            time: 0.0,
            consts_s,
            consts_v,
        }
    }

    pub fn push(&mut self, creation: f32) {
        for &f in SCALARS {
            let c = if f == CREATION { creation } else { self.consts_s[f] };
            self.s[f].push(c);
            self.is[f].push(c);
        }
        for &f in VECTORS {
            self.v[f].push(self.consts_v[f]);
            self.iv[f].push(self.consts_v[f]);
        }
        self.killed.push(false);
        self.len += 1;
    }

    pub fn clear(&mut self) {
        for &f in SCALARS {
            self.s[f].clear();
            self.is[f].clear();
        }
        for &f in VECTORS {
            self.v[f].clear();
            self.iv[f].clear();
        }
        self.killed.clear();
        self.kills = 0;
        self.len = 0;
    }

    pub fn age(&self, i: usize) -> f32 {
        self.time - self.s[CREATION][i]
    }

    pub fn kill(&mut self, i: usize) {
        if !self.killed[i] {
            self.killed[i] = true;
            self.kills += 1;
        }
    }

    /// Copies the current values of `first..first + count` into the initial columns.
    pub fn save_initial(&mut self, first: usize, count: usize) {
        for &f in SCALARS {
            let (cur, init) = (&self.s[f], &mut self.is[f]);
            init[first..first + count].copy_from_slice(&cur[first..first + count]);
        }
        for &f in VECTORS {
            let (cur, init) = (&self.v[f], &mut self.iv[f]);
            init[first..first + count].copy_from_slice(&cur[first..first + count]);
        }
    }

    /// Removes the killed particles, keeping the survivors in order.
    pub fn compact(&mut self) {
        if self.kills == 0 {
            return;
        }
        let keep = &self.killed;
        fn retain<T: Copy>(col: &mut Vec<T>, keep: &[bool]) {
            if col.is_empty() {
                return;
            }
            let mut w = 0;
            for r in 0..col.len() {
                if !keep[r] {
                    col[w] = col[r];
                    w += 1;
                }
            }
            col.truncate(w);
        }
        for &f in SCALARS {
            retain(&mut self.s[f], keep);
            retain(&mut self.is[f], keep);
        }
        for &f in VECTORS {
            retain(&mut self.v[f], keep);
            retain(&mut self.iv[f], keep);
        }
        self.len -= self.kills;
        self.killed.clear();
        self.killed.resize(self.len, false);
        self.kills = 0;
    }

    /// A scalar of particle `i` (angles in radians).
    pub fn scalar(&self, f: i32, i: usize, initial: bool) -> f32 {
        if !is_scalar(f) {
            return 0.0;
        }
        if initial { self.is[f as usize][i] } else { self.s[f as usize][i] }
    }

    pub fn vector(&self, f: i32, i: usize, initial: bool) -> Vec3 {
        if !is_vector(f) {
            return [0.0; 3];
        }
        if initial { self.iv[f as usize][i] } else { self.v[f as usize][i] }
    }
}
