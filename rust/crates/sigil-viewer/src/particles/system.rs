//! One particle system and its children (VRF `ParticleSystemSimulation`, through Grimoire's
//! `simulation.ts`, MIT): loading a definition, the frame and substep loop, emission and
//! initialization, operators with kills, parent events, control points shared downward,
//! seeding, endcap and replay.

use serde_json::Value;

use super::fields::{self as fl, Particles};
use super::functions::{EVENT_KILLED, Emitter, ForceGen, Initializer, Operator, PreEmission, Scratch};
use super::kv;
use super::providers::ParseCx;
use super::render::Renderer;
use super::smath::OPERATOR_STRIDE;
use super::state::State;
use crate::math::Vec3;

const FIRST_FRAME_STEP: f32 = 0.05;
const MINIMUM_FRAME_TIME: f32 = 1e-22;
pub const MAX_DEPTH: usize = 12;
/// A wide tree loads exponentially many systems even within `MAX_DEPTH`: an effect loads at
/// most this many, a bundle at most `MAX_BUNDLE_SYSTEMS`, and a system at most
/// `MAX_CHILDREN` children. Past them the rest are diagnostics, not loads.
pub const MAX_EFFECT_SYSTEMS: usize = 256;
pub const MAX_BUNDLE_SYSTEMS: usize = 512;
pub const MAX_CHILDREN: usize = 32;

/// Where definitions come from: the bundle's systems by lowercase name, with their stored
/// KV3 format.
pub trait Source {
    fn definition(&self, name: &str) -> Option<(&Value, Option<&str>)>;

    /// A decoded snapshot by path (`particles/x/y.vsnap`, lowercase), when the source has it.
    fn snapshot(&self, _path: &str) -> Option<std::sync::Arc<super::snapshot::Snapshot>> {
        None
    }
}

pub struct System {
    pub name: String,
    pub behavior: i32,
    initial_particles: usize,
    pub max_particles: usize,
    max_step: f32,
    min_step: f32,
    pre_sim: f32,
    stop_after: f32,
    pub st: State,
    pub p: Particles,
    pre: Vec<PreEmission>,
    emitters: Vec<Emitter>,
    inits: Vec<Initializer>,
    ops: Vec<Operator>,
    forces: Vec<ForceGen>,
    pub renderers: Vec<Renderer>,
    pub children: Vec<System>,
    child_slot_count: usize,
    pub start_delay: f32,
    pub end_cap_child: bool,
    slot: usize,
    first_frame_pending: bool,
    particles_created: usize,
    id_counter: i32,
    listen_mask: i32,
    frozen: bool,
    sc: Scratch,
    cp_writes: Vec<(usize, Vec3, Option<[Vec3; 3]>)>,
    kill_idx: Vec<usize>,
}

/// Does the stored format predate vpcf62 (or is it unknown, so every upgrade step runs)?
fn needs_vpcf62_upgrade(format: Option<&str>) -> bool {
    match format.and_then(|f| f.strip_prefix("vpcf")).and_then(|n| n.parse::<i32>().ok()) {
        Some(n) => n <= 61,
        None => true,
    }
}

/// The vpcf61 to vpcf62 step: `C_INIT_CreateWithinSphereTransform` negates the Y of its
/// local coordinate system speed inputs.
fn upgrade_vpcf62(def: &mut Value) {
    fn neg_y(v: &mut Value, key: &str) {
        if let Some(a) = v.get_mut(key).and_then(Value::as_array_mut)
            && let Some(y) = a.get_mut(1)
            && let Some(n) = y.as_f64()
        {
            *y = serde_json::json!(-n);
        }
    }
    fn neg_float(v: &mut Value) {
        let Some(o) = v.as_object_mut() else { return };
        let ty = o.get("m_nType").and_then(Value::as_str).unwrap_or("PF_TYPE_LITERAL").to_string();
        let keys: &[&str] = match ty.as_str() {
            "PF_TYPE_LITERAL" => &["m_flLiteralValue"],
            "PF_TYPE_RANDOM_UNIFORM" | "PF_TYPE_RANDOM_BIASED" => &["m_flRandomMin", "m_flRandomMax"],
            _ => &[],
        };
        for k in keys {
            if let Some(n) = o.get(*k).and_then(Value::as_f64) {
                o.insert((*k).to_string(), serde_json::json!(-n));
            }
        }
    }
    let Some(inits) = def.get_mut("m_Initializers").and_then(Value::as_array_mut) else { return };
    for init in inits {
        if init.get("_class").and_then(Value::as_str) != Some("C_INIT_CreateWithinSphereTransform") {
            continue;
        }
        for key in ["m_LocalCoordinateSystemSpeedMin", "m_LocalCoordinateSystemSpeedMax"] {
            let Some(input) = init.get_mut(key) else { continue };
            if !input.is_object() {
                continue;
            }
            match input.get("m_nType").and_then(Value::as_str).unwrap_or("PVEC_TYPE_LITERAL") {
                "PVEC_TYPE_LITERAL" => neg_y(input, "m_vLiteralValue"),
                "PVEC_TYPE_RANDOM_UNIFORM" => {
                    neg_y(input, "m_vRandomMin");
                    neg_y(input, "m_vRandomMax");
                }
                "PVEC_TYPE_FLOAT_COMPONENTS" => {
                    if let Some(y) = input.get_mut("m_FloatComponentY") {
                        neg_float(y);
                    }
                }
                _ => {}
            }
        }
    }
}

impl System {
    /// Loads `name` and its children. `cap` bounds each system's particle pool; diagnostics
    /// collect in `cx`. `None` when the definition is missing or not a particle system.
    pub fn load(name: &str, src: &dyn Source, cap: usize, cx: &mut ParseCx, ancestors: &mut Vec<String>) -> Option<System> {
        let key = name.trim_end_matches("_c").to_ascii_lowercase();
        if cx.effect_loaded >= MAX_EFFECT_SYSTEMS || cx.loaded >= MAX_BUNDLE_SYSTEMS {
            cx.note(format!("skipped {key}: past the budget of {MAX_EFFECT_SYSTEMS} systems an effect, {MAX_BUNDLE_SYSTEMS} a bundle"));
            return None;
        }
        cx.effect_loaded += 1;
        cx.loaded += 1;
        let Some((raw, format)) = src.definition(&key) else {
            cx.diagnostics.push(format!("missing system {key}"));
            return None;
        };
        if kv::s(raw, "_class").is_some_and(|c| c != "CParticleSystemDefinition") {
            cx.diagnostics.push(format!("{key} is not a CParticleSystemDefinition"));
            return None;
        }
        let mut def = raw.clone();
        if needs_vpcf62_upgrade(format) {
            upgrade_vpcf62(&mut def);
        }
        let prev_system = std::mem::replace(&mut cx.system, key.clone());
        let behavior = kv::i(&def, "m_nBehaviorVersion", 0);
        let max_particles = (kv::i(&def, "m_nMaxParticles", 1000).clamp(1, 20000) as usize).min(cap.max(1));
        let max_step = kv::f(&def, "m_flMaximumTimeStep", 0.1);
        let mut consts_s = [0.0f32; fl::COUNT];
        let mut consts_v = [[0.0f32; 3]; fl::COUNT];
        consts_s[fl::LIFE] = 1.0;
        consts_s[fl::ALPHA] = 1.0;
        consts_s[fl::ALPHA2] = 1.0;
        consts_v[fl::COLOR] = [1.0; 3];
        consts_s[fl::RADIUS] = 5.0;
        consts_s[fl::TRAIL_LENGTH] = 0.1;
        consts_s[fl::FORCE_SCALE] = 1.0;
        consts_v[fl::NORMAL] = [0.0, 0.0, 1.0];
        consts_v[fl::GLOW_RGB] = [1.0; 3];
        consts_s[fl::GLOW_ALPHA] = 1.0;
        consts_s[fl::EXTRA1] = 1.0;
        consts_s[fl::EXTRA2] = 1.0;
        consts_s[33] = -1.0;
        consts_v[fl::BONE_INDICES] = [fl::BONE_NONE; 3];
        consts_s[fl::BONE_INDEX3] = fl::BONE_NONE;
        if let Some(c) = kv::get(&def, "m_ConstantColor").and_then(Value::as_array) {
            consts_v[fl::COLOR] = kv::color(&def, "m_ConstantColor", [1.0; 3]);
            if let Some(a) = c.get(3) {
                consts_s[fl::ALPHA] = kv::number(a).map_or(0.0, |x| x.trunc() as f32 / 255.0);
            }
        }
        consts_s[fl::RADIUS] = kv::f(&def, "m_flConstantRadius", consts_s[fl::RADIUS]);
        consts_s[fl::LIFE] = kv::f(&def, "m_flConstantLifespan", consts_s[fl::LIFE]);
        consts_s[fl::ROLL] = kv::f(&def, "m_flConstantRotation", 0.0);
        consts_s[fl::ROLL_SPEED] = kv::f(&def, "m_flConstantRotationSpeed", 0.0);
        consts_v[fl::NORMAL] = kv::v3(&def, "m_ConstantNormal", consts_v[fl::NORMAL]);
        consts_s[fl::SEQUENCE] = kv::f(&def, "m_nConstantSequenceNumber", 0.0);
        consts_s[fl::SEQUENCE2] = kv::f(&def, "m_nConstantSequenceNumber1", 0.0);

        let mut sys = System {
            name: key.clone(),
            behavior,
            initial_particles: kv::i(&def, "m_nInitialParticles", 0).max(0) as usize,
            max_particles,
            max_step: if max_step > 0.0 { max_step } else { 0.1 },
            min_step: kv::f(&def, "m_flMinimumTimeStep", 0.0),
            pre_sim: kv::f(&def, "m_flPreSimulationTime", 0.0),
            stop_after: kv::f(&def, "m_flStopSimulationAfterTime", 1e9),
            st: State::new(max_particles),
            p: Particles::new(consts_s, consts_v, max_particles),
            pre: Vec::new(),
            emitters: Vec::new(),
            inits: Vec::new(),
            ops: Vec::new(),
            forces: Vec::new(),
            renderers: Vec::new(),
            children: Vec::new(),
            child_slot_count: 0,
            start_delay: 0.0,
            end_cap_child: false,
            slot: 0,
            first_frame_pending: true,
            particles_created: 0,
            id_counter: 0,
            listen_mask: 0,
            frozen: false,
            sc: Scratch::default(),
            cp_writes: Vec::new(),
            kill_idx: Vec::new(),
        };
        let disabled = |v: &Value| kv::b(v, "m_bDisableOperator", false);
        let class = |v: &Value| kv::s(v, "_class").unwrap_or("").to_string();
        for v in kv::array(&def, "m_PreEmissionOperators").iter().filter(|v| !disabled(v)) {
            match PreEmission::parse(&class(v), v, cx) {
                Some(f) => sys.pre.push(f),
                None => cx.note(format!("unsupported pre-emission operator {}", class(v))),
            }
        }
        for (k, v) in kv::array(&def, "m_Emitters").iter().filter(|v| !disabled(v)).enumerate() {
            match Emitter::parse(&class(v), v, k as i32, cx) {
                Some(f) => sys.emitters.push(f),
                None => cx.note(format!("unsupported emitter {}", class(v))),
            }
        }
        for v in kv::array(&def, "m_Initializers").iter().filter(|v| !disabled(v)) {
            match Initializer::parse(&class(v), v, cx) {
                Some(f) => sys.inits.push(f),
                None => cx.note(format!("unsupported initializer {}", class(v))),
            }
        }
        for v in kv::array(&def, "m_Operators").iter().filter(|v| !disabled(v)) {
            match Operator::parse(&class(v), v, cx) {
                Some(f) => sys.ops.push(f),
                None => cx.note(format!("unsupported operator {}", class(v))),
            }
        }
        for v in kv::array(&def, "m_ForceGenerators").iter().filter(|v| !disabled(v)) {
            match ForceGen::parse(&class(v), v, behavior, cx) {
                Some(f) => sys.forces.push(f),
                None => cx.note(format!("unsupported force generator {}", class(v))),
            }
        }
        for v in kv::array(&def, "m_Constraints").iter().filter(|v| !disabled(v)) {
            cx.note(format!("unsupported constraint {}", class(v)));
        }
        for v in kv::array(&def, "m_Renderers").iter().filter(|v| !disabled(v)) {
            match Renderer::parse(&class(v), v, behavior, cx) {
                Some(r) => sys.renderers.push(r),
                None => cx.note(format!("unsupported renderer {}", class(v))),
            }
        }
        if sys.pre_sim > 0.0 {
            cx.note(format!("pre-simulation of {}s runs as ordinary steps", sys.pre_sim));
        }
        // Children: breadth of this definition's m_Children, depth bounded.
        let children = kv::array(&def, "m_Children");
        sys.child_slot_count = children.len();
        ancestors.push(key.clone());
        for (slot, c) in children.iter().enumerate() {
            if slot >= MAX_CHILDREN {
                cx.note(format!("skipped {} children past the first {MAX_CHILDREN}", children.len() - MAX_CHILDREN));
                break;
            }
            if kv::b(c, "m_bDisableChild", false) {
                continue;
            }
            let Some(child) = kv::s(c, "m_ChildRef") else { continue };
            let child_key = child.trim_end_matches("_c").to_ascii_lowercase();
            if ancestors.contains(&child_key) {
                cx.note(format!("skipped child {child_key}: it is this system or a parent"));
                continue;
            }
            if ancestors.len() >= MAX_DEPTH {
                cx.note(format!("skipped child {child_key}: deeper than {MAX_DEPTH} levels"));
                continue;
            }
            let Some(mut ch) = System::load(&child_key, src, cap, cx, ancestors) else { continue };
            ch.start_delay = kv::f(c, "m_flDelay", 0.0);
            ch.end_cap_child = kv::b(c, "m_bEndCap", false);
            ch.slot = slot;
            ch.st.age = if ch.end_cap_child { f32::MIN } else { -ch.start_delay };
            sys.children.push(ch);
        }
        ancestors.pop();
        for ch in &sys.children {
            for e in &ch.emitters {
                sys.listen_mask |= e.event_mask();
            }
        }
        // The authored snapshot, bound to its control point here and below (Grimoire's
        // `publishSnapshot`).
        if let Some(path) = kv::s(&def, "m_hSnapshot").filter(|p| !p.is_empty()) {
            let key = path.replace('\\', "/").trim_end_matches("_c").to_ascii_lowercase();
            match src.snapshot(&key) {
                Some(snap) => sys.set_snapshot(kv::i(&def, "m_nSnapshotControlPoint", 0), &snap),
                None => cx.note(format!("snapshot {key} is not in the bundle")),
            }
        }
        cx.system = prev_system;
        sys.seed(super::smath::string_token(&key) as i32);
        Some(sys)
    }

    /// Binds `snapshot` to control point `cp` here and in every descendant.
    pub fn set_snapshot(&mut self, cp: i32, snapshot: &std::sync::Arc<super::snapshot::Snapshot>) {
        if !(0..super::state::CONTROL_POINTS as i32).contains(&cp) {
            return;
        }
        self.st.snapshots[cp as usize] = Some(snapshot.clone());
        for c in &mut self.children {
            c.set_snapshot(cp, snapshot);
        }
    }

    /// Hands the tree control point 0's bone palette for this frame (the last one becomes
    /// the previous, for bone velocity).
    pub fn set_skin(&mut self, palette: &std::sync::Arc<Vec<crate::math::Mat4>>) {
        self.st.skin_prev = self.st.skin.take();
        self.st.skin = Some((palette.clone(), self.st.age));
        for c in &mut self.children {
            c.set_skin(palette);
        }
    }

    /// Seeds the tree (VRF `SeedRandom`): children take `seed + 129 * (slot + 1)`, this system
    /// `seed + 129 * child count`, and every function starts over.
    pub fn seed(&mut self, seed: i32) {
        self.apply_seed(seed, 0);
        self.reset_functions_tree();
    }

    fn apply_seed(&mut self, seed: i32, counter: i32) {
        let child_seed = |slot: usize| seed.wrapping_add(129i32.wrapping_mul(slot as i32 + 1));
        for k in 0..self.children.len() {
            let s = child_seed(self.children[k].slot);
            let c = (k as i32 + 1) * self.children[k].max_particles as i32;
            self.children[k].apply_seed(s, c);
        }
        self.st.rnd.start(seed.wrapping_add(129i32.wrapping_mul(self.child_slot_count as i32)), counter);
        self.id_counter = 0;
    }

    fn reset_functions_tree(&mut self) {
        for e in &mut self.emitters {
            e.reset(&mut self.st);
        }
        for i in &mut self.inits {
            i.reset(&mut self.st);
        }
        for o in &mut self.ops {
            o.reset();
        }
        for p in &mut self.pre {
            p.reset();
        }
        for c in &mut self.children {
            c.reset_functions_tree();
        }
    }

    pub fn get_children(&self) -> &[System] {
        &self.children
    }

    /// Writes control point `i` here and in every descendant.
    pub fn set_cp(&mut self, i: usize, pos: Vec3, axes: Option<[Vec3; 3]>) {
        self.st.set_cp(i, pos, axes);
        for c in &mut self.children {
            c.set_cp(i, pos, axes);
        }
    }

    pub fn orient_cp(&mut self, i: usize, forward: Vec3) {
        self.st.orient_cp(i, forward);
        for c in &mut self.children {
            c.orient_cp(i, forward);
        }
    }

    pub fn start(&mut self) {
        self.st.emission_stopped = false;
        for e in &mut self.emitters {
            e.start_emission();
        }
        for c in &mut self.children {
            if !c.end_cap_child {
                c.start();
            }
        }
    }

    pub fn stop_emission(&mut self, remove_all: bool, play_end_cap: bool) {
        let (mut remove_all, mut play_end_cap) = (remove_all, play_end_cap);
        if self.frozen {
            self.frozen = false;
            remove_all = true;
            play_end_cap = false;
        }
        self.st.emission_stopped = true;
        for e in &mut self.emitters {
            e.stop_emission();
        }
        if remove_all {
            self.p.clear();
            self.st.count = 0;
        }
        if !self.end_cap_child || play_end_cap {
            self.st.in_end_cap = true;
        }
        if play_end_cap {
            self.st.end_cap_start_age = self.st.age;
        }
        for c in &mut self.children {
            if !c.end_cap_child || !play_end_cap {
                c.stop_emission(remove_all, play_end_cap);
                continue;
            }
            c.st.age = 0.0;
            c.st.end_cap_start_age = 0.0;
            c.enter_end_cap();
            c.start();
        }
    }

    fn enter_end_cap(&mut self) {
        self.st.in_end_cap = true;
        for c in &mut self.children {
            c.enter_end_cap();
        }
    }

    /// Kills everything, rewinds the clock and starts again with the same seed.
    pub fn replay(&mut self) {
        self.stop_emission(true, false);
        self.rewind(true);
        self.start();
    }

    fn rewind(&mut self, root: bool) {
        for c in &mut self.children {
            c.rewind(false);
        }
        self.particles_created = 0;
        self.first_frame_pending = true;
        self.frozen = false;
        let st = &mut self.st;
        st.age = if root {
            0.0
        } else if self.end_cap_child {
            f32::MIN
        } else {
            -self.start_delay
        };
        st.step_start_age = 1e23;
        st.current_step = 0.0;
        st.previous_step = FIRST_FRAME_STEP;
        st.operator_time_base = 0.0;
        st.end_cap_start_age = f32::MIN;
        st.in_end_cap = false;
        st.rnd.generation = st.rnd.generation.wrapping_add(1);
        for e in &mut self.emitters {
            e.reset(st);
        }
        for i in &mut self.inits {
            i.reset(st);
        }
        for o in &mut self.ops {
            o.reset();
        }
        for p in &mut self.pre {
            p.reset();
        }
    }

    /// Whether the tree has run, holds no particles and cannot make more.
    pub fn is_finished(&self) -> bool {
        if self.first_frame_pending || self.st.age <= 0.0 {
            return false;
        }
        if self.particle_count() > 0 {
            return false;
        }
        if self.emitters.iter().any(|e| e.may_create_more(&self.st)) {
            return false;
        }
        self.children.iter().all(|c| (c.end_cap_child && !c.st.in_end_cap) || c.is_finished())
    }

    pub fn particle_count(&self) -> usize {
        self.p.len + self.children.iter().map(System::particle_count).sum::<usize>()
    }

    /// Advances the tree by one frame.
    pub fn update(&mut self, frame_time: f32, camera: Vec3) {
        self.simulate(frame_time, None, camera);
    }

    fn simulate(&mut self, frame_time: f32, parent_len: Option<usize>, camera: Vec3) {
        if self.frozen {
            return;
        }
        self.st.camera = camera;
        if self.st.age < 0.0 {
            if frame_time >= MINIMUM_FRAME_TIME {
                self.st.age += frame_time;
                self.st.record_control_points(frame_time);
            }
            return;
        }
        if self.first_frame_pending {
            self.first_frame(parent_len);
            if self.pre_sim > 0.0 {
                let mut steps = 0;
                while self.st.age < self.pre_sim && steps < 200 {
                    let s = self.max_step.min(self.pre_sim - self.st.age).max(1e-4);
                    self.step(s, parent_len);
                    steps += 1;
                }
            }
        }
        if frame_time < MINIMUM_FRAME_TIME {
            return;
        }
        let mut remaining = frame_time.min(self.max_step * 10.0);
        while remaining > 0.0 {
            let step = if remaining > self.max_step {
                self.max_step
            } else if self.min_step > remaining {
                self.min_step
            } else {
                remaining
            };
            remaining -= step;
            self.step(step, parent_len);
        }
        let len = self.p.len;
        for c in &mut self.children {
            c.simulate(frame_time, Some(len), camera);
        }
        self.st.record_control_points(frame_time);
        if self.st.age > self.stop_after {
            self.frozen = true;
        }
    }

    fn first_frame(&mut self, parent_len: Option<usize>) {
        self.first_frame_pending = false;
        self.st.current_step = 0.0;
        self.st.step_start_age = 1e23;
        self.st.record_control_points(FIRST_FRAME_STEP);
        self.st.rnd.operator_offset = 0;
        for k in 0..self.pre.len() {
            let s = self.pre[k].c.strength(&mut self.st);
            if s > 0.0 {
                self.run_pre(k);
                self.st.record_control_points(FIRST_FRAME_STEP);
            }
            self.st.rnd.operator_offset += OPERATOR_STRIDE;
        }
        let first = self.p.len;
        let count = self.initial_particles.min(self.max_particles.saturating_sub(first));
        for _ in 0..count {
            self.p.push(self.st.age);
        }
        self.initialize(first, count, 0, -1, None);
        let _ = parent_len;
    }

    fn run_pre(&mut self, k: usize) {
        let mut writes = std::mem::take(&mut self.cp_writes);
        self.pre[k].operate(&self.st, &mut writes);
        for &(i, pos, axes) in &writes {
            self.set_cp(i, pos, axes);
        }
        self.cp_writes = writes;
    }

    fn step(&mut self, step: f32, parent_len: Option<usize>) {
        if self.st.current_step != 0.0 {
            self.st.previous_step = self.st.current_step;
        }
        self.st.step_start_age = self.st.age;
        self.st.age += step;
        self.st.current_step = step;
        self.p.time = self.st.age;
        self.st.rnd.operator_offset = 0;
        for k in 0..self.pre.len() {
            let s = self.pre[k].c.strength(&mut self.st);
            if s <= 0.0 {
                continue;
            }
            self.run_pre(k);
            self.st.rnd.operator_offset += OPERATOR_STRIDE;
        }
        self.emit(0, None, parent_len);
        self.st.rnd.operator_offset = 0;
        if self.p.len > 0 {
            for k in 0..self.ops.len() {
                let s = self.ops[k].c.strength(&mut self.st);
                if s <= 0.0 {
                    continue;
                }
                self.ops[k].operate(&mut self.st, &mut self.p, step, s, &self.forces, &mut self.sc);
                self.apply_kills();
                if self.p.len == 0 {
                    break;
                }
                self.st.rnd.operator_offset += OPERATOR_STRIDE;
            }
        }
    }

    /// Runs the emitters listening for `event` (0: the ones that run every step) and
    /// initializes what they create.
    fn emit(&mut self, event: i32, parent: Option<(&Particles, &[usize])>, parent_len: Option<usize>) {
        let mut times = std::mem::take(&mut self.sc.times);
        for k in 0..self.emitters.len() {
            let mask = self.emitters[k].event_mask();
            let listens = if event != 0 { mask & event != 0 } else { mask == 0 };
            if !listens {
                continue;
            }
            let s = self.emitters[k].c.strength(&mut self.st);
            if s <= 0.0 {
                continue;
            }
            let free = self.max_particles.saturating_sub(self.p.len);
            self.emitters[k].emit(&mut self.st, s, parent_len, free, &mut times);
            let first = self.p.len;
            for &t in &times {
                self.p.push(t);
            }
            let index = self.emitters[k].index;
            self.initialize(first, times.len(), fl::mask(fl::CREATION as i32), index, parent);
        }
        self.sc.times = times;
    }

    fn initialize(&mut self, first: usize, count: usize, written: u64, emitter: i32, parent: Option<(&Particles, &[usize])>) {
        if count == 0 {
            return;
        }
        self.st.count = self.p.len;
        self.p.time = self.st.age;
        let mut written = written;
        for k in 0..self.inits.len() {
            let init = &mut self.inits[k];
            if init.emitter != -1 && init.emitter != emitter {
                continue;
            }
            if matches!(init.kind, super::functions::Init::FromParentKilled { .. }) && parent.is_none() {
                continue;
            }
            if init.c.strength(&mut self.st) <= 0.0 {
                continue;
            }
            if self.behavior < 6 && init.written & !written == 0 {
                continue;
            }
            self.st.written = written;
            fill_ids(&mut self.p, first, count, self.id_counter, self.st.rnd.seed);
            init.run(&mut self.st, &mut self.p, first, count, &mut self.sc, parent);
            if init.random {
                self.id_counter += align4(first + count) as i32 - first as i32;
            }
            written |= init.written;
        }
        fill_ids(&mut self.p, first, count, self.id_counter, self.st.rnd.seed);
        self.id_counter += align4(first + count) as i32 - first as i32;
        self.p.save_initial(first, count);
        self.particles_created += count;
    }

    /// Hands the killed particles to the children listening for kills, then removes them.
    fn apply_kills(&mut self) {
        if self.p.kills == 0 {
            return;
        }
        if self.listen_mask & EVENT_KILLED != 0 {
            self.kill_idx.clear();
            self.kill_idx.extend((0..self.p.len).filter(|&i| self.p.killed[i]));
            let len = self.p.len;
            for c in &mut self.children {
                if c.emitters.iter().any(|e| e.event_mask() & EVENT_KILLED != 0) && c.st.age >= 0.0 {
                    c.st.event_count = self.kill_idx.len();
                    c.emit(EVENT_KILLED, Some((&self.p, &self.kill_idx)), Some(len));
                    c.st.event_count = 0;
                }
            }
        }
        self.p.compact();
        self.st.count = self.p.len;
    }
}

fn align4(x: usize) -> usize {
    (x + 3) & !3
}

fn fill_ids(p: &mut Particles, first: usize, count: usize, counter: i32, seed: i32) {
    let ids = &mut p.s[fl::PARTICLE_ID];
    for i in 0..count {
        ids[first + i] = ((counter + i as i32 + seed) & 0xfff) as f32;
    }
}
