//! Playing a bundle's effects (Grimoire's `host/effectHost.ts` and `mapParticleHost.ts`, MIT):
//! binding control point 0 (and 1) to the model's attachments or a map entity's matrix,
//! control point configurations, seeding from the placement, stepping with `min(dt, 0.1)` and
//! replaying finished effects.

use serde_json::Value;

use super::bundle::{Bundle, Effect, EffectAttach};
use super::kv;
use super::providers::ParseCx;
use super::render::{Instance, View};
use super::smath::{Xf, euler_axes_deg, string_token};
use super::system::{Source, System};
use crate::math::{Mat4, Vec3};

/// The most particles one system holds, whatever its definition asks.
pub const SYSTEM_CAP: usize = 2000;
/// The most cards drawn per frame over every effect.
pub const TOTAL_CAP: usize = 20_000;

impl Source for Bundle {
    fn definition(&self, name: &str) -> Option<(&Value, Option<&str>)> {
        self.systems.get(name).map(|s| (&s.data, s.format.as_deref()))
    }

    fn snapshot(&self, path: &str) -> Option<std::sync::Arc<super::snapshot::Snapshot>> {
        self.snapshots.get(path).cloned()
    }
}

#[derive(Clone, Debug)]
struct Driver {
    cp: usize,
    kind: String,
    name: String,
    offset: Vec3,
    angles: Vec3,
}

fn on_point(kind: &str) -> bool {
    kind == "point" || kind == "point_follow"
}

fn follows(kind: &str) -> bool {
    kind.ends_with("_follow")
}

pub struct EffectHost {
    pub effect: Effect,
    pub sys: System,
    drivers: Vec<Driver>,
    preview_point: Option<String>,
    uses_configuration: bool,
    placed_once: bool,
    seeded: bool,
    pending_restart: bool,
    seeded_frame: Option<Mat4>,
    looping: bool,
    /// Stepped and drawn. An animation-event host waits idle until its clip fires it.
    pub active: bool,
    /// A stop was asked for; the host runs on until its particles are gone.
    pub stopped: bool,
    /// A stop asked for before the first step, applied once seeding has reset the emitters.
    deferred_stop: Option<(bool, bool)>,
    /// The bundle effect this host plays (an event effect has several hosts).
    group: usize,
    /// When this host was last fired, to reuse the oldest of an event's hosts.
    fired: u64,
}

fn rigid_axes(m: &Mat4) -> [Vec3; 3] {
    let x = Xf::from_mat(m).rigid();
    [x.x, x.y, x.z]
}

impl EffectHost {
    pub fn new(effect: &Effect, bundle: &Bundle, cx: &mut ParseCx) -> Option<EffectHost> {
        cx.effect_loaded = 0;
        let sys = System::load(&effect.system, bundle, SYSTEM_CAP, cx, &mut Vec::new())?;
        let map = effect.transform.is_some();
        let uses_configuration = effect.attachment.is_none() && !map;
        let mut drivers = Vec::new();
        let mut preview_point = None;
        if uses_configuration {
            let def = bundle.systems.get(&effect.system).map(|s| &s.data);
            let configs = def.map_or(&[][..], |d| kv::array(d, "m_controlPointConfigurations"));
            let named = |w: &str| configs.iter().find(|c| kv::s(c, "m_name").is_some_and(|n| n.eq_ignore_ascii_case(w)));
            let chosen = effect.configuration.as_deref().and_then(named).or_else(|| named("preview")).or_else(|| configs.first());
            let staged: Vec<usize> = effect.control_points.iter().map(|c| c.index).collect();
            let all: Vec<Driver> = chosen
                .map(|c| kv::array(c, "m_drivers"))
                .unwrap_or(&[])
                .iter()
                .map(|d| Driver {
                    cp: kv::i(d, "m_iControlPoint", 0).clamp(0, 63) as usize,
                    kind: kv::s(d, "m_iAttachType").unwrap_or("").to_ascii_lowercase().trim_start_matches("pattach_").to_string(),
                    name: kv::s(d, "m_attachmentName").unwrap_or("").to_string(),
                    offset: kv::v3(d, "m_vecOffset", [0.0; 3]),
                    angles: kv::v3(d, "m_angOffset", [0.0; 3]),
                })
                .collect();
            if let Some(first) = all.first()
                && !staged.contains(&first.cp)
                && on_point(&first.kind)
            {
                preview_point = Some(first.name.clone());
            }
            drivers = all.into_iter().filter(|d| !staged.contains(&d.cp)).collect();
        }
        Some(EffectHost {
            effect: effect.clone(),
            sys,
            drivers,
            preview_point,
            uses_configuration,
            placed_once: false,
            seeded: false,
            pending_restart: false,
            seeded_frame: None,
            // An animation-event effect plays once per event (Grimoire's `effectHost.ts`).
            looping: effect.source != "anim-event" && effect.clip.is_none(),
            active: effect.clip.is_none(),
            stopped: false,
            deferred_stop: None,
            group: 0,
            fired: 0,
        })
    }

    /// Starts this host over as a new instance of its effect (Grimoire creates a host per
    /// event): placed, seeded from the placement and started on the next step.
    pub fn fire(&mut self, order: u64) {
        self.sys.replay();
        self.placed_once = false;
        self.seeded = false;
        self.pending_restart = false;
        self.seeded_frame = None;
        self.stopped = false;
        self.deferred_stop = None;
        self.active = true;
        self.fired = order;
    }

    /// Stops emission, with the end cap or not; an immediate stop drops the particles too.
    pub fn stop(&mut self, immediately: bool, end_cap: bool) {
        if !self.active || self.stopped {
            return;
        }
        self.stopped = true;
        if self.seeded {
            self.sys.stop_emission(immediately, end_cap);
        } else {
            // Seeding on the first step resets the emitters: stop after it.
            self.deferred_stop = Some((immediately, end_cap));
        }
    }

    fn attached_frame(a: &EffectAttach, attachment: &dyn Fn(&str) -> Option<Mat4>) -> Mat4 {
        let offset = Mat4::translation(a.offset);
        let frame = |name: &str| if name.is_empty() { Mat4::IDENTITY } else { attachment(name).unwrap_or(Mat4::IDENTITY) };
        match a.kind.as_str() {
            "point" | "point_follow" => frame(&a.point).mul(&offset),
            "worldorigin" => offset,
            _ => offset,
        }
    }

    fn set_cp_to(&mut self, i: usize, m: &Mat4) {
        let pos = [m.0[3][0], m.0[3][1], m.0[3][2]];
        let fwd = [m.0[0][0], m.0[0][1], m.0[0][2]];
        let axes = (fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2] > 1e-12).then(|| rigid_axes(m));
        self.sys.set_cp(i, pos, axes);
    }

    /// Places the bound control points for this frame.
    pub fn place(&mut self, attachment: &dyn Fn(&str) -> Option<Mat4>) {
        let first = !self.placed_once;
        self.placed_once = true;
        if let Some(m) = self.effect.transform {
            if first {
                self.set_cp_to(0, &m);
                for k in 0..self.effect.control_points.len() {
                    let cp = self.effect.control_points[k].clone();
                    if let Some(t) = cp.transform {
                        self.set_cp_to(cp.index, &t);
                    }
                    if let Some(v) = cp.value {
                        self.sys.set_cp(cp.index, v, None);
                    }
                }
            }
            return;
        }
        if first {
            for k in 0..self.effect.control_points.len() {
                let cp = self.effect.control_points[k].clone();
                if let Some(t) = cp.transform {
                    self.set_cp_to(cp.index, &t);
                }
                if let Some(v) = cp.value {
                    self.sys.set_cp(cp.index, v, None);
                }
            }
        }
        if self.uses_configuration {
            for k in 0..self.drivers.len() {
                let d = self.drivers[k].clone();
                if !first && !follows(&d.kind) {
                    continue;
                }
                let point = on_point(&d.kind) && !d.name.is_empty();
                let attach = if point { attachment(&d.name).unwrap_or(Mat4::IDENTITY) } else { Mat4::IDENTITY };
                let pos = attach.point(d.offset);
                if point {
                    let a = Xf::from_mat(&attach).rigid();
                    let e = euler_axes_deg(d.angles);
                    // Row-vector `angles * attach`: the attachment rotates the angle offset's axes.
                    let axes = if d.angles == [0.0; 3] { [a.x, a.y, a.z] } else { [a.vector(e[0]), a.vector(e[1]), a.vector(e[2])] };
                    self.sys.set_cp(d.cp, pos, Some(axes));
                } else if first {
                    self.sys.set_cp(d.cp, pos, None);
                    if d.angles == [0.0; 3] {
                        self.sys.orient_cp(d.cp, [0.0; 3]);
                    } else {
                        let pos = self.sys.st.cps[d.cp].pos;
                        self.sys.set_cp(d.cp, pos, Some(euler_axes_deg(d.angles)));
                    }
                } else {
                    self.sys.set_cp(d.cp, pos, None);
                }
            }
            if let Some(name) = self.preview_point.clone()
                && let Some(m) = attachment(&name)
            {
                let axes = self.sys.st.cps[0].axes;
                self.sys.set_cp(0, [m.0[3][0], m.0[3][1], m.0[3][2]], Some(axes));
            }
            return;
        }
        if let Some(a) = self.effect.attachment.clone() {
            let frame = Self::attached_frame(&a, attachment);
            if self.seeded_frame != Some(frame) {
                self.seeded_frame = Some(frame);
                self.set_cp_to(0, &frame);
            }
        }
        if let Some(a) = self.effect.attachment_cp1.clone() {
            let frame = Self::attached_frame(&a, attachment);
            self.set_cp_to(1, &frame);
        }
    }

    fn placement_seed(&self) -> i32 {
        if let Some(s) = self.effect.seed {
            return s as i32;
        }
        let p = if self.uses_configuration { [0.0; 3] } else { self.sys.st.cps[0].pos };
        let h = |x: f32, k: i32| (x.round_ties_even() as i32).wrapping_mul(k);
        (string_token(&self.sys.name) as i32) ^ h(p[0], 73_856_093) ^ h(p[1], 19_349_663) ^ h(p[2], 83_492_791)
    }

    pub fn step(&mut self, dt: f32, attachment: &dyn Fn(&str) -> Option<Mat4>, camera: Vec3) {
        if !self.placed_once {
            self.place(attachment);
            self.sys.start();
        } else {
            self.place(attachment);
        }
        if !self.seeded {
            self.seeded = true;
            let seed = self.placement_seed();
            self.sys.seed(seed);
            // An event effect stopped before its first step stays stopped.
            if let Some((immediately, end_cap)) = self.deferred_stop.take() {
                self.sys.stop_emission(immediately, end_cap);
            }
        }
        if self.pending_restart {
            self.pending_restart = false;
            self.sys.replay();
        }
        self.sys.update(dt.min(0.1), camera);
        if self.sys.is_finished() {
            if self.looping && !self.stopped {
                self.pending_restart = true;
            } else {
                // A finished event effect (or a stopped one) is removed until fired again.
                self.active = false;
            }
        }
    }
}

/// One drawable renderer of one system in an effect tree.
#[derive(Clone, Debug)]
pub struct Slot {
    pub host: usize,
    /// Child indices from the root system down.
    pub path: Vec<usize>,
    pub renderer: usize,
}

/// How many instances of one animation-event effect may play at once. Grimoire makes a host
/// per event and drops it when it finishes; here each event effect keeps this many hosts (so
/// the renderer's draws stay fixed) and a new event reuses an idle one, else the oldest.
pub const EVENT_INSTANCES: usize = 2;

/// Every active effect of a bundle, simulated on the CPU.
pub struct Effects {
    pub hosts: Vec<EffectHost>,
    pub slots: Vec<Slot>,
    diagnostics: Vec<String>,
    order: Vec<u32>,
    dist: Vec<f32>,
    /// The clip the viewer plays and how far through it (0 to 1) it was last frame.
    clip: Option<(String, f32)>,
    fired: u64,
    /// The model bones (name, inverse bind pose) skinned snapshots deform by.
    bones: Vec<(String, Mat4)>,
}

fn collect_slots(sys: &System, host: usize, path: &mut Vec<usize>, out: &mut Vec<Slot>) {
    for r in 0..sys.renderers.len() {
        out.push(Slot { host, path: path.clone(), renderer: r });
    }
    for (k, c) in sys.children.iter().enumerate() {
        path.push(k);
        collect_slots(c, host, path, out);
        path.pop();
    }
}

fn system_at<'a>(sys: &'a mut System, path: &[usize]) -> &'a mut System {
    match path.split_first() {
        None => sys,
        Some((&k, rest)) => system_at(&mut sys.children[k], rest),
    }
}

impl Effects {
    pub fn new(bundle: &Bundle) -> Effects {
        let mut cx = ParseCx::default();
        let mut hosts = Vec::new();
        for (group, e) in bundle.effects.iter().enumerate() {
            // Animation-event effects wait for their clip; the rest play when active.
            let count = if e.clip.is_some() {
                EVENT_INSTANCES
            } else if e.start_active {
                1
            } else {
                0
            };
            for _ in 0..count {
                if let Some(mut h) = EffectHost::new(e, bundle, &mut cx) {
                    h.group = group;
                    hosts.push(h);
                }
            }
        }
        let mut slots = Vec::new();
        for (k, h) in hosts.iter().enumerate() {
            collect_slots(&h.sys, k, &mut Vec::new(), &mut slots);
        }
        let mut diagnostics = bundle.issues.clone();
        diagnostics.extend(cx.diagnostics);
        // Bones and inverse bind poses, kept only when a function skins to them.
        let bones = if cx.skinned { bundle.bones.iter().map(|(n, bind)| (n.clone(), bind.inverse())).collect() } else { Vec::new() };
        Effects { hosts, slots, diagnostics, order: Vec::new(), dist: Vec::new(), clip: None, fired: 0, bones }
    }

    /// Reports the clip the viewer plays (its glb name and how far through it, 0 to 1), or
    /// none, for the animation-event effects (Grimoire's `heroParticleHost.ts` `setClip`): a
    /// clip change stops the effects that end with their clip and passes the new clip's
    /// events from its start; otherwise the events in (last, now] pass, both sides of a loop.
    pub fn set_clip(&mut self, state: Option<(&str, f32)>) {
        let state = state.map(|(name, f)| (name, if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 }));
        let previous = self.clip.take();
        match (state, previous) {
            (Some((name, now)), Some((was, last))) if was == name => {
                if now >= last {
                    self.pass_events(name, last, now);
                } else {
                    // The clip looped (or was sought back).
                    self.pass_events(name, last, 1.0);
                    self.pass_events(name, -1.0, now);
                }
                self.clip = Some((was, now));
            }
            (state, previous) => {
                if let Some((was, _)) = &previous {
                    for h in &mut self.hosts {
                        let ends = h.effect.clip.as_ref().is_some_and(|c| c.name == *was && c.stop_on_clip_change);
                        if ends {
                            h.stop(false, false);
                        }
                    }
                }
                if let Some((name, now)) = state {
                    self.pass_events(name, -1.0, now);
                    self.clip = Some((name.to_string(), now));
                }
            }
        }
    }

    /// Stops and fires the event effects of `clip` whose times fall in (from, to].
    fn pass_events(&mut self, clip: &str, from: f32, to: f32) {
        let within = |t: f32| t > from && t <= to;
        let mut k = 0;
        while k < self.hosts.len() {
            // One effect's hosts sit together: handle them as a group.
            let group = self.hosts[k].group;
            let end = (k..self.hosts.len()).find(|&j| self.hosts[j].group != group).unwrap_or(self.hosts.len());
            let Some(events) = self.hosts[k].effect.clip.clone().filter(|c| c.name == clip) else {
                k = end;
                continue;
            };
            for stop in events.stops.iter().filter(|s| within(s.time)) {
                for h in &mut self.hosts[k..end] {
                    h.stop(stop.immediately, stop.end_cap);
                }
            }
            if within(events.start) {
                let pick = (k..end).find(|&j| !self.hosts[j].active).or_else(|| (k..end).min_by_key(|&j| self.hosts[j].fired));
                if let Some(j) = pick {
                    self.fired += 1;
                    self.hosts[j].fire(self.fired);
                }
            }
            k = end;
        }
    }

    /// Unsupported classes and inputs, missing systems and textures, as `system: what`.
    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn push_diagnostic(&mut self, d: String) {
        if !self.diagnostics.contains(&d) {
            self.diagnostics.push(d);
        }
    }

    /// Steps every effect; `attachment` resolves a name to its Source-space frame now and
    /// `camera` is in Source space.
    pub fn step(&mut self, dt: f32, attachment: &dyn Fn(&str) -> Option<Mat4>, camera: Vec3) {
        if !self.bones.is_empty() {
            // Control point 0's bones this frame, for the skinned snapshot functions.
            let palette: Vec<Mat4> = self.bones.iter().map(|(name, inverse)| attachment(name).map_or(Mat4::IDENTITY, |m| m.mul(inverse))).collect();
            let palette = std::sync::Arc::new(palette);
            for h in self.hosts.iter_mut().filter(|h| h.active) {
                h.sys.set_skin(&palette);
            }
        }
        for h in self.hosts.iter_mut().filter(|h| h.active) {
            h.step(dt, attachment, camera);
        }
    }

    pub fn particle_count(&self) -> usize {
        self.hosts.iter().filter(|h| h.active).map(|h| h.sys.particle_count()).sum()
    }

    /// Effect instances playing now (an idle animation-event host does not count).
    pub fn active_count(&self) -> usize {
        self.hosts.iter().filter(|h| h.active).count()
    }

    /// Builds every slot's cards into `out`, recording each slot's instance range in
    /// `ranges`, and its shading constants in `shading`.
    pub fn build(&mut self, view: &View, sheets: &std::collections::HashMap<String, super::bundle::Sheet>, out: &mut Vec<Instance>, ranges: &mut Vec<(u32, u32)>, shading: &mut Vec<[f32; 5]>) {
        out.clear();
        ranges.clear();
        shading.clear();
        for s in &self.slots {
            let start = out.len() as u32;
            let active = self.hosts[s.host].active;
            let sys = system_at(&mut self.hosts[s.host].sys, &s.path);
            let System { renderers, st, p, .. } = sys;
            let r = &mut renderers[s.renderer];
            shading.push(r.shading(st));
            if active && out.len() < TOTAL_CAP {
                r.build(st, p, view, sheets.get(&r.texture), out, &mut self.order, &mut self.dist);
                out.truncate(TOTAL_CAP);
            }
            ranges.push((start, out.len() as u32));
        }
    }
}
