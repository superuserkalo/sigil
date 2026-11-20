//! `C_OP_RenderSprites` and `C_OP_RenderTrails` on the CPU side: the per-renderer parameters
//! and the instance each live particle draws as, ported from Grimoire's `renderSprites.ts`,
//! `renderTrails.ts`, `functionRenderer.ts`, `colorBlend.ts` and `spritecardRenderer.ts`
//! (MIT). The card bases, sheet frames, colour blend and size clamps are worked out here in
//! Source space and handed to the GPU in world space; the shader only builds the corners,
//! samples and blends.

use serde_json::Value;

use super::bundle::Sheet;
use super::fields::{self as fl, Particles};
use super::kv;
use super::providers::{NumIn, ParseCx, VecIn};
use super::smath::{euler_axes, rotate_about, safe_normalize, sat, smoothstep, srgb_to_linear};
use super::state::State;
use crate::math::{Mat4, Vec3, add, cross, dot, length, normalize, scale, sub};

/// One card on the GPU (seven `vec4`s, world space).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    /// Centre, half-width along `axis_u`.
    pub pos_radius: [f32; 4],
    /// Unit axis; `w` is the basis mode: 0 camera facing, 1 camera facing about Source up,
    /// 2 the axes given.
    pub axis_u: [f32; 4],
    /// Unit axis; `w` is the roll in radians for the camera modes.
    pub axis_v: [f32; 4],
    /// Vertex colour (linear when gamma corrected) and alpha.
    pub color: [f32; 4],
    pub uv_a: [f32; 4],
    pub uv_b: [f32; 4],
    /// Frame blend, yaw (camera modes), half-height along `axis_v` (0 = the half-width), unused.
    pub misc: [f32; 4],
}

pub const BLEND_MOD2X: i32 = 5;
pub const BLEND_LIGHTEN: i32 = 6;

const BLEND_MODES: &[(&str, i32)] = &[
    ("PARTICLE_OUTPUT_BLEND_MODE_ALPHA", 0),
    ("PARTICLE_OUTPUT_BLEND_MODE_ADD", 1),
    ("PARTICLE_OUTPUT_BLEND_MODE_BLEND_ADD", 2),
    ("PARTICLE_OUTPUT_BLEND_MODE_HALF_BLEND_ADD", 3),
    ("PARTICLE_OUTPUT_BLEND_MODE_NEG_HALF_BLEND_ADD", 4),
    ("PARTICLE_OUTPUT_BLEND_MODE_MOD2X", 5),
    ("PARTICLE_OUTPUT_BLEND_MODE_LIGHTEN", 6),
];
const ORIENTATIONS: &[(&str, i32)] = &[
    ("PARTICLE_ORIENTATION_SCREEN_ALIGNED", 0),
    ("PARTICLE_ORIENTATION_SCREEN_Z_ALIGNED", 1),
    ("PARTICLE_ORIENTATION_WORLD_Z_ALIGNED", 2),
    ("PARTICLE_ORIENTATION_ALIGN_TO_PARTICLE_NORMAL", 3),
    ("PARTICLE_ORIENTATION_SCREENALIGN_TO_PARTICLE_NORMAL", 4),
    ("PARTICLE_ORIENTATION_FULL_3AXIS_ROTATION", 5),
];
const ANIMATION_TYPES: &[(&str, i32)] = &[("ANIMATION_TYPE_FIXED_RATE", 0), ("ANIMATION_TYPE_FIT_LIFETIME", 1), ("ANIMATION_TYPE_MANUAL_FRAMES", 2)];
const COLOR_BLENDS: &[(&str, i32)] = &[
    ("PARTICLE_COLOR_BLEND_MULTIPLY", 0),
    ("PARTICLE_COLOR_BLEND_MULTIPLY2X", 1),
    ("PARTICLE_COLOR_BLEND_DIVIDE", 2),
    ("PARTICLE_COLOR_BLEND_ADD", 3),
    ("PARTICLE_COLOR_BLEND_SUBTRACT", 4),
    ("PARTICLE_COLOR_BLEND_MOD2X", 5),
    ("PARTICLE_COLOR_BLEND_SCREEN", 6),
    ("PARTICLE_COLOR_BLEND_MAX", 7),
    ("PARTICLE_COLOR_BLEND_MIN", 8),
    ("PARTICLE_COLOR_BLEND_REPLACE", 9),
    ("PARTICLE_COLOR_BLEND_AVERAGE", 10),
    ("PARTICLE_COLOR_BLEND_NEGATE", 11),
    ("PARTICLE_COLOR_BLEND_LUMINANCE", 12),
];
const SORTING: &[(&str, i32)] = &[("PARTICLE_SORTING_NEAREST", 0), ("PARTICLE_SORTING_CREATION_TIME", 1)];

pub const DEFAULT_SPRITE: &str = "materials/particle/base_sprite.vtex";
pub const DEFAULT_TRAIL: &str = "materials/particle/base_trail.vtex";

#[derive(Clone, Debug)]
pub struct Trail {
    length_scale: f32,
    fade_in: f32,
    min_length: f32,
    max_length: f32,
    ratio: f32,
    forward_shift: f32,
    head_alpha: f32,
    tail_alpha: f32,
}

#[derive(Clone, Debug)]
pub struct Renderer {
    pub texture: String,
    pub blend: i32,
    pub depth_test: bool,
    pub depth_reverse: bool,
    trail: Option<Trail>,
    radius_scale: NumIn,
    alpha_scale: NumIn,
    roll_scale: NumIn,
    color_scale: VecIn,
    color_blend: i32,
    alpha2: i32,
    sort_nearest: bool,
    camera_bias: f32,
    orientation: i32,
    yaw_with_normal: bool,
    seq_override: NumIn,
    raw_sequences: bool,
    min_size: f32,
    max_size: f32,
    start_fade_size: f32,
    end_fade_size: f32,
    start_fade_dot: f32,
    end_fade_dot: f32,
    anim_type: i32,
    anim_rate: f32,
    anim_fps: bool,
    pub blend_frames: bool,
    pub overbright: NumIn,
    add_self: NumIn,
    pub diffuse: f32,
    pub self_illum: NumIn,
    to_zero: NumIn,
    to_one: NumIn,
    gamma: bool,
    center: [f32; 2],
}

/// The camera and placement a frame is built for.
pub struct View {
    /// Camera position in Source space.
    pub camera: Vec3,
    pub to_world: Mat4,
    /// World units per Source unit.
    pub world_scale: f32,
}

impl Renderer {
    pub fn parse(class: &str, v: &Value, behavior: i32, cx: &mut ParseCx) -> Option<Renderer> {
        let trail = match class {
            "C_OP_RenderSprites" => None,
            "C_OP_RenderTrails" => Some(Trail {
                length_scale: kv::f(v, "m_flLengthScale", 1.0),
                fade_in: kv::f(v, "m_flLengthFadeInTime", 0.0),
                min_length: kv::f(v, "m_flMinLength", 0.0),
                max_length: kv::f(v, "m_flMaxLength", 2000.0),
                ratio: kv::f(v, "m_flConstrainRadiusToLengthRatio", 1.0),
                forward_shift: kv::f(v, "m_flForwardShift", 0.0),
                head_alpha: kv::f(v, "m_flHeadAlphaScale", 1.0),
                tail_alpha: kv::f(v, "m_flTailAlphaScale", 1.0),
            }),
            _ => return None,
        };
        let num = |k: &str, d: f32, cx: &mut ParseCx| NumIn::parse_or(v, k, d, cx);
        let default_texture = if trail.is_some() { DEFAULT_TRAIL } else { DEFAULT_SPRITE };
        let mut texture = None;
        let mut layers = 0;
        for layer in kv::array(v, "m_vecTexturesInput") {
            if !kv::b(layer, "m_bEnabled", true) {
                continue;
            }
            let ty = kv::s(layer, "m_nTextureType").unwrap_or("SPRITECARD_TEXTURE_DIFFUSE");
            if ty != "SPRITECARD_TEXTURE_DIFFUSE" && ty != "SPRITECARD_TEXTURE_ZOOM" {
                if ty != "SPRITECARD_TEXTURE_ANIMMOTIONVEC" && ty != "SPRITECARD_TEXTURE_NORMALMAP" {
                    cx.note(format!("{class} texture layer {ty} is not drawn"));
                }
                continue;
            }
            layers += 1;
            if texture.is_none() {
                texture = Some(kv::s(layer, "m_hTexture").map_or_else(|| default_texture.to_string(), |t| t.trim_end_matches("_c").to_ascii_lowercase()));
            }
        }
        if layers > 1 {
            cx.note(format!("{class} draws only the first of its {layers} texture layers"));
        }
        let mut anim_type = kv::enumv(v, "m_nAnimationType", ANIMATION_TYPES, 0);
        if behavior < 10 && trail.is_none() {
            anim_type = 0;
        }
        let blend = kv::enumv(v, "m_nOutputBlendMode", BLEND_MODES, 0);
        if kv::b(v, "m_bRefract", false) {
            cx.note(format!("{class} refraction is not drawn"));
        }
        if kv::enumv(v, "m_nFeatheringMode", &[("PARTICLE_DEPTH_FEATHERING_OFF", 0), ("PARTICLE_DEPTH_FEATHERING_ON_OPTIONAL", 1), ("PARTICLE_DEPTH_FEATHERING_ON_REQUIRED", 2)], 0) != 0 {
            cx.note(format!("{class} depth feathering is not applied"));
        }
        let disable_z = kv::b(v, "m_bDisableZBuffering", false);
        Some(Renderer {
            texture: texture.unwrap_or_else(|| default_texture.to_string()),
            blend,
            depth_test: blend == BLEND_LIGHTEN || !disable_z,
            depth_reverse: blend != BLEND_LIGHTEN && !disable_z && kv::b(v, "m_bReverseZBuffering", false),
            radius_scale: num("m_flRadiusScale", 1.0, cx),
            alpha_scale: num("m_flAlphaScale", 1.0, cx),
            roll_scale: num("m_flRollScale", 1.0, cx),
            color_scale: VecIn::parse_or(v, "m_vecColorScale", [1.0; 3], cx),
            color_blend: kv::enumv(v, "m_nColorBlendType", COLOR_BLENDS, 0),
            alpha2: kv::field(v, "m_nAlpha2Field", fl::ALPHA2),
            sort_nearest: kv::enumv(v, "m_nSortMethod", SORTING, 0) == 0,
            camera_bias: kv::get(v, "VisibilityInputs").map_or(0.0, |vis| kv::f(vis, "m_flCameraBias", 0.0)),
            orientation: kv::enumv(v, "m_nOrientationType", ORIENTATIONS, 0).clamp(0, 5),
            yaw_with_normal: kv::b(v, "m_bUseYawWithNormalAligned", false),
            seq_override: num("m_nSequenceOverride", -1.0, cx),
            raw_sequences: kv::b(v, "m_bSequenceNumbersAreRawSequenceIndices", false),
            min_size: kv::f(v, "m_flMinSize", 0.0),
            max_size: kv::f(v, "m_flMaxSize", 5000.0),
            start_fade_size: kv::f(v, "m_flStartFadeSize", 1e8),
            end_fade_size: kv::f(v, "m_flEndFadeSize", 2e8),
            start_fade_dot: kv::f(v, "m_flStartFadeDot", 1.0),
            end_fade_dot: kv::f(v, "m_flEndFadeDot", 2.0),
            anim_type,
            anim_rate: kv::f(v, "m_flAnimationRate", 0.1),
            anim_fps: kv::b(v, "m_bAnimateInFPS", false),
            blend_frames: kv::b(v, "m_bBlendFramesSeq0", true),
            overbright: num("m_flOverbrightFactor", 1.0, cx),
            add_self: num("m_flAddSelfAmount", 0.0, cx),
            diffuse: kv::f(v, "m_flDiffuseAmount", 1.0),
            self_illum: num("m_flSelfIllumAmount", 0.0, cx),
            to_zero: num("m_flSourceAlphaValueToMapToZero", 0.0, cx),
            to_one: num("m_flSourceAlphaValueToMapToOne", 1.0, cx),
            gamma: kv::b(v, "m_bGammaCorrectVertexColors", true),
            center: [kv::f(v, "m_flCenterXOffset", 0.0), kv::f(v, "m_flCenterYOffset", 0.0)],
            trail,
        })
    }

    /// The per-draw shading constants: overbright, add-self multiplier, self-illum, and the
    /// alpha remap (scale, offset).
    pub fn shading(&self, st: &mut State) -> [f32; 5] {
        let add_self = match self.add_self.literal() {
            Some(x) if x <= 0.0 => 1.0,
            _ => 1.0 + self.add_self.next(st),
        };
        let zero = self.to_zero.next(st);
        let one = self.to_one.next(st);
        [self.overbright.next(st), add_self, self.self_illum.next(st), 1.0 / (one - zero), -zero]
    }

    /// Appends this renderer's cards for `p` to `out`. `order` and `dist` are reused buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn build(&self, st: &mut State, p: &Particles, view: &View, sheet: Option<&Sheet>, out: &mut Vec<Instance>, order: &mut Vec<u32>, dist: &mut Vec<f32>) {
        let n = p.len;
        if n == 0 {
            return;
        }
        let radius_scale = self.radius_scale.next(st);
        let alpha_scale = self.alpha_scale.next(st);
        let roll_scale = self.roll_scale.next(st);
        if radius_scale == 0.0 || alpha_scale == 0.0 {
            return;
        }
        let color_scale = convert_color_scale(self.color_blend, self.color_scale.next(st));
        let seq_override = {
            let o = self.seq_override.next(st).trunc() as i32;
            if o == -1 { -1 } else { sheet.map_or(-1, |s| s.index_of(o).unwrap_or(0) as i32) }
        };
        order.clear();
        order.extend((0..n as u32).rev());
        if self.sort_nearest && n > 1 {
            dist.clear();
            dist.extend((0..n).map(|i| {
                let d = sub(p.v[fl::POSITION][i], view.camera);
                dot(d, d)
            }));
            order.sort_unstable_by(|&a, &b| dist[b as usize].total_cmp(&dist[a as usize]).then(a.cmp(&b)));
        }
        let mode_normal = self.orientation == 3 || self.orientation == 4;
        let zero_yaw = mode_normal && !self.yaw_with_normal;
        let alpha2 = self.alpha2;
        let ws = view.world_scale;
        let world_dir = |v: Vec3| {
            let w = view.to_world.vector(v);
            let l = length(w);
            if l > 1e-12 { scale(w, 1.0 / l) } else { [0.0; 3] }
        };
        let step = st.current_step.max(1e-6);
        for &k in order.iter() {
            let i = k as usize;
            let a2 = if alpha2 >= 0 && alpha2 != fl::NONE as i32 && fl::is_scalar(alpha2) { p.s[alpha2 as usize][i] } else { 1.0 };
            let mut color = blend_color(self.color_blend, p.v[fl::COLOR][i], p.s[fl::ALPHA][i] * a2, color_scale, alpha_scale);
            if color[3] < 1.0 / 255.0 {
                continue;
            }
            let pos = p.v[fl::POSITION][i];
            let mut center = if self.camera_bias == 0.0 { pos } else { add(pos, scale(safe_normalize(sub(view.camera, pos)), self.camera_bias)) };
            let cam_dist = length(sub(view.camera, center));
            let mut radius = p.s[fl::RADIUS][i] * radius_scale;
            radius = radius.max(self.min_size * cam_dist).min(self.max_size * cam_dist);
            if radius > self.start_fade_size * cam_dist {
                let f = 1.0 - smoothstep(self.start_fade_size * cam_dist, self.end_fade_size * cam_dist, radius);
                color[0] *= f;
                color[1] *= f;
                color[2] *= f;
            }
            let roll = p.s[fl::ROLL][i] * roll_scale;
            let yaw = if zero_yaw { 0.0 } else { p.s[fl::YAW][i] };
            let normal = p.v[fl::NORMAL][i];
            let mut half_v = 0.0;
            let (mode, u, v) = if let Some(t) = &self.trail {
                let d = sub(p.v[fl::PREV][i], pos);
                let dlen = length(d);
                if dlen <= 1e-6 {
                    continue;
                }
                let dir = scale(d, 1.0 / dlen);
                let mut len = t.length_scale * p.s[fl::TRAIL_LENGTH][i] * dlen / step;
                if t.fade_in > 0.0 {
                    len *= (p.age(i) / t.fade_in).min(1.0);
                }
                len = len.clamp(t.min_length, t.max_length);
                radius = radius.min(t.ratio * len);
                center = add(pos, scale(dir, len * (0.5 - t.forward_shift)));
                let to_cam = sub(view.camera, center);
                let side = normalize(cross(dir, to_cam));
                color[3] *= 0.5 * (t.head_alpha + t.tail_alpha);
                half_v = len * 0.5;
                (2.0, side, dir)
            } else {
                match self.orientation {
                    0 => (0.0, [0.0; 3], [0.0; 3]),
                    1 => (1.0, [0.0; 3], [0.0; 3]),
                    2 => quad_basis([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], roll, yaw),
                    3 => {
                        let r = if normal[2].abs() > 0.1 { [0.0, -1.0, 0.0] } else { [0.0, 0.0, 1.0] };
                        let up = normalize(cross(r, normal));
                        quad_basis(cross(up, normal), up, roll, yaw)
                    }
                    4 => {
                        // The camera forward is the direction to the card for this mode.
                        let nn = normalize(normal);
                        let fwd = safe_normalize(sub(center, view.camera));
                        let w = cross(nn, fwd);
                        if dot(w, w) < 1e-12 { (0.0, [0.0; 3], [0.0; 3]) } else { quad_basis(nn, normalize(w), roll, yaw) }
                    }
                    _ => {
                        let [x, y, _] = euler_axes(p.s[fl::PITCH][i], yaw, roll);
                        (2.0, x, y)
                    }
                }
            };
            if mode_normal && mode == 2.0 {
                let to_cam = safe_normalize(sub(view.camera, center));
                let fade = 1.0 - smoothstep(self.start_fade_dot, self.end_fade_dot, dot(normal, to_cam).abs());
                color[3] *= fade;
            }
            if color[3] < 1.0 / 255.0 {
                continue;
            }
            if self.gamma {
                color = [srgb_to_linear(color[0]), srgb_to_linear(color[1]), srgb_to_linear(color[2]), color[3]];
            }
            let (uv_a, uv_b, blend) = match sheet {
                Some(sh) if !sh.sequences.is_empty() => {
                    let seq = if seq_override >= 0 { seq_override as usize } else { self.sequence_lane(sh, p.s[fl::SEQUENCE][i]) };
                    let age = match self.anim_type {
                        1 => {
                            let life = p.s[fl::LIFE][i];
                            if life > 0.0 { p.age(i) / life } else { 0.0 }
                        }
                        2 => p.s[fl::MANUAL_FRAME][i],
                        _ => p.age(i),
                    };
                    let rate = if self.anim_type == 2 { 1.0 } else { self.anim_rate };
                    let (a, b, t) = sh.frames(seq, age * rate, self.anim_fps);
                    (a, b, if self.blend_frames { t } else { 0.0 })
                }
                _ => ([0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, 1.0], 0.0),
            };
            let wc = view.to_world.point(center);
            let wcenter = if self.center != [0.0; 2] && mode == 2.0 { add(add(wc, scale(world_dir(u), self.center[0] * radius * ws)), scale(world_dir(v), self.center[1] * radius * ws)) } else { wc };
            let (wu, wv) = if mode == 2.0 { (world_dir(u), world_dir(v)) } else { ([0.0; 3], [0.0; 3]) };
            out.push(Instance {
                pos_radius: [wcenter[0], wcenter[1], wcenter[2], radius * ws],
                axis_u: [wu[0], wu[1], wu[2], mode],
                axis_v: [wv[0], wv[1], wv[2], roll],
                color,
                uv_a,
                uv_b,
                misc: [blend, yaw, half_v * ws, 0.0],
            });
        }
    }

    fn sequence_lane(&self, sheet: &Sheet, number: f32) -> usize {
        let n = number.round_ties_even() as i32;
        if self.raw_sequences {
            return (n.max(0) as usize).min(sheet.sequences.len() - 1);
        }
        sheet.index_of(n).unwrap_or(0)
    }
}

/// VRF `quadBasis`: rolls the card axes, then yaws the right axis about the up axis.
fn quad_basis(r: Vec3, u: Vec3, roll: f32, yaw: f32) -> (f32, Vec3, Vec3) {
    let (s, c) = roll.sin_cos();
    let mut right = add(scale(r, c), scale(u, s));
    let up = sub(scale(u, c), scale(r, s));
    if yaw != 0.0 && dot(up, up) > 1e-12 {
        right = rotate_about(right, normalize(up), yaw);
    }
    (2.0, right, up)
}

fn convert_color_scale(kind: i32, s: Vec3) -> Vec3 {
    match kind {
        1 | 5 => scale(s, 2.0),
        3 | 4 | 7 | 8 | 9 | 10 => scale(s, 255.0),
        6 => [1.0 - s[0], 1.0 - s[1], 1.0 - s[2]],
        _ => s,
    }
}

/// The particle colour through the renderer's colour scale, in bytes and back (VRF
/// `ApplyColorBlend`).
fn blend_color(kind: i32, c: Vec3, alpha: f32, s: Vec3, alpha_scale: f32) -> [f32; 4] {
    let bytes = [sat(c[0]) * 255.0, sat(c[1]) * 255.0, sat(c[2]) * 255.0];
    let a = sat(alpha) * 255.0;
    let lum = s[0] * 0.2125 + s[1] * 0.7154 + s[2] * 0.0721;
    let mut out = [0.0; 4];
    for k in 0..3 {
        let (x, y) = (bytes[k], s[k]);
        let v = match kind {
            2 => {
                let q = x / y;
                if q.is_finite() { q } else { 0.0 }
            }
            3 => x + y,
            4 => x - y,
            6 => 255.0 - (255.0 - x) * y,
            7 => x.max(y),
            8 => x.min(y),
            9 => y,
            10 => (x + y) * 0.5,
            11 => 255.0 - x * y,
            12 => lum * x,
            _ => x * y,
        };
        out[k] = v.clamp(0.0, 255.0) / 255.0;
    }
    out[3] = (a * alpha_scale).clamp(0.0, 255.0) / 255.0;
    out
}
