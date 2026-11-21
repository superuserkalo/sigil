//! A system's running state (VRF `ParticleSystemRenderState`): its clock, endcap phase,
//! random draws and 64 control points.

use super::smath::{Rand, Xf, lerpv, sat, slerp_axes, vector_vectors};
use crate::math::{Vec3, length, scale};

pub const CONTROL_POINTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlPoint {
    pub pos: Vec3,
    pub axes: [Vec3; 3],
    pub prev_pos: Vec3,
    pub prev_axes: [Vec3; 3],
    pub change_time: f32,
    pub written: bool,
}

const AXES: [Vec3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

impl Default for ControlPoint {
    fn default() -> Self {
        ControlPoint { pos: [0.0; 3], axes: AXES, prev_pos: [0.0; 3], prev_axes: AXES, change_time: -1.0, written: false }
    }
}

impl ControlPoint {
    pub fn xf(&self) -> Xf {
        Xf { o: self.pos, x: self.axes[0], y: self.axes[1], z: self.axes[2] }
    }

    pub fn rotation(&self) -> Xf {
        Xf { o: [0.0; 3], ..self.xf() }
    }

    pub fn record_step(&mut self) {
        self.prev_pos = self.pos;
        self.prev_axes = self.axes;
    }
}

/// How far into the current step `time` falls, for interpolating a control point between
/// its previous and current state (VRF `ControlPoint.StepFraction`).
pub fn step_fraction(st: &State, time: f32) -> f32 {
    let step = st.current_step;
    if step == 0.0 || st.age == time {
        return 1.0;
    }
    sat((step - (st.age - time)) / step)
}

#[derive(Clone, Debug)]
pub struct State {
    pub age: f32,
    pub step_start_age: f32,
    pub current_step: f32,
    pub previous_step: f32,
    pub in_end_cap: bool,
    pub end_cap_start_age: f32,
    pub operator_time_base: f32,
    pub cps: Vec<ControlPoint>,
    pub rnd: Rand,
    /// The fields the initializers before the running one wrote (VRF `InitializedFields`).
    pub written: u64,
    /// Parent particles that raised the event an event emitter is emitting for.
    pub event_count: usize,
    pub max_particles: usize,
    pub count: usize,
    pub emission_stopped: bool,
    pub camera: Vec3,
    /// The snapshot bound to each control point (a system's `m_hSnapshot`, shared down).
    pub snapshots: Vec<Option<std::sync::Arc<super::snapshot::Snapshot>>>,
    /// Control point 0's model bones as `posed * inverse bind` (Source model space), with the
    /// system age they were set at; and the palette before it, for bone velocity.
    pub skin: Option<(std::sync::Arc<Vec<crate::math::Mat4>>, f32)>,
    pub skin_prev: Option<(std::sync::Arc<Vec<crate::math::Mat4>>, f32)>,
}

impl State {
    pub fn new(max_particles: usize) -> State {
        State {
            age: 0.0,
            step_start_age: 1e23,
            current_step: 0.0,
            previous_step: 0.05,
            in_end_cap: false,
            end_cap_start_age: 0.0,
            operator_time_base: 0.0,
            cps: vec![ControlPoint::default(); CONTROL_POINTS],
            rnd: Rand::default(),
            written: 0,
            event_count: 0,
            max_particles,
            count: 0,
            emission_stopped: false,
            camera: [0.0; 3],
            snapshots: vec![None; CONTROL_POINTS],
            skin: None,
            skin_prev: None,
        }
    }

    pub fn cp(&self, i: i32) -> &ControlPoint {
        &self.cps[i.clamp(0, CONTROL_POINTS as i32 - 1) as usize]
    }

    pub fn end_cap_age(&self) -> f32 {
        self.age - self.end_cap_start_age
    }

    pub fn cp_position_at(&self, i: i32, time: f32) -> Vec3 {
        let cp = self.cp(i);
        lerpv(cp.prev_pos, cp.pos, step_fraction(self, time))
    }

    /// The control point's frame at `time` within the step, rotation slerped.
    pub fn cp_xf_at(&self, i: i32, time: f32, orient: bool) -> Xf {
        let cp = self.cp(i);
        let t = step_fraction(self, time);
        let o = lerpv(cp.prev_pos, cp.pos, t);
        if !orient {
            return Xf { o, ..Xf::IDENTITY };
        }
        let [x, y, z] = slerp_axes(cp.prev_axes, cp.axes, t);
        Xf { o, x, y, z }
    }

    /// Writes a control point (position, and axes when given), stamping its change time.
    pub fn set_cp(&mut self, i: usize, pos: Vec3, axes: Option<[Vec3; 3]>) {
        if i >= CONTROL_POINTS {
            return;
        }
        let age = self.age;
        let cp = &mut self.cps[i];
        cp.written = true;
        let mut changed = cp.pos != pos;
        cp.pos = pos;
        if let Some(a) = axes
            && a != cp.axes
        {
            cp.axes = a;
            changed = true;
        }
        if changed {
            cp.change_time = age;
        }
    }

    /// Points a control point along `forward` (VRF `orientation` setter).
    pub fn orient_cp(&mut self, i: usize, forward: Vec3) {
        if i >= CONTROL_POINTS {
            return;
        }
        let pos = self.cps[i].pos;
        let l = length(forward);
        let axes = if l == 0.0 {
            AXES
        } else {
            let f = scale(forward, 1.0 / l);
            let (left, up) = vector_vectors(f);
            [f, left, up]
        };
        self.set_cp(i, pos, Some(axes));
    }

    pub fn record_control_points(&mut self, elapsed: f32) {
        self.previous_step = elapsed;
        for cp in &mut self.cps {
            cp.record_step();
        }
    }
}
