//! Materials: the glTF PBR channels plus the `extras.morphic` Source 2 payload vpkmerge
//! writes (schema 2 to 4, `docs/hero-glb-material-data.md` in vpkmerge). The rules follow
//! Grimoire's viewer (MIT, `src/lib/deadlockMaterial.ts`, `source2NprMaterial.ts`,
//! `source2ColorCorrection.ts`, `citadelNprOutline.ts`):
//!
//! - `pbr.vfx` with `F_USE_NPR_LIGHTING` (and not unlit) takes the Citadel NPR lighting.
//! - Its normal map's red and green are a hemi-octahedral normal (R+G, R-G), green down.
//! - Albedo: color correction (contrast about the texture's reflectivity, saturation with
//!   squared Rec. 709 weights, brightness), then the authored `g_vColorTint1`, decoded from
//!   sRGB once, through the tint mask's red channel. The exporter copied the raw tint into the
//!   base color factor; a factor that matches it is removed so it is not applied twice.
//! - Metalness is the MR texture's blue, roughness its green (the exporter's ORM packing).
//! - Rim mask: the tint mask's green. Outline growth: `g_tNprOutlineMask` red times 0.1 game
//!   units. `F_SOLID_COLOR_OUTLINE` (enemy team color) and see-through materials get no hull.
//! - Self-illumination: `F_SELF_ILLUM` with the emissive texture as mask, gated on its scale.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlphaMode {
    Opaque,
    Mask(f32),
    Blend,
    Additive,
}

/// A texture binding: a glTF texture index and the UV set it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub texture: usize,
    pub uv: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Morphic {
    pub schema: i64,
    pub shader: String,
    pub blend_mode: String,
    pub flags: HashSet<String>,
    pub ints: HashMap<String, i64>,
    pub floats: HashMap<String, f32>,
    pub vectors: HashMap<String, [f32; 4]>,
    pub textures: HashMap<String, usize>,
    pub dynamic: HashSet<String>,
    pub dynamic_textures: HashSet<String>,
    pub reflectivity: Option<[f32; 3]>,
}

impl Morphic {
    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
    pub fn float(&self, names: &[&str], default: f32) -> f32 {
        names.iter().find_map(|n| self.floats.get(*n).copied().or_else(|| self.vectors.get(*n).map(|v| v[0]))).unwrap_or(default)
    }
    pub fn is_pbr(&self) -> bool {
        self.shader.eq_ignore_ascii_case("pbr.vfx")
    }

    fn parse(v: &serde_json::Value) -> Morphic {
        let mut m = Morphic {
            schema: v.get("schema_version").and_then(|x| x.as_i64()).unwrap_or(0),
            shader: v.get("shader").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            blend_mode: v.get("blend_mode").map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).unwrap_or_default(),
            ..Default::default()
        };
        let num = |x: &serde_json::Value| -> Option<f64> { x.as_f64().or_else(|| x.as_array().and_then(|a| a.first()).and_then(|f| f.as_f64())).or_else(|| x.as_bool().map(|b| if b { 1.0 } else { 0.0 })) };
        for key in ["flags", "ints"] {
            if let Some(map) = v.get(key).and_then(|x| x.as_object()) {
                for (k, val) in map {
                    let n = num(val).unwrap_or(0.0);
                    m.ints.insert(k.clone(), n as i64);
                    if k.starts_with("F_") && n != 0.0 {
                        m.flags.insert(k.clone());
                    }
                }
            }
        }
        if let Some(map) = v.get("floats").and_then(|x| x.as_object()) {
            for (k, val) in map {
                if let Some(n) = num(val) {
                    m.floats.insert(k.clone(), n as f32);
                }
            }
        }
        if let Some(map) = v.get("vectors").and_then(|x| x.as_object()) {
            for (k, val) in map {
                if let Some(a) = val.as_array() {
                    let mut out = [0.0f32, 0.0, 0.0, 1.0];
                    for (i, f) in a.iter().take(4).enumerate() {
                        out[i] = f.as_f64().unwrap_or(0.0) as f32;
                    }
                    m.vectors.insert(k.clone(), out);
                }
            }
        }
        if let Some(map) = v.get("textures").and_then(|x| x.as_object()) {
            for (k, val) in map {
                let index = val.as_u64().or_else(|| val.get("index").and_then(|i| i.as_u64()));
                if let Some(i) = index {
                    m.textures.insert(k.clone(), i as usize);
                }
            }
        }
        for (key, set) in [("dynamic_params", &mut m.dynamic), ("dynamic_texture_params", &mut m.dynamic_textures)] {
            if let Some(map) = v.get(key).and_then(|x| x.as_object()) {
                set.extend(map.keys().cloned());
            }
        }
        m.reflectivity = v
            .get("texture_reflectivity")
            .and_then(|r| r.get("g_tColor"))
            .or_else(|| v.get("preview_albedo_average"))
            .and_then(|a| a.as_array())
            .filter(|a| a.len() >= 3)
            .map(|a| [a[0].as_f64().unwrap_or(1.0) as f32, a[1].as_f64().unwrap_or(1.0) as f32, a[2].as_f64().unwrap_or(1.0) as f32]);
        m
    }
}

#[derive(Clone, Debug)]
pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub base_tex: Option<Slot>,
    pub normal_tex: Option<Slot>,
    pub normal_scale: f32,
    pub mr_tex: Option<Slot>,
    pub metallic: f32,
    pub roughness: f32,
    pub occlusion_tex: Option<Slot>,
    pub occlusion_strength: f32,
    pub emissive_tex: Option<Slot>,
    pub emissive: [f32; 3],
    pub emissive_strength: f32,
    pub alpha: AlphaMode,
    pub double_sided: bool,
    pub unlit: bool,
    pub transmission: f32,
    pub morphic: Option<Morphic>,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            name: String::new(),
            base_color: [1.0; 4],
            base_tex: None,
            normal_tex: None,
            normal_scale: 1.0,
            mr_tex: None,
            metallic: 0.0,
            roughness: 0.8,
            occlusion_tex: None,
            occlusion_strength: 1.0,
            emissive_tex: None,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha: AlphaMode::Opaque,
            double_sided: false,
            unlit: false,
            transmission: 0.0,
            morphic: None,
        }
    }
}

/// Everything the shader needs from a material, decided once at load.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub npr: bool,
    pub hemi_oct_normals: bool,
    pub base_factor: [f32; 4],
    /// Linear tint matrix rows, applied through the tint mask's red.
    pub tint_matrix: [[f32; 3]; 3],
    pub tint_offset: [f32; 3],
    pub tint_masked: bool,
    pub csb: Option<[f32; 3]>,
    pub reflectivity: [f32; 3],
    pub tint_mask: Option<Slot>,
    pub outline_mask: Option<Slot>,
    /// Outline hull growth in game units for a white mask; 0 means no hull.
    pub outline_thickness: f32,
    pub self_illum: Option<SelfIllum>,
    pub hidden_shell_ok: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelfIllum {
    pub scale: f32,
    pub tint: [f32; 3],
    pub albedo_factor: f32,
    pub mask: Option<Slot>,
}

pub const NPR_OUTLINE_THICKNESS: f32 = 0.1;

fn slot(info: Option<gltf::texture::Info>) -> Option<Slot> {
    info.map(|i| Slot { texture: i.texture().index(), uv: i.tex_coord() })
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Squared, normalized Rec. 709 weights (VRF's MatrixColorCorrect2 saturation axis).
pub fn saturation_weights() -> [f32; 3] {
    let r = [0.2126f32, 0.7152, 0.0722];
    let l2 = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
    [r[0] * r[0] / l2, r[1] * r[1] / l2, r[2] * r[2] / l2]
}

impl Material {
    pub fn from_gltf(m: &gltf::Material) -> Material {
        let pbr = m.pbr_metallic_roughness();
        let morphic = m.extras().as_ref().and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok()).and_then(|v| v.get("morphic").map(Morphic::parse));
        let mut alpha = match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask(m.alpha_cutoff().unwrap_or(0.5)),
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        };
        if let Some(mo) = &morphic
            && (mo.flag("F_ADDITIVE_BLEND") || mo.blend_mode.contains("additive"))
        {
            alpha = AlphaMode::Additive;
        }
        Material {
            name: m.name().unwrap_or_default().to_string(),
            base_color: pbr.base_color_factor(),
            base_tex: slot(pbr.base_color_texture()),
            normal_tex: m.normal_texture().map(|n| Slot { texture: n.texture().index(), uv: n.tex_coord() }),
            normal_scale: m.normal_texture().map(|n| n.scale()).unwrap_or(1.0),
            mr_tex: slot(pbr.metallic_roughness_texture()),
            metallic: pbr.metallic_factor(),
            roughness: pbr.roughness_factor(),
            occlusion_tex: m.occlusion_texture().map(|o| Slot { texture: o.texture().index(), uv: o.tex_coord() }),
            occlusion_strength: m.occlusion_texture().map(|o| o.strength()).unwrap_or(1.0),
            emissive_tex: slot(m.emissive_texture()),
            emissive: m.emissive_factor(),
            emissive_strength: m.emissive_strength().unwrap_or(1.0),
            alpha,
            double_sided: m.double_sided(),
            unlit: m.unlit(),
            transmission: m.transmission().map(|t| t.transmission_factor()).unwrap_or(0.0),
            morphic,
        }
    }

    pub fn resolve(&self) -> Resolved {
        let mut r = Resolved {
            npr: false,
            hemi_oct_normals: false,
            base_factor: self.base_color,
            tint_matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            tint_offset: [0.0; 3],
            tint_masked: false,
            csb: None,
            reflectivity: [1.0; 3],
            tint_mask: None,
            outline_mask: None,
            outline_thickness: 0.0,
            self_illum: None,
            hidden_shell_ok: false,
        };
        let Some(m) = &self.morphic else { return r };
        let pbr = m.is_pbr();
        r.npr = pbr && m.flag("F_USE_NPR_LIGHTING") && !m.flag("F_UNLIT") && !self.unlit;
        r.hemi_oct_normals = r.npr && self.normal_tex.is_some();
        let tex = |name: &str| m.textures.get(name).map(|&t| Slot { texture: t, uv: 0 });
        r.tint_mask = tex("g_tTintMaskRimLightMask").or_else(|| tex("g_tTintMask"));
        r.reflectivity = m.reflectivity.unwrap_or([1.0; 3]);
        // Color correction, unless identity or driven by an expression.
        if let Some(c) = m.vectors.get("g_vAlbedoContrastSaturationBrightness1")
            && !m.dynamic.contains("g_vAlbedoContrastSaturationBrightness1")
            && ((c[0] - 1.0).abs() > 1e-4 || (c[1] - 1.0).abs() > 1e-4 || (c[2] - 1.0).abs() > 1e-4)
        {
            r.csb = Some([c[0], c[1], c[2]]);
        }
        // The authored tint, decoded from sRGB once (source2TintPlan).
        let raw = m.vectors.get("g_vColorTint1");
        if let Some(raw) = raw
            && pbr
            && m.schema >= 2
            && !m.dynamic.contains("g_vColorTint1")
            && raw[..3].iter().all(|v| v.is_finite())
        {
            let lin = [srgb_to_linear(raw[0]), srgb_to_linear(raw[1]), srgb_to_linear(raw[2])];
            let f = self.base_color;
            let matches = |v: [f32; 3]| (f[0] - v[0]).abs() < 1e-6 && (f[1] - v[1]).abs() < 1e-6 && (f[2] - v[2]).abs() < 1e-6;
            if matches([raw[0], raw[1], raw[2]]) || matches(lin) {
                r.base_factor = [1.0, 1.0, 1.0, f[3]];
                let mode = m.ints.get("g_nTextureColorTintMode1").copied().unwrap_or(0);
                r.tint_matrix = tint_matrix(lin, mode);
                if mode == 1 {
                    let applied = mul3(&r.tint_matrix, lin);
                    r.tint_offset = [lin[0] - applied[0], lin[1] - applied[1], lin[2] - applied[2]];
                }
                r.tint_masked = m.ints.get("g_bMaskColorTint1").copied().unwrap_or(1) != 0;
            }
        }
        // Outline hull eligibility (hasCitadelNprOutline).
        let see_through = ["F_GLASS", "F_TRANSLUCENT", "F_ADVANCED_TRANSLUCENCY", "F_CLOAK", "F_DISABLE_NPR_OUTLINE", "F_SOLID_COLOR_OUTLINE", "F_UNLIT"].iter().any(|f| m.flag(f));
        if r.npr && !see_through && !m.dynamic_textures.contains("g_tNprOutlineMask") && matches!(self.alpha, AlphaMode::Opaque | AlphaMode::Mask(_)) {
            r.outline_mask = tex("g_tNprOutlineMask");
            if r.outline_mask.is_some() {
                // `g_bOverrideNprOutline` swaps in the material's own thickness.
                let overridden = m.ints.get("g_bOverrideNprOutline").is_some_and(|v| *v != 0) || m.flag("F_OVERRIDE_NPR_OUTLINE");
                r.outline_thickness = if overridden { m.float(&["g_flOverrideNprOutlineThickness"], 0.2) } else { NPR_OUTLINE_THICKNESS }.max(0.0);
            }
        }
        // An authored outline shell may show only as a plain opaque NPR surface.
        r.hidden_shell_ok = r.npr
            && !["F_GLASS", "F_TRANSLUCENT", "F_ADVANCED_TRANSLUCENCY", "F_ADDITIVE_BLEND", "F_ALPHA_TEST", "F_CLOAK", "F_UNLIT", "F_JITTER_VERTICES", "F_RENDER_BACKFACES"].iter().any(|f| m.flag(f))
            && m.dynamic.is_empty()
            && m.dynamic_textures.is_empty()
            && matches!(self.alpha, AlphaMode::Opaque);
        // Self-illumination: scale first.
        if m.flag("F_SELF_ILLUM") {
            let scale = m.float(&["g_flSelfIllumScale1", "g_flSelfIllumScale"], 0.0);
            if scale > 0.0 {
                let tint = m.vectors.get("g_vSelfIllumTint1").or_else(|| m.vectors.get("g_vSelfIllumTint")).map(|v| [v[0], v[1], v[2]]).unwrap_or([1.0; 3]);
                let additive = self.alpha == AlphaMode::Additive;
                let mask = tex("g_tSelfIllumMask").or(self.emissive_tex);
                let albedo_factor = m.float(&["g_flSelfIllumAlbedoFactor1"], if additive && mask.is_none() { 1.0 } else { 0.0 });
                r.self_illum = Some(SelfIllum { scale, tint, albedo_factor, mask });
            }
        }
        r
    }

    /// Whether a primitive marked `morphic_shell` shows: only a plain opaque NPR shell.
    pub fn shell_visible(&self, r: &Resolved) -> bool {
        r.hidden_shell_ok
    }
}

fn mul3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// VRF's MatrixColorTint3 (rows): multiply, preserve luminance (1) or mod2x (2).
pub fn tint_matrix(tint: [f32; 3], mode: i64) -> [[f32; 3]; 3] {
    if mode != 1 {
        let k = if mode == 2 { 2.0 } else { 1.0 };
        return [[tint[0] * k, 0.0, 0.0], [0.0, tint[1] * k, 0.0], [0.0, 0.0, tint[2] * k]];
    }
    let peak = tint[0].max(tint[1]).max(tint[2]);
    let sat = if peak == 0.0 { 0.0 } else { (peak - tint[0].min(tint[1]).min(tint[2])) / peak };
    let xy = 1.0 - sat;
    let z = 1.0 - sat * sat * 0.85;
    let w = saturation_weights();
    let d = z - xy;
    [[xy + d * w[0], d * w[1], d * w[2]], [d * w[0], xy + d * w[1], d * w[2]], [d * w[0], d * w[1], xy + d * w[2]]]
}

/// Grimoire's `applySource2ColorCorrection`: linear, unclamped.
pub fn color_correct(c: [f32; 3], csb: [f32; 3], pivot: [f32; 3]) -> [f32; 3] {
    let mut r = [0.0; 3];
    for i in 0..3 {
        r[i] = ((c[i] - pivot[i]) * csb[0] + pivot[i]) * csb[2];
    }
    let w = saturation_weights();
    let gray = r[0] * w[0] + r[1] * w[1] + r[2] * w[2];
    [r[0] * csb[1] + gray * (1.0 - csb[1]), r[1] * csb[1] + gray * (1.0 - csb[1]), r[2] * csb[1] + gray * (1.0 - csb[1])]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn morphic(json: &str) -> Material {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        Material { base_color: [0.5, 0.25, 0.125, 1.0], morphic: Some(Morphic::parse(&v)), ..Default::default() }
    }

    #[test]
    fn npr_needs_pbr_and_the_flag() {
        let m = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","flags":{"F_USE_NPR_LIGHTING":1}}"#);
        assert!(m.resolve().npr);
        let m = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","flags":{"F_USE_NPR_LIGHTING":1,"F_UNLIT":1}}"#);
        assert!(!m.resolve().npr);
        let m = morphic(r#"{"schema_version":4,"shader":"generic.vfx","flags":{"F_USE_NPR_LIGHTING":1}}"#);
        assert!(!m.resolve().npr);
    }

    #[test]
    fn an_exported_raw_tint_is_decoded_once_and_moved_off_the_factor() {
        let m = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","vectors":{"g_vColorTint1":[0.5,0.25,0.125,1]}}"#);
        let r = m.resolve();
        assert_eq!(r.base_factor, [1.0, 1.0, 1.0, 1.0]);
        assert!((r.tint_matrix[0][0] - srgb_to_linear(0.5)).abs() < 1e-6);
        assert!(r.tint_masked);
    }

    #[test]
    fn an_edited_factor_is_left_alone() {
        let mut m = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","vectors":{"g_vColorTint1":[0.9,0.9,0.9,1]}}"#);
        m.base_color = [0.1, 0.2, 0.3, 1.0];
        let r = m.resolve();
        assert_eq!(r.base_factor, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(r.tint_matrix, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn solid_color_outlines_get_no_hull() {
        let base = r#"{"schema_version":4,"shader":"pbr.vfx","flags":{"F_USE_NPR_LIGHTING":1FLAG},"textures":{"g_tNprOutlineMask":3}}"#;
        assert!(morphic(&base.replace("FLAG", "")).resolve().outline_thickness > 0.0);
        assert_eq!(morphic(&base.replace("FLAG", ",\"F_SOLID_COLOR_OUTLINE\":1")).resolve().outline_thickness, 0.0);
        assert_eq!(morphic(&base.replace("FLAG", ",\"F_GLASS\":1")).resolve().outline_thickness, 0.0);
    }

    #[test]
    fn self_illum_is_gated_on_its_scale() {
        let on = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","flags":{"F_SELF_ILLUM":1},"floats":{"g_flSelfIllumScale1":2.6}}"#);
        assert_eq!(on.resolve().self_illum.map(|s| s.scale), Some(2.6));
        let off = morphic(r#"{"schema_version":4,"shader":"pbr.vfx","flags":{"F_SELF_ILLUM":1},"floats":{"g_flSelfIllumScale1":0}}"#);
        assert!(off.resolve().self_illum.is_none());
    }

    #[test]
    fn color_correction_pivots_on_reflectivity() {
        let p = [0.3, 0.3, 0.3];
        let same = color_correct(p, [2.0, 1.0, 1.0], p);
        assert!((same[0] - 0.3).abs() < 1e-6, "the pivot is a fixed point of contrast");
        let gray = color_correct([0.6, 0.2, 0.1], [1.0, 0.0, 1.0], p);
        assert!((gray[0] - gray[1]).abs() < 1e-6 && (gray[1] - gray[2]).abs() < 1e-6);
    }

    #[test]
    fn preserve_luminance_tint_keeps_white_white_when_unsaturated() {
        let m = tint_matrix([0.5, 0.5, 0.5], 1);
        let v = mul3(&m, [1.0, 1.0, 1.0]);
        assert!(v.iter().all(|c| (c - 1.0).abs() < 1e-5), "{v:?}");
    }
}
