//! The particle functions this port runs: emitters, initializers, operators, force
//! generators and pre-emission operators, each a faithful port of Grimoire's TypeScript
//! (MIT), which ports VRF's C# (MIT). Classes outside this set are reported, never guessed.

// Functions index several particle columns in step; the enums live in short Vecs.
#![allow(clippy::needless_range_loop, clippy::large_enum_variant)]

use serde_json::Value;

use super::fields::{self as fl, Particles, apply_set, apply_set_v, float_range, is_angle, is_scalar, is_vector, mask, reads_initial};
use super::kv;
use super::providers::{Mapping, NumIn, ParseCx, TransIn, VecIn};
use super::smath::{EPS, Xf, bias, clamp_either, euler_axes_deg, lerp, lerpv, mulv, pow_quarter, safe_normalize, sat, smoothstep};
use super::state::State;
use crate::math::{Vec3, add, cross, dot, length, scale, sub};

/// Reused per-call buffers, so a step allocates nothing once warm.
#[derive(Default)]
pub struct Scratch {
    pub a: Vec<f32>,
    pub b: Vec<f32>,
    pub c: Vec<f32>,
    pub d: Vec<f32>,
    pub e: Vec<f32>,
    pub f: Vec<f32>,
    pub va: Vec<Vec3>,
    pub vb: Vec<Vec3>,
    pub vc: Vec<Vec3>,
    pub comp: [Vec<f32>; 3],
    pub times: Vec<f32>,
}

const END_CAP: &[(&str, i32)] = &[("PARTICLE_ENDCAP_ALWAYS_ON", -1), ("PARTICLE_ENDCAP_ENDCAP_OFF", 0), ("PARTICLE_ENDCAP_ENDCAP_ON", 1)];

/// The members every function shares: strength, endcap phase and the fade window.
#[derive(Clone, Debug)]
pub struct Common {
    strength: NumIn,
    end_cap: i32,
    fade: [f32; 4],
    oscillate: f32,
    fast: bool,
    unity: bool,
}

impl Common {
    pub fn parse(v: &Value, cx: &mut ParseCx) -> Common {
        let strength = NumIn::parse_or(v, "m_flOpStrength", 1.0, cx);
        let end_cap = kv::enumv(v, "m_nOpEndCapState", END_CAP, -1);
        let fade = [kv::f(v, "m_flOpStartFadeInTime", 0.0), kv::f(v, "m_flOpEndFadeInTime", 0.0), kv::f(v, "m_flOpStartFadeOutTime", 0.0), kv::f(v, "m_flOpEndFadeOutTime", 0.0)];
        let unity = fade == [0.0; 4];
        let fast = unity && end_cap == -1 && strength.literal() == Some(1.0);
        Common { strength, end_cap, fade, oscillate: kv::f(v, "m_flOpFadeOscillatePeriod", 0.0), fast, unity }
    }

    fn function_time(&self, st: &State) -> f32 {
        if self.oscillate > 0.0 {
            return ((1.0 / self.oscillate) * st.age).rem_euclid(1.0);
        }
        (st.age - st.operator_time_base).max(0.0)
    }

    fn past_fade_out(&self, st: &State) -> bool {
        self.oscillate == 0.0 && self.fade[3] > 0.0 && self.function_time(st) > self.fade[3]
    }

    /// CheckIfOperatorShouldRun: the strength this step, 0 to skip.
    pub fn strength(&self, st: &mut State) -> f32 {
        if self.fast {
            return 1.0;
        }
        if self.end_cap != -1 && st.in_end_cap != (self.end_cap == 1) {
            return 0.0;
        }
        let s = self.strength.next(st);
        if self.unity || s <= 0.0 {
            return s;
        }
        s.max(0.0) * fade_in_out(self.fade, self.function_time(st))
    }
}

fn fade_in_out(f: [f32; 4], t: f32) -> f32 {
    let [in0, mut in1, mut out0, mut out1] = f;
    if in0 > t {
        return 0.0;
    }
    if out1 > 0.0 && out1 < t {
        return 0.0;
    }
    in1 = in1.max(in0);
    out0 = out0.max(in1);
    out1 = out1.max(out0);
    let mut s: f32 = 1.0;
    if in1 > t && in1 > in0 {
        s = s.min((t - in0) / (in1 - in0));
    }
    if out1 > out0 && t > out0 {
        s = s.min(1.0 - (t - out0) / (out1 - out0));
    }
    s
}

pub const EVENT_KILLED: i32 = 2;
const EVENT_TYPES: &[(&str, i32)] = &[
    ("PARTICLE_EVENT_TYPE_MASK_NONE", 0),
    ("PARTICLE_EVENT_TYPE_MASK_SPAWNED", 1),
    ("PARTICLE_EVENT_TYPE_MASK_KILLED", 2),
    ("PARTICLE_EVENT_TYPE_MASK_COLLISION", 4),
    ("PARTICLE_EVENT_TYPE_MASK_FIRST_COLLISION", 8),
    ("PARTICLE_EVENT_TYPE_MASK_COLLISION_STOPPED", 16),
    ("PARTICLE_EVENT_TYPE_MASK_KILLED_ON_COLLISION", 32),
    ("PARTICLE_EVENT_TYPE_MASK_USER_1", 64),
    ("PARTICLE_EVENT_TYPE_MASK_USER_2", 128),
    ("PARTICLE_EVENT_TYPE_MASK_USER_3", 256),
    ("PARTICLE_EVENT_TYPE_MASK_USER_4", 512),
    ("PARTICLE_EVENT_TYPE_MASK_CULLED_ON_SPAWN", 1024),
];

#[derive(Clone, Debug)]
pub struct Continuous {
    duration_in: NumIn,
    start_in: NumIn,
    rate: NumIn,
    scale_per_parent: f32,
    from_killed: bool,
    event: i32,
    limit: i32,
    force_first: bool,
    force_last: bool,
    steady_positive: bool,
    steady_time: f32,
    acc: f64,
    emitted: i64,
    time_base: f32,
    duration: f32,
    window_start: f32,
    emitting: bool,
    force_pending: bool,
    /// The rate scales by the bound snapshot's row count.
    snapshot: super::snapshot::Binding,
}

#[derive(Clone, Debug)]
pub struct Instant {
    count: NumIn,
    start_in: NumIn,
    parent_scale: NumIn,
    max_per_frame: i32,
    from_killed: f32,
    event: i32,
    remaining: i64,
    emit_time: f32,
    time_base: f32,
    computed: bool,
    event_settled: bool,
    /// One particle per row of the bound snapshot, at the rows' creation times.
    snapshot: super::snapshot::Binding,
    reset_on_changes: bool,
    snap_index: usize,
    snap_emit_index: usize,
    /// The snapshot the walk last read (its address), to restart it on a change.
    walked: usize,
    emitting: bool,
}

impl Instant {
    fn snapshot_bound(&self) -> bool {
        self.snapshot.is_bound()
    }

    /// The snapshot walk (Grimoire's `updateParticlesToEmit`): one particle per row, released
    /// as the time since the emitter started passes each row's creation time (all at once
    /// without that column). With `m_bResetSnapshotIndexOnChanges` a changed snapshot starts
    /// the walk over, and the walk never finishes.
    fn walk_snapshot(&mut self, st: &State, time: f32) {
        let snap = self.snapshot.resolve(st).cloned();
        let address = snap.as_ref().map_or(0, |s| std::sync::Arc::as_ptr(s) as usize);
        if self.reset_on_changes && address != self.walked {
            self.snap_index = 0;
            self.snap_emit_index = 0;
        }
        self.walked = address;
        let can_finish = !self.reset_on_changes;
        let Some(snap) = snap else {
            self.computed = can_finish;
            return;
        };
        let count = snap.count;
        let Some(created) = snap.scalar(fl::CREATION) else {
            self.remaining += count.saturating_sub(self.snap_index) as i64;
            self.snap_index = self.snap_index.max(count);
            self.computed = can_finish;
            return;
        };
        if self.snap_emit_index >= count {
            self.remaining = 0;
            self.computed = can_finish;
            return;
        }
        let elapsed = time - self.time_base;
        while self.snap_index < count && (self.snap_index >= created.len() || created[self.snap_index] <= elapsed) {
            self.snap_index += 1;
            self.remaining += 1;
        }
    }
}

#[derive(Clone, Debug)]
pub enum EmitterKind {
    Continuous(Continuous),
    Instant(Instant),
}

#[derive(Clone, Debug)]
pub struct Emitter {
    pub c: Common,
    pub index: i32,
    pub kind: EmitterKind,
}

impl Emitter {
    pub fn parse(class: &str, v: &Value, index: i32, cx: &mut ParseCx) -> Option<Emitter> {
        let c = Common::parse(v, cx);
        let kind = match class {
            "C_OP_ContinuousEmitter" => EmitterKind::Continuous(Continuous {
                duration_in: NumIn::parse_or(v, "m_flEmissionDuration", 0.0, cx),
                start_in: NumIn::parse_or(v, "m_flStartTime", 0.0, cx),
                rate: NumIn::parse_or(v, "m_flEmitRate", 100.0, cx),
                scale_per_parent: kv::f(v, "m_flScalePerParentParticle", 0.0),
                from_killed: kv::b(v, "m_bInitFromKilledParentParticles", false),
                event: kv::enumv(v, "m_nEventType", EVENT_TYPES, EVENT_KILLED),
                limit: kv::i(v, "m_nLimitPerUpdate", 0),
                force_first: kv::b(v, "m_bForceEmitOnFirstUpdate", false),
                force_last: kv::b(v, "m_bForceEmitOnLastUpdate", false),
                steady_positive: true,
                steady_time: 0.0,
                acc: 0.0,
                emitted: 0,
                time_base: 0.0,
                duration: 0.0,
                window_start: -1.0,
                emitting: true,
                force_pending: false,
                snapshot: super::snapshot::Binding::parse(v, "m_nSnapshotControlPoint", -1, false),
            }),
            "C_OP_InstantaneousEmitter" => EmitterKind::Instant(Instant {
                count: NumIn::parse_or(v, "m_nParticlesToEmit", 100.0, cx),
                start_in: NumIn::parse_or(v, "m_flStartTime", 0.0, cx),
                parent_scale: NumIn::parse_or(v, "m_flParentParticleScale", -1.0, cx),
                max_per_frame: kv::i(v, "m_nMaxEmittedPerFrame", -1),
                from_killed: kv::f(v, "m_flInitFromKilledParentParticles", 0.0),
                event: kv::enumv(v, "m_nEventType", EVENT_TYPES, EVENT_KILLED),
                remaining: 0,
                emit_time: -1.0,
                time_base: 0.0,
                computed: false,
                event_settled: false,
                snapshot: super::snapshot::Binding::parse(v, "m_nSnapshotControlPoint", -1, false),
                reset_on_changes: kv::b(v, "m_bResetSnapshotIndexOnChanges", false),
                snap_index: 0,
                snap_emit_index: 0,
                walked: 0,
                emitting: true,
            }),
            _ => return None,
        };
        Some(Emitter { c, index, kind })
    }

    /// The parent events the emitter listens for, 0 for an emitter that runs every step.
    pub fn event_mask(&self) -> i32 {
        match &self.kind {
            EmitterKind::Continuous(e) if e.from_killed => e.event,
            EmitterKind::Instant(e) if e.from_killed > 0.0 => e.event,
            _ => 0,
        }
    }

    pub fn reset(&mut self, st: &mut State) {
        match &mut self.kind {
            EmitterKind::Continuous(e) => {
                e.steady_positive = true;
                e.steady_time = 0.0;
                if let Some((t, v)) = e.rate.steady() {
                    e.steady_time = t;
                    e.steady_positive = v > 0.0;
                }
                e.acc = 0.0;
                e.emitted = 0;
                e.time_base = 0.0;
                e.window_start = -1.0;
                e.emitting = true;
                e.force_pending = false;
                let _ = st;
            }
            EmitterKind::Instant(e) => {
                e.remaining = 0;
                e.emit_time = -1.0;
                e.time_base = 0.0;
                e.computed = false;
                e.event_settled = false;
                e.snap_index = 0;
                e.snap_emit_index = 0;
                e.walked = 0;
                e.emitting = true;
            }
        }
    }

    pub fn start_emission(&mut self) {
        match &mut self.kind {
            EmitterKind::Continuous(e) => e.emitting = true,
            EmitterKind::Instant(e) => e.emitting = true,
        }
    }

    pub fn stop_emission(&mut self) {
        match &mut self.kind {
            EmitterKind::Continuous(e) => {
                if e.force_last {
                    e.force_pending = true;
                }
                e.emitting = false;
            }
            EmitterKind::Instant(e) => e.emitting = false,
        }
    }

    pub fn may_create_more(&self, st: &State) -> bool {
        match &self.kind {
            EmitterKind::Continuous(e) => {
                if !e.emitting && !e.force_pending {
                    return false;
                }
                if e.window_start == -1.0 {
                    return !st.in_end_cap || st.age <= 2.0;
                }
                if e.from_killed || self.c.past_fade_out(st) {
                    return false;
                }
                if e.duration != 0.0 && st.age - st.current_step > e.window_start + e.duration {
                    return false;
                }
                e.steady_positive || st.age <= e.window_start + e.steady_time
            }
            EmitterKind::Instant(e) => {
                if e.from_killed > 0.0 {
                    e.emitting && (!e.event_settled || e.remaining > 0)
                } else {
                    e.emitting && (!e.computed || e.remaining > 0)
                }
            }
        }
    }

    /// Pushes the creation times of the particles this step emits into `times`.
    pub fn emit(&mut self, st: &mut State, strength: f32, parent_count: Option<usize>, free: usize, times: &mut Vec<f32>) {
        times.clear();
        let past_fade = self.c.past_fade_out(st);
        match &mut self.kind {
            EmitterKind::Continuous(e) => {
                e.duration = e.duration_in.next(st);
                let start = e.time_base + e.start_in.next(st);
                e.window_start = start;
                let time = st.age;
                let step = st.current_step;
                if start > time {
                    return;
                }
                let mut rate = (strength * e.rate.at_age(st, if matches!(e.rate, NumIn::CollectionAge(_)) { st.age - start } else { st.age })).max(0.0);
                if rate <= 0.0 {
                    return;
                }
                let finite = e.duration != 0.0 || !e.steady_positive;
                let span_len = if e.steady_positive {
                    e.duration
                } else if e.duration == 0.0 {
                    e.steady_time
                } else {
                    e.duration.min(e.steady_time)
                };
                let (previous, mut current, span, budget);
                current = time;
                if e.from_killed {
                    let events = st.event_count;
                    if events == 0 {
                        return;
                    }
                    if finite {
                        current = current.max(start).min(start + span_len);
                    }
                    previous = current;
                    span = 0.0;
                    budget = events as f64;
                } else {
                    if e.scale_per_parent > 0.0 {
                        rate *= parent_count.map_or(e.scale_per_parent, |n| n as f32 * e.scale_per_parent);
                    }
                    if e.snapshot.has_cp() {
                        rate *= e.snapshot.count(st) as f32;
                    }
                    if rate <= 0.0 || (!e.emitting && !e.force_pending) || past_fade {
                        return;
                    }
                    if e.duration != 0.0 && time - step > start + e.duration {
                        return;
                    }
                    let mut prev = st.step_start_age;
                    if finite {
                        prev = prev.max(start);
                        current = current.min(start + span_len);
                    }
                    previous = prev;
                    span = (current - previous).min(step);
                    budget = (rate * span).max(0.0) as f64;
                }
                e.acc += budget;
                let mut count = ((e.acc + 0.001).floor() as i64 - e.emitted).max(0);
                if e.force_first && time - step == 0.0 {
                    count = count.max(1);
                }
                if e.force_pending {
                    e.force_pending = false;
                    count = count.max(1);
                }
                if e.limit > 0 {
                    count = count.min(e.limit as i64);
                }
                if count <= 0 {
                    return;
                }
                e.emitted += count;
                let count = count.min(free as i64);
                if count <= 0 {
                    return;
                }
                let creation_step = span / count as f32;
                let mut t = creation_step + previous;
                for _ in 0..count {
                    let pt = t.min(current);
                    times.push(pt);
                    t = pt + creation_step;
                }
            }
            EmitterKind::Instant(e) => {
                if !e.emitting {
                    return;
                }
                let time = st.age;
                if e.from_killed > 0.0 {
                    let burst = e.count.next(st) as i64;
                    e.remaining = (st.event_count as f32 * (burst as f32 * e.from_killed)) as i64;
                    e.emit_time = time;
                    if e.count.literal().is_some() && time > 0.0 {
                        e.event_settled = true;
                    }
                } else {
                    e.emit_time = e.time_base + e.start_in.next(st);
                    if e.emit_time > time {
                        return;
                    }
                    if !e.snapshot_bound() {
                        if !e.computed {
                            let mut k = 1.0;
                            if let Some(n) = parent_count {
                                let ps = e.parent_scale.next(st);
                                if ps != -1.0 {
                                    k = n as f32 * ps;
                                }
                            }
                            e.remaining = ((k * e.count.next(st)) as i64).max(0);
                            e.computed = true;
                        }
                    } else if !e.computed {
                        e.walk_snapshot(st, time);
                    }
                }
                if e.emit_time > st.age || e.remaining <= 0 {
                    return;
                }
                let cap = if e.max_per_frame >= 0 { e.max_per_frame as i64 } else { st.max_particles as i64 };
                let claimed = cap.min(e.remaining);
                e.remaining -= claimed;
                let count = (free as i64).min((claimed as f32 * strength) as i64);
                let snap = if e.snapshot_bound() { e.snapshot.resolve(st).cloned() } else { None };
                let created = snap.as_ref().and_then(|s| s.scalar(fl::CREATION));
                for _ in 0..count.max(0) {
                    let offset = created.and_then(|c| c.get(e.snap_emit_index)).copied().unwrap_or(0.0);
                    e.snap_emit_index += 1;
                    times.push(e.emit_time + offset);
                }
            }
        }
    }
}

const SCALAR_EXPRESSIONS: &[(&str, i32)] = &[
    ("SCALAR_EXPRESSION_UNINITIALIZED", -1),
    ("SCALAR_EXPRESSION_ADD", 0),
    ("SCALAR_EXPRESSION_SUBTRACT", 1),
    ("SCALAR_EXPRESSION_MUL", 2),
    ("SCALAR_EXPRESSION_DIVIDE", 3),
    ("SCALAR_EXPRESSION_INPUT_1", 4),
    ("SCALAR_EXPRESSION_MIN", 5),
    ("SCALAR_EXPRESSION_MAX", 6),
    ("SCALAR_EXPRESSION_MOD", 7),
    ("SCALAR_EXPRESSION_EQUAL", 8),
    ("SCALAR_EXPRESSION_GT", 9),
    ("SCALAR_EXPRESSION_LT", 10),
];

fn scalar_expression(e: i32, a: f32, b: f32) -> Option<f32> {
    Some(match e {
        0 => a + b,
        1 => a - b,
        2 => a * b,
        3 => {
            if b == 0.0 {
                0.0
            } else {
                a / b
            }
        }
        4 => a,
        5 => a.min(b),
        6 => a.max(b),
        7 => {
            if b == 0.0 {
                a
            } else {
                a - b * (a / b).trunc()
            }
        }
        8 => (a == b) as i32 as f32,
        9 => (a > b) as i32 as f32,
        10 => (a < b) as i32 as f32,
        _ => return None,
    })
}

#[derive(Clone, Debug)]
pub enum Init {
    Float { field: i32, value: NumIn, strength: NumIn, method: i32 },
    Sphere {
        xf: TransIn,
        r_min: NumIn,
        r_max: NumIn,
        s_min: NumIn,
        s_max: NumIn,
        exp: f32,
        local_min: VecIn,
        local_max: VecIn,
        bias: VecIn,
        bias_abs: Vec3,
        local: bool,
        out: i32,
        vel: i32,
    },
    Offset { min: VecIn, max: VecIn, xf: TransIn, local: bool, proportional: bool },
    Box { min: VecIn, max: VecIn, cp: i32, local: bool },
    Color { min: Vec3, max: Vec3, field: i32 },
    YawFlip { percent: f32 },
    Sequence { min: i32, max: i32, shuffle: bool, linear: bool, weighted: Vec<(i32, f32)>, total: f32, cursor0: f32, cursor: f32, table: [i32; 64], next: i32 },
    Ring { even: bool, radius: NumIn, thickness: NumIn, per_orbit: NumIn, s_min: NumIn, s_max: NumIn, roll: NumIn, pitch: NumIn, yaw: NumIn, xf: TransIn, xy_only: bool, orbit: u32 },
    Vec { field: i32, value: VecIn, method: i32, normalized: bool, write_prev: bool },
    DistanceToCp { field: i32, in_min: NumIn, in_max: NumIn, out_min: NumIn, out_max: NumIn, cp: i32, method: i32, active: bool, scale: Vec3, bias: f32 },
    ScalarExpr { field: i32, a: NumIn, b: NumIn, expr: i32, method: i32, remap: Option<Mapping> },
    FromParentKilled { field: i32 },
    Skinned(Box<super::snapshot::SkinnedInit>),
}

#[derive(Clone, Debug)]
pub struct Initializer {
    pub c: Common,
    pub emitter: i32,
    pub random: bool,
    pub written: u64,
    pub kind: Init,
}

impl Initializer {
    pub fn parse(class: &str, v: &Value, cx: &mut ParseCx) -> Option<Initializer> {
        cx.random_inputs = false;
        let c = Common::parse(v, cx);
        let num = |k: &str, d: f32, cx: &mut ParseCx| NumIn::parse_or(v, k, d, cx);
        let vec = |k: &str, d: Vec3, cx: &mut ParseCx| VecIn::parse_or(v, k, d, cx);
        let method = kv::enumv(v, "m_nSetMethod", fl::SET_METHODS, 0);
        let cp0 = TransIn::Cp { cp: 0, orient: true };
        let pp = mask(fl::POSITION as i32) | mask(fl::PREV as i32);
        let (kind, written) = match class {
            "C_INIT_InitFloat" => {
                let field = kv::field(v, "m_nOutputField", fl::RADIUS);
                (Init::Float { field, value: num("m_InputValue", 0.0, cx), strength: num("m_InputStrength", 1.0, cx), method }, mask(field))
            }
            "C_INIT_CreateWithinSphereTransform" => {
                let out = kv::field(v, "m_nFieldOutput", fl::POSITION);
                let vel = kv::field(v, "m_nFieldVelocity", fl::PREV);
                let w = if out == fl::POSITION as i32 { pp | mask(vel) } else { mask(out) | mask(vel) };
                (
                    Init::Sphere {
                        xf: TransIn::parse_or(v, "m_TransformInput", cp0, cx),
                        r_min: num("m_fRadiusMin", 0.0, cx),
                        r_max: num("m_fRadiusMax", 0.0, cx),
                        s_min: num("m_fSpeedMin", 0.0, cx),
                        s_max: num("m_fSpeedMax", 0.0, cx),
                        exp: kv::f(v, "m_fSpeedRandExp", 1.0).clamp(-255.0, 255.0),
                        local_min: vec("m_LocalCoordinateSystemSpeedMin", [0.0; 3], cx),
                        local_max: vec("m_LocalCoordinateSystemSpeedMax", [0.0; 3], cx),
                        bias: vec("m_vecDistanceBias", [1.0; 3], cx),
                        bias_abs: kv::v3(v, "m_vecDistanceBiasAbs", [0.0; 3]),
                        local: kv::b(v, "m_bLocalCoords", false),
                        out,
                        vel,
                    },
                    w,
                )
            }
            "C_INIT_PositionOffset" => (
                Init::Offset {
                    min: vec("m_OffsetMin", [0.0; 3], cx),
                    max: vec("m_OffsetMax", [0.0; 3], cx),
                    xf: TransIn::parse_or(v, "m_TransformInput", cp0, cx),
                    local: kv::b(v, "m_bLocalCoords", false),
                    proportional: kv::b(v, "m_bProportional", false),
                },
                pp,
            ),
            "C_INIT_CreateWithinBox" => (
                Init::Box { min: vec("m_vecMin", [0.0; 3], cx), max: vec("m_vecMax", [0.0; 3], cx), cp: kv::i(v, "m_nControlPointNumber", 0).clamp(0, 63), local: kv::b(v, "m_bLocalSpace", false) },
                pp,
            ),
            "C_INIT_RandomColor" => {
                let field = kv::field(v, "m_nFieldOutput", fl::COLOR);
                if kv::f(v, "m_flTintPerc", 0.0) > 0.0 {
                    cx.note("C_INIT_RandomColor light tint (m_flTintPerc) is not applied".into());
                }
                (Init::Color { min: kv::color(v, "m_ColorMin", [1.0; 3]), max: kv::color(v, "m_ColorMax", [1.0; 3]), field }, mask(field))
            }
            "C_INIT_RandomYawFlip" => (Init::YawFlip { percent: kv::f(v, "m_flPercent", 0.5) }, mask(fl::YAW as i32)),
            "C_INIT_RandomSequence" => {
                let mut min = kv::i(v, "m_nSequenceMin", 0);
                let mut max = kv::i(v, "m_nSequenceMax", 0);
                let shuffle = kv::b(v, "m_bShuffle", false);
                let linear = kv::b(v, "m_bLinear", false);
                let mut weighted = Vec::new();
                let mut total = 0.0;
                let entries = kv::array(v, "m_WeightedList");
                if !entries.is_empty() {
                    min = i32::MAX;
                    max = 0;
                    for e in entries {
                        let s = kv::i(e, "m_nSequence", 0);
                        let w = kv::f(e, "m_flRelativeWeight", 1.0);
                        weighted.push((s, w));
                        min = min.min(s);
                        max = max.max(s);
                        if w > 0.0 {
                            total += w;
                        }
                    }
                }
                let cursor0 = if !weighted.is_empty() && (shuffle || linear) { min as f32 } else { 0.0 };
                (Init::Sequence { min, max, shuffle, linear, weighted, total, cursor0, cursor: cursor0, table: [0; 64], next: 0 }, mask(fl::SEQUENCE as i32))
            }
            "C_INIT_RingWave" => (
                Init::Ring {
                    even: kv::b(v, "m_bEvenDistribution", false),
                    per_orbit: num("m_flParticlesPerOrbit", -1.0, cx),
                    radius: num("m_flInitialRadius", 0.0, cx),
                    thickness: num("m_flThickness", 0.0, cx),
                    s_min: num("m_flInitialSpeedMin", 0.0, cx),
                    s_max: num("m_flInitialSpeedMax", 0.0, cx),
                    roll: num("m_flRoll", 0.0, cx),
                    pitch: num("m_flPitch", 0.0, cx),
                    yaw: num("m_flYaw", 0.0, cx),
                    xf: TransIn::parse_or(v, "m_TransformInput", cp0, cx),
                    xy_only: kv::b(v, "m_bXYVelocityOnly", true),
                    orbit: 0,
                },
                pp,
            ),
            "C_INIT_InitVec" => {
                let field = kv::field(v, "m_nOutputField", fl::COLOR);
                let write_prev = kv::b(v, "m_bWritePreviousPosition", false);
                let w = if write_prev && field == fl::POSITION as i32 { pp } else { mask(field) };
                (Init::Vec { field, value: vec("m_InputValue", [0.0; 3], cx), method, normalized: kv::b(v, "m_bNormalizedOutput", false), write_prev }, w)
            }
            "C_INIT_DistanceToCPInit" => {
                let field = kv::field(v, "m_nFieldOutput", fl::RADIUS);
                if kv::b(v, "m_bLOS", false) {
                    cx.note("C_INIT_DistanceToCPInit line of sight is not traced".into());
                }
                (
                    Init::DistanceToCp {
                        field,
                        in_min: num("m_flInputMin", 0.0, cx),
                        in_max: num("m_flInputMax", 128.0, cx),
                        out_min: num("m_flOutputMin", 0.0, cx),
                        out_max: num("m_flOutputMax", 1.0, cx),
                        cp: kv::i(v, "m_nStartCP", 0).clamp(0, 63),
                        method,
                        active: kv::b(v, "m_bActiveRange", false),
                        scale: kv::v3(v, "m_vecDistanceScale", [1.0; 3]),
                        bias: kv::f(v, "m_flRemapBias", 0.5),
                    },
                    mask(field),
                )
            }
            "C_INIT_SetAttributeToScalarExpression" => {
                let field = kv::field(v, "m_nOutputField", fl::RADIUS);
                (
                    Init::ScalarExpr {
                        field,
                        a: num("m_flInput1", 0.0, cx),
                        b: num("m_flInput2", 0.0, cx),
                        expr: kv::enumv(v, "m_nExpression", SCALAR_EXPRESSIONS, 0),
                        method,
                        remap: kv::get(v, "m_flOutputRemap").filter(|x| x.is_object()).map(Mapping::parse_remap),
                    },
                    mask(field),
                )
            }
            "C_INIT_InitSkinnedPositionFromCPSnapshot" => {
                cx.skinned = true;
                let (init, written) = super::snapshot::SkinnedInit::parse(v, cx);
                (Init::Skinned(Box::new(init)), written)
            }
            "C_INIT_InitFromParentKilled" => {
                let field = kv::field(v, "m_nAttributeToCopy", usize::MAX);
                let copies = field >= 0 && field != fl::NONE as i32;
                (Init::FromParentKilled { field }, if copies { mask(field) } else { 0 })
            }
            _ => return None,
        };
        Some(Initializer { c, emitter: kv::i(v, "m_nAssociatedEmitterIndex", -1), random: cx.random_inputs, written, kind })
    }

    pub fn reset(&mut self, st: &mut State) {
        match &mut self.kind {
            Init::Ring { orbit, .. } => *orbit = 0,
            Init::Skinned(s) => s.reset(),
            Init::Sequence { min, max, shuffle, linear, cursor0, cursor, table, next, .. } => {
                *cursor = *cursor0;
                *table = [0; 64];
                *next = 0;
                if *shuffle || *linear {
                    *next = *min;
                    for s in (*min).max(0)..=(*max).min(63) {
                        table[s as usize] = s;
                    }
                    if *shuffle {
                        shuffle_table(table, *min, *max, st);
                    }
                }
            }
            _ => {}
        }
    }

    /// Runs over the new particles `first..first + n`. `parent` is the parent's particles and
    /// the indices of the ones that raised the event, for event emitters.
    pub fn run(&mut self, st: &mut State, p: &mut Particles, first: usize, n: usize, sc: &mut Scratch, parent: Option<(&Particles, &[usize])>) {
        let unwritten = |st: &State, f: usize| st.written & (1u64 << f) == 0;
        match &mut self.kind {
            Init::Float { field, value, strength, method } => {
                value.fill(st, p, first, n, &mut sc.a);
                strength.fill(st, p, first, n, &mut sc.b);
                let f = *field;
                if is_vector(f) {
                    for k in 0..n {
                        let cur = p.v[f as usize][first + k][0];
                        let t = apply_set(sc.a[k], cur, cur, st.current_step, *method);
                        p.v[f as usize][first + k][0] = lerp(cur, t, sc.b[k]);
                    }
                    return;
                }
                if !is_scalar(f) {
                    return;
                }
                let conv = is_angle(f) && *method != fl::SET_SCALE_INITIAL && *method != fl::SET_SCALE_CURRENT;
                let integer = f == fl::PARTICLE_ID as i32;
                for k in 0..n {
                    let v = if conv { sc.a[k].to_radians() } else { sc.a[k] };
                    let cur = p.s[f as usize][first + k];
                    let r = lerp(cur, apply_set(v, cur, cur, st.current_step, *method), sc.b[k]);
                    p.s[f as usize][first + k] = if integer { r.trunc() } else { r };
                }
            }
            Init::Sphere { xf, r_min, r_max, s_min, s_max, exp, local_min, local_max, bias, bias_abs, local, out, vel } => {
                let writes_vel = *vel >= 0 && *vel != fl::NONE as i32 && unwritten(st, *vel as usize);
                r_min.fill(st, p, first, n, &mut sc.a);
                r_max.fill(st, p, first, n, &mut sc.b);
                s_min.fill(st, p, first, n, &mut sc.c);
                s_max.fill(st, p, first, n, &mut sc.d);
                bias.fill(st, p, first, n, &mut sc.va, &mut sc.comp);
                local_min.fill(st, p, first, n, &mut sc.vb, &mut sc.comp);
                local_max.fill(st, p, first, n, &mut sc.vc, &mut sc.comp);
                for k in 0..n {
                    let i = first + k;
                    let (mut rv, frac) = st.rnd.unit_ball();
                    for a in 0..3 {
                        if bias_abs[a] != 0.0 {
                            rv[a] = rv[a].abs();
                        }
                    }
                    let b = sc.va[k];
                    let mut dir = safe_normalize(mulv(rv, b));
                    let t = xf.at(st, p.s[fl::CREATION][i]);
                    if *local && (b != [1.0; 3] || *bias_abs != [0.0; 3]) {
                        dir = t.vector(dir);
                    }
                    let world = add(t.o, scale(dir, lerp(sc.a[k], sc.b[k], frac)));
                    if is_vector(*out) {
                        p.v[*out as usize][i] = world;
                    }
                    if !writes_vel {
                        continue;
                    }
                    let draw = st.rnd.next();
                    let speed = lerp(sc.c[k], sc.d[k], pow_quarter(draw, *exp));
                    let lcs = st.rnd.between_per_component(sc.vb[k], sc.vc[k]);
                    let vl = t.vector(lcs);
                    let mut velocity = sub(scale(scale(dir, speed), -1.0), vl);
                    if *vel == fl::PREV as i32 {
                        velocity = scale(velocity, st.previous_step);
                    } else if *out == fl::POSITION as i32 {
                        p.v[fl::PREV][i] = world;
                    }
                    if is_vector(*vel) {
                        p.v[*vel as usize][i] = add(world, velocity);
                    }
                }
            }
            Init::Offset { min, max, xf, local, proportional } => {
                let rot = if *local { xf.now(st) } else { Xf::IDENTITY };
                min.fill(st, p, first, n, &mut sc.va, &mut sc.comp);
                max.fill(st, p, first, n, &mut sc.vb, &mut sc.comp);
                for k in 0..n {
                    let i = first + k;
                    let mut off = st.rnd.between_per_component(sc.va[k], sc.vb[k]);
                    if *proportional {
                        off = scale(off, p.s[fl::RADIUS][i]);
                    }
                    if *local {
                        off = rot.vector(off);
                    }
                    p.v[fl::POSITION][i] = add(p.v[fl::POSITION][i], off);
                    p.v[fl::PREV][i] = add(p.v[fl::PREV][i], off);
                }
            }
            Init::Box { min, max, cp, local } => {
                let write_prev = unwritten(st, fl::PREV);
                min.fill(st, p, first, n, &mut sc.va, &mut sc.comp);
                max.fill(st, p, first, n, &mut sc.vb, &mut sc.comp);
                for k in 0..n {
                    let i = first + k;
                    let mut pos = st.rnd.between_per_component(sc.va[k], sc.vb[k]);
                    let time = p.s[fl::CREATION][i];
                    pos = if *local { st.cp_xf_at(*cp, time, true).point(pos) } else { add(pos, st.cp_position_at(*cp, time)) };
                    p.v[fl::POSITION][i] = pos;
                    if write_prev {
                        p.v[fl::PREV][i] = pos;
                    }
                }
            }
            Init::Color { min, max, field } => {
                for k in 0..n {
                    let c = lerpv(*min, *max, st.rnd.next());
                    if is_vector(*field) {
                        p.v[*field as usize][first + k] = c;
                    }
                }
            }
            Init::YawFlip { percent } => {
                for k in 0..n {
                    if st.rnd.next() < *percent {
                        let y = &mut p.s[fl::YAW][first + k];
                        *y = (*y % std::f32::consts::PI) + std::f32::consts::PI;
                    }
                }
            }
            Init::Sequence { min, max, shuffle, linear, weighted, total, cursor, table, next, .. } => {
                for k in 0..n {
                    let seq = if !weighted.is_empty() {
                        let mut s = 0;
                        if *total > 0.0 {
                            let mut pick = if *linear { *cursor } else { *total * st.rnd.next() };
                            for &(es, w) in weighted.iter() {
                                if w <= 0.0 {
                                    continue;
                                }
                                pick -= w;
                                s = es;
                                if pick < 0.0 {
                                    break;
                                }
                            }
                        }
                        if *linear {
                            *cursor += 1.0;
                            if *cursor >= total.ceil() {
                                *cursor = 0.0;
                            }
                        }
                        s
                    } else if *shuffle || *linear {
                        if *next > *max {
                            if *shuffle {
                                shuffle_table(table, *min, *max, st);
                            }
                            *next = *min;
                        }
                        let s = if (0..64).contains(next) { table[*next as usize] } else { 0 };
                        *next += 1;
                        s
                    } else {
                        *min + st.rnd.next_index(*max - *min + 1)
                    };
                    p.s[fl::SEQUENCE][first + k] = seq as f32;
                }
            }
            Init::Ring { even, radius, thickness, per_orbit, s_min, s_max, roll, pitch, yaw, xf, xy_only, orbit } => {
                thickness.fill(st, p, first, n, &mut sc.a);
                let per = per_orbit.next(st);
                radius.fill(st, p, first, n, &mut sc.b);
                pitch.fill(st, p, first, n, &mut sc.c);
                yaw.fill(st, p, first, n, &mut sc.d);
                roll.fill(st, p, first, n, &mut sc.e);
                s_min.fill(st, p, first, n, &mut sc.f);
                s_max.fill(st, p, first, n, &mut sc.times);
                let write_prev = unwritten(st, fl::PREV);
                let active = p.len;
                let dt = st.previous_step;
                for k in 0..n {
                    let i = first + k;
                    let thick = scale(st.rnd.unit_ball().0, sc.a[k]);
                    let r = sc.b[k];
                    let m = xf.at(st, p.s[fl::CREATION][i]);
                    *orbit = orbit.wrapping_add(1);
                    let angle = if *even {
                        let po = if per == -1.0 { active as f32 } else { per };
                        *orbit as f32 * (std::f32::consts::TAU / po.max(1.0))
                    } else {
                        *orbit as f32 * (st.rnd.next() * std::f32::consts::TAU)
                    };
                    let [ax, ay, az] = euler_axes_deg([sc.c[k], sc.d[k], sc.e[k]]);
                    let ring = Xf { o: [0.0; 3], x: ax, y: ay, z: az }.vector([r * angle.cos(), r * angle.sin(), 0.0]);
                    let pos = m.point(add(ring, thick));
                    let outward = safe_normalize(sub(pos, m.o));
                    let speed = st.rnd.next_between(sc.f[k], sc.times[k]);
                    let mut prev = sub(pos, scale(outward, speed * dt));
                    if *xy_only {
                        prev[2] = pos[2];
                    }
                    p.v[fl::POSITION][i] = pos;
                    if write_prev {
                        p.v[fl::PREV][i] = prev;
                    }
                }
            }
            Init::Vec { field, value, method, normalized, write_prev } => {
                value.fill(st, p, first, n, &mut sc.va, &mut sc.comp);
                if !is_vector(*field) {
                    return;
                }
                let f = *field as usize;
                for k in 0..n {
                    let i = first + k;
                    let cur = p.v[f][i];
                    let mut v = apply_set_v(sc.va[k], cur, cur, st.current_step, *method);
                    if *normalized {
                        v = safe_normalize(v);
                    }
                    p.v[f][i] = v;
                    if *write_prev && f == fl::POSITION {
                        p.v[fl::PREV][i] = v;
                    }
                }
            }
            Init::DistanceToCp { field, in_min, in_max, out_min, out_max, cp, method, active, scale: dscale, bias: rbias } => {
                let cpp = st.cp(*cp).pos;
                in_min.fill(st, p, first, n, &mut sc.a);
                in_max.fill(st, p, first, n, &mut sc.b);
                out_min.fill(st, p, first, n, &mut sc.c);
                out_max.fill(st, p, first, n, &mut sc.d);
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                let alpha = f == fl::ALPHA || f == fl::ALPHA2;
                for k in 0..n {
                    let i = first + k;
                    let (i0, i1) = (sc.a[k], sc.b[k]);
                    let (mut o0, mut o1) = (sc.c[k], sc.d[k]);
                    if alpha {
                        o0 = sat(o0);
                        o1 = sat(o1);
                    }
                    let d = length(mulv(sub(cpp, p.v[fl::POSITION][i]), *dscale));
                    if *active && (d < i0 || d > i1) {
                        continue;
                    }
                    let mut v = if i0 == i1 { if d >= i1 { o1 } else { o0 } } else { lerp(o0, o1, sat((d - i0) / (i1 - i0))) };
                    if *rbias != 0.5 {
                        v = bias(v, *rbias);
                    }
                    let cur = p.s[f][i];
                    p.s[f][i] = apply_set(v, cur, cur, st.current_step, *method);
                }
            }
            Init::ScalarExpr { field, a, b, expr, method, remap } => {
                a.fill(st, p, first, n, &mut sc.a);
                b.fill(st, p, first, n, &mut sc.b);
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                let (lo, hi) = float_range(*field);
                for k in 0..n {
                    let i = first + k;
                    let cur = p.s[f][i];
                    let mut v = scalar_expression(*expr, sc.a[k], sc.b[k]).unwrap_or(cur);
                    if let Some(m) = remap {
                        v = m.apply(v);
                    }
                    p.s[f][i] = apply_set(v, cur, cur, st.current_step, *method).clamp(lo, hi);
                }
            }
            Init::Skinned(s) => s.run(st, p, first, n, &mut sc.a),
            Init::FromParentKilled { field } => {
                let f = *field;
                let Some((parent, indices)) = parent else { return };
                if f < 0 || f == fl::NONE as i32 || indices.is_empty() || !unwritten(st, f as usize) {
                    return;
                }
                let mut next = 0;
                for k in 0..n {
                    let src = indices[next];
                    next = if next + 1 < indices.len() { next + 1 } else { 0 };
                    if src >= parent.len {
                        continue;
                    }
                    if is_vector(f) {
                        p.v[f as usize][first + k] = parent.v[f as usize][src];
                    } else if is_scalar(f) {
                        p.s[f as usize][first + k] = parent.s[f as usize][src];
                    }
                }
            }
        }
    }
}

fn shuffle_table(table: &mut [i32; 64], min: i32, max: i32, st: &mut State) {
    let get = |t: &[i32; 64], s: i32| if (0..64).contains(&s) { t[s as usize] } else { 0 };
    let mut i = max;
    while i > min {
        let j = min + st.rnd.next_index(i - min + 1);
        let a = get(table, i);
        let b = get(table, j);
        if (0..64).contains(&i) {
            table[i as usize] = b;
        }
        if (0..64).contains(&j) {
            table[j as usize] = a;
        }
        i -= 1;
    }
}

#[derive(Clone, Debug)]
pub enum Force {
    Attract { scale: Vec3, amount: NumIn, min_dist: NumIn, amount_min: NumIn, apply_min: bool, falloff: f32, xf: TransIn },
    Twist { amount: f32, axis: Vec3, local: bool, cp: i32, axis_cp: i32 },
}

#[derive(Clone, Debug)]
pub struct ForceGen {
    pub c: Common,
    pub kind: Force,
}

impl ForceGen {
    pub fn parse(class: &str, v: &Value, behavior: i32, cx: &mut ParseCx) -> Option<ForceGen> {
        let c = Common::parse(v, cx);
        let kind = match class {
            "C_OP_AttractToControlPoint" => Force::Attract {
                scale: kv::v3(v, "m_vecComponentScale", [1.0; 3]),
                amount: NumIn::parse_or(v, "m_fForceAmount", 100.0, cx),
                min_dist: NumIn::parse_or(v, "m_fMinimumDistance", 0.0, cx),
                amount_min: NumIn::parse_or(v, "m_fForceAmountMin", 0.0, cx),
                apply_min: kv::b(v, "m_bApplyMinForce", false),
                falloff: kv::f(v, "m_fFalloffPower", 2.0),
                xf: TransIn::parse_or(v, "m_TransformInput", TransIn::Cp { cp: 0, orient: false }, cx),
            },
            "C_OP_TwistAroundAxis" => {
                let cp = kv::i(v, "m_nControlPointNumber", 0);
                Force::Twist {
                    amount: kv::f(v, "m_fForceAmount", 0.0),
                    axis: kv::v3(v, "m_TwistAxis", [0.0, 0.0, 1.0]),
                    local: kv::b(v, "m_bLocalSpace", false),
                    cp,
                    axis_cp: if behavior >= 3 { cp } else { 0 },
                }
            }
            _ => return None,
        };
        Some(ForceGen { c, kind })
    }

    fn generate(&self, st: &mut State, p: &Particles, forces: &mut [Vec3], strength: f32, sc: &mut Scratch) {
        match &self.kind {
            Force::Attract { scale: cs, amount, min_dist, amount_min, apply_min, falloff, xf } => {
                let t = xf.now(st);
                let s = scale(t.vector(*cs), strength);
                amount.fill(st, p, 0, p.len, &mut sc.a);
                if *apply_min {
                    amount_min.fill(st, p, 0, p.len, &mut sc.b);
                }
                min_dist.fill(st, p, 0, p.len, &mut sc.c);
                for i in 0..p.len {
                    let mut a = sc.a[i];
                    if *apply_min {
                        a = a.max(sc.b[i]);
                    }
                    let diff = sub(p.v[fl::POSITION][i], t.o);
                    let mut d = length(diff);
                    if d < sc.c[i] {
                        d = 1.0;
                    }
                    if d <= EPS {
                        continue;
                    }
                    let mag = (a / d) * pow_quarter(d, -*falloff);
                    forces[i] = sub(forces[i], mulv(scale(diff, mag), s));
                }
            }
            Force::Twist { amount, axis, local, cp, axis_cp } => {
                let ax = safe_normalize(if *local { st.cp(*axis_cp).rotation().vector(*axis) } else { *axis });
                let center = st.cp(*cp).pos;
                let k = amount * strength;
                for i in 0..p.len {
                    let delta = sub(p.v[fl::POSITION][i], center);
                    if dot(delta, delta) <= EPS {
                        continue;
                    }
                    let dir = safe_normalize(delta);
                    let al = 1.0 - dot(dir, ax);
                    if al * al <= EPS {
                        continue;
                    }
                    forces[i] = add(forces[i], scale(cross(dir, ax), k));
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Op {
    Decay { rope: bool },
    RigidSkin(super::snapshot::RigidSkin),
    Movement { gravity: VecIn, drag: NumIn },
    PositionLock {
        xf: TransIn,
        start: [f32; 3],
        end: [f32; 3],
        range: f32,
        range_bias: NumIn,
        jump2: f32,
        prev_scale: f32,
        lock_rot: bool,
        scale: VecIn,
        out: i32,
        out_prev: i32,
        prev_xf: Option<Xf>,
    },
    SetFloat { field: i32, value: NumIn, method: i32, lerp: NumIn },
    SetVec { field: i32, value: VecIn, method: i32, lerp: NumIn, normalized: bool },
    Ramp { rate: f32, start: f32, end: f32, field: i32 },
    InterpolateRadius { start: f32, end: f32, s0: NumIn, s1: NumIn, ease: bool, bias: f32 },
    FadeIn { time: f32, field: i32 },
    FadeOut { time: f32, field: i32 },
    ColorInterpolate { fade: Vec3, start: f32, end: f32, field: i32, ease: bool },
    SpinUpdate,
    EndCapTimedDecay { time: NumIn },
    LerpEndCapScalar { field: i32, output: f32, time: f32, start_age: f32 },
    FadeAndKill { in0: f32, in1: f32, out0: f32, out1: f32, a0: f32, a1: f32 },
    AlphaDecay { min: f32 },
    ScalarExpr { field: i32, a: NumIn, b: NumIn, expr: i32, method: i32, remap: Option<Mapping> },
    Oscillate { field: i32, rate: Vec3, freq: Vec3, mult: f32, add: f32, offset: bool },
}

#[derive(Clone, Debug)]
pub struct Operator {
    pub c: Common,
    pub kind: Op,
}

impl Operator {
    pub fn parse(class: &str, v: &Value, cx: &mut ParseCx) -> Option<Operator> {
        let c = Common::parse(v, cx);
        let num = |k: &str, d: f32, cx: &mut ParseCx| NumIn::parse_or(v, k, d, cx);
        let method = kv::enumv(v, "m_nSetMethod", fl::SET_METHODS, 0);
        let kind = match class {
            "C_OP_Decay" => Op::Decay { rope: kv::b(v, "m_bRopeDecay", false) },
            "C_OP_BasicMovement" => Op::Movement { gravity: VecIn::parse_or(v, "m_Gravity", [0.0; 3], cx), drag: num("m_fDrag", 0.0, cx) },
            "C_OP_PositionLock" => {
                let t = kv::f(v, "m_flJumpThreshold", 512.0);
                Op::PositionLock {
                    xf: TransIn::parse_or(v, "m_TransformInput", TransIn::Cp { cp: 0, orient: true }, cx),
                    start: [kv::f(v, "m_flStartTime_min", 1.0), kv::f(v, "m_flStartTime_max", 1.0), (kv::f(v, "m_flStartTime_exp", 1.0) * 4.0).trunc() / 4.0],
                    end: [kv::f(v, "m_flEndTime_min", 1.0), kv::f(v, "m_flEndTime_max", 1.0), (kv::f(v, "m_flEndTime_exp", 1.0) * 4.0).trunc() / 4.0],
                    range: kv::f(v, "m_flRange", 0.0),
                    range_bias: num("m_flRangeBias", 0.2, cx),
                    jump2: t * t,
                    prev_scale: kv::f(v, "m_flPrevPosScale", 1.0),
                    lock_rot: kv::b(v, "m_bLockRot", false),
                    scale: VecIn::parse_or(v, "m_vecScale", [1.0; 3], cx),
                    out: kv::field(v, "m_nFieldOutput", fl::POSITION),
                    out_prev: kv::field(v, "m_nFieldOutputPrev", fl::PREV),
                    prev_xf: None,
                }
            }
            "C_OP_SetFloat" => Op::SetFloat { field: kv::field(v, "m_nOutputField", fl::RADIUS), value: num("m_InputValue", 0.0, cx), method, lerp: num("m_Lerp", 1.0, cx) },
            "C_OP_SetVec" => Op::SetVec {
                field: kv::field(v, "m_nOutputField", fl::COLOR),
                value: VecIn::parse_or(v, "m_InputValue", [0.0; 3], cx),
                method,
                lerp: num("m_Lerp", 1.0, cx),
                normalized: kv::b(v, "m_bNormalizedOutput", false),
            },
            "C_OP_RampScalarLinearSimple" => Op::Ramp { rate: kv::f(v, "m_Rate", 0.0), start: kv::f(v, "m_flStartTime", 0.0), end: kv::f(v, "m_flEndTime", 1.0), field: kv::field(v, "m_nField", fl::RADIUS) },
            "C_OP_InterpolateRadius" => {
                let b = kv::f(v, "m_flBias", 0.5);
                Op::InterpolateRadius {
                    start: kv::f(v, "m_flStartTime", 0.0),
                    end: kv::f(v, "m_flEndTime", 1.0),
                    s0: num("m_flStartScale", 1.0, cx),
                    s1: num("m_flEndScale", 1.0, cx),
                    ease: kv::b(v, "m_bEaseInAndOut", false),
                    bias: if b == 0.0 { 0.5 } else { b },
                }
            }
            "C_OP_FadeInSimple" => Op::FadeIn { time: kv::f(v, "m_flFadeInTime", 0.25), field: kv::field(v, "m_nFieldOutput", fl::ALPHA) },
            "C_OP_FadeOutSimple" => Op::FadeOut { time: kv::f(v, "m_flFadeOutTime", 0.25), field: kv::field(v, "m_nFieldOutput", fl::ALPHA) },
            "C_OP_ColorInterpolate" => Op::ColorInterpolate {
                fade: kv::color(v, "m_ColorFade", [1.0; 3]),
                start: kv::f(v, "m_flFadeStartTime", 0.0),
                end: kv::f(v, "m_flFadeEndTime", 1.0),
                field: kv::field(v, "m_nFieldOutput", fl::COLOR),
                ease: kv::b(v, "m_bEaseInOut", true),
            },
            "C_OP_SpinUpdate" => Op::SpinUpdate,
            "C_OP_EndCapTimedDecay" => Op::EndCapTimedDecay { time: num("m_flDecayTime", 0.0, cx) },
            "C_OP_LerpEndCapScalar" => Op::LerpEndCapScalar { field: kv::field(v, "m_nFieldOutput", fl::RADIUS), output: kv::f(v, "m_flOutput", 1.0), time: kv::f(v, "m_flLerpTime", 1.0), start_age: -1.0 },
            "C_OP_FadeAndKill" => {
                let (mut in0, mut in1) = (kv::f(v, "m_flStartFadeInTime", 0.0), kv::f(v, "m_flEndFadeInTime", 0.5));
                let (mut out0, mut out1) = (kv::f(v, "m_flStartFadeOutTime", 0.5), kv::f(v, "m_flEndFadeOutTime", 1.0));
                in1 = in1.max(in0);
                out1 = out1.max(out0);
                if in0 > out0 {
                    std::mem::swap(&mut in0, &mut out0);
                }
                if in1 > out1 {
                    std::mem::swap(&mut in1, &mut out1);
                }
                Op::FadeAndKill { in0, in1, out0, out1, a0: kv::f(v, "m_flStartAlpha", 1.0), a1: kv::f(v, "m_flEndAlpha", 0.0) }
            }
            "C_OP_AlphaDecay" => Op::AlphaDecay { min: kv::f(v, "m_flMinAlpha", 0.0).max(0.0) },
            "C_OP_SetAttributeToScalarExpression" => Op::ScalarExpr {
                field: kv::field(v, "m_nOutputField", fl::RADIUS),
                a: num("m_flInput1", 0.0, cx),
                b: num("m_flInput2", 0.0, cx),
                expr: kv::enumv(v, "m_nExpression", SCALAR_EXPRESSIONS, 0),
                method,
                remap: kv::get(v, "m_flOutputRemap").filter(|x| x.is_object()).map(Mapping::parse_remap),
            },
            "C_OP_SnapshotRigidSkinToBones" => {
                cx.skinned = true;
                Op::RigidSkin(super::snapshot::RigidSkin::parse(v))
            }
            "C_OP_OscillateVectorSimple" => Op::Oscillate {
                field: kv::field(v, "m_nField", fl::POSITION),
                rate: kv::v3(v, "m_Rate", [0.0; 3]),
                freq: kv::v3(v, "m_Frequency", [1.0; 3]),
                mult: kv::f(v, "m_flOscMult", 2.0),
                add: kv::f(v, "m_flOscAdd", 0.5),
                offset: kv::b(v, "m_bOffset", false),
            },
            _ => return None,
        };
        Some(Operator { c, kind })
    }

    pub fn reset(&mut self) {
        match &mut self.kind {
            Op::PositionLock { prev_xf, .. } => *prev_xf = None,
            Op::LerpEndCapScalar { start_age, .. } => *start_age = -1.0,
            _ => {}
        }
    }

    pub fn operate(&mut self, st: &mut State, p: &mut Particles, dt: f32, strength: f32, forces: &[ForceGen], sc: &mut Scratch) {
        let n = p.len;
        match &mut self.kind {
            Op::RigidSkin(o) => o.operate(st, p),
            Op::Decay { rope } => {
                if !*rope {
                    for i in 0..n {
                        let life = p.s[fl::LIFE][i];
                        if life <= 0.0 || life <= p.age(i) {
                            p.kill(i);
                        }
                    }
                } else if n > 0 {
                    let last = n - 1;
                    for i in 0..n {
                        let dead = |k: usize| p.age(k) > p.s[fl::LIFE][k];
                        if dead(i) && dead((i + 1).min(last)) && dead((i + 2).min(last)) {
                            p.kill(i);
                        }
                    }
                }
            }
            Op::Movement { gravity, drag } => {
                let g = gravity.next(st);
                let dt2 = dt * dt;
                let dv = drag.next(st).clamp(0.0, 1.0 - EPS);
                let ratio = if st.previous_step > 0.0 { dt / st.previous_step } else { 1.0 };
                let df = ratio * (1.0 - dv).powf(dt * 30.0);
                sc.vc.clear();
                sc.vc.resize(n, [0.0; 3]);
                let mut fv = std::mem::take(&mut sc.vc);
                for fg in forces {
                    let s = fg.c.strength(st);
                    if s <= 0.0 {
                        continue;
                    }
                    fg.generate(st, p, &mut fv, s, sc);
                }
                for i in 0..n {
                    let pos = p.v[fl::POSITION][i];
                    let acc = add(g, fv[i]);
                    let step = add(scale(sub(pos, p.v[fl::PREV][i]), df), scale(acc, dt2 * p.s[fl::FORCE_SCALE][i]));
                    p.v[fl::PREV][i] = pos;
                    p.v[fl::POSITION][i] = add(pos, step);
                }
                sc.vc = fv;
            }
            Op::PositionLock { xf, start, end, range, range_bias, jump2, prev_scale, lock_rot, scale: vscale, out, out_prev, prev_xf } => {
                let cur = xf.now(st);
                let s = vscale.next(st);
                let prev = *prev_xf.get_or_insert_with(|| xf.at(st, st.step_start_age));
                let delta = sub(cur.o, prev.o);
                // The rotation from the previous frame to this one, applied about the origin
                // as VRF's `previousInverse * current` in row-vector form.
                let rot = |v: Vec3| cur.point(prev.inv_vector(sub(v, prev.o)));
                *prev_xf = Some(cur);
                let jump = *jump2 != 0.0 && dot(delta, delta) > *jump2;
                let always = start[0] >= 1.0;
                let rb = range_bias.next(st);
                if !is_vector(*out) {
                    return;
                }
                let (o, op) = (*out as usize, *out_prev);
                let moves_prev = *prev_scale != 0.0 && is_vector(op);
                for i in 0..n {
                    let mut window = 1.0;
                    if !always {
                        let sd = st.rnd.next();
                        let ed = st.rnd.next();
                        let s0 = lerp(start[0], start[1], sd.powf(start[2]));
                        let e0 = lerp(end[0], end[1], ed.powf(end[2]));
                        let lf = sat(p.age(i) / p.s[fl::LIFE][i]);
                        window = if jump { 1.0 } else { 1.0 - sat((lf - s0) / (e0 - s0)) };
                        if window <= 0.0 {
                            continue;
                        }
                    }
                    let cp = p.v[o][i];
                    let cf = p.age(i).min(dt) / dt;
                    let mut mv = scale(delta, strength * cf * window);
                    let mut rw = if always { strength } else { window };
                    if *range != 0.0 {
                        let probe = if always { mv } else { mulv(mv, s) };
                        let fd = length(sub(cur.o, add(cp, probe)));
                        let fade = bias((fd / *range).min(1.0), rb);
                        mv = scale(mv, 1.0 - fade);
                        rw = 1.0 - fade * rw;
                    }
                    if *lock_rot {
                        let w = scale(s, rw * cf);
                        p.v[o][i] = add(cp, mulv(sub(rot(cp), cp), w));
                        if moves_prev {
                            let pp = p.v[op as usize][i];
                            p.v[op as usize][i] = add(pp, scale(mulv(sub(rot(pp), pp), w), *prev_scale));
                        }
                    } else {
                        let sm = mulv(mv, s);
                        p.v[o][i] = add(cp, sm);
                        if moves_prev {
                            p.v[op as usize][i] = add(p.v[op as usize][i], scale(sm, *prev_scale));
                        }
                    }
                }
            }
            Op::SetFloat { field, value, method, lerp: lerp_in } => {
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                let (lo, hi) = float_range(*field);
                let rad = is_angle(*field) && *method != fl::SET_SCALE_INITIAL && *method != fl::SET_SCALE_CURRENT;
                lerp_in.fill(st, p, 0, n, &mut sc.a);
                value.fill(st, p, 0, n, &mut sc.b);
                for i in 0..n {
                    let v = if rad { sc.b[i].to_radians() } else { sc.b[i] };
                    let l = sat(sc.a[i]);
                    let cur = p.s[f][i];
                    if *method == fl::SET_RAMP_CURRENT {
                        p.s[f][i] = (l * (strength * dt) * v + cur).clamp(lo, hi);
                        continue;
                    }
                    let init = p.is[f][i];
                    let target = apply_set(v, init, cur, st.current_step, *method);
                    let r = lerp(cur, target, l).clamp(lo, hi);
                    p.s[f][i] = if strength >= 1.0 { r } else { lerp(if reads_initial(*method) { init } else { cur }, r, strength) };
                }
            }
            Op::SetVec { field, value, method, lerp: lerp_in, normalized } => {
                value.fill(st, p, 0, n, &mut sc.va, &mut sc.comp);
                lerp_in.fill(st, p, 0, n, &mut sc.a);
                if !is_vector(*field) {
                    return;
                }
                let f = *field as usize;
                let clamps = f == fl::COLOR || f == fl::GLOW_RGB;
                for i in 0..n {
                    let la = sat(sc.a[i] * strength);
                    let cur = p.v[f][i];
                    let mut t = apply_set_v(sc.va[i], p.iv[f][i], cur, st.current_step, *method);
                    if clamps {
                        t = [sat(t[0]), sat(t[1]), sat(t[2])];
                    }
                    let mut r = lerpv(cur, t, la);
                    if *normalized && r != [0.0; 3] {
                        r = safe_normalize(r);
                    }
                    p.v[f][i] = r;
                }
            }
            Op::Ramp { rate, start, end, field } => {
                if !is_scalar(*field) {
                    return;
                }
                let step = *rate * strength * dt;
                let (lo, hi) = float_range(*field);
                let f = *field as usize;
                for i in 0..n {
                    let life = p.s[fl::LIFE][i];
                    let na = p.age(i) / life;
                    if life > 0.0 && *start <= na && na < *end {
                        p.s[f][i] = (p.s[f][i] + step).clamp(lo, hi);
                    }
                }
            }
            Op::InterpolateRadius { start, end, s0, s1, ease, bias: b } => {
                if *end <= *start {
                    return;
                }
                s0.fill(st, p, 0, n, &mut sc.a);
                s1.fill(st, p, 0, n, &mut sc.b);
                let window = *end - *start;
                for i in 0..n {
                    let life = p.s[fl::LIFE][i];
                    let t = if life > 0.0 { p.age(i) / life } else { 0.0 };
                    let ff = dt / life;
                    let short = life * window <= dt;
                    let wend = if short { *start + ff } else { *end };
                    if !(life > 0.0 && t >= *start && t <= ff + wend) {
                        continue;
                    }
                    let mut ts = (t - *start) / window;
                    if *ease {
                        ts = ts * ts * (3.0 - 2.0 * ts);
                    } else if *b != 0.5 {
                        ts = bias(ts, *b);
                    }
                    let k = clamp_either(lerp(sc.a[i], sc.b[i], ts), sc.a[i], sc.b[i]);
                    p.s[fl::RADIUS][i] = p.is[fl::RADIUS][i] * k;
                }
            }
            Op::FadeIn { time, field } => {
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                for i in 0..n {
                    let age = p.age(i);
                    let len = *time * p.s[fl::LIFE][i];
                    if (age - dt).max(0.0) < len {
                        p.s[f][i] = smoothstep(0.0, len, age) * p.is[f][i];
                    }
                }
            }
            Op::FadeOut { time, field } => {
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                let fs = 1.0 - *time;
                for i in 0..n {
                    let t = p.age(i) / p.s[fl::LIFE][i];
                    if fs < t {
                        let faded = (1.0 - smoothstep(fs, 1.0, t)) * p.is[f][i];
                        p.s[f][i] = faded.min(p.s[f][i]);
                    }
                }
            }
            Op::ColorInterpolate { fade, start, end, field, ease } => {
                if *end == *start || !is_vector(*field) {
                    return;
                }
                let f = *field as usize;
                for i in 0..n {
                    let life = p.s[fl::LIFE][i];
                    let mut fr = sat((p.age(i) / life - *start) / (*end - *start));
                    if *ease {
                        fr = (fr * -2.0 + 3.0) * fr * fr;
                    }
                    let init = p.iv[f][i];
                    let mut c = p.v[f][i];
                    if life > 0.0 {
                        c = lerpv(init, *fade, fr);
                    }
                    p.v[f][i] = if strength >= 1.0 { c } else { lerpv(init, c, strength) };
                }
            }
            Op::SpinUpdate => {
                for i in 0..n {
                    let step = p.age(i).min(dt) * strength;
                    p.s[fl::ROLL][i] += p.s[fl::ROLL_SPEED][i] * step;
                }
            }
            Op::EndCapTimedDecay { time } => {
                if !st.in_end_cap || st.end_cap_age() <= time.next(st) {
                    return;
                }
                for i in 0..n {
                    p.kill(i);
                }
            }
            Op::LerpEndCapScalar { field, output, time, start_age } => {
                if !st.in_end_cap || !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                if *start_age < 0.0 {
                    *start_age = st.age;
                    let (cur, init) = (&p.s[f], &mut p.is[f]);
                    init.copy_from_slice(cur);
                }
                let t = sat((st.age - *start_age) / (*time + EPS));
                for i in 0..n {
                    let target = lerp(p.is[f][i], *output, t);
                    p.s[f][i] = lerp(p.s[f][i], target, strength);
                }
            }
            Op::FadeAndKill { in0, in1, out0, out1, a0, a1 } => {
                for i in 0..n {
                    let (age, life) = (p.age(i), p.s[fl::LIFE][i]);
                    if life < age {
                        p.kill(i);
                        continue;
                    }
                    let t = if life > 0.0 { age / life } else { 0.0 };
                    let init = p.is[fl::ALPHA][i];
                    let cur = p.s[fl::ALPHA][i];
                    let a = if t >= *out0 && t < *out1 {
                        init * (smoothstep(*out0, *out1, t) * (*a1 - 1.0) + 1.0)
                    } else if t >= *in0 && t < *in1 {
                        init * (smoothstep(*in0, *in1, t) * (1.0 - *a0) + *a0)
                    } else {
                        cur
                    };
                    p.s[fl::ALPHA][i] = lerp(cur, a, strength);
                }
            }
            Op::AlphaDecay { min } => {
                let th = *min + EPS;
                for i in 0..n {
                    if p.s[fl::ALPHA][i] * p.s[fl::ALPHA2][i] <= th {
                        p.kill(i);
                    }
                }
            }
            Op::ScalarExpr { field, a, b, expr, method, remap } => {
                a.fill(st, p, 0, n, &mut sc.a);
                b.fill(st, p, 0, n, &mut sc.b);
                if !is_scalar(*field) {
                    return;
                }
                let f = *field as usize;
                let (lo, hi) = float_range(*field);
                let scales = !matches!(*expr, 8..=10);
                for i in 0..n {
                    let mut e = scalar_expression(*expr, sc.a[i], sc.b[i]);
                    if scales && let Some(x) = e.as_mut() {
                        *x *= strength;
                    }
                    let mut o = e.unwrap_or(p.s[f][i]);
                    if let Some(m) = remap {
                        o = m.apply(o);
                    }
                    p.s[f][i] = apply_set(o, p.is[f][i], p.s[f][i], st.current_step, *method).clamp(lo, hi);
                }
            }
            Op::Oscillate { field, rate, freq, mult, add: oadd, offset } => {
                if !is_vector(*field) {
                    return;
                }
                let phase = *mult * st.age + *oadd;
                let step = strength * dt;
                let sinpi = |x: f32| (x * std::f32::consts::PI).sin();
                let delta = [sinpi(freq[0] * phase), sinpi(freq[1] * phase), sinpi(freq[2] * phase)];
                let value = mulv(scale(*rate, step), delta);
                let f = *field as usize;
                let normalized = matches!(f, fl::COLOR | fl::GLOW_RGB);
                for i in 0..n {
                    let mut o = add(p.v[f][i], value);
                    if normalized {
                        o = [sat(o[0]), sat(o[1]), sat(o[2])];
                    }
                    p.v[f][i] = o;
                    if *offset && f == fl::POSITION {
                        p.v[fl::PREV][i] = add(p.v[fl::PREV][i], value);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Pre {
    SetPositions { points: [(i32, Vec3); 4], once: bool, world: bool, head: i32, orient: bool, ran: bool },
}

#[derive(Clone, Debug)]
pub struct PreEmission {
    pub c: Common,
    pub kind: Pre,
}

impl PreEmission {
    pub fn parse(class: &str, v: &Value, cx: &mut ParseCx) -> Option<PreEmission> {
        let c = Common::parse(v, cx);
        let kind = match class {
            "C_OP_SetControlPointPositions" => Pre::SetPositions {
                points: [
                    (kv::i(v, "m_nCP1", 1), kv::v3(v, "m_vecCP1Pos", [128.0, 0.0, 0.0])),
                    (kv::i(v, "m_nCP2", 2), kv::v3(v, "m_vecCP2Pos", [0.0, 128.0, 0.0])),
                    (kv::i(v, "m_nCP3", 3), kv::v3(v, "m_vecCP3Pos", [-128.0, 0.0, 0.0])),
                    (kv::i(v, "m_nCP4", 4), kv::v3(v, "m_vecCP4Pos", [0.0, -128.0, 0.0])),
                ],
                once: kv::b(v, "m_bSetOnce", false),
                world: kv::b(v, "m_bUseWorldLocation", false),
                head: kv::i(v, "m_nHeadLocation", 0),
                orient: kv::b(v, "m_bOrient", false),
                ran: false,
            },
            _ => return None,
        };
        Some(PreEmission { c, kind })
    }

    pub fn reset(&mut self) {
        let Pre::SetPositions { ran, .. } = &mut self.kind;
        *ran = false;
    }

    /// The control point writes this step, as (index, position, axes); the system applies
    /// them to itself and its children.
    pub fn operate(&mut self, st: &State, out: &mut Vec<(usize, Vec3, Option<[Vec3; 3]>)>) {
        out.clear();
        let Pre::SetPositions { points, once, world, head, orient, ran } = &mut self.kind;
        if *once {
            if *ran {
                return;
            }
            *ran = true;
        }
        let h = st.cp(*head);
        for &(cp, pos) in points.iter() {
            if !(0..64).contains(&cp) {
                continue;
            }
            if *world {
                out.push((cp as usize, pos, None));
            } else {
                out.push((cp as usize, h.xf().point(pos), if *orient { Some(h.axes) } else { None }));
            }
        }
    }
}
