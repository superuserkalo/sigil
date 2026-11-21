//! `C_OP_RenderRopes` on the CPU, ported from Grimoire's `render/renderRopes.ts` (MIT), itself
//! a port of ValveResourceFormat's `RenderRopes.cs` and the spline helpers of
//! `CableMeshBuilder.cs` (MIT).
//!
//! The particles are threaded into a ribbon: a Catmull-Rom spline through consecutive
//! particles, each segment cut into a power of two of pieces picked from the rope's screen
//! coverage, its width axis from the spline tangent and the plane the orientation selects
//! (kept on one hemisphere along the strip), V from the world arc length over
//! `m_flTextureVWorldSize` with scroll and offset. The ribbon breaks where the split field
//! (`RopeSegmentId` by default) changes. Each piece is one strip quad ([`Instance`] mode 3)
//! drawn by the spritecard pipeline, so ropes need no pipeline of their own.

use serde_json::Value;

use super::super::bundle::Sheet;
use super::super::fields::{self as fl, Particles};
use super::super::kv;
use super::super::providers::{NumIn, ParseCx};
use super::super::smath::{smoothstep, srgb_to_linear};
use super::super::state::State;
use super::{Instance, Renderer, View, blend_color, convert_color_scale};
use crate::math::{Vec3, add, cross, dot, length, normalize, scale, sub};

/// The shared index buffer of Grimoire's strip covers 65532 indices, six a quad.
const MAX_QUADS: usize = 65532 / 6;
const MAX_SUBDIVISIONS: i32 = 128;
/// The V offset carried when the first particle dies grows by the length of the last segment
/// of the first batch of this many.
const SEGMENT_BATCH_SIZE: usize = 527;
const ROPE_SEGMENT_ID: usize = 47;
const MINIMUM_LENGTH_SQUARED: f32 = 1e-10;
const FLOAT_EPSILON: f32 = 1.192_092_9e-7;
/// `ribbonWidthAxis`: past this share of Z a ribbon turns about world X instead of Z.
const MAX_RIBBON_VERTICAL_SHARE: f32 = 0.999;

#[derive(Clone, Debug)]
pub struct Rope {
    orientation_field: usize,
    min_tess: i32,
    max_tess: i32,
    tess_scale: f32,
    v_world_size: NumIn,
    v_scroll_rate: NumIn,
    v_offset: NumIn,
    v_params_cp: i32,
    clamp_v: bool,
    scale_cp1: i32,
    scale_cp2: i32,
    scale_v_size: f32,
    scale_v_scroll: f32,
    scale_v_offset: f32,
    use_scalar: bool,
    scalar_field: usize,
    scalar_scale: f32,
    reverse: bool,
    closed: bool,
    split_field: i32,
    sort_by_segment: bool,
    fading: bool,
    min_size: f32,
    max_size: f32,
    start_fade_size: NumIn,
    end_fade_size: NumIn,
    // Carried between frames.
    v_accumulator: f32,
    first_segment_length: f32,
    first_particle: Option<u32>,
}

struct Node {
    index: usize,
    segment: u32,
    pos: Vec3,
    radius: f32,
    color: [f32; 4],
    orientation: Vec3,
    scalar: f32,
}

struct Sample {
    pos: Vec3,
    axis: Vec3,
    facing: Vec3,
    radius: f32,
    color: [f32; 4],
}

fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1 + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

fn catmull_rom_v(p: [Vec3; 4], t: f32) -> Vec3 {
    [0, 1, 2].map(|k| catmull_rom(p[0][k], p[1][k], p[2][k], p[3][k], t))
}

/// The derivative of [`catmull_rom`] with respect to `t`.
fn catmull_rom_tangent(p: [Vec3; 4], t: f32) -> Vec3 {
    let t2 = t * t;
    [0, 1, 2].map(|k| {
        let (a, b, c, d) = (p[0][k], p[1][k], p[2][k], p[3][k]);
        0.5 * (-a + c + (4.0 * a - 10.0 * b + 8.0 * c - 2.0 * d) * t + (-3.0 * a + 9.0 * b - 9.0 * c + 3.0 * d) * t2)
    })
}

fn safe_normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let l2 = dot(v, v);
    if l2 > MINIMUM_LENGTH_SQUARED { scale(v, 1.0 / l2.sqrt()) } else { fallback }
}

/// The unit axis across a ribbon running along `dir` and lying flat in the plane facing
/// `plane_normal` (Grimoire's `ribbonWidthAxis`): parallel inputs turn it about world Z, or
/// world X when the ribbon runs nearly vertical.
pub fn ribbon_width_axis(dir: Vec3, plane_normal: Vec3) -> Vec3 {
    let w = cross(dir, plane_normal);
    if dot(w, w) > MINIMUM_LENGTH_SQUARED {
        return normalize(w);
    }
    let reference = if dir[2].abs() < MAX_RIBBON_VERTICAL_SHARE { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let f = cross(dir, reference);
    if dot(f, f) > 0.0 { normalize(f) } else { [1.0, 0.0, 0.0] }
}

/// How much of the view a sphere fills, 0 to 1 (VRF `ScreenSizeFraction`): all of it when
/// the camera is inside.
pub fn screen_size_fraction(center: Vec3, radius: f32, camera: Vec3, projection: f32) -> f32 {
    let d = sub(center, camera);
    let d2 = dot(d, d);
    if radius * radius > d2 {
        return 1.0;
    }
    (radius * projection / d2.sqrt()).clamp(0.0, 1.0)
}

/// The tessellation level (subdivisions are `1 << level`) for a rope of `count` nodes
/// spanning `diagonal` around `center`.
pub fn tessellation_level(coverage: f32, count: usize, tess_scale: f32, min: i32, max: i32) -> u32 {
    let metric = coverage.sqrt() * 1024.0 / count.max(1) as f32;
    let raw = (metric * tess_scale * 0.1).trunc() as i32;
    let mut sub = raw.min(max);
    if min > sub {
        sub = min;
    }
    let sub = sub.clamp(1, MAX_SUBDIVISIONS);
    if sub == 1 { 0 } else { ((sub - 1) as f32).log2().floor() as u32 + 1 }
}

impl Rope {
    pub fn parse(v: &Value, cx: &mut ParseCx) -> Rope {
        let num = |k: &str, d: f32, cx: &mut ParseCx| NumIn::parse_or(v, k, d, cx);
        if kv::b(v, "m_bDrawAsOpaque", false) {
            cx.note("C_OP_RenderRopes m_bDrawAsOpaque draws in the translucent pass".to_string());
        }
        let field = |k: &str, d: usize| -> usize {
            let f = kv::field(v, k, d);
            if (0..fl::COUNT as i32).contains(&f) { f as usize } else { d }
        };
        Rope {
            orientation_field: field("m_nVectorFieldForOrientation", fl::NORMAL),
            min_tess: kv::i(v, "m_nMinTesselation", 1),
            max_tess: kv::i(v, "m_nMaxTesselation", 128),
            tess_scale: kv::f(v, "m_flTessScale", 1.0),
            v_world_size: num("m_flTextureVWorldSize", 10.0, cx),
            v_scroll_rate: num("m_flTextureVScrollRate", 0.0, cx),
            v_offset: num("m_flTextureVOffset", 0.0, cx),
            v_params_cp: kv::i(v, "m_nTextureVParamsCP", -1),
            clamp_v: kv::b(v, "m_bClampV", false),
            scale_cp1: kv::i(v, "m_nScaleCP1", -1),
            scale_cp2: kv::i(v, "m_nScaleCP2", -1),
            scale_v_size: kv::f(v, "m_flScaleVSizeByControlPointDistance", 0.0),
            scale_v_scroll: kv::f(v, "m_flScaleVScrollByControlPointDistance", 0.0),
            scale_v_offset: kv::f(v, "m_flScaleVOffsetByControlPointDistance", 0.0),
            use_scalar: kv::b(v, "m_bUseScalarForTextureCoordinate", false),
            scalar_field: field("m_nScalarFieldForTextureCoordinate", fl::CREATION),
            scalar_scale: kv::f(v, "m_flScalarAttributeTextureCoordScale", 1.0),
            reverse: kv::b(v, "m_bReverseOrder", false),
            closed: kv::b(v, "m_bClosedLoop", false),
            split_field: kv::field(v, "m_nSplitField", ROPE_SEGMENT_ID),
            sort_by_segment: kv::b(v, "m_bSortBySegmentID", false),
            fading: kv::b(v, "m_bEnableFadingAndClamping", false),
            min_size: kv::f(v, "m_flMinSize", 0.0),
            max_size: kv::f(v, "m_flMaxSize", 2000.0),
            start_fade_size: num("m_flStartFadeSize", 1000.0, cx),
            end_fade_size: num("m_flEndFadeSize", 2000.0, cx),
            v_accumulator: 0.0,
            first_segment_length: 0.0,
            first_particle: None,
        }
    }

    fn splits(&self) -> bool {
        self.split_field >= 0 && self.split_field != fl::NONE as i32 && fl::is_scalar(self.split_field)
    }

    /// A closed loop wraps, an open rope clamps, so a segment's tangents never pass its ends.
    fn resolve(&self, index: isize, count: usize) -> usize {
        let n = count as isize;
        if self.closed {
            if index < 0 {
                return count - 1;
            }
            return if index >= n { (index - n) as usize } else { index as usize };
        }
        index.clamp(0, n - 1) as usize
    }

    /// Scroll rate, V offset and one over the texture's world size: a V params control point
    /// replaces all three, else the control point distance factors add to them.
    fn texture_v(&self, st: &mut State) -> (f32, f32, f32) {
        let mut scroll = self.v_scroll_rate.next(st);
        let mut offset = self.v_offset.next(st);
        let mut size = self.v_world_size.next(st);
        if self.v_params_cp >= 0 {
            let p = st.cp(self.v_params_cp).pos;
            return (p[1], p[2], 1.0 / p[0].max(FLOAT_EPSILON));
        }
        let modulated = (self.scale_v_size > 0.0 || self.scale_v_scroll > 0.0 || self.scale_v_offset > 0.0) && self.scale_cp1 > -1 && self.scale_cp2 > -1;
        if modulated {
            let d = length(sub(st.cp(self.scale_cp1).pos, st.cp(self.scale_cp2).pos));
            if self.scale_v_size > 0.0 {
                size += self.scale_v_size * d;
            }
            if self.scale_v_scroll > 0.0 {
                scroll += self.scale_v_scroll * d;
            }
            if self.scale_v_offset > 0.0 {
                offset += self.scale_v_offset * d;
            }
        }
        (scroll, offset, 1.0 / size.max(FLOAT_EPSILON))
    }

    /// Wraps the first segment's length into the carried V offset when a different particle
    /// leads the rope, as when its oldest particle dies.
    fn carry_v_offset(&mut self, p: &Particles, one_over: f32) {
        let id = p.s[fl::PARTICLE_ID][0].to_bits();
        if let Some(first) = self.first_particle
            && first != id
        {
            self.v_accumulator = (self.v_accumulator + self.first_segment_length) % (1.0 / one_over);
        }
        self.first_particle = Some(id);
    }
}

impl Renderer {
    /// The particle order a rope walks: storage order (reversed on request), grouped by
    /// segment in the order each first appears when sorted.
    fn rope_order(rope: &Rope, p: &Particles, out: &mut Vec<u32>) {
        let n = p.len;
        out.clear();
        out.extend((0..n as u32).map(|k| if rope.reverse { (n as u32) - 1 - k } else { k }));
        if rope.sort_by_segment && rope.splits() {
            let seg = &p.s[rope.split_field as usize];
            let mut first: Vec<(u32, usize)> = Vec::new();
            let rank = |bits: u32, first: &mut Vec<(u32, usize)>| match first.iter().find(|(b, _)| *b == bits) {
                Some(&(_, r)) => r,
                None => {
                    first.push((bits, first.len()));
                    first.len() - 1
                }
            };
            let ranks: Vec<usize> = out.iter().map(|&i| rank(seg[i as usize].to_bits(), &mut first)).collect();
            let mut keyed: Vec<(usize, u32)> = ranks.into_iter().zip(out.iter().copied()).collect();
            keyed.sort_by_key(|&(r, _)| r);
            out.clear();
            out.extend(keyed.into_iter().map(|(_, i)| i));
        }
    }

    /// Appends the rope's strip quads for `p` to `out`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build_rope(&mut self, st: &mut State, p: &Particles, view: &View, sheet: Option<&Sheet>, out: &mut Vec<Instance>, order: &mut Vec<u32>) {
        let n = p.len;
        if n < 2 {
            return;
        }
        let radius_scale = self.radius_scale.next(st);
        let alpha_scale = self.alpha_scale.next(st);
        if radius_scale == 0.0 || alpha_scale == 0.0 {
            return;
        }
        let color_scale = convert_color_scale(self.color_blend, self.color_scale.next(st));
        let Some(mut rope) = self.rope.take() else { return };
        self.rope_quads(&mut rope, st, p, view, sheet, out, order, (alpha_scale, radius_scale, color_scale));
        self.rope = Some(rope);
    }

    #[allow(clippy::too_many_arguments)]
    fn rope_quads(&self, rope: &mut Rope, st: &mut State, p: &Particles, view: &View, sheet: Option<&Sheet>, out: &mut Vec<Instance>, order: &mut Vec<u32>, scales: (f32, f32, Vec3)) {
        let (alpha_scale, radius_scale, color_scale) = scales;
        Self::rope_order(rope, p, order);
        let alpha2 = self.alpha2;
        let splits = rope.splits();
        let nodes: Vec<Node> = order
            .iter()
            .map(|&k| {
                let i = k as usize;
                let a2 = if alpha2 >= 0 && alpha2 != fl::NONE as i32 && fl::is_scalar(alpha2) { p.s[alpha2 as usize][i] } else { 1.0 };
                // The alpha scale applies twice, as in Grimoire: with the particle's alpha and
                // again where the colour scale combines.
                let color = blend_color(self.color_blend, p.v[fl::COLOR][i], p.s[fl::ALPHA][i] * a2 * alpha_scale, color_scale, alpha_scale);
                Node {
                    index: i,
                    segment: if splits { p.s[rope.split_field as usize][i].to_bits() } else { 0 },
                    pos: p.v[fl::POSITION][i],
                    radius: p.s[fl::RADIUS][i] * radius_scale,
                    color,
                    orientation: if fl::is_vector(rope.orientation_field as i32) { p.v[rope.orientation_field][i] } else { [0.0, 0.0, 1.0] },
                    scalar: if rope.use_scalar && fl::is_scalar(rope.scalar_field as i32) { p.s[rope.scalar_field][i] * rope.scalar_scale } else { 0.0 },
                }
            })
            .collect();
        let count = nodes.len();
        let segments = if rope.closed { count } else { count - 1 };

        // One subdivision level for the whole rope, from how much of the view it fills.
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for nd in &nodes {
            for k in 0..3 {
                lo[k] = lo[k].min(nd.pos[k]);
                hi[k] = hi[k].max(nd.pos[k]);
            }
        }
        let coverage = screen_size_fraction(scale(add(lo, hi), 0.5), length(sub(hi, lo)), view.camera, view.projection);
        let subdivisions = 1usize << tessellation_level(coverage, count, rope.tess_scale, rope.min_tess, rope.max_tess);

        let (scroll, v_offset, one_over) = rope.texture_v(st);
        rope.carry_v_offset(p, one_over);
        let start_fade = rope.start_fade_size.next(st);
        let end_fade = rope.end_fade_size.next(st);
        // A sheet places its frames by the card coordinate (the first sequence, at the
        // system's age); without one V runs along the rope.
        let frame = match sheet {
            Some(sh) if !sh.sequences.is_empty() => Some(sh.frames(0, st.age * self.anim_rate, self.anim_fps).0),
            _ => None,
        };
        let max_quads = MAX_QUADS.min(segments * subdivisions);
        let fade_dots = self.start_fade_dot < 1.0 && self.end_fade_dot > self.start_fade_dot;
        let (fading, min_size, max_size, clamp_v, closed) = (rope.fading, rope.min_size, rope.max_size, rope.clamp_v, rope.closed);
        let normal_aligned = self.orientation == 3;
        let ws = view.world_scale;
        let world_dir = |v: Vec3| {
            let w = view.to_world.vector(v);
            let l = length(w);
            if l > 1e-12 { scale(w, 1.0 / l) } else { [0.0; 3] }
        };

        let sample = |n: [&Node; 4], t: f32| -> Sample {
            let pts = [n[0].pos, n[1].pos, n[2].pos, n[3].pos];
            let pos = catmull_rom_v(pts, t);
            let radius = catmull_rom(n[0].radius, n[1].radius, n[2].radius, n[3].radius, t);
            let tangent = safe_normalize_or(catmull_rom_tangent(pts, t), [1.0, 0.0, 0.0]);
            let orient = add(n[1].orientation, scale(sub(n[2].orientation, n[1].orientation), t));
            let plane = match self.orientation {
                3 => orient,
                2 => [0.0, 0.0, 1.0],
                _ => sub(view.camera, pos),
            };
            let mix = |a: f32, b: f32| a + (b - a) * t;
            let (c1, c2) = (n[1].color, n[2].color);
            Sample {
                pos,
                axis: ribbon_width_axis(tangent, plane),
                facing: if normal_aligned { orient } else { tangent },
                radius,
                color: [mix(c1[0], c2[0]), mix(c1[1], c2[1]), mix(c1[2], c2[2]), mix(c1[3], c2[3])],
            }
        };
        // The fades and size clamp the shader applies in Grimoire, per sample here.
        let finish = |s: &Sample| -> (f32, [f32; 4]) {
            let mut radius = s.radius;
            let mut color = s.color;
            let cam_dist = length(sub(view.camera, s.pos));
            if fading {
                radius = radius.max(min_size * cam_dist).min(max_size * cam_dist);
                if radius > start_fade * cam_dist {
                    let f = 1.0 - smoothstep(start_fade * cam_dist, end_fade * cam_dist, radius);
                    color[3] *= f;
                }
            }
            if fade_dots {
                let to_cam = safe_normalize_or(sub(view.camera, s.pos), [0.0, 0.0, 1.0]);
                color[3] *= 1.0 - smoothstep(self.start_fade_dot, self.end_fade_dot, dot(normalize(s.facing), to_cam).abs());
            }
            if self.gamma {
                color = [srgb_to_linear(color[0]), srgb_to_linear(color[1]), srgb_to_linear(color[2]), color[3]];
            }
            (radius, color)
        };

        let mut quads = 0;
        let mut arc = scroll * st.age + rope.v_accumulator;
        let mut strip_axis = [0.0f32; 3];
        let map_v = |v: f32| {
            let r = (v + v_offset) * one_over;
            if clamp_v { r.clamp(0.0, 1.0) } else { r }
        };
        for seg in 0..segments {
            if quads >= max_quads {
                break;
            }
            let n1 = &nodes[rope.resolve(seg as isize, count)];
            let n2 = &nodes[rope.resolve(seg as isize + 1, count)];
            let seg_len = length(sub(n1.pos, n2.pos));
            let ends_at_first = n2.index == 0 && !closed;
            if seg < SEGMENT_BATCH_SIZE {
                rope.first_segment_length = seg_len;
            }
            let mut v_start = arc;
            let mut v_end = if ends_at_first { 0.0 } else { arc + seg_len };
            arc += seg_len;
            // A join between two rope segments, or one of no length, is skipped (V still runs).
            if n1.segment != n2.segment || seg_len == 0.0 {
                strip_axis = [0.0; 3];
                continue;
            }
            let before = &nodes[rope.resolve(seg as isize - 1, count)];
            let after = &nodes[rope.resolve(seg as isize + 2, count)];
            let n0 = if before.segment == n1.segment { before } else { n1 };
            let n3 = if after.segment == n2.segment { after } else { n2 };
            if rope.use_scalar {
                v_start = n1.scalar;
                v_end = if n2.index == 0 { n1.scalar } else { n2.scalar };
            }
            let mut prev = sample([n0, n1, n2, n3], 0.0);
            if dot(prev.axis, strip_axis) < 0.0 {
                prev.axis = scale(prev.axis, -1.0);
            }
            strip_axis = prev.axis;
            let mut prev_done = finish(&prev);
            for step in 1..=subdivisions {
                if quads >= max_quads {
                    break;
                }
                let t0 = (step - 1) as f32 / subdivisions as f32;
                let t1 = step as f32 / subdivisions as f32;
                let mut cur = sample([n0, n1, n2, n3], t1);
                if dot(cur.axis, strip_axis) < 0.0 {
                    cur.axis = scale(cur.axis, -1.0);
                }
                strip_axis = cur.axis;
                let cur_done = finish(&cur);
                // u runs against the width axis (a rope's card is mirrored across its width).
                let uv = match frame {
                    Some(r) => [r[2], r[1] + (r[3] - r[1]) * t0, r[0], r[1] + (r[3] - r[1]) * t1],
                    None => [1.0, map_v(v_start + (v_end - v_start) * t0), 0.0, map_v(v_start + (v_end - v_start) * t1)],
                };
                quads += 1;
                if prev_done.1[3] >= 1.0 / 255.0 || cur_done.1[3] >= 1.0 / 255.0 {
                    let a = view.to_world.point(prev.pos);
                    let b = view.to_world.point(cur.pos);
                    let (au, bu) = (world_dir(prev.axis), world_dir(cur.axis));
                    out.push(Instance {
                        pos_radius: [a[0], a[1], a[2], prev_done.0 * ws],
                        axis_u: [au[0], au[1], au[2], 3.0],
                        axis_v: [b[0], b[1], b[2], cur_done.0 * ws],
                        color: prev_done.1,
                        uv_a: uv,
                        uv_b: cur_done.1,
                        misc: [bu[0], bu[1], bu[2], 0.0],
                    });
                }
                prev = cur;
                prev_done = cur_done;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catmull_rom_passes_its_middle_points() {
        let p = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 1.0, 0.0], [3.0, 1.0, 0.0]];
        assert_eq!(catmull_rom_v(p, 0.0), p[1]);
        assert_eq!(catmull_rom_v(p, 1.0), p[2]);
        // The tangent at the start is half the span from the point before to the one after.
        assert_eq!(catmull_rom_tangent(p, 0.0), [1.0, 0.5, 0.0]);
    }

    #[test]
    fn ribbon_width_axis_falls_back_when_parallel() {
        assert_eq!(ribbon_width_axis([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
        // Along the plane normal: about world Z, or X when vertical.
        assert_eq!(ribbon_width_axis([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]), [0.0, -1.0, 0.0]);
        assert_eq!(ribbon_width_axis([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]), [0.0, 1.0, 0.0]);
    }

    #[test]
    fn tessellation_follows_coverage_and_bounds() {
        // Grimoire: metric = sqrt(coverage) * 1024 / nodes, times tess scale times 0.1.
        assert_eq!(tessellation_level(1.0, 10, 1.0, 1, 128), 4); // 10 pieces -> 16
        assert_eq!(tessellation_level(1.0, 10, 1.0, 1, 4), 2); // capped at 4
        assert_eq!(tessellation_level(0.0, 10, 1.0, 1, 128), 0); // floor of 1
        assert_eq!(tessellation_level(0.0, 10, 1.0, 9, 4), 4); // the floor wins over the cap
        assert_eq!(screen_size_fraction([0.0; 3], 10.0, [0.0, 0.0, 5.0], 2.0), 1.0);
        assert!((screen_size_fraction([0.0; 3], 1.0, [0.0, 0.0, 10.0], 2.0) - 0.2).abs() < 1e-6);
    }
}
