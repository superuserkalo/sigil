//! The Hideout's baked lighting beyond the irradiance lightmap: volume textures (probe atlas,
//! fog irradiance) as VTV files, data textures and light cookies as PNG, the probe and fog
//! volume entities, and the export progress meter.
//!
//! Ported from Grimoire's `hideoutScene.ts` and `hideoutAtmosphere.ts` (MIT), with the
//! vpkmerge subcommands they shell out to (`texture-volume`, `catalog texture --thumbs`)
//! replaced by the library calls behind them.

use std::sync::Mutex;

use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use super::super::kv::{flag, num, text, vec3};
use super::super::space;
use super::super::stack::compiled;
use super::{HideoutFogVolume, HideoutProbe};
use crate::error::{Error, Result};

/// `VTV1` read as a little-endian u32.
pub(crate) const VTV_MAGIC: u32 = 0x3156_5456;
/// Bytes before the texels.
pub(crate) const VTV_HEADER: usize = 20;

/// The texel kinds a VTV file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VtvKind {
    /// RGBA, one byte per channel.
    Rgba8 = 1,
    /// RGBA, IEEE half floats.
    Rgba16F = 2,
    /// One 16 bit UNORM channel.
    R16 = 3,
}

impl VtvKind {
    pub(crate) fn texel_bytes(self) -> usize {
        match self {
            VtvKind::Rgba8 => 4,
            VtvKind::Rgba16F => 8,
            VtvKind::R16 => 2,
        }
    }

    fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(VtvKind::Rgba8),
            2 => Some(VtvKind::Rgba16F),
            3 => Some(VtvKind::R16),
            _ => None,
        }
    }
}

/// A VTV header: magic, width, height, depth, kind, all u32 LE.
pub(crate) fn vtv_header(dims: [u32; 3], kind: VtvKind) -> [u8; VTV_HEADER] {
    let mut out = [0u8; VTV_HEADER];
    for (index, value) in [VTV_MAGIC, dims[0], dims[1], dims[2], kind as u32]
        .into_iter()
        .enumerate()
    {
        out[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    out
}

/// The dimensions and kind of a VTV file, checking that the texels fill it exactly.
pub(crate) fn read_vtv_header(bytes: &[u8]) -> Option<([u32; 3], VtvKind)> {
    let word = |index: usize| {
        bytes
            .get(index * 4..index * 4 + 4)
            .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
    };
    if word(0)? != VTV_MAGIC {
        return None;
    }
    let dims = [word(1)?, word(2)?, word(3)?];
    let kind = VtvKind::from_code(word(4)?)?;
    let texels = dims
        .iter()
        .try_fold(1usize, |total, value| total.checked_mul(*value as usize))?;
    (texels > 0 && bytes.len() == VTV_HEADER + texels * kind.texel_bytes()).then_some((dims, kind))
}

/// The largest compiled texture read for a lightmap, cookie or volume (the file comes from the
/// stack, where a mod can put anything): particles.rs's `MAX_TEXTURE_BYTES`.
pub(crate) const MAX_COMPILED_TEXTURE: usize = 64 * 1024 * 1024;
/// The most a decoded texture or volume may take, checked from its header before decoding.
pub(crate) const MAX_DECODED_TEXTURE: u64 = 256 * 1024 * 1024;

/// Refuses a texture whose file or decoded size (width x height x depth x the widest texel
/// it can decode to) is past the caps, before any of it is decoded.
pub(crate) fn check_texture_budget(bytes: &[u8], width: u32, height: u32, depth: u32, texel: u64) -> Result<()> {
    if bytes.len() > MAX_COMPILED_TEXTURE {
        return Err(Error::msg(format!("texture: a {} MB file is over the {} MB cap", bytes.len() >> 20, MAX_COMPILED_TEXTURE >> 20)));
    }
    let decoded = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(u64::from(depth)))
        .and_then(|n| n.checked_mul(texel));
    match decoded {
        Some(n) if n <= MAX_DECODED_TEXTURE => Ok(()),
        _ => Err(Error::msg(format!("texture: {width}x{height}x{depth} decodes past the {} MB cap", MAX_DECODED_TEXTURE >> 20))),
    }
}

/// `check_texture_budget` for a compiled texture on disk that vpkmerge decodes to float HDR
/// (the lightmap, the sky and reflection cubes): the file size, then its header's size at
/// 16 bytes a texel, six faces for a cube.
pub(crate) fn check_texture_file(path: &std::path::Path) -> Result<()> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_COMPILED_TEXTURE as u64 {
        return Err(Error::msg(format!("texture: a {} MB file is over the {} MB cap", len >> 20, MAX_COMPILED_TEXTURE >> 20)));
    }
    let bytes = std::fs::read(path)?;
    let info = morphic::inspect(&bytes).map_err(|error| Error::msg(format!("texture: {error:?}")))?;
    let faces = if info.flags.contains(morphic::TextureFlags::CUBE_TEXTURE) { 6 } else { 1 };
    check_texture_budget(&bytes, u32::from(info.width), u32::from(info.height), u32::from(info.depth).max(1) * faces, 16)
}

/// A compiled texture's mip 0, every slice, as a VTV file: x fastest, then y, then z, no
/// color conversion. A 2D texture is one slice. vpkmerge's `texture-volume`
/// (`texture_volume_bytes`), uncompressed. Bounded by `check_texture_budget` before decoding.
pub(crate) fn texture_vtv(bytes: &[u8]) -> Result<(Vec<u8>, [u32; 3], VtvKind)> {
    if bytes.len() > MAX_COMPILED_TEXTURE {
        return Err(Error::msg("volume texture: the file is over the size cap"));
    }
    let failed = |error: morphic::DecodeError| Error::msg(format!("volume texture: {error:?}"));
    let info = morphic::inspect(bytes).map_err(failed)?;
    // RGBA16F is the widest texel a slice decodes to.
    check_texture_budget(bytes, u32::from(info.width), u32::from(info.height), u32::from(info.depth), 8)?;
    let volume = info.flags.contains(morphic::TextureFlags::VOLUME_TEXTURE);
    if info.flags.contains(morphic::TextureFlags::CUBE_TEXTURE) || (!volume && info.depth != 1) {
        return Err(Error::msg("volume texture: not a volume or 2D texture"));
    }
    let dims = [
        u32::from(info.width),
        u32::from(info.height),
        u32::from(info.depth),
    ];
    let options = |slice: u16| morphic::DecodeOptions {
        slice,
        ..Default::default()
    };
    if info.format == morphic::TextureFormat::R16 {
        let mut out = vtv_header(dims, VtvKind::R16).to_vec();
        for slice in 0..info.depth {
            for value in morphic::decode_r16_at(bytes, &options(slice)).map_err(failed)? {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        return check_size(out, dims, VtvKind::R16);
    }
    let mut out = Vec::new();
    let mut kind = None;
    for slice in 0..info.depth {
        let image = morphic::decode_at(bytes, &options(slice)).map_err(failed)?;
        let this = match &image.data {
            morphic::ImageData::Rgba8(_) => VtvKind::Rgba8,
            morphic::ImageData::Rgba16F(_) => VtvKind::Rgba16F,
        };
        match kind {
            None => {
                out.extend_from_slice(&vtv_header(dims, this));
                kind = Some(this);
            }
            Some(first) if first != this => {
                return Err(Error::msg("volume texture: slices disagree on texel kind"));
            }
            Some(_) => {}
        }
        match image.data {
            morphic::ImageData::Rgba8(pixels) => out.extend_from_slice(&pixels),
            morphic::ImageData::Rgba16F(pixels) => {
                for value in pixels {
                    out.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
        }
    }
    let kind = kind.ok_or_else(|| Error::msg("volume texture: no slices"))?;
    check_size(out, dims, kind)
}

fn check_size(
    bytes: Vec<u8>,
    dims: [u32; 3],
    kind: VtvKind,
) -> Result<(Vec<u8>, [u32; 3], VtvKind)> {
    if read_vtv_header(&bytes).is_none() {
        return Err(Error::msg(
            "volume texture: decoded size does not match its header",
        ));
    }
    Ok((bytes, dims, kind))
}

/// A compiled texture as an RGBA8 PNG no larger than `max_edge`, bytes as stored (a UNORM
/// data texture stays UNORM, an sRGB color texture stays sRGB encoded). vpkmerge's `catalog
/// texture --thumbs`.
pub(crate) fn texture_png(bytes: &[u8], max_edge: u32) -> Result<Vec<u8>> {
    if bytes.len() > MAX_COMPILED_TEXTURE {
        return Err(Error::msg("texture png: the file is over the size cap"));
    }
    // The thumbnail decodes the top mip whole before scaling it: refuse a huge one first.
    if let Ok(info) = morphic::inspect(bytes) {
        check_texture_budget(bytes, u32::from(info.width), u32::from(info.height), u32::from(info.depth).max(1), 8)?;
    }
    vpkmerge_core::texture_catalog::thumbnail_png(bytes, max_edge)
        .map(|thumbnail| thumbnail.png)
        .map_err(|error| Error::msg(format!("texture png: {error:?}")))
}

/// `cookies/<stem>_<sha8>.png` for a light cookie path: two folders can hold a cookie of
/// the same name. Grimoire's `exportCookie` naming.
pub(crate) fn cookie_file(entry: &str) -> String {
    let source = compiled(entry);
    let stem: String = source
        .trim_end_matches(".vtex_c")
        .rsplit('/')
        .next()
        .unwrap_or("cookie")
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(source.as_bytes());
    let hash: String = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("cookies/{stem}_{hash}.png")
}

/// A Source entity's placement without scale: origin and angles.
pub(crate) fn unscaled_matrix(entity: &serde_json::Map<String, Json>) -> space::Mat4 {
    space::entity_matrix(
        vec3(entity.get("origin")).unwrap_or([0.0; 3]),
        vec3(entity.get("angles")).unwrap_or([0.0; 3]),
        [1.0; 3],
    )
}

/// glTF world to the entity box's `[0, 1]^3`, from `origin`, `angles`, `box_mins`, `box_maxs`.
pub(crate) fn entity_world_to_local(entity: &serde_json::Map<String, Json>) -> Option<[f32; 16]> {
    vec3(entity.get("origin"))?;
    let min = vec3(entity.get("box_mins"))?;
    let max = vec3(entity.get("box_maxs"))?;
    space::box_world_to_local(&unscaled_matrix(entity), min, max).map(|m| space::to_f32(&m))
}

/// A light probe volume: its atlas, its shadow atlas, and the volume with `file`, `shadows`
/// and `atlasSize` left for the caller (one atlas is exported for all volumes).
/// Grimoire's probe volume read (`hideoutScene.ts`, the `lightprobetexture` branch).
pub(crate) fn probe_volume(
    entity: &serde_json::Map<String, Json>,
    rgbm: bool,
) -> Option<(String, Option<String>, HideoutProbe)> {
    let texture = text(entity.get("lightprobetexture"))?.to_string();
    let positive = |key: &str| num(entity.get(key)).filter(|value| *value > 0.0);
    let size = [
        positive("light_probe_size_x")?,
        positive("light_probe_size_y")?,
        positive("light_probe_size_z")?,
    ];
    let min = vec3(entity.get("box_mins"))?;
    let max = vec3(entity.get("box_maxs"))?;
    let world_to_local = entity_world_to_local(entity)?;
    let offset = [
        num(entity.get("light_probe_atlas_x")).unwrap_or(0.0),
        num(entity.get("light_probe_atlas_Y")).unwrap_or(0.0),
        num(entity.get("light_probe_atlas_Z")).unwrap_or(0.0),
    ];
    let fade_dists = vec3(entity.get("edge_fade_dists")).unwrap_or([0.0; 3]);
    let fade: [f32; 3] = std::array::from_fn(|axis| {
        if fade_dists[axis] > 0.0 {
            ((max[axis] - min[axis]) / fade_dists[axis]) as f32
        } else {
            0.0
        }
    });
    let shadow = text(entity.get("lightprobetexture_dlshd")).map(str::to_string);
    Some((
        texture,
        shadow,
        HideoutProbe {
            file: String::new(),
            shadows: None,
            world_to_local,
            size: size.map(|value| value as f32),
            offset: offset.map(|value| value as f32),
            atlas_size: [0; 3],
            fade,
            priority: num(entity.get("indoor_outdoor_level")).unwrap_or(0.0) as i32,
            encoding: if rgbm { "rgbm" } else { "linear" }.into(),
        },
    ))
}

/// What an `env_volumetric_fog_volume` becomes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FogVolume {
    Volume(HideoutFogVolume),
    /// Shape 2, the height-fog box, which needs edge width and depth terms.
    HeightFog,
    /// Disabled, another class, or missing a required key.
    Skip,
}

/// Grimoire's `exportMapFog` density boxes (`hideoutAtmosphere.ts`).
pub(crate) fn fog_volume(entity: &serde_json::Map<String, Json>) -> FogVolume {
    if text(entity.get("classname")) != Some("env_volumetric_fog_volume")
        || flag(entity.get("StartDisabled")) == Some(true)
    {
        return FogVolume::Skip;
    }
    let Some(shape) = num(entity.get("Shape")) else {
        return FogVolume::Skip;
    };
    if shape > 1.0 {
        return FogVolume::HeightFog;
    }
    let non_negative = |key: &str| num(entity.get(key)).filter(|value| *value >= 0.0);
    let (Some(strength), Some(falloff), Some(world_to_local)) = (
        non_negative("FogStrength"),
        non_negative("FalloffExponent"),
        entity_world_to_local(entity),
    ) else {
        return FogVolume::Skip;
    };
    FogVolume::Volume(HideoutFogVolume {
        shape: if shape >= 1.0 { "sphere" } else { "box" }.into(),
        world_to_local,
        strength: strength as f32,
        falloff: falloff as f32,
        indirect_strength: (flag(entity.get("OverrideFogIndirectStrength")) == Some(true))
            .then(|| non_negative("FogIndirectStrength"))
            .flatten()
            .map(|value| value as f32),
        tint: (flag(entity.get("OverrideTintColor")) == Some(true))
            .then(|| vec3(entity.get("TintColor")))
            .flatten()
            .map(|color| color.map(|value| (value / 255.0) as f32)),
    })
}

/// Monotonic progress over `0.0..=1.0`, reported to an optional sink from any thread.
pub(crate) struct Meter<'a> {
    sink: Option<&'a (dyn Fn(f32) + Sync)>,
    high: Mutex<f32>,
}

impl<'a> Meter<'a> {
    pub(crate) fn new(sink: Option<&'a (dyn Fn(f32) + Sync)>) -> Self {
        Self {
            sink,
            high: Mutex::new(0.0),
        }
    }

    /// Report `value` when it is past everything reported so far.
    pub(crate) fn report(&self, value: f32) {
        let Some(sink) = self.sink else {
            return;
        };
        let value = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            return;
        };
        if let Ok(mut high) = self.high.lock()
            && value > *high
        {
            *high = value;
            // Under the lock, so two threads never report out of order.
            sink(value);
        }
    }

    pub(crate) fn span(&self) -> Span<'_, 'a> {
        Span {
            meter: self,
            start: 0.0,
            end: 1.0,
        }
    }
}

/// A part of a [`Meter`]'s range: `at(0.0)` is `start`, `at(1.0)` is `end`.
#[derive(Clone, Copy)]
pub(crate) struct Span<'m, 'a> {
    meter: &'m Meter<'a>,
    start: f32,
    end: f32,
}

impl<'m, 'a> Span<'m, 'a> {
    pub(crate) fn at(&self, fraction: f32) {
        self.meter
            .report(self.start + (self.end - self.start) * fraction.clamp(0.0, 1.0));
    }

    /// The part of this span from `from` to `to` (fractions of it).
    pub(crate) fn sub(&self, from: f32, to: f32) -> Span<'m, 'a> {
        let width = self.end - self.start;
        Span {
            meter: self.meter,
            start: self.start + width * from.clamp(0.0, 1.0),
            end: self.start + width * to.clamp(0.0, 1.0),
        }
    }

    pub(crate) fn done(&self) {
        self.at(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(json: serde_json::Value) -> serde_json::Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    #[test]
    fn viewer_vtv_header_round_trips() {
        let header = vtv_header([4, 2, 3], VtvKind::Rgba16F);
        assert_eq!(&header[..4], b"VTV1");
        let mut file = header.to_vec();
        file.resize(VTV_HEADER + 4 * 2 * 3 * 8, 0);
        assert_eq!(read_vtv_header(&file), Some(([4, 2, 3], VtvKind::Rgba16F)));
        // A short file, a wrong magic and an unknown kind are all rejected.
        assert_eq!(read_vtv_header(&file[..file.len() - 1]), None);
        let mut wrong = file.clone();
        wrong[0] = b'X';
        assert_eq!(read_vtv_header(&wrong), None);
        let mut unknown = file.clone();
        unknown[16] = 9;
        assert_eq!(read_vtv_header(&unknown), None);
        let mut r16 = vtv_header([2, 2, 1], VtvKind::R16).to_vec();
        r16.extend_from_slice(&[0; 8]);
        assert_eq!(read_vtv_header(&r16), Some(([2, 2, 1], VtvKind::R16)));
    }

    #[test]
    fn viewer_texture_budget_refuses_crafted_sizes() {
        // A header claiming 16384 x 16384 x 64 RGBA16F texels (128 GB) is refused unread.
        assert!(check_texture_budget(&[0u8; 64], 16384, 16384, 64, 8).is_err());
        // Overflowing products are refused, not wrapped.
        assert!(check_texture_budget(&[0u8; 64], u32::MAX, u32::MAX, u32::MAX, u64::MAX).is_err());
        // A file over the compiled cap is refused whatever its header says.
        assert!(check_texture_budget(&vec![0u8; MAX_COMPILED_TEXTURE + 1], 4, 4, 1, 4).is_err());
        assert!(texture_vtv(&vec![0u8; MAX_COMPILED_TEXTURE + 1]).is_err());
        assert!(texture_png(&vec![0u8; MAX_COMPILED_TEXTURE + 1], 64).is_err());
        // A real probe atlas fits.
        assert!(check_texture_budget(&[0u8; 64], 64, 64, 6 * 32, 8).is_ok());
    }

    #[test]
    fn viewer_texture_vtv_rejects_non_textures() {
        assert!(texture_vtv(b"not a texture").is_err());
    }

    #[test]
    fn viewer_cookie_files_are_stable_and_flat() {
        let file = cookie_file("materials/Lights/Cookie Fan.vtex");
        assert!(file.starts_with("cookies/cookie_fan_"));
        assert!(file.ends_with(".png"));
        assert_eq!(file.len(), "cookies/cookie_fan_".len() + 8 + 4);
        // Same path, same name; same stem in another folder, another name.
        assert_eq!(file, cookie_file("materials/lights/cookie fan.vtex_c"));
        assert_ne!(file, cookie_file("materials/other/cookie fan.vtex"));
    }

    fn apply(m: &[f32; 16], p: [f64; 3]) -> [f64; 3] {
        let m = m.map(f64::from);
        space::transform_point(&m, p)
    }

    #[test]
    fn viewer_world_to_local_maps_box_corners_to_0_and_1() {
        let volume = entity(serde_json::json!({
            "origin": [100, -50, 20], "angles": [0, 90, 0],
            "box_mins": [-10, -20, 0], "box_maxs": [30, 20, 40],
        }));
        let matrix = entity_world_to_local(&volume).unwrap();
        let placement = unscaled_matrix(&volume);
        let corner = |local: [f64; 3]| space::point(space::transform_point(&placement, local));
        let low = apply(&matrix, corner([-10.0, -20.0, 0.0]));
        let high = apply(&matrix, corner([30.0, 20.0, 40.0]));
        let mid = apply(&matrix, corner([10.0, 0.0, 20.0]));
        for axis in 0..3 {
            assert!(low[axis].abs() < 1e-6, "{low:?}");
            assert!((high[axis] - 1.0).abs() < 1e-6, "{high:?}");
            assert!((mid[axis] - 0.5).abs() < 1e-6, "{mid:?}");
        }
        // An empty box has no mapping.
        let flat = entity(serde_json::json!({
            "origin": [0, 0, 0], "box_mins": [0, 0, 0], "box_maxs": [1, 0, 1],
        }));
        assert!(entity_world_to_local(&flat).is_none());
    }

    #[test]
    fn viewer_probe_volume_reads_grimoire_fields() {
        let volume = entity(serde_json::json!({
            "classname": "env_combined_light_probe_volume",
            "lightprobetexture": "maps/dl_hideout/lpv.vtex", "lightprobetexture_dlshd": "maps/dl_hideout/lpv_shd.vtex",
            "origin": [0, 0, 0], "box_mins": [-64, -64, -32], "box_maxs": [64, 64, 32],
            "light_probe_size_x": 8, "light_probe_size_y": 8, "light_probe_size_z": 4,
            "light_probe_atlas_x": 16, "light_probe_atlas_Y": 0, "light_probe_atlas_Z": 4,
            "edge_fade_dists": [32, 0, 16], "indoor_outdoor_level": 2,
        }));
        let (texture, shadow, probe) = probe_volume(&volume, false).unwrap();
        assert_eq!(texture, "maps/dl_hideout/lpv.vtex");
        assert_eq!(shadow.as_deref(), Some("maps/dl_hideout/lpv_shd.vtex"));
        assert_eq!(probe.size, [8.0, 8.0, 4.0]);
        assert_eq!(probe.offset, [16.0, 0.0, 4.0]);
        assert_eq!(probe.fade, [4.0, 0.0, 4.0]);
        assert_eq!(probe.priority, 2);
        assert_eq!(probe.encoding, "linear");
        assert_eq!(probe_volume(&volume, true).unwrap().2.encoding, "rgbm");
        let mut missing = volume.clone();
        missing.remove("light_probe_size_z");
        assert!(probe_volume(&missing, false).is_none());
    }

    #[test]
    fn viewer_fog_volumes_follow_grimoire() {
        let base = serde_json::json!({
            "classname": "env_volumetric_fog_volume", "origin": [0, 0, 0],
            "box_mins": [-1, -1, -1], "box_maxs": [1, 1, 1], "Shape": 1,
            "FogStrength": 2.0, "FalloffExponent": 0.5,
            "OverrideFogIndirectStrength": true, "FogIndirectStrength": 0.25,
            "OverrideTintColor": false, "TintColor": [255, 0, 0],
        });
        let FogVolume::Volume(volume) = fog_volume(&entity(base.clone())) else {
            panic!("expected a volume");
        };
        assert_eq!(volume.shape, "sphere");
        assert_eq!(volume.strength, 2.0);
        assert_eq!(volume.falloff, 0.5);
        assert_eq!(volume.indirect_strength, Some(0.25));
        assert_eq!(volume.tint, None);
        let mut height = base.clone();
        height["Shape"] = serde_json::json!(2);
        assert_eq!(fog_volume(&entity(height)), FogVolume::HeightFog);
        let mut disabled = base;
        disabled["StartDisabled"] = serde_json::json!(true);
        assert_eq!(fog_volume(&entity(disabled)), FogVolume::Skip);
    }

    #[test]
    fn viewer_meter_is_monotonic_over_spans() {
        let seen = Mutex::new(Vec::new());
        let sink = |value: f32| seen.lock().unwrap().push(value);
        let meter = Meter::new(Some(&sink));
        let room = meter.span().sub(0.5, 1.0);
        room.at(0.5);
        room.at(0.2);
        meter.span().sub(0.0, 0.5).done();
        room.done();
        assert_eq!(*seen.lock().unwrap(), vec![0.75, 1.0]);
        Meter::new(None).span().done();
    }
}
