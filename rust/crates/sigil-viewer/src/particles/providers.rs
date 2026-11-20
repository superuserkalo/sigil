//! Float, vector and transform inputs (VRF `CParticleFloatInput`, `CParticleVecInput`,
//! `CParticleTransformInput`), their attribute mapping and piecewise curves. Ported from
//! Grimoire's `numberProviders.ts`, `attributeMapping.ts`, `utils/curve.ts`,
//! `vectorProviders.ts` and `transformProviders.ts` (MIT), themselves ports of VRF (MIT).

#![allow(clippy::needless_range_loop)]

use serde_json::Value;

use super::fields::{self, Particles};
use super::kv;
use super::smath::{Xf, bias_from_parameter, clamp_either, lerp, lerpv, mulv, sat};
use super::state::State;
use crate::math::{Vec3, length, sub};

/// Collects the unsupported inputs and classes a definition uses, and whether the function
/// being parsed draws per-particle random values.
#[derive(Default)]
pub struct ParseCx {
    pub diagnostics: Vec<String>,
    pub random_inputs: bool,
    pub next_slot: i32,
    pub system: String,
}

impl ParseCx {
    pub fn note(&mut self, what: String) {
        let line = format!("{}: {what}", self.system);
        if !self.diagnostics.contains(&line) {
            self.diagnostics.push(line);
        }
    }
}

const MAP_TYPES: &[(&str, i32)] = &[
    ("PF_MAP_TYPE_INVALID", -1),
    ("PF_MAP_TYPE_DIRECT", 0),
    ("PF_MAP_TYPE_MULT", 1),
    ("PF_MAP_TYPE_REMAP", 2),
    ("PF_MAP_TYPE_REMAP_BIASED", 3),
    ("PF_MAP_TYPE_CURVE", 4),
    ("PF_MAP_TYPE_NOTCHED", 5),
    ("PF_MAP_TYPE_ROUND", 6),
    ("PF_MAP_TYPE_MIN", 7),
    ("PF_MAP_TYPE_MAX", 8),
    ("PF_MAP_TYPE_MOD", 9),
];
const BIAS_TYPES: &[(&str, i32)] = &[("PF_BIAS_TYPE_STANDARD", 0), ("PF_BIAS_TYPE_GAIN", 1), ("PF_BIAS_TYPE_EXPONENTIAL", 2)];
const ROUND_TYPES: &[(&str, i32)] = &[("PF_ROUND_TYPE_NEAREST", 0), ("PF_ROUND_TYPE_FLOOR", 1), ("PF_ROUND_TYPE_CEIL", 2)];

#[derive(Clone, Debug)]
enum Map {
    Direct,
    Mult(f32),
    Remap { i0: f32, i1: f32, o0: f32, o1: f32 },
    RemapBiased { i0: f32, i1: f32, o0: f32, o1: f32, kind: i32, param: f32 },
    Curve(Curve),
    Notched { min: f32, max: f32, inside: f32, outside: f32 },
    Round(i32),
    Min(f32),
    Max(f32),
    Mod(f32),
}

/// The mapping half of a float input (VRF `AttributeMapping`).
#[derive(Clone, Debug)]
pub struct Mapping {
    map: Map,
    looped: bool,
}

impl Mapping {
    pub const DIRECT: Mapping = Mapping { map: Map::Direct, looped: false };

    pub fn parse(v: &Value) -> Mapping {
        let looped = kv::s(v, "m_nInputMode").is_some_and(|m| m.eq_ignore_ascii_case("PF_INPUT_MODE_LOOPED"));
        let f = |k: &str, d: f32| kv::f(v, k, d);
        let map = match kv::enumv(v, "m_nMapType", MAP_TYPES, 0) {
            1 => Map::Mult(f("m_flMultFactor", 1.0)),
            2 => Map::Remap { i0: f("m_flInput0", 0.0), i1: f("m_flInput1", 1.0), o0: f("m_flOutput0", 0.0), o1: f("m_flOutput1", 1.0) },
            3 => Map::RemapBiased {
                i0: f("m_flInput0", 0.0),
                i1: f("m_flInput1", 1.0),
                o0: f("m_flOutput0", 0.0),
                o1: f("m_flOutput1", 1.0),
                kind: kv::enumv(v, "m_nBiasType", BIAS_TYPES, 0),
                param: f("m_flBiasParameter", 0.0),
            },
            4 => Map::Curve(Curve::parse(kv::get(v, "m_Curve"), looped)),
            5 => {
                let (a, b) = (f("m_flNotchedRangeMin", 0.0), f("m_flNotchedRangeMax", 1.0));
                Map::Notched { min: a.min(b), max: a.max(b), inside: f("m_flNotchedOutputInside", 1.0), outside: f("m_flNotchedOutputOutside", 0.0) }
            }
            6 => Map::Round(kv::enumv(v, "m_nRoundType", ROUND_TYPES, 0)),
            7 => Map::Min(f("m_flCompareValue", 0.0)),
            8 => Map::Max(f("m_flCompareValue", 0.0)),
            9 => Map::Mod(f("m_flCompareValue", 0.0)),
            _ => Map::Direct,
        };
        Mapping { map, looped }
    }

    /// A `CParticleRemapFloatInput` (`m_flOutputRemap`): a random type maps 0..1 onto its range.
    pub fn parse_remap(v: &Value) -> Mapping {
        let f = |k: &str, d: f32| kv::f(v, k, d);
        match kv::s(v, "m_nType").unwrap_or("PF_TYPE_INVALID") {
            "PF_TYPE_RANDOM_UNIFORM" => Mapping { map: Map::Remap { i0: 0.0, i1: 1.0, o0: f("m_flRandomMin", 0.0), o1: f("m_flRandomMax", 1.0) }, looped: false },
            "PF_TYPE_RANDOM_BIASED" => Mapping {
                map: Map::RemapBiased {
                    i0: 0.0,
                    i1: 1.0,
                    o0: f("m_flRandomMin", 0.0),
                    o1: f("m_flRandomMax", 1.0),
                    kind: kv::enumv(v, "m_nBiasType", BIAS_TYPES, 0),
                    param: f("m_flBiasParameter", 0.0),
                },
                looped: false,
            },
            _ => Mapping::parse(v),
        }
    }

    fn fraction(&self, x: f32, i0: f32, i1: f32) -> f32 {
        let mut x = x;
        if self.looped && i1 != 0.0 {
            x = i1 * (x / i1).rem_euclid(1.0);
        }
        let range = i1 - i0;
        if range == 0.0 {
            return if i0 <= x { 1.0 } else { 0.0 };
        }
        (x - i0) / range
    }

    pub fn apply(&self, x: f32) -> f32 {
        match &self.map {
            Map::Direct => x,
            Map::Mult(k) => x * k,
            Map::Remap { i0, i1, o0, o1 } => clamp_either(lerp(*o0, *o1, self.fraction(x, *i0, *i1)), *o0, *o1),
            Map::RemapBiased { i0, i1, o0, o1, kind, param } => {
                let t = bias_from_parameter(self.fraction(x, *i0, *i1).min(1.0), *param, *kind);
                clamp_either(lerp(*o0, *o1, t), *o0, *o1)
            }
            Map::Curve(c) => c.evaluate(x),
            Map::Notched { min, max, inside, outside } => {
                if x >= *min && x <= *max {
                    *inside
                } else {
                    *outside
                }
            }
            Map::Round(kind) => match kind {
                1 => x.floor(),
                2 => x.ceil(),
                _ => x.round_ties_even(),
            },
            Map::Min(c) => x.min(*c),
            Map::Max(c) => x.max(*c),
            Map::Mod(c) => {
                if *c == 0.0 {
                    x
                } else {
                    x - (x / c).trunc() * c
                }
            }
        }
    }

    /// The input from which an unlooped mapping holds its final value, and that value.
    pub fn steady(&self) -> Option<(f32, f32)> {
        if self.looped {
            return None;
        }
        match &self.map {
            Map::Remap { i1, o1, .. } | Map::RemapBiased { i1, o1, .. } => Some((*i1, *o1)),
            Map::Curve(c) => c.end(),
            Map::Notched { max, outside, .. } => Some((*max, *outside)),
            Map::Min(c) => Some((*c, *c)),
            _ => None,
        }
    }
}

const TANGENT_SPLINE: u8 = 0;
const TANGENT_LINEAR: u8 = 1;
const TANGENT_FLAT: u8 = 2;
const TANGENT_FREE: u8 = 3;
const TANGENT_MIRROR: u8 = 4;
const TANGENT_SINE: u8 = 5;

fn tangent(name: Option<&str>) -> u8 {
    match name.unwrap_or("") {
        "CURVE_TANGENT_LINEAR" => TANGENT_LINEAR,
        "CURVE_TANGENT_FLAT" => TANGENT_FLAT,
        "CURVE_TANGENT_FREE" => TANGENT_FREE,
        "CURVE_TANGENT_MIRROR" => TANGENT_MIRROR,
        "CURVE_TANGENT_SINE" => TANGENT_SINE,
        _ => TANGENT_SPLINE,
    }
}

/// A piecewise cubic Hermite curve (VRF `PiecewiseCurve`).
#[derive(Clone, Debug, Default)]
pub struct Curve {
    x: Vec<f32>,
    y: Vec<f32>,
    slope_in: Vec<f32>,
    slope_out: Vec<f32>,
    domain_min: [f32; 2],
    domain_max: [f32; 2],
    looped: bool,
}

impl Curve {
    pub fn parse(v: Option<&Value>, looped: bool) -> Curve {
        let empty = Value::Null;
        let v = v.unwrap_or(&empty);
        let d2 = |k: &str| {
            let a = kv::v3(v, k, [0.0; 3]);
            [a[0], a[1]]
        };
        let splines = kv::array(v, "m_spline");
        let tangents = kv::array(v, "m_tangents");
        let n = splines.len().min(tangents.len());
        let mut c = Curve { domain_min: d2("m_vDomainMins"), domain_max: d2("m_vDomainMaxs"), looped, ..Curve::default() };
        let mut ins = Vec::with_capacity(n);
        let mut outs = Vec::with_capacity(n);
        for k in 0..n {
            let s = &splines[k];
            let mut x = kv::f(s, "x", 0.0);
            if k > 0 {
                x = x.max(c.x[k - 1] + 0.0001);
            }
            c.x.push(x);
            c.y.push(kv::f(s, "y", 0.0));
            c.slope_in.push(kv::f(s, "m_flSlopeIncoming", 0.0));
            c.slope_out.push(kv::f(s, "m_flSlopeOutgoing", 0.0));
            ins.push(tangent(kv::s(&tangents[k], "m_nIncomingTangent")));
            outs.push(tangent(kv::s(&tangents[k], "m_nOutgoingTangent")));
        }
        c.compute_slopes(&ins, &outs);
        c
    }

    /// A curve through `(x, y)` keys with spline tangents, for code-built definitions.
    #[cfg(test)]
    pub fn from_keys(keys: &[(f32, f32)], domain_min: [f32; 2], domain_max: [f32; 2]) -> Curve {
        let mut c = Curve { domain_min, domain_max, ..Curve::default() };
        for &(x, y) in keys {
            c.x.push(x);
            c.y.push(y);
            c.slope_in.push(0.0);
            c.slope_out.push(0.0);
        }
        let t = vec![TANGENT_SPLINE; keys.len()];
        c.compute_slopes(&t, &t);
        c
    }

    fn compute_slopes(&mut self, ins: &[u8], outs: &[u8]) {
        const STEEP: f32 = 1.60305;
        const SHALLOW: f32 = 0.0413377;
        let n = self.x.len();
        if n == 0 {
            return;
        }
        let last = n - 1;
        for i in 0..n {
            let (mut dxp, mut dyp, mut sp) = (0.0, 0.0, 0.0);
            let (mut dxn, mut dyn_, mut sn) = (0.0, 0.0, 0.0);
            let mut sc = 0.0;
            if i > 0 {
                dyp = self.y[i] - self.y[i - 1];
                dxp = self.x[i] - self.x[i - 1];
                sp = dyp / dxp;
            }
            if i < last {
                if i > 0 {
                    sc = (self.y[i + 1] - self.y[i - 1]) / (self.x[i + 1] - self.x[i - 1]);
                }
                dyn_ = self.y[i + 1] - self.y[i];
                dxn = self.x[i + 1] - self.x[i];
                sn = dyn_ / dxn;
            } else if i > 0 {
                sc = sp;
            }
            if i == 0 {
                sc = sn;
            }
            match ins[i] {
                TANGENT_LINEAR => self.slope_in[i] = sp,
                TANGENT_SPLINE => self.slope_in[i] = sc,
                TANGENT_FREE => {}
                TANGENT_SINE => self.slope_in[i] = (if dxp == 0.0 { 1.0 } else { 1.0 / dxp }) * (if dyp <= 0.0 { -STEEP } else { -SHALLOW }),
                _ => self.slope_in[i] = 0.0,
            }
            match outs[i] {
                TANGENT_LINEAR => self.slope_out[i] = sn,
                TANGENT_SPLINE => self.slope_out[i] = sc,
                TANGENT_FREE => {}
                TANGENT_MIRROR => self.slope_out[i] = self.slope_in[i],
                TANGENT_SINE => self.slope_out[i] = (if dxn == 0.0 { 1.0 } else { 1.0 / dxn }) * (if dyn_ <= 0.0 { SHALLOW } else { STEEP }),
                _ => self.slope_out[i] = 0.0,
            }
            if ins[i] == TANGENT_MIRROR {
                self.slope_in[i] = self.slope_out[i];
            }
            let _ = TANGENT_FLAT;
        }
    }

    pub fn evaluate(&self, value: f32) -> f32 {
        let n = self.x.len();
        if n < 2 {
            return 0.0;
        }
        let mut value = value;
        if self.looped && self.domain_max[0] != 0.0 {
            value = (value / self.domain_max[0]).rem_euclid(1.0) * self.domain_max[0];
        }
        // The first key after the first that is not below `value`, or the last.
        let (mut lo, mut hi) = (1usize, n - 1);
        while lo < hi {
            let mid = (lo + hi) >> 1;
            if self.x[mid] < value {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let (end, start) = (lo, lo - 1);
        let dx = self.x[end] - self.x[start];
        let mut t = value - self.x[start];
        if dx != 0.0 {
            t /= dx;
        }
        let t = sat(t);
        let (t2, t3) = (t * t, t * t * t);
        let y = self.y[start] * (2.0 * t3 - 3.0 * t2 + 1.0)
            + self.slope_out[start] * dx * (t3 - 2.0 * t2 + t)
            + self.slope_in[end] * dx * (t3 - t2)
            + self.y[end] * (3.0 * t2 - 2.0 * t3);
        y.max(self.domain_min[1]).min(self.domain_max[1])
    }

    pub fn end(&self) -> Option<(f32, f32)> {
        if self.x.len() < 2 { None } else { Some((self.domain_max[0], *self.y.last().unwrap_or(&0.0))) }
    }
}

/// A float input.
#[derive(Clone, Debug)]
pub enum NumIn {
    Lit(f32),
    Random { min: f32, max: f32, sign_flip: bool, varying: bool, bias: Option<(i32, f32)>, slot: i32 },
    CollectionAge(Mapping),
    EndCapAge(Mapping),
    ParticleAge(Mapping),
    ParticleAgeNorm(Mapping),
    ParticleFloat { field: i32, initial: bool, map: Mapping },
    ParticleVecComp { field: i32, comp: usize, initial: bool, map: Mapping },
    ParticleSpeed(Mapping),
    ParticleNumber { map: Mapping, normalized: bool },
    CpComponent { cp: i32, comp: usize, map: Mapping },
    CpDistance { a: i32, b: i32, map: Mapping },
    Named(Mapping),
    Lod(f32),
}

const RANDOM_SIGN_OFFSET: i32 = 37;

impl NumIn {
    pub fn parse_or(v: &Value, key: &str, default: f32, cx: &mut ParseCx) -> NumIn {
        match kv::get(v, key) {
            None => NumIn::Lit(default),
            Some(x) => NumIn::parse(x, cx),
        }
    }

    pub fn parse(v: &Value, cx: &mut ParseCx) -> NumIn {
        if !v.is_object() {
            return NumIn::Lit(kv::number(v).unwrap_or(0.0) as f32);
        }
        let ty = kv::s(v, "m_nType").unwrap_or("PF_TYPE_LITERAL");
        let map = || Mapping::parse(v);
        let comp = |k: &str| kv::i(v, k, 0).clamp(0, 2) as usize;
        match ty {
            "PF_TYPE_LITERAL" | "PF_TYPE_INVALID" => NumIn::Lit(kv::f(v, "m_flLiteralValue", 0.0)),
            "PF_TYPE_RANDOM_UNIFORM" | "PF_TYPE_RANDOM_BIASED" => {
                cx.random_inputs = true;
                cx.next_slot += 1;
                let biased = ty == "PF_TYPE_RANDOM_BIASED";
                NumIn::Random {
                    min: kv::f(v, "m_flRandomMin", 0.0),
                    max: kv::f(v, "m_flRandomMax", 1.0),
                    sign_flip: kv::b(v, "m_bHasRandomSignFlip", false),
                    varying: kv::s(v, "m_nRandomMode").is_some_and(|m| m == "PF_RANDOM_MODE_VARYING"),
                    bias: biased.then(|| (kv::enumv(v, "m_nBiasType", BIAS_TYPES, 0), kv::f(v, "m_flBiasParameter", 0.0))),
                    slot: cx.next_slot,
                }
            }
            "PF_TYPE_COLLECTION_AGE" => NumIn::CollectionAge(map()),
            "PF_TYPE_ENDCAP_AGE" => NumIn::EndCapAge(map()),
            "PF_TYPE_PARTICLE_AGE" => NumIn::ParticleAge(map()),
            "PF_TYPE_PARTICLE_AGE_NORMALIZED" => NumIn::ParticleAgeNorm(map()),
            "PF_TYPE_PARTICLE_FLOAT" | "PF_TYPE_PARTICLE_INITIAL_FLOAT" => {
                NumIn::ParticleFloat { field: kv::field(v, "m_nScalarAttribute", fields::RADIUS), initial: ty.contains("INITIAL"), map: map() }
            }
            "PF_TYPE_PARTICLE_VECTOR_COMPONENT" | "PF_TYPE_PARTICLE_INITIAL_VECTOR_COMPONENT" => NumIn::ParticleVecComp {
                field: kv::field(v, "m_nVectorAttribute", fields::COLOR),
                comp: comp("m_nVectorComponent"),
                initial: ty.contains("INITIAL"),
                map: map(),
            },
            "PF_TYPE_PARTICLE_SPEED" => NumIn::ParticleSpeed(map()),
            "PF_TYPE_PARTICLE_NUMBER" | "PF_TYPE_PARTICLE_NUMBER_NORMALIZED" => NumIn::ParticleNumber { map: map(), normalized: ty.ends_with("NORMALIZED") },
            "PF_TYPE_CONTROL_POINT_COMPONENT" => NumIn::CpComponent { cp: kv::i(v, "m_nControlPoint", 0), comp: comp("m_nVectorComponent"), map: map() },
            "PF_TYPE_CONTROL_POINT_DISTANCE" => NumIn::CpDistance { a: kv::i(v, "m_nControlPoint", 0), b: kv::i(v, "m_nControlPointCompare", 0), map: map() },
            "PF_TYPE_NAMED_VALUE" => NumIn::Named(map()),
            "PF_TYPE_PARTICLE_DETAIL_LEVEL" => NumIn::Lod(kv::f(v, "m_flLOD0", 0.0)),
            other => {
                cx.note(format!("float input {other} is not supported, using its literal value"));
                NumIn::Lit(kv::f(v, "m_flLiteralValue", 0.0))
            }
        }
    }

    pub fn literal(&self) -> Option<f32> {
        if let NumIn::Lit(x) = self { Some(*x) } else { None }
    }

    pub fn is_per_particle(&self) -> bool {
        matches!(
            self,
            NumIn::Random { .. } | NumIn::ParticleAge(_) | NumIn::ParticleAgeNorm(_) | NumIn::ParticleFloat { .. } | NumIn::ParticleVecComp { .. } | NumIn::ParticleSpeed(_) | NumIn::ParticleNumber { .. }
        )
    }

    fn shape_random(min: f32, max: f32, bias: Option<(i32, f32)>, draw: f32, sign: f32) -> f32 {
        let draw = match bias {
            Some((kind, param)) => bias_from_parameter(draw.min(1.0), param, kind),
            None => draw,
        };
        let v = clamp_either(lerp(min, max, draw), min, max);
        if sign < 0.5 { -v } else { v }
    }

    /// The value for the whole collection: per-particle inputs give 0, a random input the
    /// draw of particle id 0 (constant) or a fresh draw (varying).
    pub fn next(&self, st: &mut State) -> f32 {
        match self {
            NumIn::Lit(x) => *x,
            NumIn::Random { min, max, sign_flip, varying, bias, slot } => {
                if *varying {
                    let d = st.rnd.next();
                    let s = if *sign_flip { st.rnd.next() } else { 1.0 };
                    Self::shape_random(*min, *max, *bias, d, s)
                } else {
                    let seed = st.rnd.input_seed(*slot);
                    let d = st.rnd.for_particle(0, seed);
                    let s = if *sign_flip { st.rnd.for_particle(0, seed.wrapping_add(RANDOM_SIGN_OFFSET)) } else { 1.0 };
                    Self::shape_random(*min, *max, *bias, d, s)
                }
            }
            NumIn::CollectionAge(m) => m.apply(st.age),
            NumIn::EndCapAge(m) => m.apply(st.end_cap_age()),
            NumIn::CpComponent { cp, comp, map } => map.apply(st.cp(*cp).pos[*comp]),
            NumIn::CpDistance { a, b, map } => map.apply(length(sub(st.cp(*a).pos, st.cp(*b).pos))),
            NumIn::Named(m) => m.apply(0.0),
            NumIn::Lod(x) => *x,
            _ => 0.0,
        }
    }

    /// `next` with a collection age measured from `age` (the continuous emitter's rate).
    pub fn at_age(&self, st: &mut State, age: f32) -> f32 {
        match self {
            NumIn::CollectionAge(m) => m.apply(age),
            _ => self.next(st),
        }
    }

    /// The age from which the input holds its final value, and that value.
    pub fn steady(&self) -> Option<(f32, f32)> {
        match self {
            NumIn::Lit(x) => Some((0.0, *x)),
            NumIn::CollectionAge(m) => m.steady(),
            NumIn::Lod(x) => Some((0.0, *x)),
            _ => None,
        }
    }

    /// Per-particle values for `first..first + n` into `out`.
    pub fn fill(&self, st: &mut State, p: &Particles, first: usize, n: usize, out: &mut Vec<f32>) {
        out.clear();
        match self {
            NumIn::Random { min, max, sign_flip, varying, bias, slot } => {
                if *varying {
                    for _ in 0..n {
                        let d = st.rnd.next();
                        let s = if *sign_flip { st.rnd.next() } else { 1.0 };
                        out.push(Self::shape_random(*min, *max, *bias, d, s));
                    }
                } else {
                    let seed = st.rnd.input_seed(*slot);
                    let ids = &p.s[fields::PARTICLE_ID];
                    for i in first..first + n {
                        let id = ids[i] as i32;
                        let d = st.rnd.for_particle(id, seed);
                        let s = if *sign_flip { st.rnd.for_particle(id, seed.wrapping_add(RANDOM_SIGN_OFFSET)) } else { 1.0 };
                        out.push(Self::shape_random(*min, *max, *bias, d, s));
                    }
                }
            }
            NumIn::ParticleAge(m) => out.extend((first..first + n).map(|i| m.apply(p.age(i)))),
            NumIn::ParticleAgeNorm(m) => out.extend((first..first + n).map(|i| {
                let life = p.s[fields::LIFE][i];
                m.apply(if life <= 0.0 { 0.0 } else { p.age(i) / life })
            })),
            NumIn::ParticleFloat { field, initial, map } => out.extend((first..first + n).map(|i| {
                let x = p.scalar(*field, i, *initial);
                map.apply(if fields::is_angle(*field) { x.to_degrees() } else { x })
            })),
            NumIn::ParticleVecComp { field, comp, initial, map } => out.extend((first..first + n).map(|i| map.apply(p.vector(*field, i, *initial)[*comp]))),
            NumIn::ParticleSpeed(m) => {
                let dt = st.current_step;
                out.extend((first..first + n).map(|i| {
                    let d = length(sub(p.v[fields::POSITION][i], p.v[fields::PREV][i]));
                    m.apply(if dt > 0.0 { d / dt } else { 0.0 })
                }))
            }
            NumIn::ParticleNumber { map, normalized } => {
                let total = p.len.max(1) as f32;
                out.extend((first..first + n).map(|i| map.apply(if *normalized { i as f32 / total } else { i as f32 })))
            }
            _ => {
                let x = self.next(st);
                out.resize(n, x);
            }
        }
    }
}

/// A colour gradient (`m_Gradient`), stops at 0..1 with 0-255 colours.
#[derive(Clone, Debug, Default)]
pub struct Gradient {
    stops: Vec<(f32, Vec3)>,
}

impl Gradient {
    pub fn parse(v: Option<&Value>) -> Gradient {
        let mut stops: Vec<(f32, Vec3)> = v
            .map(|g| kv::array(g, "m_Stops"))
            .unwrap_or(&[])
            .iter()
            .map(|s| (kv::f(s, "m_flPosition", 0.0).clamp(0.0, 1.0), kv::v3(s, "m_Color", [0.0; 3])))
            .collect();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Gradient { stops }
    }

    pub fn evaluate(&self, t: f32) -> Vec3 {
        let s = &self.stops;
        if s.is_empty() {
            return [1.0; 3];
        }
        let k = |c: Vec3| [c[0] / 255.0, c[1] / 255.0, c[2] / 255.0];
        let idx = s.iter().rposition(|st| st.0 <= t);
        let Some(i) = idx else { return k(s[0].1) };
        if i == s.len() - 1 {
            return k(s[i].1);
        }
        let span = s[i + 1].0 - s[i].0;
        if span < 1e-5 {
            return k(s[i].1);
        }
        let b = lerpv(s[i].1, s[i + 1].1, (t - s[i].0) / span);
        k([b[0].trunc(), b[1].trunc(), b[2].trunc()])
    }
}

/// A vector input.
#[derive(Clone, Debug)]
pub enum VecIn {
    Lit(Vec3),
    Components(Box<[NumIn; 3]>),
    Interp { input: Box<NumIn>, i0: f32, i1: f32, o0: Vec3, o1: Vec3, clamp: bool, gradient: Option<Gradient> },
    CpValue { cp: i32, scale: Vec3 },
    CpDelta { cp: i32, delta: i32 },
    CpRelPos { cp: i32, pos: Vec3 },
    CpRelDir { cp: i32, dir: Vec3 },
    Random { min: Vec3, max: Vec3 },
    ParticleVector { field: i32, initial: bool },
    Velocity,
}

impl VecIn {
    pub fn parse_or(v: &Value, key: &str, default: Vec3, cx: &mut ParseCx) -> VecIn {
        match kv::get(v, key) {
            None => VecIn::Lit(default),
            Some(x) => VecIn::parse(x, cx),
        }
    }

    pub fn parse(v: &Value, cx: &mut ParseCx) -> VecIn {
        if let Some(s) = v.as_str() {
            if s == "pvec_color_white" {
                return VecIn::Lit([1.0; 3]);
            }
            if s == "pvec_color_black" {
                return VecIn::Lit([0.0; 3]);
            }
        }
        if !v.is_object() {
            return VecIn::Lit(kv::vec3_of(v).unwrap_or([0.0; 3]));
        }
        let ty = kv::s(v, "m_nType").unwrap_or("PVEC_TYPE_LITERAL");
        let cp = || kv::i(v, "m_nControlPoint", 0);
        let num = |k: &str, cx: &mut ParseCx| NumIn::parse_or(v, k, 0.0, cx);
        match ty {
            "PVEC_TYPE_LITERAL" => VecIn::Lit(kv::v3(v, "m_vLiteralValue", [0.0; 3])),
            "PVEC_TYPE_LITERAL_COLOR" => VecIn::Lit(kv::color(v, "m_LiteralColor", [0.0; 3])),
            "PVEC_TYPE_FLOAT_COMPONENTS" => VecIn::Components(Box::new([num("m_FloatComponentX", cx), num("m_FloatComponentY", cx), num("m_FloatComponentZ", cx)])),
            "PVEC_TYPE_FLOAT_INTERP_CLAMPED" | "PVEC_TYPE_FLOAT_INTERP_OPEN" | "PVEC_TYPE_FLOAT_INTERP_GRADIENT" => VecIn::Interp {
                input: Box::new(num("m_FloatInterp", cx)),
                i0: kv::f(v, "m_flInterpInput0", 0.0),
                i1: kv::f(v, "m_flInterpInput1", 1.0),
                o0: kv::v3(v, "m_vInterpOutput0", [0.0; 3]),
                o1: kv::v3(v, "m_vInterpOutput1", [1.0; 3]),
                clamp: ty.ends_with("CLAMPED"),
                gradient: ty.ends_with("GRADIENT").then(|| Gradient::parse(kv::get(v, "m_Gradient"))),
            },
            "PVEC_TYPE_CP_VALUE" => VecIn::CpValue { cp: cp(), scale: kv::v3(v, "m_vCPValueScale", [1.0; 3]) },
            "PVEC_TYPE_CP_DELTA" => VecIn::CpDelta { cp: cp(), delta: kv::i(v, "m_nDeltaControlPoint", 0) },
            "PVEC_TYPE_CP_RELATIVE_POSITION" => VecIn::CpRelPos { cp: cp(), pos: kv::v3(v, "m_vCPRelativePosition", [0.0; 3]) },
            "PVEC_TYPE_CP_RELATIVE_DIR" => VecIn::CpRelDir { cp: cp(), dir: kv::v3(v, "m_vCPRelativeDir", [1.0, 0.0, 0.0]) },
            "PVEC_TYPE_RANDOM_UNIFORM" => {
                cx.random_inputs = true;
                VecIn::Random { min: kv::v3(v, "m_vRandomMin", [0.0; 3]), max: kv::v3(v, "m_vRandomMax", [0.0; 3]) }
            }
            "PVEC_TYPE_PARTICLE_VECTOR" | "PVEC_TYPE_PARTICLE_INITIAL_VECTOR" => {
                VecIn::ParticleVector { field: kv::field(v, "m_nVectorAttribute", fields::COLOR), initial: ty.contains("INITIAL") }
            }
            "PVEC_TYPE_PARTICLE_VELOCITY" => VecIn::Velocity,
            other => {
                cx.note(format!("vector input {other} is not supported, using its literal value"));
                VecIn::Lit(kv::v3(v, "m_vLiteralValue", [0.0; 3]))
            }
        }
    }

    pub fn is_per_particle(&self) -> bool {
        match self {
            VecIn::Random { .. } | VecIn::ParticleVector { .. } | VecIn::Velocity => true,
            VecIn::Components(c) => c.iter().any(NumIn::is_per_particle),
            VecIn::Interp { input, .. } => input.is_per_particle(),
            _ => false,
        }
    }

    fn interp(x: f32, i0: f32, i1: f32, o0: Vec3, o1: Vec3, clamp: bool, gradient: &Option<Gradient>) -> Vec3 {
        let mut t = (x - i0) / (i1 - i0);
        if let Some(g) = gradient {
            return g.evaluate(t);
        }
        if clamp {
            t = sat(t);
        }
        lerpv(o0, o1, t)
    }

    pub fn next(&self, st: &mut State) -> Vec3 {
        match self {
            VecIn::Lit(x) => *x,
            VecIn::Components(c) => [c[0].next(st), c[1].next(st), c[2].next(st)],
            VecIn::Interp { input, i0, i1, o0, o1, clamp, gradient } => {
                let x = input.next(st);
                Self::interp(x, *i0, *i1, *o0, *o1, *clamp, gradient)
            }
            VecIn::CpValue { cp, scale } => mulv(st.cp(*cp).pos, *scale),
            VecIn::CpDelta { cp, delta } => sub(st.cp(*cp).pos, st.cp(*delta).pos),
            VecIn::CpRelPos { cp, pos } => st.cp(*cp).xf().point(*pos),
            VecIn::CpRelDir { cp, dir } => st.cp(*cp).rotation().vector(*dir),
            VecIn::Random { min, max } => {
                let r = [st.rnd.next(), st.rnd.next(), st.rnd.next()];
                [lerp(min[0], max[0], r[0]), lerp(min[1], max[1], r[1]), lerp(min[2], max[2], r[2])]
            }
            _ => [0.0; 3],
        }
    }

    /// Per-particle values, with scratch for the component inputs.
    pub fn fill(&self, st: &mut State, p: &Particles, first: usize, n: usize, out: &mut Vec<Vec3>, scratch: &mut [Vec<f32>; 3]) {
        out.clear();
        match self {
            VecIn::Random { min, max } => {
                for _ in 0..n {
                    let r = [st.rnd.next(), st.rnd.next(), st.rnd.next()];
                    out.push([lerp(min[0], max[0], r[0]), lerp(min[1], max[1], r[1]), lerp(min[2], max[2], r[2])]);
                }
            }
            VecIn::ParticleVector { field, initial } => out.extend((first..first + n).map(|i| p.vector(*field, i, *initial))),
            VecIn::Velocity => {
                let dt = st.current_step;
                out.extend((first..first + n).map(|i| {
                    let d = sub(p.v[fields::POSITION][i], p.v[fields::PREV][i]);
                    if dt > 0.0 { [d[0] / dt, d[1] / dt, d[2] / dt] } else { [0.0; 3] }
                }));
            }
            VecIn::Components(c) if self.is_per_particle() => {
                let [a, b, d] = scratch;
                c[0].fill(st, p, first, n, a);
                c[1].fill(st, p, first, n, b);
                c[2].fill(st, p, first, n, d);
                out.extend((0..n).map(|k| [a[k], b[k], d[k]]));
            }
            VecIn::Interp { input, i0, i1, o0, o1, clamp, gradient } if input.is_per_particle() => {
                input.fill(st, p, first, n, &mut scratch[0]);
                out.extend(scratch[0].iter().map(|&x| Self::interp(x, *i0, *i1, *o0, *o1, *clamp, gradient)));
            }
            _ => {
                let x = self.next(st);
                out.resize(n, x);
            }
        }
    }
}

/// A transform input: a control point's frame, with or without its orientation.
#[derive(Clone, Copy, Debug)]
pub enum TransIn {
    Cp { cp: i32, orient: bool },
    Identity,
}

impl TransIn {
    pub fn parse_or(v: &Value, key: &str, default: TransIn, cx: &mut ParseCx) -> TransIn {
        let Some(x) = kv::get(v, key) else { return default };
        if !x.is_object() {
            return TransIn::Cp { cp: kv::number(x).unwrap_or(0.0).clamp(0.0, 63.0) as i32, orient: true };
        }
        let ty = kv::s(x, "m_nType").unwrap_or("PT_TYPE_CONTROL_POINT");
        match ty {
            "PT_TYPE_CONTROL_POINT" => TransIn::Cp { cp: kv::i(x, "m_nControlPoint", 0), orient: kv::b(x, "m_bUseOrientation", true) },
            "PT_TYPE_CONTROL_POINT_RANGE" => {
                cx.note("transform input PT_TYPE_CONTROL_POINT_RANGE reads its first control point".into());
                TransIn::Cp { cp: kv::i(x, "m_nControlPoint", 0), orient: kv::b(x, "m_bUseOrientation", true) }
            }
            "PT_TYPE_INVALID" => TransIn::Identity,
            other => {
                cx.note(format!("transform input {other} is not supported, using identity"));
                TransIn::Identity
            }
        }
    }

    pub fn now(&self, st: &State) -> Xf {
        match self {
            TransIn::Cp { cp, orient: true } => st.cp(*cp).xf(),
            TransIn::Cp { cp, orient: false } => Xf { o: st.cp(*cp).pos, ..Xf::IDENTITY },
            TransIn::Identity => Xf::IDENTITY,
        }
    }

    pub fn at(&self, st: &State, time: f32) -> Xf {
        match self {
            TransIn::Cp { cp, orient } => st.cp_xf_at(*cp, time, *orient),
            TransIn::Identity => Xf::IDENTITY,
        }
    }
}
