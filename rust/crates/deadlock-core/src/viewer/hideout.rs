//! The Hideout (`maps/dl_hideout`) as a scene file, its models as glbs, the hero at the spawn.
//!
//! Ported from Grimoire's `hideoutScene.ts`, `hideoutAtmosphere.ts` and `source2Entity.ts`
//! (MIT): world nodes and aggregate fragments, entity lumps (lights, the player spawn, model
//! props, looping animated props, sky, fog, the nearest reflection probe), the baked
//! lightmap (irradiance, plus the directional and shadow channels), light probe volumes,
//! light cookies, fog irradiance and density volumes, the master post volume (fixed
//! exposure, tone curve, bloom, color correction) and the camera behind the spawn. Grimoire
//! also merges the static models into one meshopt bundle and adds map particles; those are
//! listed in `omissions` instead of being faked.
//!
//! The room is cached per game build and per set of mods that change it. A mod changes the
//! room when it ships any file the unmodded room read (`deps.json`) or anything under
//! `maps/dl_hideout`; hero skins usually do not, so the modded room is usually the original.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod bake;
mod post;

use bake::{FogVolume, Meter, Span};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::cache::{self, Staging};
use super::hero::{HeroExportRequest, export_hero_with, export_slot};
use super::kv::{array, flag, num, resource_json, text, vec3};
use super::space::{self, Mat4};
use super::stack::{
    self, Source, SourceKind, SourceUsage, Stack, compiled, normalize, source_fingerprint,
};
use super::{PIPELINE_VERSION, SCHEMA, VPKMERGE_REV, Variant, check, citadel_dir, exporter_error};
use crate::error::{Error, Result};
use crate::import::Library;

const WORLD: &str = "maps/dl_hideout/world.vwrld_c";
const ROOM_FILE: &str = "room.json";
const DEPS_FILE: &str = "deps.json";
/// Part of the room's cache key: bumped when the room export changes shape, so an older
/// room folder is never served.
const ROOM_REVISION: &str = "hideout-2";
/// Longest edge of the directional and shadow lightmap PNGs.
const LIGHTMAP_EDGE: u32 = 2048;
/// Longest edge of a light cookie PNG.
const COOKIE_EDGE: u32 = 512;
/// Grimoire's fog volume limit.
const FOG_VOLUME_LIMIT: usize = 16;

/// One placed model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutModel {
    /// glb path relative to the scene file's folder.
    pub glb: String,
    /// Placement, glTF space, column major.
    pub transform: [f32; 16],
    /// sRGB multiplier and opacity, 0 to 1, or null for white.
    pub tint: Option<[f32; 4]>,
    /// Draw only this primitive of the glb (an aggregate fragment), or null for all of it.
    pub primitive: Option<u32>,
    /// glTF animation to loop, for animated props.
    pub animation: Option<String>,
}

/// One light that is not fully baked into the lightmap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutLight {
    /// `directional`, `point` or `spot`.
    pub kind: String,
    /// sRGB, 0 to 1.
    pub color: [f32; 3],
    /// Candela for point and spot lights; the authored brightness for directional.
    pub intensity: f32,
    /// Meters, glTF space.
    pub position: [f32; 3],
    /// The way the light shines, unit length, glTF space.
    pub direction: [f32; 3],
    /// Meters.
    pub range: f32,
    /// Spot inner and outer angle in degrees as authored, or null.
    pub cone: Option<[f32; 2]>,
    /// Source entity class: `light_omni2`, `light_barn`, `light_rect`, `light_environment`.
    pub classname: String,
    /// False for a baked light kept only because it lights fog.
    pub realtime: bool,
    /// Color temperature in kelvin when the light uses one, else null.
    pub temperature: Option<f32>,
    /// Baked shadow channel 0 to 3 for a stationary light, else null.
    pub shadow_channel: Option<i32>,
    pub fog_strength: Option<f32>,
    /// Linear brightness as the game's light shader takes it: `2^brightness *
    /// brightnessscale` for omni, barn and rect lights, `brightness * brightnessscale` for
    /// the sun.
    #[serde(default)]
    pub brightness: f32,
    /// Range falloff skirt (`skirt`, default 0.1) and near skirt (`skirt_near`, default 0),
    /// null for the sun.
    #[serde(default)]
    pub skirt: Option<f32>,
    #[serde(default)]
    pub skirt_near: Option<f32>,
    /// True when the light is baked and kept only because it lights fog (`!realtime`).
    #[serde(default)]
    pub fog_only: bool,
    /// Light cookie for an omni or barn light: an sRGB PNG relative to the scene folder, at
    /// most 512 px, or null.
    #[serde(default)]
    pub cookie: Option<String>,
    /// The light entity's own axes in glTF space, unit length: Source local +X (forward),
    /// +Y (left), +Z (up).
    #[serde(default)]
    pub basis: [[f32; 3]; 3],
}

/// A volume texture with the box it fills.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutVolume {
    /// VTV file relative to the scene folder.
    pub file: String,
    /// Column major: a glTF world point (meters) to the box's `[0, 1]^3`, in the entity's
    /// Source local axes (x forward, y left, z up).
    pub world_to_local: [f32; 16],
}

/// An `env_volumetric_fog_volume` density box or sphere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutFogVolume {
    /// `box` (shape 0) or `sphere` (shape 1).
    pub shape: String,
    /// Column major: a glTF world point to the volume's `[0, 1]^3`, Source local axes.
    pub world_to_local: [f32; 16],
    pub strength: f32,
    /// Falloff exponent.
    pub falloff: f32,
    /// Overrides the fog's indirect strength inside the volume, or null.
    pub indirect_strength: Option<f32>,
    /// Overrides the fog's tint inside the volume, 0 to 1, or null.
    pub tint: Option<[f32; 3]>,
}

/// One light probe volume. Every volume samples the same atlas (Grimoire supports one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutProbe {
    /// The irradiance atlas, VTV, relative to the scene folder. Its depth holds six equal
    /// bands, one per cube face (+X, +Y, +Z, -X, -Y, -Z, Source axes).
    pub file: String,
    /// The direct light shadow atlas (VTV), or null.
    pub shadows: Option<String>,
    /// Column major: a glTF world point to the volume's `[0, 1]^3`, Source local axes.
    pub world_to_local: [f32; 16],
    /// Probes per axis (`light_probe_size_x/y/z`).
    pub size: [f32; 3],
    /// Where the volume starts in the atlas, texels (`light_probe_atlas_x/Y/Z`).
    pub offset: [f32; 3],
    /// The atlas texture's width, height and depth (all six bands).
    pub atlas_size: [u32; 3],
    /// Box size over `edge_fade_dists` per axis; 0 on an axis without an edge fade.
    pub fade: [f32; 3],
    /// `indoor_outdoor_level`: higher wins where volumes overlap.
    pub priority: i32,
    /// `linear`, or `rgbm` for maps baked with game lightmap version 0.
    pub encoding: String,
}

/// The `.vpost` tone curve. `whitePoint` is in stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutTonemap {
    pub exposure_bias: f32,
    pub shoulder_strength: f32,
    pub linear_strength: f32,
    pub linear_angle: f32,
    pub toe_strength: f32,
    pub toe_num: f32,
    pub toe_denom: f32,
    pub white_point: f32,
}

/// The `.vpost` bloom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutBloom {
    /// `add`, `screen` or `blur`.
    pub blend_mode: String,
    pub bloom_strength: f32,
    pub screen_bloom_strength: f32,
    pub blur_bloom_strength: f32,
    pub threshold: f32,
    pub threshold_width: f32,
    pub blur_weights: [f32; 5],
    pub blur_tints: [[f32; 3]; 5],
}

/// The `.vpost` color correction volume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutColorCorrection {
    /// Raw RGBA8, `dimension^3 * 4` bytes, x fastest, then y, then z, as the vpost stores it.
    pub file: String,
    pub dimension: u32,
}

/// The master post processing volume's `.vpost`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutPost {
    pub tonemap: Option<HideoutTonemap>,
    pub bloom: Option<HideoutBloom>,
    pub color_correction: Option<HideoutColorCorrection>,
}

/// The master volumetric fog controller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutFog {
    /// sRGB tint, 0 to 1.
    pub color: [f32; 3],
    pub strength: f32,
    pub indirect_strength: f32,
    /// Meters.
    pub draw_distance: f32,
    /// Fade-in start and end, meters.
    pub fade_in: [f32; 2],
    /// The fog box: placement (glTF space) and local min and max (meters, glTF axes).
    pub transform: [f32; 16],
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// The `fogirradiancevolume` (VTV) over the fog box, or null.
    #[serde(default)]
    pub irradiance: Option<HideoutVolume>,
    /// Density volumes, at most 16.
    #[serde(default)]
    pub volumes: Vec<HideoutFogVolume>,
}

/// A cube map exported as six Radiance `.hdr` faces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutSky {
    /// `px, nx, py, ny, pz, nz` relative to the scene folder. The faces are in Source axes as
    /// vpkmerge writes them: `px` looks along Source +X, which is glTF +Z.
    pub cubemap: Vec<String>,
    /// `exp2` of this scales the sky (the sky material's exposure biases).
    pub exposure_bias: f32,
    /// Linear multiplier: the entity tint times its brightness scale.
    pub tint: [f32; 3],
    /// For a reflection probe: where it was captured, glTF space. Null for the sky.
    pub position: Option<[f32; 3]>,
}

/// The baked irradiance lightmap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutLightmap {
    /// Radiance `.hdr`, relative to the scene folder.
    pub file: String,
    pub uv_scale: [f32; 2],
    /// The glTF `TEXCOORD_n` the room glbs carry lightmap UVs in. morphic writes every
    /// stream with its Source semantic index, and Source 2 maps keep them in set 3. A
    /// primitive without it is not lightmapped (Grimoire lights it from the probes).
    #[serde(default = "lightmap_uv_set")]
    pub uv_set: u32,
    /// `directional_irradiance_sh2_lum` as a UNORM RGBA8 PNG (no sRGB), or null.
    #[serde(default)]
    pub direction: Option<String>,
    /// `direct_light_shadows` as a UNORM RGBA8 PNG, or null.
    #[serde(default)]
    pub shadows: Option<String>,
}

fn lightmap_uv_set() -> u32 {
    3
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutCamera {
    pub position: [f32; 3],
    pub target: [f32; 3],
    /// Vertical field of view in degrees.
    pub fov: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutSpawn {
    /// glTF space.
    pub position: [f32; 3],
    /// Source yaw in degrees (about Source Z, which is glTF Y).
    pub yaw_degrees: f32,
}

/// The hero placed in the room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutHero {
    pub hero: String,
    /// Relative to the scene folder.
    pub glb: String,
    pub sidecar: String,
    /// Same as the scene's `heroAnchor`.
    pub transform: [f32; 16],
    pub key: String,
}

/// The Hideout scene file. See `docs/design/3d-viewer.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HideoutScene {
    pub schema: u32,
    pub map: String,
    pub variant: Variant,
    pub key: String,
    pub units: String,
    pub up: String,
    /// The fixed exposure of the master post volume (1 when the map has none), absent when
    /// it adapts (then `autoExposure` is in `omissions`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f32>,
    /// The master post volume's tone curve, bloom and color correction, or null.
    #[serde(default)]
    pub post: Option<HideoutPost>,
    pub models: Vec<HideoutModel>,
    pub lights: Vec<HideoutLight>,
    pub fog: Option<HideoutFog>,
    pub sky: Option<HideoutSky>,
    pub reflection: Option<HideoutSky>,
    pub lightmap: Option<HideoutLightmap>,
    /// Light probe volumes, all on one atlas.
    #[serde(default)]
    pub probes: Vec<HideoutProbe>,
    pub camera: Option<HideoutCamera>,
    /// Where the hero stands: the player spawn turned to face the camera. glTF space.
    pub hero_anchor: Option<[f32; 16]>,
    pub hero: Option<HideoutHero>,
    pub spawn: Option<HideoutSpawn>,
    pub sources: Vec<SourceUsage>,
    /// Resources that failed to export. Their instances are left out.
    pub failures: Vec<String>,
    /// Parts of the game's scene this file does not carry.
    pub omissions: Vec<String>,
    /// The map's particle bundle (`particles/bundle.json`): its `info_particle_system`
    /// effects, or null when it has none.
    #[serde(default)]
    pub particles: Option<String>,
    pub pipeline: String,
}

/// Everything [`export_hideout_with`] takes.
#[derive(Clone)]
pub struct HideoutExportRequest<'a> {
    pub game_root: &'a Path,
    pub cache_dir: &'a Path,
    pub variant: Variant,
    /// Hero to stand at the spawn, codename or display name, exported with the same variant.
    pub hero: Option<&'a str>,
    pub extra_slots: &'a [u16],
    pub library: Option<&'a Library>,
    /// Longest texture edge for room models. Default 256, Grimoire's room cap.
    pub max_texture_edge: Option<u32>,
    /// Longest texture edge for the hero. Default 1024.
    pub hero_texture_edge: Option<u32>,
    /// Called with how far the export has come, 0.0 to 1.0, never going back, from worker
    /// threads too. A cache hit reports 1.0 at once.
    pub progress: Option<&'a (dyn Fn(f32) + Sync)>,
}

impl std::fmt::Debug for HideoutExportRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HideoutExportRequest")
            .field("game_root", &self.game_root)
            .field("cache_dir", &self.cache_dir)
            .field("variant", &self.variant)
            .field("hero", &self.hero)
            .field("extra_slots", &self.extra_slots)
            .field("library", &self.library.is_some())
            .field("max_texture_edge", &self.max_texture_edge)
            .field("hero_texture_edge", &self.hero_texture_edge)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

impl<'a> HideoutExportRequest<'a> {
    pub fn new(game_root: &'a Path, cache_dir: &'a Path, variant: Variant) -> Self {
        Self {
            game_root,
            cache_dir,
            variant,
            hero: None,
            extra_slots: &[],
            library: None,
            max_texture_edge: Some(256),
            hero_texture_edge: Some(1024),
            progress: None,
        }
    }
}

/// Export the Hideout, optionally with a hero at the spawn, and return the scene file.
pub fn export_hideout(
    game_root: &Path,
    cache_dir: &Path,
    variant: Variant,
    hero_choice: Option<&str>,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let mut request = HideoutExportRequest::new(game_root, cache_dir, variant);
    request.hero = hero_choice;
    export_hideout_with(&request, cancel)
}

/// Export the Hideout with every option. The scene file is `scene.json` without a hero and
/// `scene-<hero key>.json` with one, in the room's cache folder; relative paths in it resolve
/// against that folder.
pub fn export_hideout_with(
    request: &HideoutExportRequest<'_>,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    check(cancel)?;
    let citadel = citadel_dir(request.game_root);
    let map = citadel.join("maps").join("dl_hideout.vpk");
    if !map.is_file() {
        return Err(Error::msg(format!("{} is missing.", map.display())));
    }
    let mut game = vec![Source {
        kind: SourceKind::Map,
        slot: None,
        file: "maps/dl_hideout.vpk".into(),
        name: "Hideout".into(),
        path: map,
        enabled: true,
    }];
    game.extend(super::sources(request.game_root, Variant::Original, None));
    if !game.iter().any(|source| source.kind == SourceKind::Base) {
        return Err(Error::msg("citadel/pak01_dir.vpk is missing."));
    }

    let meter = Meter::new(request.progress);
    let whole = meter.span();
    let (hero_span, room_span) = if request.hero.is_some() {
        (whole.sub(0.0, 0.3), whole.sub(0.3, 1.0))
    } else {
        (whole.sub(0.0, 0.0), whole)
    };

    // The hero first: it takes the export slot on its own, and the room takes it after.
    let hero = match request.hero {
        Some(query) => {
            // The hero's own 0..1 lands in the hero's share.
            let hero_progress = |value: f32| hero_span.at(value);
            let mut hero_request = HeroExportRequest::new(
                request.game_root,
                request.cache_dir,
                query,
                request.variant,
            );
            hero_request.extra_slots = request.extra_slots;
            hero_request.library = request.library;
            hero_request.max_texture_edge = request.hero_texture_edge;
            if request.progress.is_some() {
                hero_request.progress = Some(&hero_progress);
            }
            let exported = export_hero_with(&hero_request, cancel)?;
            hero_span.done();
            Some(exported)
        }
        None => None,
    };

    // A modded room needs the original first (its `deps.json` says which mods matter). When
    // the original is not cached yet it takes most of the room's share.
    let original_cached = read_room(&room_dir(
        request.cache_dir,
        &game,
        Variant::Original,
        request.max_texture_edge,
    ))
    .is_some();
    let split = if request.variant == Variant::Original {
        1.0
    } else if original_cached {
        0.0
    } else {
        0.8
    };
    let original = ensure_room(
        request.cache_dir,
        &game,
        Variant::Original,
        request.max_texture_edge,
        cancel,
        room_span.sub(0.0, split),
    )?;
    let (room_dir, mut scene) = if request.variant == Variant::Modded {
        let mut mods: Vec<Source> = super::sources(
            request.game_root,
            Variant::Modded,
            Some(request.extra_slots),
        )
        .into_iter()
        .filter(|source| source.kind.is_mod())
        .collect();
        if let Some(library) = request.library {
            stack::name_sources(&mut mods, library);
        }
        let deps: BTreeSet<String> = std::fs::read(original.0.join(DEPS_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let relevant: Vec<Source> = mods
            .into_iter()
            .filter(|source| {
                stack::open_directory(&source.path).is_ok_and(|directory| {
                    directory
                        .by_path
                        .keys()
                        .any(|path| path.starts_with("maps/dl_hideout") || deps.contains(path))
                })
            })
            .collect();
        if relevant.is_empty() {
            original
        } else {
            let mut sources = relevant;
            sources.extend(game.iter().cloned());
            ensure_room(
                request.cache_dir,
                &sources,
                Variant::Modded,
                request.max_texture_edge,
                cancel,
                room_span.sub(split, 1.0),
            )?
        }
    } else {
        original
    };
    check(cancel)?;

    let file = match &hero {
        Some(exported) => {
            let codename = exported
                .glb
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            let transform = scene
                .hero_anchor
                .unwrap_or_else(|| space::to_f32(&space::IDENTITY));
            scene.hero = Some(HideoutHero {
                hero: codename.clone(),
                glb: format!("../../heroes/{}/{codename}.glb", exported.key),
                sidecar: format!("../../heroes/{}/{codename}.json", exported.key),
                transform,
                key: exported.key.clone(),
            });
            format!("scene-{}.json", exported.key)
        }
        None => "scene.json".to_string(),
    };
    let path = room_dir.join(file);
    cache::write_atomic(&path, &serde_json::to_vec_pretty(&scene)?)?;
    cache::touch(&room_dir);
    whole.done();
    Ok(path)
}

fn room_key(sources: &[Source], variant: Variant, max_texture_edge: Option<u32>) -> String {
    let mut parts = vec![
        PIPELINE_VERSION.to_string(),
        VPKMERGE_REV.to_string(),
        ROOM_REVISION.into(),
        variant.as_str().into(),
        format!("{max_texture_edge:?}"),
    ];
    parts.extend(sources.iter().map(source_fingerprint));
    cache::key(&parts)
}

fn room_dir(
    cache_dir: &Path,
    sources: &[Source],
    variant: Variant,
    max_texture_edge: Option<u32>,
) -> PathBuf {
    cache_dir
        .join("hideout")
        .join(room_key(sources, variant, max_texture_edge))
}

/// The room for a stack, built once per key.
fn ensure_room(
    cache_dir: &Path,
    sources: &[Source],
    variant: Variant,
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
    span: Span<'_, '_>,
) -> Result<(PathBuf, HideoutScene)> {
    let key = room_key(sources, variant, max_texture_edge);
    let parent = cache_dir.join("hideout");
    let dir = parent.join(&key);
    if let Some(scene) = read_room(&dir) {
        cache::touch(&dir);
        span.done();
        return Ok((dir, scene));
    }
    let _slot = export_slot();
    if let Some(scene) = read_room(&dir) {
        span.done();
        return Ok((dir, scene));
    }
    check(cancel)?;
    let stack = Stack::open(sources)?.record();
    let staging = Staging::begin(&parent, &key)?;
    let mut scene = build_room(&stack, max_texture_edge, cancel, &staging.dir, span)?;
    scene.key = key;
    scene.variant = variant;
    scene.sources = stack.usage();
    std::fs::write(
        staging.dir.join(ROOM_FILE),
        serde_json::to_vec_pretty(&scene)?,
    )?;
    std::fs::write(
        staging.dir.join(DEPS_FILE),
        serde_json::to_vec(&stack.recorded())?,
    )?;
    let dir = staging.commit()?;
    span.done();
    Ok((dir, scene))
}

fn read_room(dir: &Path) -> Option<HideoutScene> {
    let scene: HideoutScene =
        serde_json::from_slice(&std::fs::read(dir.join(ROOM_FILE)).ok()?).ok()?;
    (scene.schema == SCHEMA && scene.pipeline == PIPELINE_VERSION).then_some(scene)
}

/// What one model export request is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ModelRequest {
    entry: String,
    /// Clip baked as a still, `ref` for the bind pose.
    pose: String,
    /// Clip kept and looped instead of baked.
    clip: Option<String>,
    material_group: Option<String>,
    overrides: Vec<(usize, u32, String)>,
}

impl ModelRequest {
    fn file(&self) -> String {
        let parts = vec![
            self.entry.clone(),
            self.pose.clone(),
            self.clip.clone().unwrap_or_default(),
            self.material_group.clone().unwrap_or_default(),
            format!("{:?}", self.overrides),
        ];
        format!("models/{}.glb", cache::key(&parts))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Static,
    Animated,
    /// The model has no meshes (a helper). Not a failure.
    Empty,
    Failed(String),
}

struct Placement {
    request: usize,
    transform: Mat4,
    tint: Option<[f32; 4]>,
    primitive: Option<u32>,
}

#[derive(Default)]
struct Requests {
    list: Vec<ModelRequest>,
    index: BTreeMap<ModelRequest, usize>,
}

impl Requests {
    fn add(&mut self, request: ModelRequest) -> usize {
        if let Some(index) = self.index.get(&request) {
            return *index;
        }
        let index = self.list.len();
        self.index.insert(request.clone(), index);
        self.list.push(request);
        index
    }
}

const OMISSIONS: &[&str] = &[
    "lightProbes",
    "particles",
    "colorCorrection",
    "tonemap",
    "autoExposure",
    "fogVolumes",
    "fogIrradiance",
    "directionalLightmap",
    "lightCookies",
    "staticBundle",
    // The precomputed shadow atlas tiles fog lights sample (R16, decodable, but the scene
    // file has no field for the atlas or the per-light tiles yet).
    "fogShadows",
];

/// Where each stage of a room build ends, as a share of the build. Models take the rest.
const STAGE_WORLD: f32 = 0.03;
const STAGE_LIGHTMAP: f32 = 0.10;
const STAGE_ENTITIES: f32 = 0.15;
const STAGE_LIGHTS: f32 = 0.22;
const STAGE_VOLUMES: f32 = 0.32;
const STAGE_REFLECTION: f32 = 0.35;
const STAGE_NODES: f32 = 0.40;
const STAGE_MODELS: f32 = 0.98;

fn build_room(
    stack: &Stack,
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
    out: &Path,
    span: Span<'_, '_>,
) -> Result<HideoutScene> {
    let world_bytes = stack
        .read(WORLD)
        .ok_or_else(|| Error::msg(format!("{WORLD} is not in the Hideout VPK.")))?;
    let world = resource_json(&world_bytes, WORLD)?;
    span.at(STAGE_WORLD);
    let mut scene = HideoutScene {
        schema: SCHEMA,
        map: "dl_hideout".into(),
        variant: Variant::Original,
        key: String::new(),
        units: "meters".into(),
        up: "+Y".into(),
        exposure: None,
        post: None,
        models: Vec::new(),
        lights: Vec::new(),
        fog: None,
        sky: None,
        reflection: None,
        lightmap: None,
        probes: Vec::new(),
        camera: None,
        hero_anchor: None,
        hero: None,
        spawn: None,
        sources: Vec::new(),
        failures: Vec::new(),
        omissions: OMISSIONS.iter().map(|item| item.to_string()).collect(),
        particles: None,
        pipeline: PIPELINE_VERSION.into(),
    };

    // Baked irradiance.
    let lighting = world.get("m_worldLightingInfo");
    if let Some(lighting) = lighting
        && flag(lighting.get("m_bHasLightmaps")) == Some(true)
        && let Some(irradiance) = array(lighting.get("m_lightMaps"))
            .iter()
            .filter_map(Json::as_str)
            .find(|entry| entry.ends_with("/directional_irradiance_sh2_dc.vtex"))
    {
        let uv = array(lighting.get("m_vLightmapUvScale"));
        let uv_scale = [
            num(uv.first()).unwrap_or(1.0) as f32,
            num(uv.get(1)).unwrap_or(1.0) as f32,
        ];
        let target = out.join("lightmap").join("irradiance.hdr");
        let exported = with_resource(stack, irradiance, out, |file| {
            std::fs::create_dir_all(out.join("lightmap"))?;
            bake::check_texture_file(file)?;
            vpkmerge_core::cubemap::export_texture_hdr(file, None, &target, 2048)
                .map_err(|error| exporter_error("lightmap", error))
        });
        match exported {
            Ok(()) => {
                let mut lightmap = HideoutLightmap {
                    file: "lightmap/irradiance.hdr".into(),
                    uv_scale,
                    uv_set: lightmap_uv_set(),
                    direction: None,
                    shadows: None,
                };
                // The SH2 luminance and shadow channels: only for the bake version Grimoire
                // decodes (hideoutScene.ts), as data PNGs.
                let version = |key: &str| num(lighting.get(key));
                if version("m_nLightmapVersionNumber") == Some(8.0)
                    && version("m_nLightmapGameVersionNumber") == Some(5.0)
                {
                    let find = |suffix: &str| {
                        array(lighting.get("m_lightMaps"))
                            .iter()
                            .filter_map(Json::as_str)
                            .find(|entry| entry.ends_with(suffix))
                    };
                    for (suffix, file, slot) in [
                        (
                            "/directional_irradiance_sh2_lum.vtex",
                            "lightmap/direction.png",
                            &mut lightmap.direction,
                        ),
                        (
                            "/direct_light_shadows.vtex",
                            "lightmap/shadows.png",
                            &mut lightmap.shadows,
                        ),
                    ] {
                        let Some(entry) = find(suffix) else {
                            continue;
                        };
                        match write_png(stack, entry, &out.join(file), LIGHTMAP_EDGE) {
                            Ok(()) => *slot = Some(file.to_string()),
                            Err(_) => scene.failures.push(compiled(entry)),
                        }
                    }
                }
                scene.lightmap = Some(lightmap);
            }
            Err(_) => scene.failures.push(compiled(irradiance)),
        }
    }
    span.at(STAGE_LIGHTMAP);
    check(cancel)?;

    // Entities, through child lumps.
    let mut entities: Vec<serde_json::Map<String, Json>> = Vec::new();
    let mut lumps: VecDeque<String> = array(world.get("m_entityLumps"))
        .iter()
        .filter_map(|lump| lump.as_str().map(str::to_string))
        .collect();
    let mut visited = BTreeSet::new();
    while let Some(lump) = lumps.pop_front() {
        if visited.len() >= 256 || !visited.insert(lump.clone()) {
            continue;
        }
        let Some(bytes) = stack.read(&compiled(&lump)) else {
            scene.failures.push(compiled(&lump));
            continue;
        };
        let Ok(decoded) = resource_json(&bytes, &lump) else {
            scene.failures.push(compiled(&lump));
            continue;
        };
        lumps.extend(
            array(decoded.get("m_childLumps"))
                .iter()
                .filter_map(|child| child.as_str().map(str::to_string)),
        );
        for item in array(decoded.get("m_entityKeyValues")) {
            if let Some(values) = item
                .get("keyValues3Data")
                .and_then(|data| data.get("values"))
                .and_then(Json::as_object)
            {
                entities.push(values.clone());
            }
        }
    }
    span.at(STAGE_ENTITIES);
    check(cancel)?;

    // The map's particle effects (info_particle_system), read through the same stack.
    match super::particles::export_map_particles(stack, &entities, out, cancel) {
        Ok(found) => scene.particles = found,
        Err(error) if super::is_cancelled(&error) => return Err(error),
        Err(error) => scene.failures.push(format!("particles: {error}")),
    }

    // The player spawn.
    let spawn = entities
        .iter()
        .find(|entity| {
            text(entity.get("classname")) == Some("info_team_spawn")
                && text(entity.get("targetname")).is_some_and(|name| name.contains("player_spawn"))
        })
        .or_else(|| {
            entities
                .iter()
                .find(|entity| text(entity.get("classname")) == Some("info_team_spawn"))
        });
    let anchor = spawn.and_then(|entity| vec3(entity.get("origin")));
    let yaw = spawn
        .and_then(|entity| vec3(entity.get("angles")))
        .map(|angles| angles[1])
        .unwrap_or(0.0);
    if let Some(origin) = anchor {
        apply_spawn(&mut scene, origin, yaw);
    } else {
        scene.omissions.push("playerSpawn".into());
    }

    let mut requests = Requests::default();
    let mut placements: Vec<Placement> = Vec::new();
    let mut animated: BTreeSet<usize> = BTreeSet::new();
    let mut cookies: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut pending = Pending::default();
    let mut post_volume: Option<&serde_json::Map<String, Json>> = None;
    let mut fog_controller: Option<&serde_json::Map<String, Json>> = None;
    let rgbm = lighting.and_then(|lighting| num(lighting.get("m_nLightmapGameVersionNumber")))
        == Some(0.0);
    // The probe atlas every volume samples, its shadow atlas, and the volumes on it.
    let mut probe_atlas: Option<(String, Option<String>)> = None;
    let mut probe_volumes: Vec<HideoutProbe> = Vec::new();

    for entity in &entities {
        let classname = text(entity.get("classname")).unwrap_or_default();
        if let Some(mut light) = map_light(entity) {
            if matches!(classname, "light_omni2" | "light_barn")
                && let Some(cookie) = text(entity.get("lightcookie"))
            {
                let file = cookies
                    .entry(compiled(cookie))
                    .or_insert_with(|| {
                        let file = bake::cookie_file(cookie);
                        write_png(stack, cookie, &out.join(&file), COOKIE_EDGE)
                            .ok()
                            .map(|()| file)
                    })
                    .clone();
                if file.is_none() {
                    pending.cookies_failed = true;
                    scene.failures.push(compiled(cookie));
                }
                light.cookie = file;
            }
            scene.lights.push(light);
        }
        if classname == "post_processing_volume"
            && flag(entity.get("master")) == Some(true)
            && post_volume.is_none()
        {
            post_volume = Some(entity);
        }
        if let Some((texture, shadow, volume)) = bake::probe_volume(entity, rgbm) {
            match &probe_atlas {
                Some((atlas, _)) if *atlas != texture => {
                    scene.omissions.push("multipleProbeAtlases".into());
                }
                _ => {
                    probe_atlas.get_or_insert((texture, shadow));
                    probe_volumes.push(volume);
                }
            }
        }
        match classname {
            "env_sky" if flag(entity.get("StartDisabled")) != Some(true) && scene.sky.is_none() => {
                match export_sky(stack, entity, out) {
                    Ok(sky) => scene.sky = sky,
                    Err(_) => scene.failures.push("sky".into()),
                }
            }
            "env_volumetric_fog_controller"
                if flag(entity.get("IsMaster")) != Some(false)
                    && flag(entity.get("StartDisabled")) != Some(true)
                    && scene.fog.is_none() =>
            {
                scene.fog = map_fog(entity);
                if scene.fog.is_some() {
                    fog_controller = Some(entity);
                }
            }
            _ => {}
        }
        if renderable(entity) {
            if text(entity.get("parentname")).is_some() {
                // Attachment-local transforms need the parent's posed attachment frame.
                scene.omissions.push("entityAttachments".into());
                continue;
            }
            let Some(model) = text(entity.get("model")) else {
                continue;
            };
            let starting = text(entity.get("StartingAnim"));
            let pose = starting
                .or_else(|| text(entity.get("IdleAnim")))
                .or_else(|| text(entity.get("defaultanim")))
                .unwrap_or("ref")
                .to_string();
            let material_group = text(entity.get("skin"))
                .filter(|skin| *skin != "default")
                .map(str::to_string);
            let looping =
                text(entity.get("StartingAnimationLoopMode")) == Some("ANIM_LOOP_MODE_LOOPING");
            let clip = starting.filter(|_| looping).map(str::to_string);
            let color = vec3(entity.get("rendercolor")).unwrap_or([255.0; 3]);
            let alpha = num(entity.get("renderamt")).unwrap_or(255.0);
            let index = requests.add(ModelRequest {
                entry: compiled(model),
                pose,
                clip: clip.clone(),
                material_group,
                overrides: Vec::new(),
            });
            if clip.is_some() {
                animated.insert(index);
            }
            placements.push(Placement {
                request: index,
                transform: entity_matrix(entity, true),
                tint: Some([
                    (color[0] / 255.0) as f32,
                    (color[1] / 255.0) as f32,
                    (color[2] / 255.0) as f32,
                    (alpha / 255.0) as f32,
                ]),
                primitive: None,
            });
        }
    }

    // The master post volume: fixed exposure and its vpost. A map without one is lit at 1.
    match post_volume {
        Some(entity) => {
            scene.exposure = post::fixed_exposure(entity);
            if let Some(path) = text(entity.get("postprocessing")) {
                match export_post(stack, path, out) {
                    Ok(exported) => scene.post = Some(exported),
                    Err(_) => scene.failures.push(compiled(path)),
                }
            }
        }
        None => scene.exposure = Some(1.0),
    }
    span.at(STAGE_LIGHTS);
    check(cancel)?;

    // Fog irradiance and density volumes, on the master controller.
    if let (Some(fog), Some(controller)) = (scene.fog.as_mut(), fog_controller) {
        for entity in &entities {
            match bake::fog_volume(entity) {
                FogVolume::Volume(volume) => {
                    if fog.volumes.len() == FOG_VOLUME_LIMIT {
                        scene.omissions.push("fogVolumeLimit".into());
                        break;
                    }
                    fog.volumes.push(volume);
                }
                FogVolume::HeightFog => scene.omissions.push("heightFogVolumes".into()),
                FogVolume::Skip => {}
            }
        }
        if let Some(texture) = text(controller.get("fogirradiancevolume")) {
            let file = "fog/irradiance.vtv";
            let exported = bake::entity_world_to_local(controller)
                .ok_or_else(|| Error::msg("fog box"))
                .and_then(|world_to_local| {
                    write_vtv(stack, texture, &out.join(file))?;
                    Ok(world_to_local)
                });
            match exported {
                Ok(world_to_local) => {
                    fog.irradiance = Some(HideoutVolume {
                        file: file.into(),
                        world_to_local,
                    })
                }
                Err(_) => scene.failures.push(compiled(texture)),
            }
        }
    }

    // Light probe volumes, on one atlas.
    if let Some((texture, shadow)) = &probe_atlas {
        let file = "probes/0.vtv";
        match write_vtv(stack, texture, &out.join(file)) {
            Ok(atlas_size) => {
                let shadows = shadow.as_ref().and_then(|shadow| {
                    let file = "probes/0-shadows.vtv";
                    match write_vtv(stack, shadow, &out.join(file)) {
                        Ok(_) => Some(file.to_string()),
                        Err(_) => {
                            scene.failures.push(compiled(shadow));
                            None
                        }
                    }
                });
                for mut volume in probe_volumes {
                    volume.file = file.into();
                    volume.shadows.clone_from(&shadows);
                    volume.atlas_size = atlas_size;
                    scene.probes.push(volume);
                }
            }
            Err(_) => scene.failures.push(compiled(texture)),
        }
    }
    span.at(STAGE_VOLUMES);
    check(cancel)?;

    // The nearest reflection probe to the spawn.
    let origin = anchor.unwrap_or([0.0; 3]);
    let probe = entities
        .iter()
        .filter(|entity| {
            matches!(
                text(entity.get("classname")),
                Some("env_cubemap" | "env_cubemap_box" | "env_combined_light_probe_volume")
            )
        })
        .filter_map(|entity| {
            Some((
                text(entity.get("cubemaptexture"))?,
                num(entity.get("array_index"))? as u16,
                vec3(entity.get("origin"))?,
            ))
        })
        .min_by(|a, b| distance2(a.2, origin).total_cmp(&distance2(b.2, origin)));
    if let Some((texture, slice, position)) = probe {
        let faces_dir = out.join("reflection");
        let exported = with_resource(stack, texture, out, |file| {
            bake::check_texture_file(file)?;
            vpkmerge_core::cubemap::export_cubemap_hdr_at(file, None, &faces_dir, slice)
                .map_err(|error| exporter_error("reflection", error))
        });
        match exported {
            Ok(faces) => {
                scene.reflection = Some(HideoutSky {
                    cubemap: faces
                        .iter()
                        .map(|face| format!("reflection/{}.hdr", face.face))
                        .collect(),
                    exposure_bias: 0.0,
                    tint: [1.0; 3],
                    position: Some(space::point(position).map(|value| value as f32)),
                })
            }
            Err(_) => scene.failures.push(compiled(texture)),
        }
    }
    span.at(STAGE_REFLECTION);
    check(cancel)?;

    // World nodes: static scene objects and aggregate fragments.
    for node in array(world.get("m_worldNodes")) {
        let Some(prefix) = text(node.get("m_worldNodePrefix")) else {
            continue;
        };
        let entry = format!("{}.vwnod_c", normalize(prefix));
        let Some(bytes) = stack.read(&entry) else {
            scene.failures.push(entry);
            continue;
        };
        let Ok(decoded) = resource_json(&bytes, &entry) else {
            scene.failures.push(entry);
            continue;
        };
        if !array(decoded.get("m_aggregateInstanceStreams")).is_empty() {
            scene.omissions.push("instanceStreams".into());
        }
        let ordinary = array(decoded.get("m_sceneObjects"));
        let overrides = array(decoded.get("m_materialOverrides"));
        for (index, object) in ordinary
            .iter()
            .chain(array(decoded.get("m_aggregateSceneObjects")))
            .enumerate()
        {
            let Some(model) = text(object.get("m_renderableModel")) else {
                continue;
            };
            // Overrides index ordinary scene objects, not aggregate fragments; order is kept for
            // VRF's precedence.
            let own: Vec<(usize, u32, String)> = if index < ordinary.len() {
                overrides
                    .iter()
                    .filter(|item| num(item.get("m_nSceneObjectIndex")) == Some(index as f64))
                    .filter_map(|item| {
                        Some((
                            num(item.get("m_nSubSceneObject"))? as usize,
                            num(item.get("m_nDrawCallIndex"))? as u32,
                            text(item.get("m_pMaterial"))?.to_string(),
                        ))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            // A world object never animates: rigged ones bake like any static prop.
            let request = requests.add(ModelRequest {
                entry: compiled(model),
                pose: "ref".into(),
                clip: None,
                material_group: None,
                overrides: own,
            });
            let tint = array(object.get("m_vTintColor"));
            let object_tint = [
                num(tint.first()).unwrap_or(1.0) as f32,
                num(tint.get(1)).unwrap_or(1.0) as f32,
                num(tint.get(2)).unwrap_or(1.0) as f32,
                1.0,
            ];
            let fragments = array(object.get("m_aggregateMeshes"));
            if object.get("m_aggregateMeshes").is_some() {
                let transforms = array(object.get("m_fragmentTransforms"));
                let mut next = 0usize;
                for fragment in fragments {
                    let Some(draw_call) = num(fragment.get("m_nDrawCallIndex")) else {
                        continue;
                    };
                    let mut transform = space::IDENTITY;
                    if flag(fragment.get("m_bHasTransform")) == Some(true) {
                        let values: Vec<f64> = array(transforms.get(next))
                            .iter()
                            .filter_map(|v| num(Some(v)))
                            .collect();
                        next += 1;
                        if values.len() != 12 {
                            scene
                                .failures
                                .push(format!("{}: missing fragment transform", compiled(model)));
                            continue;
                        }
                        transform = [
                            values[0], values[4], values[8], 0.0, values[1], values[5], values[9],
                            0.0, values[2], values[6], values[10], 0.0, values[3], values[7],
                            values[11], 1.0,
                        ];
                    }
                    let fragment_tint = vec3(fragment.get("m_vTintColor")).unwrap_or([255.0; 3]);
                    placements.push(Placement {
                        request,
                        transform,
                        tint: Some([
                            (fragment_tint[0] / 255.0) as f32,
                            (fragment_tint[1] / 255.0) as f32,
                            (fragment_tint[2] / 255.0) as f32,
                            1.0,
                        ]),
                        primitive: Some(draw_call as u32),
                    });
                }
            } else {
                placements.push(Placement {
                    request,
                    transform: object_rows(object.get("m_vTransform"))
                        .map_or(space::IDENTITY, |rows| space::rows_matrix(&rows)),
                    tint: (object_tint != [1.0; 4]).then_some(object_tint),
                    primitive: None,
                });
            }
        }
    }
    span.at(STAGE_NODES);
    check(cancel)?;

    // Export every distinct model in parallel.
    let outcomes = export_models(
        stack,
        &requests.list,
        max_texture_edge,
        cancel,
        out,
        span.sub(STAGE_NODES, STAGE_MODELS),
    )?;
    for (index, outcome) in outcomes.iter().enumerate() {
        if let Outcome::Failed(reason) = outcome {
            scene
                .failures
                .push(format!("{}: {reason}", requests.list[index].entry));
        }
        if animated.contains(&index) && *outcome == Outcome::Static {
            scene.omissions.push("animatedProps".into());
        }
    }
    for placement in placements {
        let request = &requests.list[placement.request];
        let outcome = &outcomes[placement.request];
        if !matches!(outcome, Outcome::Static | Outcome::Animated) {
            continue;
        }
        scene.models.push(HideoutModel {
            glb: request.file(),
            transform: space::to_f32(&space::to_gltf(&placement.transform)),
            tint: placement.tint,
            primitive: placement.primitive,
            animation: (*outcome == Outcome::Animated)
                .then(|| request.clip.clone())
                .flatten(),
        });
    }
    if scene.models.is_empty() {
        return Err(Error::msg("No Hideout geometry could be exported."));
    }
    settle_omissions(&mut scene, &pending);
    scene.omissions.sort();
    scene.omissions.dedup();
    scene.failures.sort();
    scene.failures.dedup();
    Ok(scene)
}

/// Camera behind the spawn as the game's third-person camera, hero turned to face it.
/// Grimoire's oracle cameras: 180 units out, eye at 62, target at 42; a 60 degree field.
fn apply_spawn(scene: &mut HideoutScene, origin: [f64; 3], yaw: f64) {
    let radians = yaw.to_radians();
    let forward = [radians.cos(), radians.sin(), 0.0];
    let eye = [
        origin[0] - forward[0] * 180.0,
        origin[1] - forward[1] * 180.0,
        origin[2] + 62.0,
    ];
    let target = [origin[0], origin[1], origin[2] + 42.0];
    scene.camera = Some(HideoutCamera {
        position: space::point(eye).map(|value| value as f32),
        target: space::point(target).map(|value| value as f32),
        fov: 60.0,
    });
    scene.spawn = Some(HideoutSpawn {
        position: space::point(origin).map(|value| value as f32),
        yaw_degrees: yaw as f32,
    });
    let anchor = space::entity_matrix(origin, [0.0, yaw + 180.0, 0.0], [1.0; 3]);
    scene.hero_anchor = Some(space::to_f32(&space::to_gltf(&anchor)));
}

fn distance2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

fn object_rows(value: Option<&Json>) -> Option<[[f64; 4]; 3]> {
    let rows = array(value);
    if rows.len() < 3 {
        return None;
    }
    let mut out = [[0.0; 4]; 3];
    for (row, values) in out.iter_mut().zip(rows) {
        let values = array(Some(values));
        if values.len() < 4 {
            return None;
        }
        for (cell, value) in row.iter_mut().zip(values) {
            *cell = num(Some(value))?;
        }
    }
    Some(out)
}

/// A model entity that draws: it names a model and an origin, carries render state and does
/// not start disabled. Brush triggers and post volumes also name models but carry no render
/// state. Grimoire's `renderableModelEntity`.
fn renderable(entity: &serde_json::Map<String, Json>) -> bool {
    text(entity.get("model")).is_some()
        && vec3(entity.get("origin")).is_some()
        && num(entity.get("renderamt")).is_some_and(|alpha| alpha > 0.0)
        && flag(entity.get("StartDisabled")) != Some(true)
}

fn entity_matrix(entity: &serde_json::Map<String, Json>, scaled: bool) -> Mat4 {
    let origin = vec3(entity.get("origin")).unwrap_or([0.0; 3]);
    let angles = vec3(entity.get("angles")).unwrap_or([0.0; 3]);
    let scales = if scaled {
        vec3(entity.get("scales")).unwrap_or([1.0; 3])
    } else {
        [1.0; 3]
    };
    space::entity_matrix(origin, angles, scales)
}

/// Source 2 static lights with a baked shadow channel are stationary; other static lights are
/// entirely in the lightmap. Grimoire's `isRealtimeMapLight`.
pub(crate) fn realtime_light(direct: f64, shadow_channel: f64) -> bool {
    direct != 0.0 && (direct != 1.0 || (0.0..=3.0).contains(&shadow_channel))
}

pub(crate) fn map_light(entity: &serde_json::Map<String, Json>) -> Option<HideoutLight> {
    let classname = text(entity.get("classname"))?;
    if !matches!(
        classname,
        "light_omni2" | "light_barn" | "light_rect" | "light_environment"
    ) {
        return None;
    }
    if flag(entity.get("enabled")) == Some(false) {
        return None;
    }
    let origin = vec3(entity.get("origin"))?;
    let color = vec3(entity.get("color"))?;
    let range = num(entity.get("range")).filter(|range| *range > 0.0)?;
    let shadow = num(entity.get("bakedshadowindex")).or_else(|| num(entity.get("bakelightindex")));
    let realtime = realtime_light(
        num(entity.get("directlight")).unwrap_or(2.0),
        shadow.unwrap_or(-1.0),
    );
    let fog_strength = match (num(entity.get("fog")), num(entity.get("fogstrength"))) {
        (Some(fog), Some(strength)) if fog != 0.0 && strength != 0.0 => Some(strength),
        _ => None,
    };
    if !realtime && fog_strength.is_none() {
        return None;
    }
    let scale = num(entity.get("brightnessscale")).unwrap_or(1.0);
    let candela = num(entity.get("brightness_candelas"))
        .or_else(|| {
            num(entity.get("brightness_lumens")).map(|lumens| lumens / (4.0 * std::f64::consts::PI))
        })
        .unwrap_or(0.0)
        * scale;
    let (kind, intensity, cone) = match classname {
        "light_environment" => (
            "directional",
            num(entity.get("brightness")).unwrap_or(1.0) * scale,
            None,
        ),
        "light_barn" => (
            "spot",
            candela,
            Some([
                num(entity.get("inner_angle")).unwrap_or(0.0) as f32,
                num(entity.get("outer_angle")).unwrap_or(180.0) as f32,
            ]),
        ),
        "light_rect" => ("spot", candela, Some([0.0, 90.0])),
        _ => ("point", candela, None),
    };
    let matrix = entity_matrix(entity, false);
    let sun = classname == "light_environment";
    let brightness = if sun {
        num(entity.get("brightness")).unwrap_or(1.0)
    } else {
        2f64.powf(num(entity.get("brightness")).unwrap_or(0.0))
    } * scale;
    let basis: [[f32; 3]; 3] =
        std::array::from_fn(|index| space::axis(&matrix, index).map(|value| value as f32));
    Some(HideoutLight {
        kind: kind.into(),
        color: color.map(|value| (value / 255.0) as f32),
        intensity: intensity as f32,
        position: space::point(origin).map(|value| value as f32),
        direction: space::direction(space::forward(&matrix)).map(|value| value as f32),
        range: (range * space::METERS_PER_INCH) as f32,
        cone,
        classname: classname.into(),
        realtime,
        temperature: (num(entity.get("colormode")).unwrap_or(0.0) != 0.0)
            .then(|| num(entity.get("colortemperature")).unwrap_or(6600.0) as f32),
        shadow_channel: shadow
            .filter(|channel| (0.0..=3.0).contains(channel))
            .map(|channel| channel as i32),
        fog_strength: fog_strength.map(|value| value as f32),
        brightness: brightness as f32,
        skirt: (!sun).then(|| num(entity.get("skirt")).unwrap_or(0.1) as f32),
        skirt_near: (!sun).then(|| num(entity.get("skirt_near")).unwrap_or(0.0) as f32),
        fog_only: !realtime,
        cookie: None,
        basis,
    })
}

/// What was asked for but failed, for [`settle_omissions`].
#[derive(Debug, Default)]
struct Pending {
    /// A light names a cookie that could not be exported.
    cookies_failed: bool,
}

/// Take out of `omissions` what the scene now carries. What failed stays.
fn settle_omissions(scene: &mut HideoutScene, pending: &Pending) {
    let lightmap = scene.lightmap.as_ref();
    let post = scene.post.as_ref();
    let exported = |item: &str| match item {
        "directionalLightmap" => lightmap.is_some_and(|lightmap| lightmap.direction.is_some()),
        "colorCorrection" => post.is_some_and(|post| post.color_correction.is_some()),
        "tonemap" => post.is_some_and(|post| post.tonemap.is_some()),
        "autoExposure" => scene.exposure.is_some(),
        "lightCookies" => !pending.cookies_failed,
        "lightProbes" => !scene.probes.is_empty(),
        "fogVolumes" => scene.fog.is_some(),
        "fogIrradiance" => scene
            .fog
            .as_ref()
            .is_some_and(|fog| fog.irradiance.is_some()),
        "particles" => scene.particles.is_some(),
        _ => false,
    };
    let kept: Vec<String> = scene
        .omissions
        .iter()
        .filter(|item| !exported(item))
        .cloned()
        .collect();
    scene.omissions = kept;
}

/// A compiled texture from the stack as a PNG at `target`, bytes as stored.
fn write_png(stack: &Stack, entry: &str, target: &Path, max_edge: u32) -> Result<()> {
    let bytes = stack
        .read(&compiled(entry))
        .ok_or_else(|| Error::msg(format!("{} is not in the stack.", compiled(entry))))?;
    let png = bake::texture_png(&bytes, max_edge)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, png)?;
    Ok(())
}

/// A compiled volume texture from the stack as a VTV file at `target`; its dimensions.
fn write_vtv(stack: &Stack, entry: &str, target: &Path) -> Result<[u32; 3]> {
    let bytes = stack
        .read(&compiled(entry))
        .ok_or_else(|| Error::msg(format!("{} is not in the stack.", compiled(entry))))?;
    let (vtv, dims, _) = bake::texture_vtv(&bytes)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, vtv)?;
    Ok(dims)
}

/// The master post volume's `.vpost`: tone curve, bloom, and the color correction volume
/// written to `post/color-correction.bin`.
fn export_post(stack: &Stack, path: &str, out: &Path) -> Result<HideoutPost> {
    let entry = compiled(path);
    let bytes = stack
        .read(&entry)
        .ok_or_else(|| Error::msg(format!("{entry} is not in the stack.")))?;
    let vpost = resource_json(&bytes, &entry)?;
    let color_correction = match post::color_correction(&vpost).map_err(Error::msg)? {
        Some((dimension, texels)) => {
            let file = "post/color-correction.bin";
            std::fs::create_dir_all(out.join("post"))?;
            std::fs::write(out.join(file), texels)?;
            Some(HideoutColorCorrection {
                file: file.into(),
                dimension,
            })
        }
        None => None,
    };
    Ok(HideoutPost {
        tonemap: post::tonemap(&vpost),
        bloom: post::bloom(&vpost),
        color_correction,
    })
}

fn map_fog(entity: &serde_json::Map<String, Json>) -> Option<HideoutFog> {
    if flag(entity.get("IndirectEnabled")) == Some(false) {
        return None;
    }
    let min = vec3(entity.get("box_mins"))?;
    let max = vec3(entity.get("box_maxs"))?;
    let tint = vec3(entity.get("TintColor")).unwrap_or([255.0; 3]);
    let meters = |value: f64| (value * space::METERS_PER_INCH) as f32;
    Some(HideoutFog {
        color: tint.map(|value| (value / 255.0) as f32),
        strength: num(entity.get("FogStrength")).unwrap_or(1.0) as f32,
        indirect_strength: num(entity.get("IndirectStrength")).unwrap_or(1.0) as f32,
        draw_distance: meters(num(entity.get("DrawDistance")).unwrap_or(0.0)),
        fade_in: [
            meters(num(entity.get("FadeInStart")).unwrap_or(0.0)),
            meters(num(entity.get("FadeInEnd")).unwrap_or(0.0)),
        ],
        transform: space::to_f32(&space::to_gltf(&entity_matrix(entity, false))),
        min: space::point(min).map(|value| value as f32),
        max: space::point(max).map(|value| value as f32),
        irradiance: None,
        volumes: Vec::new(),
    })
}

/// The enabled `env_sky`'s cube, with its sky shader exposure (Grimoire's `exportMapSky`).
fn export_sky(
    stack: &Stack,
    entity: &serde_json::Map<String, Json>,
    out: &Path,
) -> Result<Option<HideoutSky>> {
    let Some(material_path) = text(entity.get("skyname")) else {
        return Ok(None);
    };
    let Some(bytes) = stack.read(&compiled(material_path)) else {
        return Ok(None);
    };
    let material = resource_json(&bytes, material_path)?;
    let Some(texture) = array(material.get("m_textureParams"))
        .iter()
        .find(|param| text(param.get("m_name")) == Some("g_tSkyTexture"))
        .and_then(|param| text(param.get("m_pValue")))
    else {
        return Ok(None);
    };
    let bias = |name: &str| {
        array(material.get("m_floatParams"))
            .iter()
            .find(|param| text(param.get("m_name")) == Some(name))
            .and_then(|param| num(param.get("m_flValue")))
            .unwrap_or(0.0)
    };
    let faces_dir = out.join("sky");
    let faces = with_resource(stack, texture, out, |file| {
        bake::check_texture_file(file)?;
        vpkmerge_core::cubemap::export_cubemap_hdr(file, None, &faces_dir)
            .map_err(|error| exporter_error("sky", error))
    })?;
    let scale = num(entity.get("brightnessscale")).unwrap_or(1.0);
    let tint = vec3(entity.get("tint_color")).unwrap_or([255.0; 3]);
    Ok(Some(HideoutSky {
        cubemap: faces
            .iter()
            .map(|face| format!("sky/{}.hdr", face.face))
            .collect(),
        exposure_bias: (bias("g_flBrightnessExposureBias") + bias("g_flRenderOnlyExposureBias"))
            as f32,
        tint: tint.map(|value| (value / 255.0 * scale) as f32),
        position: None,
    }))
}

/// Run a vpkmerge file exporter on a resource from the stack, through a scratch file (the
/// exporters read a path, and the winning copy may be a mod's).
fn with_resource<T>(
    stack: &Stack,
    entry: &str,
    scratch: &Path,
    run: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let entry = compiled(entry);
    let bytes = stack
        .read(&entry)
        .ok_or_else(|| Error::msg(format!("{entry} is not in the stack.")))?;
    let extension = entry.rsplit('.').next().unwrap_or("bin");
    let file = scratch.join(format!(
        ".resource-{}.{extension}",
        cache::key(std::slice::from_ref(&entry))
    ));
    std::fs::write(&file, bytes)?;
    let result = run(&file);
    let _ = std::fs::remove_file(&file);
    result
}

/// The most models exported at once.
const MAX_EXPORT_WORKERS: usize = 4;

fn export_models(
    stack: &Stack,
    requests: &[ModelRequest],
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
    out: &Path,
    span: Span<'_, '_>,
) -> Result<Vec<Outcome>> {
    std::fs::create_dir_all(out.join("models"))?;
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let total = requests.len().max(1) as f32;
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new(vec![None; requests.len()]);
    // Each worker holds a decoded model and its textures at once; four bound the peak memory on
    // a machine with many cores (eight decoding the largest props together ran short).
    let workers = std::thread::available_parallelism()
        .map_or(4, |count| count.get())
        .clamp(1, MAX_EXPORT_WORKERS);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(request) = requests.get(index) else {
                        return;
                    };
                    // A model the decoder panics on is one failed prop, not a failed room.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        export_model(stack, request, max_texture_edge, out)
                    }))
                    .unwrap_or_else(|_| Err(Error::msg("the exporter stopped on this model")))
                    .unwrap_or_else(|error| Outcome::Failed(error.to_string()));
                    if let Ok(mut guard) = results.lock() {
                        guard[index] = Some(outcome);
                    }
                    let count = finished.fetch_add(1, Ordering::Relaxed) + 1;
                    span.at(count as f32 / total);
                }
            });
        }
    });
    check(cancel)?;
    let results = results
        .into_inner()
        .map_err(|_| Error::msg("Hideout export worker panicked."))?;
    Ok(results
        .into_iter()
        .map(|outcome| outcome.unwrap_or_else(|| Outcome::Failed("not exported".into())))
        .collect())
}

fn export_model(
    stack: &Stack,
    request: &ModelRequest,
    max_texture_edge: Option<u32>,
    out: &Path,
) -> Result<Outcome> {
    let bytes = stack
        .read(&request.entry)
        .ok_or_else(|| Error::msg("not in the stack"))?;
    let variant = morphic::model::ModelVariant {
        material_group: request.material_group.clone(),
        draw_material_overrides: request
            .overrides
            .iter()
            .map(
                |(mesh_index, draw_call_index, material)| morphic::model::DrawMaterialOverride {
                    mesh_index: *mesh_index,
                    draw_call_index: *draw_call_index,
                    material: material.clone(),
                },
            )
            .collect(),
        ..Default::default()
    };
    let mut model = morphic::model::decode_variant(&bytes, &variant)
        .map_err(|error| exporter_error("decoding", error))?;
    if model.meshes.is_empty() {
        return Ok(Outcome::Empty);
    }
    let mut outcome = Outcome::Static;
    if let Some(clip) = &request.clip {
        model
            .animations
            .retain(|animation| animation.name.eq_ignore_ascii_case(clip));
        if model.animations.is_empty() {
            // Without its clip the prop still stands, in the pose it starts in.
            model = morphic::model::bake_pose(&model, &[request.pose.as_str()], 0);
        } else {
            for animation in &mut model.animations {
                animation.name.clone_from(clip);
                animation.looping = true;
            }
            outcome = Outcome::Animated;
        }
    } else {
        model = morphic::model::bake_pose(&model, &[request.pose.as_str()], 0);
    }
    let options = morphic::model::GlbOptions {
        max_texture_edge,
        include_outline_shells: false,
    };
    let glb = morphic::model::to_glb_textured(&model, stack, options)
        .map_err(|error| exporter_error("writing", error))?;
    std::fs::write(out.join(request.file()), glb)?;
    Ok(outcome)
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    fn entity(json: Json) -> serde_json::Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    #[test]
    fn viewer_hideout_light_brightness_rule() {
        let omni = map_light(&entity(serde_json::json!({
            "classname": "light_omni2", "origin": [0, 0, 0], "color": [255, 255, 255], "range": 100,
            "brightness": 2, "brightnessscale": 0.5, "skirt": 0.3, "directlight": 2,
        })))
        .unwrap();
        assert_eq!(omni.brightness, 2.0);
        assert_eq!(omni.skirt, Some(0.3));
        assert_eq!(omni.skirt_near, Some(0.0));
        assert!(!omni.fog_only);
        assert_eq!(omni.cookie, None);

        let barn = map_light(&entity(serde_json::json!({
            "classname": "light_barn", "origin": [0, 0, 0], "angles": [0, 90, 0], "color": [255, 255, 255],
            "range": 100, "directlight": 2,
        })))
        .unwrap();
        // No brightness: 2^0, scale 1. Default skirts.
        assert_eq!(barn.brightness, 1.0);
        assert_eq!(barn.skirt, Some(0.1));
        // Yaw 90: forward is Source +Y (glTF +X), left Source -X (glTF -Z), up glTF +Y.
        let close = |a: [f32; 3], b: [f32; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5);
        assert!(close(barn.basis[0], [1.0, 0.0, 0.0]), "{:?}", barn.basis);
        assert!(close(barn.basis[1], [0.0, 0.0, -1.0]), "{:?}", barn.basis);
        assert!(close(barn.basis[2], [0.0, 1.0, 0.0]), "{:?}", barn.basis);
        assert!(close(barn.direction, barn.basis[0]));

        let sun = map_light(&entity(serde_json::json!({
            "classname": "light_environment", "origin": [0, 0, 0], "color": [255, 255, 255], "range": 100,
            "brightness": 3, "brightnessscale": 2, "directlight": 2,
        })))
        .unwrap();
        assert_eq!(sun.brightness, 6.0);
        assert_eq!(sun.skirt, None);
        assert_eq!(sun.skirt_near, None);

        let fog = map_light(&entity(serde_json::json!({
            "classname": "light_omni2", "origin": [0, 0, 0], "color": [255, 255, 255], "range": 100,
            "directlight": 1, "fog": 1, "fogstrength": 0.5,
        })))
        .unwrap();
        assert!(fog.fog_only && !fog.realtime);
    }

    fn scene() -> HideoutScene {
        HideoutScene {
            schema: SCHEMA,
            map: "dl_hideout".into(),
            variant: Variant::Original,
            key: String::new(),
            units: "meters".into(),
            up: "+Y".into(),
            exposure: None,
            post: None,
            models: Vec::new(),
            lights: Vec::new(),
            fog: None,
            sky: None,
            reflection: None,
            lightmap: None,
            probes: Vec::new(),
            camera: None,
            hero_anchor: None,
            hero: None,
            spawn: None,
            sources: Vec::new(),
            failures: Vec::new(),
            omissions: OMISSIONS.iter().map(|item| item.to_string()).collect(),
            particles: None,
            pipeline: PIPELINE_VERSION.into(),
        }
    }

    #[test]
    fn viewer_hideout_omissions_drop_what_is_exported() {
        // Nothing exported: everything stays except cookies, which stay only when one failed.
        let mut bare = scene();
        settle_omissions(&mut bare, &Pending::default());
        for item in [
            "lightProbes",
            "particles",
            "colorCorrection",
            "tonemap",
            "autoExposure",
            "fogVolumes",
            "fogIrradiance",
            "directionalLightmap",
            "staticBundle",
        ] {
            assert!(bare.omissions.iter().any(|kept| kept == item), "{item}");
        }
        assert!(!bare.omissions.iter().any(|kept| kept == "lightCookies"));

        let mut full = scene();
        full.exposure = Some(1.0);
        full.post = Some(HideoutPost {
            tonemap: post::tonemap(&serde_json::json!({
                "m_bHasTonemapParams": true,
                "m_toneMapParams": { "m_flExposureBias": 0, "m_flShoulderStrength": 0, "m_flLinearStrength": 0,
                    "m_flLinearAngle": 0, "m_flToeStrength": 0, "m_flToeNum": 0, "m_flToeDenom": 1, "m_flWhitePoint": 2 },
            })),
            bloom: None,
            color_correction: Some(HideoutColorCorrection {
                file: "post/color-correction.bin".into(),
                dimension: 32,
            }),
        });
        full.lightmap = Some(HideoutLightmap {
            file: "lightmap/irradiance.hdr".into(),
            uv_scale: [1.0, 1.0],
            uv_set: 3,
            direction: Some("lightmap/direction.png".into()),
            shadows: Some("lightmap/shadows.png".into()),
        });
        full.probes.push(HideoutProbe {
            file: "probes/0.vtv".into(),
            shadows: None,
            world_to_local: [0.0; 16],
            size: [1.0; 3],
            offset: [0.0; 3],
            atlas_size: [1, 1, 6],
            fade: [0.0; 3],
            priority: 0,
            encoding: "linear".into(),
        });
        full.fog = Some(HideoutFog {
            color: [1.0; 3],
            strength: 1.0,
            indirect_strength: 1.0,
            draw_distance: 1.0,
            fade_in: [0.0, 1.0],
            transform: [0.0; 16],
            min: [0.0; 3],
            max: [1.0; 3],
            irradiance: Some(HideoutVolume {
                file: "fog/irradiance.vtv".into(),
                world_to_local: [0.0; 16],
            }),
            volumes: Vec::new(),
        });
        settle_omissions(&mut full, &Pending::default());
        assert_eq!(
            full.omissions,
            vec!["particles", "staticBundle", "fogShadows"]
        );

        let mut failed = scene();
        settle_omissions(
            &mut failed,
            &Pending {
                cookies_failed: true,
            },
        );
        assert!(failed.omissions.iter().any(|kept| kept == "lightCookies"));
    }

    #[test]
    fn viewer_hideout_scene_json_shape() {
        let mut auto = scene();
        auto.lightmap = Some(HideoutLightmap {
            file: "lightmap/irradiance.hdr".into(),
            uv_scale: [1.0, 1.0],
            uv_set: 3,
            direction: None,
            shadows: None,
        });
        let json = serde_json::to_value(&auto).unwrap();
        // Auto exposure: the field is left out; post is null; probes an empty list.
        assert!(json.get("exposure").is_none());
        assert!(json["post"].is_null());
        assert_eq!(json["probes"], serde_json::json!([]));
        assert_eq!(json["lightmap"]["uvSet"], 3);
        assert!(json["lightmap"]["direction"].is_null());
        assert!(json["lightmap"]["shadows"].is_null());
        auto.exposure = Some(1.0);
        let json = serde_json::to_value(&auto).unwrap();
        assert_eq!(json["exposure"], 1.0);
        let back: HideoutScene = serde_json::from_value(json).unwrap();
        assert_eq!(back, auto);

        let light = map_light(&entity(serde_json::json!({
            "classname": "light_omni2", "origin": [0, 0, 0], "color": [255, 255, 255], "range": 100,
        })))
        .unwrap();
        let json = serde_json::to_value(&light).unwrap();
        for key in [
            "brightness",
            "skirt",
            "skirtNear",
            "fogOnly",
            "cookie",
            "basis",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }
        let probe = bake::probe_volume(
            &entity(serde_json::json!({
                "lightprobetexture": "a.vtex", "origin": [0, 0, 0], "box_mins": [0, 0, 0], "box_maxs": [1, 1, 1],
                "light_probe_size_x": 1, "light_probe_size_y": 1, "light_probe_size_z": 1,
            })),
            false,
        )
        .unwrap()
        .2;
        let json = serde_json::to_value(&probe).unwrap();
        for key in [
            "file",
            "shadows",
            "worldToLocal",
            "size",
            "offset",
            "atlasSize",
            "fade",
            "priority",
            "encoding",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }
        let fog = HideoutFogVolume {
            shape: "box".into(),
            world_to_local: [0.0; 16],
            strength: 1.0,
            falloff: 1.0,
            indirect_strength: None,
            tint: None,
        };
        let json = serde_json::to_value(&fog).unwrap();
        for key in [
            "shape",
            "worldToLocal",
            "strength",
            "falloff",
            "indirectStrength",
            "tint",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }
        let json = serde_json::to_value(HideoutColorCorrection {
            file: "post/color-correction.bin".into(),
            dimension: 32,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "file": "post/color-correction.bin", "dimension": 32 })
        );
    }
}
