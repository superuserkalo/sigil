//! The Hideout's baked and volumetric data on the GPU: the SH2 lightmap (DC irradiance, the
//! packed L1 direction and the stationary lights' shadow mask), the light probe volume atlas,
//! the fog irradiance volume and density volumes, light cookies, and the per-light extras
//! (Source units, skirt, cookie, baked shadow channel). Read on any thread
//! (`HideoutData::load`), uploaded on the viewer's (`HideoutGpu::new`).
//!
//! Ported from Grimoire (MIT): `source2Lightmap.ts`, `source2ProbeVolume.ts`,
//! `source2DynamicLights.ts`, `source2LightCookies.ts` and `source2Atmosphere.ts`, which follow
//! ValveResourceFormat's lighting shaders.

use std::path::Path;

use crate::gpu::{Gpu, f16_bits};
use crate::math::Mat4;
use crate::scene::{SceneDesc, SceneLight};

/// Light probe volumes and fog volumes the shader reads at most.
pub const MAX_PROBES: usize = 8;
pub const MAX_FOG_VOLUMES: usize = 16;
/// Fog shadow tiles the shader reads: six (an omni's faces) for each of the eight lights.
pub const MAX_FOG_SHADOW_TILES: usize = 48;
/// One cookie's square edge in the strip.
pub const COOKIE_EDGE: u32 = 256;

/// A VTV1 volume: the header Grimoire's `texture-volume` writes, uncompressed.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: VolumeFormat,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeFormat {
    Rgba8,
    Rgba16F,
    R16,
}

pub const VTV_MAGIC: u32 = 0x3156_5456;

pub fn read_vtv(bytes: &[u8]) -> Result<Volume, String> {
    let word = |i: usize| bytes.get(i * 4..i * 4 + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if word(0) != Some(VTV_MAGIC) {
        return Err("not a VTV1 volume".into());
    }
    let (w, h, d, c) = (word(1).unwrap_or(0), word(2).unwrap_or(0), word(3).unwrap_or(0), word(4).unwrap_or(0));
    let format = match c {
        1 => VolumeFormat::Rgba8,
        2 => VolumeFormat::Rgba16F,
        3 => VolumeFormat::R16,
        _ => return Err(format!("unknown VTV component code {c}")),
    };
    let texel = match format {
        VolumeFormat::Rgba8 => 4,
        VolumeFormat::Rgba16F => 8,
        VolumeFormat::R16 => 2,
    };
    if w == 0 || h == 0 || d == 0 || w > 4096 || h > 4096 || d > 4096 {
        return Err(format!("VTV volume of {w}x{h}x{d}"));
    }
    let need = w as usize * h as usize * d as usize * texel;
    let data = bytes.get(20..20 + need).ok_or("VTV volume is truncated")?;
    Ok(Volume { width: w, height: h, depth: d, format, bytes: data.to_vec() })
}

/// Everything read from disk for a scene's Hideout lighting.
#[derive(Default)]
pub struct HideoutData {
    /// DC irradiance (linear RGBA32F), width, height.
    pub lightmap: Option<(u32, u32, Vec<[f32; 4]>)>,
    pub direction: Option<(u32, u32, Vec<u8>)>,
    pub shadows: Option<(u32, u32, Vec<u8>)>,
    pub probes: Option<Volume>,
    pub probe_shadows: Option<Volume>,
    pub fog_irradiance: Option<Volume>,
    /// The fog lights' precomputed shadow atlas (R16, depth 1).
    pub fog_shadows: Option<Volume>,
    /// Square sRGB RGBA8 cookies, `COOKIE_EDGE` each, by light index in `desc.lights`.
    pub cookies: Vec<(usize, Vec<u8>)>,
    /// Colour correction volume, RGBA8, and its edge.
    pub lut: Option<(u32, Vec<u8>)>,
    pub errors: Vec<String>,
}

fn png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?.into_rgba8();
    let (w, h) = img.dimensions();
    Ok((w, h, img.into_raw()))
}

impl HideoutData {
    pub fn load(desc: &SceneDesc) -> HideoutData {
        let mut d = HideoutData::default();
        let note = |e: String, errors: &mut Vec<String>| errors.push(e);
        if let Some(lm) = &desc.lightmap {
            match std::fs::read(&lm.file).map_err(|e| format!("{}: {e}", lm.file.display())).and_then(|b| image::load_from_memory_with_format(&b, image::ImageFormat::Hdr).map_err(|e| format!("{}: {e}", lm.file.display()))) {
                Ok(img) => {
                    let img = img.into_rgba32f();
                    let (w, h) = img.dimensions();
                    d.lightmap = Some((w, h, img.into_raw().as_chunks::<4>().0.to_vec()));
                }
                Err(e) => note(e, &mut d.errors),
            }
            for (slot, file) in [(&mut d.direction, &lm.direction), (&mut d.shadows, &lm.shadows)] {
                if let Some(f) = file {
                    match png(f) {
                        Ok(v) => *slot = Some(v),
                        Err(e) => d.errors.push(e),
                    }
                }
            }
        }
        let volume = |p: &Path, errors: &mut Vec<String>| match std::fs::read(p).map_err(|e| format!("{}: {e}", p.display())).and_then(|b| read_vtv(&b).map_err(|e| format!("{}: {e}", p.display()))) {
            Ok(v) => Some(v),
            Err(e) => {
                errors.push(e);
                None
            }
        };
        if let Some(first) = desc.probes.first() {
            d.probes = volume(&first.file, &mut d.errors).filter(|v| v.format != VolumeFormat::R16);
            d.probe_shadows = first.shadows.as_deref().and_then(|p| volume(p, &mut d.errors));
        }
        if let Some((file, _)) = desc.fog.as_ref().and_then(|f| f.irradiance.as_ref()) {
            d.fog_irradiance = volume(file, &mut d.errors).filter(|v| v.format != VolumeFormat::R16);
        }
        if let Some(file) = desc.fog.as_ref().and_then(|f| f.shadows.as_ref()) {
            d.fog_shadows = volume(file, &mut d.errors).filter(|v| v.format == VolumeFormat::R16 && v.depth == 1);
        }
        for (i, l) in desc.lights.iter().enumerate() {
            let Some(cookie) = &l.cookie else { continue };
            match png(cookie) {
                Ok((w, h, rgba)) => {
                    let img = image::RgbaImage::from_raw(w, h, rgba).unwrap_or_default();
                    let small = image::imageops::resize(&img, COOKIE_EDGE, COOKIE_EDGE, image::imageops::FilterType::Triangle);
                    d.cookies.push((i, small.into_raw()));
                }
                Err(e) => d.errors.push(e),
            }
        }
        if let Some((file, dim)) = &desc.post.lut {
            match std::fs::read(file) {
                Ok(b) if b.len() >= (*dim as usize).pow(3) * 4 => d.lut = Some((*dim, b[..(*dim as usize).pow(3) * 4].to_vec())),
                Ok(_) => d.errors.push(format!("{}: too short for a {dim}^3 table", file.display())),
                Err(e) => d.errors.push(format!("{}: {e}", file.display())),
            }
        }
        d
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProbeGpu {
    pub world_to_local: [f32; 16],
    /// size xyz, rgbm
    pub size: [f32; 4],
    /// atlas offset xyz (voxels)
    pub offset: [f32; 4],
    /// fade per axis (size over fade distances), unused
    pub fade: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FogVolumeGpu {
    pub world_to_local: [f32; 16],
    /// sphere, strength, falloff, indirect strength override (< 0: none)
    pub params: [f32; 4],
    /// linear tint, has tint
    pub tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightExGpu {
    /// Source units, skirt, cookie layer (< 0: none), fog strength
    pub a: [f32; 4],
    /// fog only, baked shadow channel (< 0: none), unused, unused
    pub b: [f32; 4],
    /// fog shadow tiles (0, 1 barn or 6 omni), first tile in `fog_shadow_rects`, shear x, y
    pub c: [f32; 4],
    /// `size_params` xyz (Source units), unused
    pub d: [f32; 4],
    /// The light's Source forward, left and up in glTF space.
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub z: [f32; 4],
}

/// The Hideout uniform (group 0 binding 6), see `HideoutUniform` in common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HideoutUniform {
    /// lightmap on, direction on, shadows on, unused
    pub lightmap: [f32; 4],
    /// uv scale, unused
    pub lightmap_scale: [f32; 4],
    /// probe count, probe shadows on, cookie layers, unused
    pub counts: [f32; 4],
    /// probe atlas size in texels (w, h, depth / 6)
    pub atlas: [f32; 4],
    pub probes: [ProbeGpu; MAX_PROBES],
    pub fog_irradiance: [f32; 16],
    /// irradiance on, controller indirect strength, controller strength, draw distance (m)
    pub fog: [f32; 4],
    /// fade in start, fade in end, volume count, steps
    pub fog_fade: [f32; 4],
    pub fog_volumes: [FogVolumeGpu; MAX_FOG_VOLUMES],
    pub lights: [LightExGpu; 8],
    /// The fog shadow atlas: width, height, on, unused.
    pub fog_shadow_atlas: [f32; 4],
    /// Tiles of the chosen lights, six per light slot, `[w, h, x, y]` in atlas uv.
    pub fog_shadow_rects: [[f32; 4]; MAX_FOG_SHADOW_TILES],
}

impl Default for HideoutUniform {
    fn default() -> Self {
        let mut u: HideoutUniform = bytemuck::Zeroable::zeroed();
        for l in &mut u.lights {
            l.a = [0.0, 0.1, -1.0, 0.0];
            l.b = [0.0, -1.0, 0.0, 0.0];
        }
        u
    }
}

/// The per-light extras for the lights the frame chose, in their order.
pub fn light_extras(u: &mut HideoutUniform, chosen: &[&SceneLight], cookie_layers: &[(usize, usize)], all: &[SceneLight]) {
    let shadows_on = u.fog_shadow_atlas[2] > 0.5;
    for (slot_index, (slot, l)) in u.lights.iter_mut().zip(chosen).enumerate() {
        let index = all.iter().position(|x| std::ptr::eq(x, *l));
        let layer = index.and_then(|i| cookie_layers.iter().find(|c| c.0 == i)).map_or(-1.0, |c| c.1 as f32);
        let basis = l.basis.unwrap_or([[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        slot.a = [if l.source_units { 1.0 } else { 0.0 }, l.skirt.max(1e-3), layer, l.fog_strength.unwrap_or(0.0)];
        slot.b = [if l.fog_only { 1.0 } else { 0.0 }, l.shadow_channel.map_or(-1.0, |c| c as f32), 0.0, 0.0];
        slot.c = [0.0; 4];
        slot.d = [0.0; 4];
        slot.x = [basis[0][0], basis[0][1], basis[0][2], 0.0];
        slot.y = [basis[1][0], basis[1][1], basis[1][2], 0.0];
        slot.z = [basis[2][0], basis[2][1], basis[2][2], 0.0];
        if let Some(f) = l.fog_shadow.as_ref().filter(|_| shadows_on && l.basis.is_some()) {
            let first = slot_index * 6;
            for (k, t) in f.tiles.iter().take(6).enumerate() {
                u.fog_shadow_rects[first + k] = *t;
            }
            slot.c = [f.tiles.len().min(6) as f32, first as f32, f.shear[0], f.shear[1]];
            slot.d = [f.size[0], f.size[1], f.size[2], 0.0];
        }
    }
}

/// The scene-level part of the uniform (everything but the per-light extras).
pub fn uniform(desc: &SceneDesc, data: &HideoutData) -> HideoutUniform {
    let mut u = HideoutUniform::default();
    if let (Some(lm), Some(_)) = (&desc.lightmap, &data.lightmap) {
        u.lightmap = [1.0, data.direction.is_some() as u8 as f32, data.shadows.is_some() as u8 as f32, 0.0];
        u.lightmap_scale = [lm.uv_scale[0], lm.uv_scale[1], 0.0, 0.0];
    }
    if let Some(v) = &data.probes {
        u.atlas = [v.width as f32, v.height as f32, (v.depth / 6).max(1) as f32, 0.0];
        let mut n = 0;
        for (slot, p) in u.probes.iter_mut().zip(&desc.probes) {
            slot.world_to_local = p.world_to_local.to_cols_array();
            slot.size = [p.size[0].max(1.0), p.size[1].max(1.0), p.size[2].max(1.0), if p.rgbm { 1.0 } else { 0.0 }];
            slot.offset = [p.offset[0], p.offset[1], p.offset[2], 0.0];
            slot.fade = [p.fade[0], p.fade[1], p.fade[2], 0.0];
            n += 1;
        }
        u.counts[0] = n as f32;
        u.counts[1] = data.probe_shadows.is_some() as u8 as f32;
    }
    u.counts[2] = data.cookies.len() as f32;
    if let Some(f) = &desc.fog {
        if let (Some((_, m)), Some(_)) = (&f.irradiance, &data.fog_irradiance) {
            u.fog_irradiance = m.to_cols_array();
            u.fog[0] = 1.0;
        } else {
            u.fog_irradiance = Mat4::IDENTITY.to_cols_array();
        }
        u.fog[1] = f.indirect_strength;
        u.fog[2] = f.strength;
        u.fog[3] = f.end;
        // Grimoire's march: 32 midpoint steps along the ray (the half resolution pass).
        u.fog_fade = [f.start, f.start.max(f.start + 0.01), f.volumes.len().min(MAX_FOG_VOLUMES) as f32, 32.0];
        if let Some(v) = &data.fog_shadows {
            u.fog_shadow_atlas = [v.width as f32, v.height as f32, 1.0, 0.0];
        }
        for (slot, v) in u.fog_volumes.iter_mut().zip(&f.volumes) {
            slot.world_to_local = v.world_to_local.to_cols_array();
            slot.params = [if v.sphere { 1.0 } else { 0.0 }, v.strength, v.falloff.max(1e-3), v.indirect_strength.unwrap_or(-1.0)];
            slot.tint = match v.tint {
                Some(t) => [t[0], t[1], t[2], 1.0],
                None => [1.0, 1.0, 1.0, 0.0],
            };
        }
    }
    u
}

pub struct HideoutGpu {
    pub buffer: wgpu::Buffer,
    pub lightmap: wgpu::TextureView,
    pub direction: wgpu::TextureView,
    pub shadows: wgpu::TextureView,
    pub probes: wgpu::TextureView,
    pub probe_shadows: wgpu::TextureView,
    pub fog: wgpu::TextureView,
    /// The fog shadow atlas as R16 unsigned integers (sampled with `textureLoad`).
    pub fog_shadows: wgpu::TextureView,
    pub cookies: wgpu::TextureView,
    /// The colour correction volume, when the map has one.
    pub lut: Option<(wgpu::TextureView, u32)>,
    /// Light index in the scene to its cookie layer.
    pub cookie_layers: Vec<(usize, usize)>,
    pub base: HideoutUniform,
}

fn texture(gpu: &Gpu, label: &str, size: wgpu::Extent3d, dimension: wgpu::TextureDimension, format: wgpu::TextureFormat, bytes: &[u8], bytes_per_texel: u32) -> wgpu::TextureView {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytes,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.width * bytes_per_texel), rows_per_image: Some(size.height) },
        size,
    );
    texture.create_view(&Default::default())
}

fn extent(w: u32, h: u32, d: u32) -> wgpu::Extent3d {
    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: d }
}

fn upload_volume(gpu: &Gpu, label: &str, v: &Volume) -> wgpu::TextureView {
    let size = extent(v.width, v.height, v.depth);
    match v.format {
        // u8 probe data is sRGB encoded (Grimoire uploads it as an sRGB texture).
        VolumeFormat::Rgba8 => texture(gpu, label, size, wgpu::TextureDimension::D3, wgpu::TextureFormat::Rgba8UnormSrgb, &v.bytes, 4),
        VolumeFormat::Rgba16F => texture(gpu, label, size, wgpu::TextureDimension::D3, wgpu::TextureFormat::Rgba16Float, &v.bytes, 8),
        // A single channel shadow volume, widened to a filterable unorm.
        VolumeFormat::R16 => {
            let bytes: Vec<u8> = v.bytes.as_chunks::<2>().0.iter().map(|c| (u16::from_le_bytes(*c) >> 8) as u8).collect();
            texture(gpu, label, size, wgpu::TextureDimension::D3, wgpu::TextureFormat::R8Unorm, &bytes, 1)
        }
    }
}

/// 1x1 stand-ins so the bind group is complete without Hideout data.
pub struct HideoutDefaults {
    pub d2: wgpu::TextureView,
    pub d2_uint: wgpu::TextureView,
    pub d2_hdr: wgpu::TextureView,
    pub d3: wgpu::TextureView,
    pub buffer: wgpu::Buffer,
}

impl HideoutDefaults {
    pub fn new(gpu: &Gpu) -> HideoutDefaults {
        use wgpu::util::DeviceExt;
        let one = extent(1, 1, 1);
        HideoutDefaults {
            d2: texture(gpu, "viewer hideout none", one, wgpu::TextureDimension::D2, wgpu::TextureFormat::Rgba8Unorm, &[255, 255, 255, 255], 4),
            d2_uint: texture(gpu, "viewer hideout none uint", one, wgpu::TextureDimension::D2, wgpu::TextureFormat::R16Uint, &[255, 255], 2),
            d2_hdr: texture(gpu, "viewer hideout none hdr", one, wgpu::TextureDimension::D2, wgpu::TextureFormat::Rgba16Float, &[0; 8], 8),
            d3: texture(gpu, "viewer hideout none 3d", one, wgpu::TextureDimension::D3, wgpu::TextureFormat::Rgba8Unorm, &[0, 0, 0, 0], 4),
            buffer: gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer hideout none"), contents: bytemuck::bytes_of(&HideoutUniform::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST }),
        }
    }
}

impl HideoutGpu {
    pub fn new(gpu: &Gpu, desc: &SceneDesc, data: &HideoutData, defaults: &HideoutDefaults) -> HideoutGpu {
        use wgpu::util::DeviceExt;
        let base = uniform(desc, data);
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("viewer hideout"), contents: bytemuck::bytes_of(&base), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let lightmap = match &data.lightmap {
            Some((w, h, texels)) => {
                let bytes: Vec<u8> = texels.iter().flat_map(|t| t.iter().flat_map(|c| f16_bits(*c).to_le_bytes())).collect();
                texture(gpu, "viewer lightmap", extent(*w, *h, 1), wgpu::TextureDimension::D2, wgpu::TextureFormat::Rgba16Float, &bytes, 8)
            }
            None => defaults.d2_hdr.clone(),
        };
        let unorm = |label: &str, v: &Option<(u32, u32, Vec<u8>)>| match v {
            Some((w, h, rgba)) => texture(gpu, label, extent(*w, *h, 1), wgpu::TextureDimension::D2, wgpu::TextureFormat::Rgba8Unorm, rgba, 4),
            None => defaults.d2.clone(),
        };
        let direction = unorm("viewer lightmap direction", &data.direction);
        let shadows = unorm("viewer lightmap shadows", &data.shadows);
        let vol = |label: &str, v: &Option<Volume>| v.as_ref().map_or_else(|| defaults.d3.clone(), |v| upload_volume(gpu, label, v));
        let probes = vol("viewer probes", &data.probes);
        let probe_shadows = vol("viewer probe shadows", &data.probe_shadows);
        let fog = vol("viewer fog irradiance", &data.fog_irradiance);
        let fog_shadows = match &data.fog_shadows {
            Some(v) => texture(gpu, "viewer fog shadows", extent(v.width, v.height, 1), wgpu::TextureDimension::D2, wgpu::TextureFormat::R16Uint, &v.bytes, 2),
            None => defaults.d2_uint.clone(),
        };
        let mut cookie_layers = Vec::new();
        let cookies = if data.cookies.is_empty() {
            defaults.d2.clone()
        } else {
            // One horizontal strip, a layer per cookie (source2LightCookies.ts).
            let n = data.cookies.len() as u32;
            let mut strip = vec![0u8; (COOKIE_EDGE * n * COOKIE_EDGE * 4) as usize];
            for (layer, (light, rgba)) in data.cookies.iter().enumerate() {
                cookie_layers.push((*light, layer));
                for y in 0..COOKIE_EDGE as usize {
                    let src = &rgba[y * COOKIE_EDGE as usize * 4..(y + 1) * COOKIE_EDGE as usize * 4];
                    let dst = (y * (COOKIE_EDGE * n) as usize + layer * COOKIE_EDGE as usize) * 4;
                    strip[dst..dst + src.len()].copy_from_slice(src);
                }
            }
            texture(gpu, "viewer cookies", extent(COOKIE_EDGE * n, COOKIE_EDGE, 1), wgpu::TextureDimension::D2, wgpu::TextureFormat::Rgba8UnormSrgb, &strip, 4)
        };
        let lut = data.lut.as_ref().map(|(dim, bytes)| (texture(gpu, "viewer color correction", extent(*dim, *dim, *dim), wgpu::TextureDimension::D3, wgpu::TextureFormat::Rgba8Unorm, bytes, 4), *dim));
        HideoutGpu { buffer, lightmap, direction, shadows, probes, probe_shadows, fog, fog_shadows, cookies, lut, cookie_layers, base }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vtv(w: u32, h: u32, d: u32, code: u32, texel: usize) -> Vec<u8> {
        let mut b = Vec::new();
        for x in [VTV_MAGIC, w, h, d, code] {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b.extend(std::iter::repeat_n(7u8, (w * h * d) as usize * texel));
        b
    }

    #[test]
    fn vtv_volumes_read_and_refuse_bad_input() {
        let v = read_vtv(&vtv(2, 3, 12, 1, 4)).unwrap();
        assert_eq!((v.width, v.height, v.depth, v.format), (2, 3, 12, VolumeFormat::Rgba8));
        assert_eq!(v.bytes.len(), 2 * 3 * 12 * 4);
        assert_eq!(read_vtv(&vtv(1, 1, 1, 2, 8)).unwrap().format, VolumeFormat::Rgba16F);
        assert!(read_vtv(&vtv(1, 1, 1, 9, 4)).is_err());
        let mut short = vtv(4, 4, 4, 1, 4);
        short.truncate(40);
        assert!(read_vtv(&short).is_err());
        assert!(read_vtv(b"nope").is_err());
    }

    #[test]
    fn the_uniform_keeps_its_layout_and_defaults() {
        assert_eq!(std::mem::size_of::<HideoutUniform>() % 16, 0);
        let u = HideoutUniform::default();
        assert_eq!(u.lights[0].a[2], -1.0, "no cookie by default");
        assert_eq!(u.lights[0].b[1], -1.0, "no baked shadow channel by default");
        let u = uniform(&SceneDesc::default(), &HideoutData::default());
        assert_eq!(u.lightmap[0], 0.0);
        assert_eq!(u.counts[0], 0.0);
    }
}
