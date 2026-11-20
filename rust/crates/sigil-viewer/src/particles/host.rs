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
            looping: effect.source != "anim-event",
        })
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
        }
        if self.pending_restart {
            self.pending_restart = false;
            self.sys.replay();
        }
        self.sys.update(dt.min(0.1), camera);
        if self.looping && self.sys.is_finished() {
            self.pending_restart = true;
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

/// Every active effect of a bundle, simulated on the CPU.
pub struct Effects {
    pub hosts: Vec<EffectHost>,
    pub slots: Vec<Slot>,
    diagnostics: Vec<String>,
    order: Vec<u32>,
    dist: Vec<f32>,
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
        for e in bundle.effects.iter().filter(|e| e.start_active) {
            if let Some(h) = EffectHost::new(e, bundle, &mut cx) {
                hosts.push(h);
            }
        }
        let mut slots = Vec::new();
        for (k, h) in hosts.iter().enumerate() {
            collect_slots(&h.sys, k, &mut Vec::new(), &mut slots);
        }
        let mut diagnostics = bundle.issues.clone();
        diagnostics.extend(cx.diagnostics);
        Effects { hosts, slots, diagnostics, order: Vec::new(), dist: Vec::new() }
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
        for h in &mut self.hosts {
            h.step(dt, attachment, camera);
        }
    }

    pub fn particle_count(&self) -> usize {
        self.hosts.iter().map(|h| h.sys.particle_count()).sum()
    }

    /// Builds every slot's cards into `out`, recording each slot's instance range in
    /// `ranges`, and its shading constants in `shading`.
    pub fn build(&mut self, view: &View, sheets: &std::collections::HashMap<String, super::bundle::Sheet>, out: &mut Vec<Instance>, ranges: &mut Vec<(u32, u32)>, shading: &mut Vec<[f32; 5]>) {
        out.clear();
        ranges.clear();
        shading.clear();
        for s in &self.slots {
            let start = out.len() as u32;
            let sys = system_at(&mut self.hosts[s.host].sys, &s.path);
            let System { renderers, st, p, .. } = sys;
            let r = &renderers[s.renderer];
            shading.push(r.shading(st));
            if out.len() < TOTAL_CAP {
                r.build(st, p, view, sheets.get(&r.texture), out, &mut self.order, &mut self.dist);
                out.truncate(TOTAL_CAP);
            }
            ranges.push((start, out.len() as u32));
        }
    }
}
