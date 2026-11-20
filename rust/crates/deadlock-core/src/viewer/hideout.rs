//! The Hideout (`maps/dl_hideout`) as a scene file, its models as glbs, the hero at the spawn.
//!
//! Ported from Grimoire's `hideoutScene.ts`, `hideoutAtmosphere.ts` and `source2Entity.ts`
//! (MIT): world nodes and aggregate fragments, entity lumps (lights, the player spawn, model
//! props, looping animated props, sky, fog, the nearest reflection probe), the baked
//! irradiance lightmap, and the camera behind the spawn. Grimoire then merges the static
//! models into one meshopt bundle and adds probes, particles and a tone curve; those are
//! listed in `omissions` instead of being faked.
//!
//! The room is cached per game build and per set of mods that change it. A mod changes the
//! room when it ships any file the unmodded room read (`deps.json`) or anything under
//! `maps/dl_hideout`; hero skins usually do not, so the modded room is usually the original.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    pub models: Vec<HideoutModel>,
    pub lights: Vec<HideoutLight>,
    pub fog: Option<HideoutFog>,
    pub sky: Option<HideoutSky>,
    pub reflection: Option<HideoutSky>,
    pub lightmap: Option<HideoutLightmap>,
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
    pub pipeline: String,
}

/// Everything [`export_hideout_with`] takes.
#[derive(Debug, Clone)]
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

    // The hero first: it takes the export slot on its own, and the room takes it after.
    let hero = match request.hero {
        Some(query) => {
            let mut hero_request = HeroExportRequest::new(
                request.game_root,
                request.cache_dir,
                query,
                request.variant,
            );
            hero_request.extra_slots = request.extra_slots;
            hero_request.library = request.library;
            hero_request.max_texture_edge = request.hero_texture_edge;
            Some(export_hero_with(&hero_request, cancel)?)
        }
        None => None,
    };

    let original = ensure_room(
        request.cache_dir,
        &game,
        Variant::Original,
        request.max_texture_edge,
        cancel,
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
    Ok(path)
}

/// The room for a stack, built once per key.
fn ensure_room(
    cache_dir: &Path,
    sources: &[Source],
    variant: Variant,
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, HideoutScene)> {
    let mut parts = vec![
        PIPELINE_VERSION.to_string(),
        VPKMERGE_REV.to_string(),
        "hideout".into(),
        variant.as_str().into(),
        format!("{max_texture_edge:?}"),
    ];
    parts.extend(sources.iter().map(source_fingerprint));
    let key = cache::key(&parts);
    let parent = cache_dir.join("hideout");
    let dir = parent.join(&key);
    if let Some(scene) = read_room(&dir) {
        cache::touch(&dir);
        return Ok((dir, scene));
    }
    let _slot = export_slot();
    if let Some(scene) = read_room(&dir) {
        return Ok((dir, scene));
    }
    check(cancel)?;
    let stack = Stack::open(sources)?.record();
    let staging = Staging::begin(&parent, &key)?;
    let mut scene = build_room(&stack, max_texture_edge, cancel, &staging.dir)?;
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
];

fn build_room(
    stack: &Stack,
    max_texture_edge: Option<u32>,
    cancel: &AtomicBool,
    out: &Path,
) -> Result<HideoutScene> {
    let world_bytes = stack
        .read(WORLD)
        .ok_or_else(|| Error::msg(format!("{WORLD} is not in the Hideout VPK.")))?;
    let world = resource_json(&world_bytes, WORLD)?;
    let mut scene = HideoutScene {
        schema: SCHEMA,
        map: "dl_hideout".into(),
        variant: Variant::Original,
        key: String::new(),
        units: "meters".into(),
        up: "+Y".into(),
        models: Vec::new(),
        lights: Vec::new(),
        fog: None,
        sky: None,
        reflection: None,
        lightmap: None,
        camera: None,
        hero_anchor: None,
        hero: None,
        spawn: None,
        sources: Vec::new(),
        failures: Vec::new(),
        omissions: OMISSIONS.iter().map(|item| item.to_string()).collect(),
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
            vpkmerge_core::cubemap::export_texture_hdr(file, None, &target, 2048)
                .map_err(|error| exporter_error("lightmap", error))
        });
        match exported {
            Ok(()) => {
                scene.lightmap = Some(HideoutLightmap {
                    file: "lightmap/irradiance.hdr".into(),
                    uv_scale,
                })
            }
            Err(_) => scene.failures.push(compiled(irradiance)),
        }
    }
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
    check(cancel)?;

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

    for entity in &entities {
        let classname = text(entity.get("classname")).unwrap_or_default();
        if let Some(light) = map_light(entity) {
            scene.lights.push(light);
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
    check(cancel)?;

    // Export every distinct model in parallel.
    let outcomes = export_models(stack, &requests.list, max_texture_edge, cancel, out)?;
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
) -> Result<Vec<Outcome>> {
    std::fs::create_dir_all(out.join("models"))?;
    let next = AtomicUsize::new(0);
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
