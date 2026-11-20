//! Particle bundles for the 3D viewer: the effects a hero carries and the Hideout's map
//! particles, written as `particles/bundle.json` plus `particles/tex/*.png` beside a hero
//! sidecar or a Hideout scene file.
//!
//! The file contract is the "Particle bundle" section of `docs/design/3d-viewer.md` (schema 1). Every
//! position and matrix in a bundle is Source space (inches, Z up, X forward), as the game data
//! is; the renderer applies the Source to glTF conversion and the hero or scene placement.
//!
//! Ported from Grimoire (MIT): `heroAmbientDiscovery.ts` (which effects a hero plays, found in
//! its model keyvalues, `scripts/heroes.vdata` and its primary weapon), `heroParticleBundle.ts`
//! (the breadth-first system tree, renderer textures, the limits), `particleModelExport.ts`
//! (model attachments), `hideoutParticles.ts` (`readMapParticles`), `source2Entity.ts` (the
//! entity matrix), `kv3Text.ts` (model keyvalues), `stringToken.ts` and the sheet reader in
//! `render/sheet.ts`. The last two follow ValveResourceFormat (MIT): `StringToken.Get` and
//! `Texture.GetSpriteSheetData`.
//!
//! Every read goes through the preview's VPK stack, so a mod that replaces a particle system
//! or its textures shows in the bundle and counts as used. Nothing here writes outside the
//! output folder the caller passes.

// The exports are called from hero.rs and hideout.rs once they are wired in; drop this then.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use super::kv::{self, array, flag, num, text, vec3};
use super::space::{self, Mat4};
use super::stack::{Stack, compiled, normalize};
use super::{SCHEMA, check};
use crate::error::Result;

/// Where a bundle is written, relative to the hero or Hideout folder. The sidecar and scene
/// field `particles` names this path.
pub(crate) const BUNDLE_PATH: &str = "particles/bundle.json";

/// Grimoire's bundle limits (`heroParticleBundle.ts`).
const MAX_SYSTEMS: usize = 160;
/// Children deeper than this below a root are left out. Roots are depth 0.
const MAX_DEPTH: usize = 12;
const MAX_TEXTURES: usize = 128;
/// Longest texture edge written.
const MAX_TEXTURE_EDGE: u32 = 1024;
/// A compiled texture larger than this is not decoded.
const MAX_TEXTURE_BYTES: usize = 64 * 1024 * 1024;

/// The texture a spritecard renderer draws when it names none (Grimoire's
/// `RENDERER_DEFAULT_TEXTURES`).
const RENDERER_DEFAULT_TEXTURES: [(&str, &str); 3] = [
    ("C_OP_RenderSprites", "materials/particle/base_sprite.vtex"),
    ("C_OP_RenderRopes", "materials/particle/base_trail.vtex"),
    ("C_OP_RenderTrails", "materials/particle/base_trail.vtex"),
];

/// Weapon fields naming a particle the game's code spawns on attachments no data names, with
/// the authored preview system that places it. Used only while the preview's children are the
/// named particle, so a mod that renames either side is not misattributed.
const CODE_SPAWNED_WEAPON_PARTICLES: [(&str, &str); 1] = [(
    "m_BatonFlameParticle",
    "particles/abilities/unicorn/unicorn_ambient_weapon.vpcf",
)];

// The bundle file (schema 1).

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Bundle {
    pub schema: u32,
    /// The hero codename, or `dl_hideout`.
    pub owner: String,
    /// Every system in the effects' trees, keyed `particles/x/y.vpcf` (lowercase, no `_c`).
    pub systems: BTreeMap<String, BundleSystem>,
    /// Every texture the systems' renderers name, keyed by the path as authored.
    pub textures: BTreeMap<String, BundleTexture>,
    pub attachments: Vec<Attachment>,
    pub effects: Vec<Effect>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BundleSystem {
    /// The raw KV3 DATA of the `.vpcf_c` as JSON, before any format upgrade.
    pub data: Json,
    /// The stored KV3 format name (`vpcf61` to `vpcf66`), or null when unknown.
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BundleTexture {
    /// Relative to the bundle file: `tex/<name>.png`.
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub sheet: Option<Sheet>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sheet {
    pub sequences: Vec<SheetSequence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SheetSequence {
    /// The sequence number a particle reaches this sequence with (VRF's
    /// `SpritesheetData.GetSequenceIndex`): the authored id for an unnamed sequence, the number
    /// for one named with digits only, and -1 for any other named one.
    pub id: i64,
    pub clamp: bool,
    pub alpha_crop: bool,
    pub no_color: bool,
    pub no_alpha: bool,
    /// The span the sequence plays over, seconds, as stored (0 when it has none of its own).
    pub duration: f32,
    pub frames: Vec<SheetFrame>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SheetFrame {
    /// Seconds.
    pub duration: f32,
    /// The frame's uncropped art rects `[u0, v0, u1, v1]`, 0 to 1, top-left origin.
    pub images: Vec<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attachment {
    pub name: String,
    pub ignore_rotation: bool,
    pub influences: Vec<Influence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Influence {
    pub bone: String,
    pub offset: [f64; 3],
    /// Quaternion `[x, y, z, w]`.
    pub rotation: [f64; 4],
    pub weight: f64,
    pub root_transform: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectAttachment {
    /// The lowercased PATTACH name without `pattach_` (`point_follow`, `absorigin_follow`, ...).
    #[serde(rename = "type")]
    pub kind: String,
    pub point: String,
    pub offset: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ControlPoint {
    pub index: u32,
    /// A `cpointN` target's entity matrix, Source space, column major.
    pub transform: Option<[f64; 16]>,
    /// A `tint_cp` colour or `data_cp` value written as the control point's position.
    pub value: Option<[f64; 3]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Effect {
    pub id: String,
    /// The root system, a key of `systems`.
    pub system: String,
    /// `model-particles-list`, `model-particle-cfg`, `model-ambient`, `hero-ambient`,
    /// `weapon-passive` or `map-entity`.
    pub source: String,
    pub attachment: Option<EffectAttachment>,
    pub attachment_cp1: Option<EffectAttachment>,
    /// The `m_controlPointConfigurations` entry to use, or null.
    pub configuration: Option<String>,
    /// Map effects: the entity matrix, Source space, column major. Null for hero effects.
    pub transform: Option<[f64; 16]>,
    pub control_points: Vec<ControlPoint>,
    pub start_active: bool,
    pub seed: u32,
}

// Hero effects.

/// Export the particle effects `hero_codename` carries: its model keyvalues
/// (`particle_cfg_list`, `particles_list`, `CCitadelHeroModelGameData_t.m_hAmbientParticle`),
/// its `scripts/heroes.vdata` `m_hAmbientParticle` (a match ambient: kept with `startActive`
/// false, as Grimoire leaves it out of the showcase), and its primary weapon's
/// `m_PassiveWeaponParticle` and code-spawned particles. `model_entry` and `model_bytes` are the
/// body model the glb was built from; its attachments go into the bundle.
///
/// Writes `out_dir/particles/bundle.json` and `out_dir/particles/tex/*.png` and returns
/// `Some("particles/bundle.json")`, or `None` (writing nothing) when the hero carries no effect
/// whose system is in the stack. Missing files are bundle issues, not errors; only a write
/// failure or `cancel` is an error.
pub(crate) fn export_hero_particles(
    stack: &Stack,
    hero_codename: &str,
    model_entry: &str,
    model_bytes: &[u8],
    out_dir: &Path,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    check(cancel)?;
    let mut found = Effects::default();
    let mut issues = Vec::new();

    // The model's keyvalues.
    let model = kv::resource_json(model_bytes, model_entry).ok();
    if model.is_none() {
        issues.push(format!("unreadable:{}", normalize(model_entry)));
    }
    if let Some(model) = &model {
        let key_value_text = model
            .get("m_modelInfo")
            .and_then(|info| text(info.get("m_keyValueText")));
        if let Some(key_value_text) = key_value_text {
            match parse_kv3_text(key_value_text) {
                Ok(key_values) => model_keyvalue_effects(&key_values, &mut found),
                Err(_) => issues.push(format!("keyValues:{}", normalize(model_entry))),
            }
        }
    }

    // heroes.vdata and the primary weapon.
    let decode = |path: &str| {
        stack
            .read(&compiled(path))
            .and_then(|bytes| kv::resource_json(&bytes, path).ok())
    };
    let hero_key = if hero_codename.starts_with("hero_") {
        hero_codename.to_ascii_lowercase()
    } else {
        format!("hero_{}", hero_codename.to_ascii_lowercase())
    };
    let heroes = decode("scripts/heroes.vdata");
    let hero = heroes.as_ref().and_then(|heroes| heroes.get(&hero_key));
    match hero {
        None => issues.push(format!("missing:scripts/heroes.vdata {hero_key}")),
        Some(hero) => {
            if let Some(ambient) = text(hero.get("m_hAmbientParticle")) {
                found.add(ambient, "hero-ambient", None, None, false);
            }
            let weapon_key = hero
                .get("m_mapBoundAbilities")
                .and_then(|slots| text(slots.get("ESlot_Weapon_Primary")));
            if let Some(weapon_key) = weapon_key {
                let abilities = decode("scripts/abilities.vdata");
                let weapon = abilities
                    .as_ref()
                    .and_then(|abilities| abilities.get(weapon_key));
                if let Some(weapon) = weapon {
                    weapon_effects(weapon, &mut found, &mut issues, &|path| decode(path));
                }
            }
        }
    }
    issues.append(&mut found.issues);
    if found.effects.is_empty() {
        return Ok(None);
    }
    check(cancel)?;

    let attachments = model
        .as_ref()
        .map(|model| model_attachments(stack, model_bytes, model))
        .unwrap_or_default();
    write_bundle(
        stack,
        hero_codename.to_ascii_lowercase(),
        found.effects,
        attachments,
        issues,
        out_dir,
        cancel,
    )
}

/// Collects discovered effects: validates and keys the system path, drops duplicates.
#[derive(Default)]
struct Effects {
    effects: Vec<Effect>,
    issues: Vec<String>,
}

impl Effects {
    fn add(
        &mut self,
        system: &str,
        source: &str,
        attachment: Option<EffectAttachment>,
        configuration: Option<String>,
        start_active: bool,
    ) {
        let key = system_key(system);
        if !valid_system(&key) {
            self.issues.push(format!("invalidSystem:{system}"));
            return;
        }
        if self
            .effects
            .iter()
            .any(|effect| effect.system == key && effect.source == source)
        {
            return;
        }
        self.effects.push(Effect {
            id: format!("ambient_{}", self.effects.len()),
            seed: seed(&key, [0.0; 3]),
            system: key,
            source: source.to_string(),
            attachment,
            attachment_cp1: None,
            configuration,
            transform: None,
            control_points: Vec::new(),
            start_active,
        });
    }
}

/// The effects a model's keyvalues (parsed `m_modelInfo.m_keyValueText`) declare.
fn model_keyvalue_effects(key_values: &Json, found: &mut Effects) {
    for entry in array(key_values.get("particle_cfg_list")) {
        if let Some(name) = text(entry.get("name")) {
            let config = text(entry.get("config")).map(str::to_string);
            found.add(name, "model-particle-cfg", None, config, true);
        }
    }
    for entry in array(key_values.get("particles_list")) {
        if let Some(name) = text(entry.get("name")) {
            let attachment = attachment_of(
                entry.get("attachment_type"),
                entry.get("attachment_point"),
                entry.get("attachment_offset"),
            );
            found.add(name, "model-particles-list", attachment, None, true);
        }
    }
    let ambient = key_values
        .get("CCitadelHeroModelGameData_t")
        .and_then(|data| text(data.get("m_hAmbientParticle")));
    if let Some(ambient) = ambient {
        found.add(ambient, "model-ambient", None, None, true);
    }
}

/// The primary weapon's passive particle and the particles its code spawns.
fn weapon_effects(
    weapon: &Json,
    found: &mut Effects,
    issues: &mut Vec<String>,
    decode: &dyn Fn(&str) -> Option<Json>,
) {
    if let Some(passive) = text(weapon.get("m_PassiveWeaponParticle")) {
        // The weapon names the attachment the game plays its passive particle on; without one
        // the effect's own control point configuration places it.
        let attachment =
            text(weapon.get("m_strPassiveWeaponAttachmentSource")).map(|point| EffectAttachment {
                kind: "point_follow".into(),
                point: point.to_string(),
                offset: [0.0; 3],
            });
        found.add(passive, "weapon-passive", attachment, None, true);
    }
    for (field, preview) in CODE_SPAWNED_WEAPON_PARTICLES {
        let Some(named) = text(weapon.get(field)) else {
            continue;
        };
        let children: Vec<String> = decode(preview)
            .map(|definition| {
                array(definition.get("m_Children"))
                    .iter()
                    .filter_map(|child| text(child.get("m_ChildRef")))
                    .map(system_key)
                    .collect()
            })
            .unwrap_or_default();
        let wanted = system_key(named);
        if children.is_empty() || children.iter().any(|child| *child != wanted) {
            issues.push(format!("weaponParticleUnplaced:{field}"));
            continue;
        }
        // Grimoire's `weapon-code` source; the contract's closed list files it under the weapon.
        found.add(
            preview,
            "weapon-passive",
            None,
            Some("preview".to_string()),
            true,
        );
    }
}

/// `attachment_type`, `attachment_point`, `attachment_offset` as an effect attachment, or None
/// without a type.
fn attachment_of(
    kind: Option<&Json>,
    point: Option<&Json>,
    offset: Option<&Json>,
) -> Option<EffectAttachment> {
    let kind = text(kind)?.to_ascii_lowercase();
    Some(EffectAttachment {
        kind: kind.strip_prefix("pattach_").unwrap_or(&kind).to_string(),
        point: point.and_then(Json::as_str).unwrap_or_default().to_string(),
        offset: vec3(offset).unwrap_or([0.0; 3]),
    })
}

/// The model's attachments: those of its first mesh block, else of the first referenced mesh
/// that has any (VRF's `Model.Read` and `SetExternalMeshData`).
fn model_attachments(stack: &Stack, model_bytes: &[u8], model: &Json) -> Vec<Attachment> {
    let embedded = morphic::resource::Resource::parse(model_bytes)
        .ok()
        .and_then(|resource| {
            let block = resource
                .blocks()
                .iter()
                .find(|block| block.kind == *b"MDAT")
                .copied()?;
            let start = block.offset as usize;
            let bytes = model_bytes.get(start..start.checked_add(block.size as usize)?)?;
            morphic::kv3::decode(bytes).ok()
        })
        .map(|mesh| parse_attachments(&kv::to_json(&mesh)))
        .unwrap_or_default();
    if !embedded.is_empty() {
        return embedded;
    }
    for mesh in array(model.get("m_refMeshes"))
        .iter()
        .filter_map(Json::as_str)
    {
        if mesh.is_empty() {
            continue;
        }
        let Some(decoded) = stack
            .read(&compiled(mesh))
            .and_then(|bytes| kv::resource_json(&bytes, mesh).ok())
        else {
            continue;
        };
        let found = parse_attachments(&decoded);
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// Every attachment of a decoded mesh, in stored order, with its influences (at most 8).
fn parse_attachments(mesh: &Json) -> Vec<Attachment> {
    array(mesh.get("m_attachments"))
        .iter()
        .filter(|entry| entry.is_object())
        .map(|entry| {
            let value = entry
                .get("value")
                .filter(|value| value.is_object())
                .unwrap_or(entry);
            let count = num(value.get("m_nInfluences"))
                .unwrap_or(0.0)
                .clamp(0.0, 8.0) as usize;
            let names = array(value.get("m_influenceNames"));
            let offsets = array(value.get("m_vInfluenceOffsets"));
            let rotations = array(value.get("m_vInfluenceRotations"));
            let weights = array(value.get("m_influenceWeights"));
            let roots = array(value.get("m_bInfluenceRootTransform"));
            Attachment {
                name: value
                    .get("m_name")
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_string(),
                ignore_rotation: value.get("m_bIgnoreRotation") == Some(&Json::Bool(true)),
                influences: (0..count)
                    .map(|index| Influence {
                        bone: names
                            .get(index)
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        offset: vec3(offsets.get(index)).unwrap_or([0.0; 3]),
                        rotation: quaternion(rotations.get(index)),
                        weight: num(weights.get(index)).unwrap_or(0.0),
                        root_transform: roots.get(index) == Some(&Json::Bool(true)),
                    })
                    .collect(),
            }
        })
        .collect()
}

fn quaternion(value: Option<&Json>) -> [f64; 4] {
    let items = array(value);
    if items.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let at = |index: usize| num(items.get(index));
    [
        at(0).unwrap_or(0.0),
        at(1).unwrap_or(0.0),
        at(2).unwrap_or(0.0),
        at(3).unwrap_or(1.0),
    ]
}

// Map effects.

/// Export the map particles of `entities` (the key values of every entity in the map's entity
/// lumps and their child lumps, as `hideout.rs` gathers them) for the Hideout. Only
/// `info_particle_system.effect_name` is supported; any other particle reference, a parented
/// entity, a snapshot override or a missing control point target is a bundle issue and its
/// entity is left out. Each effect carries its entity matrix and control points in Source
/// space.
///
/// Writes `out_dir/particles/bundle.json` and `out_dir/particles/tex/*.png` and returns
/// `Some("particles/bundle.json")`, or `None` (writing nothing) when no supported particle
/// entity names a system in the stack.
pub(crate) fn export_map_particles(
    stack: &Stack,
    entities: &[Map<String, Json>],
    out_dir: &Path,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    check(cancel)?;
    let (effects, issues) = read_map_particles(entities);
    if effects.is_empty() {
        return Ok(None);
    }
    write_bundle(
        stack,
        "dl_hideout".into(),
        effects,
        Vec::new(),
        issues,
        out_dir,
        cancel,
    )
}

/// Grimoire's `readMapParticles`: the supported particle entities as effects, and the issues
/// of every particle reference that is not one (deduplicated, in entity order).
fn read_map_particles(entities: &[Map<String, Json>]) -> (Vec<Effect>, Vec<String>) {
    let mut named: BTreeMap<&str, &Map<String, Json>> = BTreeMap::new();
    for entity in entities {
        if let Some(name) = text(entity.get("targetname")) {
            named.insert(name, entity);
        }
    }
    let mut effects = Vec::new();
    let mut issues = Issues::default();
    for (ordinal, entity) in entities.iter().enumerate() {
        let classname = entity
            .get("classname")
            .and_then(Json::as_str)
            .unwrap_or_default();
        for (field, resource) in entity {
            let Some(resource) = resource.as_str() else {
                continue;
            };
            let system = system_key(resource);
            if !resource.starts_with("particles/")
                || !(resource.ends_with(".vpcf") || resource.ends_with(".vpcf_c"))
                || !valid_system(&system)
            {
                continue;
            }
            let mut own = Vec::new();
            if classname != "info_particle_system" || field != "effect_name" {
                own.push(format!("unsupportedEntity:{classname}"));
            }
            if let Some(parent) = text(entity.get("parentname")) {
                own.push(format!("entityParent:{parent}"));
            }
            if let Some(snapshot) = text(entity.get("snapshot_file")) {
                own.push(format!("snapshotOverride:{snapshot}"));
            }
            let mut control_points = Vec::new();
            for index in 0..64u32 {
                let Some(target_name) = text(entity.get(&format!("cpoint{index}"))) else {
                    continue;
                };
                let Some(target) = named.get(target_name) else {
                    own.push(format!("missingControlPoint:{index}:{target_name}"));
                    continue;
                };
                if let Some(parent) = text(target.get("parentname")) {
                    own.push(format!("entityParent:{parent}"));
                }
                control_points.push(ControlPoint {
                    index,
                    transform: Some(entity_matrix(target)),
                    value: None,
                });
            }
            if let Some(index) = control_point_index(entity.get("tint_cp")) {
                control_points.push(ControlPoint {
                    index,
                    transform: None,
                    value: Some(vec3(entity.get("tint_cp_color")).unwrap_or([255.0; 3])),
                });
            }
            if let Some(index) = control_point_index(entity.get("data_cp")) {
                control_points.push(ControlPoint {
                    index,
                    transform: None,
                    value: Some(vec3(entity.get("data_cp_value")).unwrap_or([0.0; 3])),
                });
            }
            if !own.is_empty() {
                own.into_iter().for_each(|issue| issues.push(issue));
                continue;
            }
            let transform = entity_matrix(entity);
            let position = seed_position(&transform, &control_points);
            effects.push(Effect {
                id: format!("entity_{ordinal}"),
                seed: seed(&system, position),
                system,
                source: "map-entity".into(),
                attachment: None,
                attachment_cp1: None,
                configuration: None,
                transform: Some(transform),
                control_points,
                start_active: flag(entity.get("start_active")) == Some(true)
                    && flag(entity.get("StartDisabled")) != Some(true),
            });
        }
    }
    (effects, issues.list)
}

/// `tint_cp` or `data_cp`: an index 0 to 63, or None for -1, a missing or a malformed value.
fn control_point_index(value: Option<&Json>) -> Option<u32> {
    let index = num(value)?;
    (index.fract() == 0.0 && (0.0..=63.0).contains(&index)).then_some(index as u32)
}

/// Grimoire's `sourceEntityMatrix`: QAngle `angles`, per-axis `scales`, then `origin`, Source
/// space, column major.
fn entity_matrix(entity: &Map<String, Json>) -> Mat4 {
    space::entity_matrix(
        vec3(entity.get("origin")).unwrap_or([0.0; 3]),
        vec3(entity.get("angles")).unwrap_or([0.0; 3]),
        vec3(entity.get("scales")).unwrap_or([1.0; 3]),
    )
}

/// Control point 0 as `placeMapParticle` leaves it: the entity origin, a `cpoint0` target's
/// origin, then a value written to index 0.
fn seed_position(transform: &Mat4, control_points: &[ControlPoint]) -> [f64; 3] {
    let mut position = [transform[12], transform[13], transform[14]];
    for point in control_points.iter().filter(|point| point.index == 0) {
        if let Some(matrix) = point.transform {
            position = [matrix[12], matrix[13], matrix[14]];
        }
    }
    for point in control_points.iter().filter(|point| point.index == 0) {
        if let Some(value) = point.value {
            position = value;
        }
    }
    position
}

// The shared bundle writer.

/// Walk the effects' system trees, export their textures and write the bundle. Effects whose
/// root system is not in the stack are dropped (with an issue); None when none is left.
fn write_bundle(
    stack: &Stack,
    owner: String,
    effects: Vec<Effect>,
    attachments: Vec<Attachment>,
    issues: Vec<String>,
    out_dir: &Path,
    cancel: &AtomicBool,
) -> Result<Option<String>> {
    let mut issues = Issues::from(issues);
    // Roots in effect order, active effects first: the system limit keeps what plays.
    let mut roots: Vec<String> = Vec::new();
    for effect in effects
        .iter()
        .filter(|effect| effect.start_active)
        .chain(effects.iter().filter(|effect| !effect.start_active))
    {
        if !roots.contains(&effect.system) {
            roots.push(effect.system.clone());
        }
    }
    let has_texture = |path: &str| {
        let entry = compiled(path);
        stack
            .layers
            .iter()
            .any(|layer| layer.directory.by_path.contains_key(&entry))
    };
    let walk = walk_systems(&roots, cancel, |path| load_system(stack, path), has_texture)?;
    walk.issues.into_iter().for_each(|issue| issues.push(issue));
    let effects: Vec<Effect> = effects
        .into_iter()
        .filter(|effect| walk.systems.contains_key(&effect.system))
        .collect();
    if effects.is_empty() {
        return Ok(None);
    }

    let particles_dir = out_dir.join("particles");
    let texture_dir = particles_dir.join("tex");
    if texture_dir.exists() {
        std::fs::remove_dir_all(&texture_dir)?;
    }
    std::fs::create_dir_all(&texture_dir)?;
    let mut textures = BTreeMap::new();
    let mut files = BTreeSet::new();
    for texture in &walk.textures {
        check(cancel)?;
        let Some(bytes) = stack.read(&compiled(texture)) else {
            issues.push(format!("missing:{texture}"));
            continue;
        };
        if bytes.len() > MAX_TEXTURE_BYTES {
            issues.push(format!("textureTooLarge:{texture}"));
            continue;
        }
        let Ok(thumbnail) = vpkmerge_core::thumbnail_png(&bytes, MAX_TEXTURE_EDGE) else {
            issues.push(format!("undecodable:{texture}"));
            continue;
        };
        let sheet = match read_texture_sheet(&bytes) {
            Ok(sheet) => sheet,
            Err(()) => {
                issues.push(format!("sheet:{texture}"));
                None
            }
        };
        let file = unique_file(&texture_file(texture), &mut files);
        std::fs::write(texture_dir.join(&file), &thumbnail.png)?;
        textures.insert(
            texture.clone(),
            BundleTexture {
                file: format!("tex/{file}"),
                width: thumbnail.width,
                height: thumbnail.height,
                sheet,
            },
        );
    }
    check(cancel)?;

    let bundle = Bundle {
        schema: SCHEMA,
        owner,
        systems: walk.systems,
        textures,
        attachments,
        effects,
        issues: issues.list,
    };
    std::fs::write(
        particles_dir.join("bundle.json"),
        serde_json::to_vec(&bundle)?,
    )?;
    Ok(Some(BUNDLE_PATH.to_string()))
}

/// Why a system did not load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadError {
    Missing,
    Unreadable,
}

/// Decode `path` (a system key) from the stack, with its stored KV3 format name.
fn load_system(
    stack: &Stack,
    path: &str,
) -> std::result::Result<(Json, Option<String>), LoadError> {
    let bytes = stack.read(&compiled(path)).ok_or(LoadError::Missing)?;
    let value = morphic::decode_kv3_resource(&bytes).map_err(|_| LoadError::Unreadable)?;
    let format = morphic::kv3_resource_data_block(&bytes)
        .ok()
        .and_then(|block| morphic::kv3::Format::from_payload(&block).ok())
        .and_then(|format| format.name())
        .filter(|name| name.starts_with("vpcf"))
        .map(str::to_string);
    Ok((kv::to_json(&value), format))
}

/// What [`walk_systems`] found.
#[derive(Debug, Default)]
struct Walk {
    systems: BTreeMap<String, BundleSystem>,
    /// Texture paths in discovery order, unique, at most [`MAX_TEXTURES`].
    textures: Vec<String>,
    issues: Vec<String>,
}

/// Every definition in each root's tree, breadth first over `m_Children[].m_ChildRef`, at most
/// [`MAX_SYSTEMS`] systems [`MAX_DEPTH`] levels below a root, and the textures their renderers
/// name (at most [`MAX_TEXTURES`] that exist). `load` decodes a system key; `has_texture`
/// says whether a texture path is in the stack. Grimoire's `exportParticleBundle` walk.
fn walk_systems(
    roots: &[String],
    cancel: &AtomicBool,
    mut load: impl FnMut(&str) -> std::result::Result<(Json, Option<String>), LoadError>,
    has_texture: impl Fn(&str) -> bool,
) -> Result<Walk> {
    let mut walk = Walk::default();
    let mut issues = Issues::default();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut texture_seen: BTreeSet<String> = BTreeSet::new();
    let mut level: Vec<(String, usize)> = roots.iter().map(|root| (system_key(root), 0)).collect();
    while !level.is_empty() {
        let mut next = Vec::new();
        for (path, depth) in level {
            if walk.systems.contains_key(&path) || failed.contains(&path) || !valid_system(&path) {
                continue;
            }
            if walk.systems.len() >= MAX_SYSTEMS {
                issues.push("limit:systems".into());
                continue;
            }
            if depth > MAX_DEPTH {
                issues.push("limit:depth".into());
                continue;
            }
            check(cancel)?;
            let (data, format) = match load(&path) {
                Ok(found) => found,
                Err(error) => {
                    issues.push(match error {
                        LoadError::Missing => format!("missing:{path}"),
                        LoadError::Unreadable => format!("unreadable:{path}"),
                    });
                    failed.insert(path);
                    continue;
                }
            };
            if text(data.get("_class")) != Some("CParticleSystemDefinition") {
                issues.push(format!("notParticleSystem:{path}"));
                failed.insert(path);
                continue;
            }
            for child in array(data.get("m_Children")) {
                if let Some(child) = text(child.get("m_ChildRef")) {
                    next.push((system_key(child), depth + 1));
                }
            }
            for texture in renderer_textures(&data) {
                if !texture_seen.insert(texture.clone()) {
                    continue;
                }
                if walk.textures.len() >= MAX_TEXTURES {
                    issues.push("limit:textures".into());
                    continue;
                }
                if has_texture(&texture) {
                    walk.textures.push(texture);
                } else {
                    issues.push(format!("missing:{texture}"));
                }
            }
            walk.systems.insert(path, BundleSystem { data, format });
        }
        level = next;
    }
    walk.issues = issues.list;
    Ok(walk)
}

/// Every texture a definition's renderers name, anywhere in the renderer block, after the
/// default texture of each spritecard renderer.
fn renderer_textures(definition: &Json) -> Vec<String> {
    let renderers = array(definition.get("m_Renderers"));
    let mut found: Vec<String> = Vec::new();
    let mut add = |path: String| {
        if !found.contains(&path) {
            found.push(path);
        }
    };
    for renderer in renderers {
        let class = text(renderer.get("_class")).unwrap_or_default();
        if let Some((_, fallback)) = RENDERER_DEFAULT_TEXTURES
            .iter()
            .find(|(name, _)| *name == class)
        {
            add((*fallback).to_string());
        }
    }
    fn visit(value: &Json, add: &mut dyn FnMut(String)) {
        match value {
            Json::String(path) => {
                let path = path.replace('\\', "/");
                if valid_texture(&path) {
                    add(path);
                }
            }
            Json::Array(items) => items.iter().for_each(|item| visit(item, add)),
            Json::Object(map) => map.values().for_each(|item| visit(item, add)),
            _ => {}
        }
    }
    for renderer in renderers {
        visit(renderer, &mut add);
    }
    found
}

/// Issues in first-seen order, without repeats.
#[derive(Default)]
struct Issues {
    list: Vec<String>,
}

impl Issues {
    fn push(&mut self, issue: String) {
        if !self.list.contains(&issue) {
            self.list.push(issue);
        }
    }
}

impl From<Vec<String>> for Issues {
    fn from(list: Vec<String>) -> Self {
        let mut issues = Issues::default();
        list.into_iter().for_each(|issue| issues.push(issue));
        issues
    }
}

// Paths and names.

/// A system path as bundles key it: forward slashes, lowercase, no `_c`.
fn system_key(path: &str) -> String {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    path.strip_suffix("_c").unwrap_or(&path).to_string()
}

fn path_chars(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
}

/// `particles/<path>.vpcf` with no `..` (Grimoire's `SYSTEM_PATH`).
fn valid_system(path: &str) -> bool {
    !path.contains("..")
        && path
            .strip_prefix("particles/")
            .and_then(|rest| rest.strip_suffix(".vpcf"))
            .is_some_and(path_chars)
}

/// `<root>/<path>.vtex` with no `..` (Grimoire's `TEXTURE_PATH`).
fn valid_texture(path: &str) -> bool {
    let Some((root, rest)) = path.split_once('/') else {
        return false;
    };
    !path.contains("..")
        && !root.is_empty()
        && root.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && rest.strip_suffix(".vtex").is_some_and(path_chars)
}

/// The PNG file name of a texture: the path with every run of other characters as `_`.
fn texture_file(texture: &str) -> String {
    let mut name = String::new();
    let mut gap = false;
    for c in texture.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !name.is_empty() {
                name.push('_');
            }
            gap = false;
            name.push(c);
        } else {
            gap = true;
        }
    }
    if gap {
        name.push('_');
    }
    format!("{name}.png")
}

fn unique_file(file: &str, used: &mut BTreeSet<String>) -> String {
    let lower = |name: &str| name.to_ascii_lowercase();
    if used.insert(lower(file)) {
        return file.to_string();
    }
    let stem = file.strip_suffix(".png").unwrap_or(file);
    (2..)
        .map(|n| format!("{stem}_{n}.png"))
        .find(|name| used.insert(lower(name)))
        .unwrap_or_default()
}

// Seeds.

/// VRF's `StringToken.Get`: MurmurHash2 of the UTF-8 bytes with ASCII letters lowercased,
/// seed `0x31415926`. 0 for an empty string.
pub(crate) fn string_token(key: &str) -> u32 {
    const SEED: u32 = 0x3141_5926;
    const M: u32 = 0x5bd1_e995;
    if key.is_empty() {
        return 0;
    }
    let bytes: Vec<u8> = key.bytes().map(|byte| byte.to_ascii_lowercase()).collect();
    let mut hash = SEED ^ bytes.len() as u32;
    let mut chunks = bytes.chunks_exact(4);
    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        hash = hash.wrapping_mul(M) ^ k;
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        let mut value = 0u32;
        for (shift, byte) in tail.iter().enumerate() {
            value |= u32::from(*byte) << (8 * shift);
        }
        hash = (hash ^ value).wrapping_mul(M);
    }
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^ (hash >> 15)
}

/// Grimoire's placement seed: the token of the system's compiled name xor the position hash
/// (`stringToken(name) ^ round(x)*73856093 ^ round(y)*19349663 ^ round(z)*83492791`).
fn seed(system: &str, position: [f64; 3]) -> u32 {
    let hash = |value: f64, prime: i32| (value.round_ties_even() as i64 as i32).wrapping_mul(prime);
    let position = hash(position[0], 73_856_093)
        ^ hash(position[1], 19_349_663)
        ^ hash(position[2], 83_492_791);
    string_token(&format!("{system}_c")) ^ position as u32
}

// Sprite sheets.

/// Bounds-checked little-endian reads with signed offsets.
struct Reader<'a> {
    bytes: &'a [u8],
}

impl Reader<'_> {
    fn slice(&self, offset: i64, size: usize) -> std::result::Result<&[u8], ()> {
        let start = usize::try_from(offset).map_err(|_| ())?;
        let end = start.checked_add(size).ok_or(())?;
        self.bytes.get(start..end).ok_or(())
    }

    fn u8(&self, offset: i64) -> std::result::Result<u8, ()> {
        Ok(self.slice(offset, 1)?[0])
    }

    fn u16(&self, offset: i64) -> std::result::Result<u16, ()> {
        let bytes = self.slice(offset, 2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&self, offset: i64) -> std::result::Result<u32, ()> {
        let bytes = self.slice(offset, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&self, offset: i64) -> std::result::Result<i32, ()> {
        Ok(self.u32(offset)? as i32)
    }

    fn f32(&self, offset: i64) -> std::result::Result<f32, ()> {
        Ok(f32::from_bits(self.u32(offset)?))
    }

    /// The offset stored at `offset`, counted from `offset` itself.
    fn relative(&self, offset: i64) -> std::result::Result<i64, ()> {
        Ok(offset + i64::from(self.i32(offset)?))
    }

    /// The null terminated string an offset stored at `offset` points to; empty for zero.
    fn offset_string(&self, offset: i64) -> std::result::Result<String, ()> {
        let relative = self.i32(offset)?;
        if relative == 0 {
            return Ok(String::new());
        }
        let start = usize::try_from(offset + i64::from(relative)).map_err(|_| ())?;
        let rest = self.bytes.get(start..).ok_or(())?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(rest.len());
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

/// The sprite sheet of a compiled `.vtex_c` (VTEX v1 extra data of type SHEET), `Ok(None)`
/// when the file is not a texture resource or has no sheet, `Err` for a malformed one.
fn read_texture_sheet(bytes: &[u8]) -> std::result::Result<Option<Sheet>, ()> {
    let reader = Reader { bytes };
    if bytes.len() < 16 || reader.u16(4)? != 12 {
        return Ok(None);
    }
    let table = reader.relative(8)?;
    let count = reader.u32(12)?.min(4096);
    for index in 0..i64::from(count) {
        let block = table + index * 12;
        if reader.slice(block, 4)? != b"DATA" {
            continue;
        }
        let data = reader.relative(block + 4)?;
        if reader.u16(data)? != 1 {
            return Ok(None);
        }
        let extra = reader.relative(data + 32)?;
        let extra_count = reader.u32(data + 36)?.min(64);
        let mut sheet = None;
        for entry in 0..i64::from(extra_count) {
            let entry = extra + entry * 12;
            let kind = reader.u32(entry)?;
            let start = reader.relative(entry + 4)?;
            let size = reader.u32(entry + 8)? as usize;
            let block = reader.slice(start, size)?;
            if kind == 2 {
                sheet = Some(read_sheet(&Reader { bytes: block })?);
            }
        }
        return Ok(sheet);
    }
    Ok(None)
}

/// `Texture.GetSpriteSheetData`: offsets are relative to where they are stored.
fn read_sheet(reader: &Reader<'_>) -> std::result::Result<Sheet, ()> {
    const UNNAMED_SEQUENCE: &str = "CDmeSheetSequence";
    let sequences_offset = i64::from(reader.i32(0)?);
    let sequence_count = reader.u32(4)?;
    let mut sequences = Vec::new();
    for sequence in 0..i64::from(sequence_count) {
        let header = sequences_offset + sequence * 32;
        let frames_offset = reader.relative(header + 8)?;
        let frame_count = reader.u32(header + 12)?;
        let mut frames = Vec::new();
        for frame in 0..i64::from(frame_count) {
            let frame = frames_offset + frame * 12;
            let images_offset = reader.relative(frame + 4)?;
            let image_count = reader.u32(frame + 8)?;
            let mut images = Vec::new();
            for image in 0..i64::from(image_count) {
                let image = images_offset + image * 32;
                // croppedMin, croppedMax, then uncroppedMin and uncroppedMax.
                images.push([
                    reader.f32(image + 16)?,
                    reader.f32(image + 20)?,
                    reader.f32(image + 24)?,
                    reader.f32(image + 28)?,
                ]);
            }
            frames.push(SheetFrame {
                duration: reader.f32(frame)?,
                images,
            });
        }
        let authored = reader.u32(header)?;
        let name = reader.offset_string(header + 20)?;
        let id = if name == UNNAMED_SEQUENCE {
            i64::from(authored as i32)
        } else if !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()) {
            name.parse::<i64>()
                .ok()
                .filter(|id| *id <= 0x7fff_ffff)
                .unwrap_or(-1)
        } else {
            -1
        };
        sequences.push(SheetSequence {
            id,
            clamp: reader.u8(header + 4)? != 0,
            alpha_crop: reader.u8(header + 5)? != 0,
            no_color: reader.u8(header + 6)? != 0,
            no_alpha: reader.u8(header + 7)? != 0,
            duration: reader.f32(header + 16)?,
            frames,
        });
    }
    Ok(Sheet { sequences })
}

// KV3 text (model keyvalues).

const MAX_KV3_TEXT: usize = 4 * 1024 * 1024;
const MAX_KV3_DEPTH: usize = 64;

/// Parse KV3 text as models embed their keyvalues (`m_modelInfo.m_keyValueText`). Typed values
/// (`resource_name:"..."`) read as their value, binary blobs (`#[ ... ]`) as byte arrays.
/// Bounded: rejects input past the size or nesting limits.
fn parse_kv3_text(text: &str) -> std::result::Result<Json, String> {
    if text.len() > MAX_KV3_TEXT {
        return Err("KV3 text exceeds limits".into());
    }
    let mut parser = TextParser {
        text,
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.skip()?;
    let root = parser.value(0)?;
    if root.is_object() {
        Ok(root)
    } else {
        Err("KV3 text root is not an object".into())
    }
}

struct TextParser<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
}

impl TextParser<'_> {
    fn fail<T>(&self, message: &str) -> std::result::Result<T, String> {
        Err(format!("KV3 text: {message} at {}", self.at))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn starts(&self, token: &str) -> bool {
        self.bytes[self.at.min(self.bytes.len())..].starts_with(token.as_bytes())
    }

    fn find(&self, token: &str, from: usize) -> Option<usize> {
        self.text.get(from..)?.find(token).map(|found| found + from)
    }

    fn skip(&mut self) -> std::result::Result<(), String> {
        loop {
            while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
                self.at += 1;
            }
            if self.starts("//") {
                self.at = self
                    .find("\n", self.at)
                    .map_or(self.bytes.len(), |end| end + 1);
            } else if self.starts("/*") {
                match self.find("*/", self.at + 2) {
                    Some(end) => self.at = end + 2,
                    None => return self.fail("unterminated comment"),
                }
            } else if self.starts("<!--") {
                match self.find("-->", self.at + 4) {
                    Some(end) => self.at = end + 3,
                    None => return self.fail("unterminated header"),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn string(&mut self) -> std::result::Result<String, String> {
        if self.starts("\"\"\"") {
            let Some(end) = self.find("\"\"\"", self.at + 3) else {
                return self.fail("unterminated multi-line string");
            };
            let mut body = &self.text[self.at + 3..end];
            self.at = end + 3;
            body = body
                .strip_prefix("\r\n")
                .or_else(|| body.strip_prefix('\n'))
                .unwrap_or(body);
            body = body
                .strip_suffix("\r\n")
                .or_else(|| body.strip_suffix('\n'))
                .unwrap_or(body);
            return Ok(body.to_string());
        }
        self.at += 1;
        let mut out = Vec::new();
        while let Some(byte) = self.peek() {
            if byte == b'"' {
                self.at += 1;
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            if byte == b'\\' && self.at + 1 < self.bytes.len() {
                let next = self.bytes[self.at + 1];
                out.push(match next {
                    b'n' => b'\n',
                    b't' => b'\t',
                    b'r' => b'\r',
                    other => other,
                });
                self.at += 2;
            } else {
                out.push(byte);
                self.at += 1;
            }
        }
        self.fail("unterminated string")
    }

    fn word(&mut self) -> std::result::Result<&str, String> {
        let start = self.at;
        while self.peek().is_some_and(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-' | b'|')
        }) {
            self.at += 1;
        }
        if self.at == start {
            return self.fail("unexpected character");
        }
        Ok(&self.text[start..self.at])
    }

    fn key(&mut self) -> std::result::Result<String, String> {
        self.skip()?;
        if self.peek() == Some(b'"') {
            self.string()
        } else {
            self.word().map(str::to_string)
        }
    }

    fn value(&mut self, depth: usize) -> std::result::Result<Json, String> {
        if depth > MAX_KV3_DEPTH {
            return self.fail("nesting too deep");
        }
        self.skip()?;
        match self.peek() {
            Some(b'{') => {
                self.at += 1;
                let mut object = Map::new();
                loop {
                    self.skip()?;
                    match self.peek() {
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Object(object));
                        }
                        None => return self.fail("unterminated object"),
                        _ => {}
                    }
                    let key = self.key()?;
                    self.skip()?;
                    if self.peek() != Some(b'=') {
                        return self.fail("expected =");
                    }
                    self.at += 1;
                    let value = self.value(depth + 1)?;
                    object.insert(key, value);
                    self.skip()?;
                    if self.peek() == Some(b',') {
                        self.at += 1;
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.skip()?;
                    match self.peek() {
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Array(items));
                        }
                        None => return self.fail("unterminated array"),
                        _ => {}
                    }
                    items.push(self.value(depth + 1)?);
                    self.skip()?;
                    if self.peek() == Some(b',') {
                        self.at += 1;
                    }
                }
            }
            Some(b'#') if self.bytes.get(self.at + 1) == Some(&b'[') => {
                self.at += 2;
                let mut items = Vec::new();
                loop {
                    self.skip()?;
                    match self.peek() {
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Array(items));
                        }
                        None => return self.fail("unterminated blob"),
                        _ => {}
                    }
                    let pair = self.text.get(self.at..self.at + 2).unwrap_or_default();
                    match u8::from_str_radix(pair, 16) {
                        Ok(byte) => items.push(Json::from(byte)),
                        Err(_) => return self.fail("bad blob byte"),
                    }
                    self.at += 2;
                }
            }
            Some(b'"') => self.string().map(Json::String),
            _ => {
                let word = self.word()?.to_string();
                // A typed or flagged value (resource_name:"...") reads as the value after it.
                if self.peek() == Some(b':') {
                    self.at += 1;
                    return self.value(depth + 1);
                }
                Ok(match word.as_str() {
                    "true" => Json::Bool(true),
                    "false" => Json::Bool(false),
                    "null" => Json::Null,
                    _ => number(&word).unwrap_or(Json::String(word)),
                })
            }
        }
    }
}

/// A finite number word as JSON: an integer when it is one.
fn number(word: &str) -> Option<Json> {
    let value: f64 = word.parse().ok().filter(|value: &f64| value.is_finite())?;
    if value.fract() == 0.0
        && value.abs() < 9_007_199_254_740_992.0
        && !word.contains(['.', 'e', 'E'])
    {
        Some(Json::from(value as i64))
    } else {
        serde_json::Number::from_f64(value).map(Json::Number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn entity(value: Json) -> Map<String, Json> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn particles_string_token_matches_grimoire() {
        // Grimoire's vfxEval.test.ts and particles.test.ts.
        assert_eq!(string_token("$SELFILLUM"), 0xbad3_4216);
        assert_eq!(string_token("$ILLUMCOLOR"), 0x07e1_f77d);
        assert_eq!(string_token("$OFFSET"), 0x80b2_e6ff);
        assert_eq!(string_token("$color"), string_token("$COLOR"));
        assert_eq!(
            string_token("particles/Abilities/X.vpcf_c"),
            string_token("particles/abilities/x.vpcf_c")
        );
        assert_eq!(string_token(""), 0);
    }

    #[test]
    fn particles_seed_hashes_the_position() {
        let token = string_token("particles/a/b.vpcf_c");
        assert_eq!(seed("particles/a/b.vpcf", [0.0; 3]), token);
        let moved = 3i32.wrapping_mul(73_856_093)
            ^ (-2i32).wrapping_mul(19_349_663)
            ^ 2i32.wrapping_mul(83_492_791);
        // 2.5 rounds half to even.
        assert_eq!(
            seed("particles/a/b.vpcf", [3.2, -2.0, 2.5]),
            token ^ moved as u32
        );
    }

    /// A compiled texture resource holding one DATA block with a VTEX v1 header and a sheet.
    fn vtex_with_sheet(name: &str) -> Vec<u8> {
        // The sheet: header, two sequences, frames, images, then the name.
        let mut sheet = vec![0u8; 8];
        let sequences_at = sheet.len();
        sheet[0..4].copy_from_slice(&(sequences_at as i32).to_le_bytes());
        sheet[4..8].copy_from_slice(&2u32.to_le_bytes());
        sheet.resize(sequences_at + 64, 0);
        let frames_at = sheet.len();
        sheet.resize(frames_at + 24, 0);
        let images_at = sheet.len();
        sheet.resize(images_at + 64, 0);
        let name_at = sheet.len();
        sheet.extend_from_slice(name.as_bytes());
        sheet.push(0);
        let put_u32 = |sheet: &mut Vec<u8>, at: usize, value: u32| {
            sheet[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        let put_f32 = |sheet: &mut Vec<u8>, at: usize, value: f32| {
            sheet[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        let rel = |from: usize, to: usize| (to as i64 - from as i64) as i32 as u32;
        // Sequence 0: id 7, clamped, unnamed, 2 frames.
        let s0 = sequences_at;
        put_u32(&mut sheet, s0, 7);
        sheet[s0 + 4] = 1;
        put_u32(&mut sheet, s0 + 8, rel(s0 + 8, frames_at));
        put_u32(&mut sheet, s0 + 12, 2);
        put_f32(&mut sheet, s0 + 16, 0.5);
        put_u32(&mut sheet, s0 + 20, 0);
        // Sequence 1: id 0, named, no frames.
        let s1 = sequences_at + 32;
        sheet[s1 + 6] = 1;
        put_u32(&mut sheet, s1 + 20, rel(s1 + 20, name_at));
        for frame in 0..2 {
            let at = frames_at + frame * 12;
            put_f32(&mut sheet, at, 0.25);
            put_u32(&mut sheet, at + 4, rel(at + 4, images_at + frame * 32));
            put_u32(&mut sheet, at + 8, 1);
            let image = images_at + frame * 32;
            let u = frame as f32 * 0.5;
            for (index, value) in [u, 0.0, u + 0.5, 0.5, u, 0.0, u + 0.5, 1.0]
                .into_iter()
                .enumerate()
            {
                put_f32(&mut sheet, image + index * 4, value);
            }
        }

        // The DATA block: 40 byte VTEX header, one extra data entry, the sheet.
        let mut data = vec![0u8; 40];
        data[0..2].copy_from_slice(&1u16.to_le_bytes());
        data[20..22].copy_from_slice(&64u16.to_le_bytes());
        data[22..24].copy_from_slice(&64u16.to_le_bytes());
        let entry_at = data.len();
        data[32..36].copy_from_slice(&rel(32, entry_at).to_le_bytes());
        data[36..40].copy_from_slice(&1u32.to_le_bytes());
        data.resize(entry_at + 12, 0);
        let sheet_at = data.len();
        data[entry_at..entry_at + 4].copy_from_slice(&2u32.to_le_bytes());
        data[entry_at + 4..entry_at + 8]
            .copy_from_slice(&rel(entry_at + 4, sheet_at).to_le_bytes());
        data[entry_at + 8..entry_at + 12].copy_from_slice(&(sheet.len() as u32).to_le_bytes());
        data.extend_from_slice(&sheet);

        // The resource: header, block table, DATA.
        let mut file = vec![0u8; 28];
        file[4..6].copy_from_slice(&12u16.to_le_bytes());
        file[8..12].copy_from_slice(&8u32.to_le_bytes());
        file[12..16].copy_from_slice(&1u32.to_le_bytes());
        file[16..20].copy_from_slice(b"DATA");
        file[20..24].copy_from_slice(&rel(20, 28).to_le_bytes());
        file[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&data);
        let size = file.len() as u32;
        file[0..4].copy_from_slice(&size.to_le_bytes());
        file
    }

    #[test]
    fn particles_sheet_reader_reads_a_hand_built_vtex() {
        let sheet = read_texture_sheet(&vtex_with_sheet("3")).unwrap().unwrap();
        assert_eq!(sheet.sequences.len(), 2);
        let first = &sheet.sequences[0];
        // Unnamed sequences have no name offset here, so they read as named "" and answer -1.
        assert_eq!(first.id, -1);
        assert!(first.clamp && !first.alpha_crop && !first.no_color && !first.no_alpha);
        assert_eq!(first.duration, 0.5);
        assert_eq!(first.frames.len(), 2);
        assert_eq!(first.frames[0].duration, 0.25);
        assert_eq!(first.frames[0].images, vec![[0.0, 0.0, 0.5, 1.0]]);
        assert_eq!(first.frames[1].images, vec![[0.5, 0.0, 1.0, 1.0]]);
        let second = &sheet.sequences[1];
        assert_eq!(second.id, 3);
        assert!(second.no_color);
        assert!(second.frames.is_empty());

        let unnamed = read_texture_sheet(&vtex_with_sheet("CDmeSheetSequence"))
            .unwrap()
            .unwrap();
        assert_eq!(unnamed.sequences[1].id, 0);
        let named = read_texture_sheet(&vtex_with_sheet("fire"))
            .unwrap()
            .unwrap();
        assert_eq!(named.sequences[1].id, -1);
    }

    #[test]
    fn particles_sheet_reader_rejects_and_skips() {
        assert_eq!(read_texture_sheet(b"not a resource"), Ok(None));
        let mut truncated = vtex_with_sheet("1");
        truncated.truncate(truncated.len() - 40);
        assert_eq!(read_texture_sheet(&truncated), Err(()));
    }

    #[test]
    fn particles_entity_matrix_follows_source2_entity() {
        let matrix = entity_matrix(&entity(json!({
            "origin": "1 2 3", "angles": [0, 90, 0], "scales": "2 2 2"
        })));
        let expected = [
            0.0, 2.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 1.0, 2.0, 3.0, 1.0,
        ];
        for (got, want) in matrix.iter().zip(expected) {
            assert!((got - want).abs() < 1e-9, "{matrix:?}");
        }
        // Pitch down 90: forward becomes -Z.
        let pitched = entity_matrix(&entity(json!({ "angles": "90 0 0" })));
        assert!((pitched[2] + 1.0).abs() < 1e-9);
    }

    #[test]
    fn particles_read_map_particles() {
        let entities = vec![
            entity(json!({
                "classname": "info_particle_system", "effect_name": "particles/Hideout/Fire.vpcf",
                "origin": [10, 20, 30], "angles": [0, 0, 0], "start_active": true,
                "cpoint1": "fire_target", "tint_cp": 2, "tint_cp_color": "255 128 0", "data_cp": -1
            })),
            entity(
                json!({ "classname": "info_target", "targetname": "fire_target", "origin": "5 5 5" }),
            ),
            entity(json!({
                "classname": "info_particle_system", "effect_name": "particles/hideout/candle.vpcf",
                "start_active": true, "StartDisabled": true, "data_cp": 3
            })),
            entity(json!({
                "classname": "info_particle_system", "effect_name": "particles/hideout/parented.vpcf",
                "start_active": true, "parentname": "door"
            })),
            entity(json!({
                "classname": "info_particle_system", "effect_name": "particles/hideout/lost.vpcf",
                "cpoint2": "nowhere"
            })),
            entity(json!({ "classname": "env_beam", "particle": "particles/hideout/beam.vpcf_c" })),
            entity(
                json!({ "classname": "info_particle_system", "effect_name": "materials/x.vmat" }),
            ),
        ];
        let (effects, issues) = read_map_particles(&entities);
        assert_eq!(effects.len(), 2);

        let fire = &effects[0];
        assert_eq!(fire.id, "entity_0");
        assert_eq!(fire.system, "particles/hideout/fire.vpcf");
        assert_eq!(fire.source, "map-entity");
        assert!(fire.start_active);
        let transform = fire.transform.unwrap();
        assert_eq!(&transform[12..15], &[10.0, 20.0, 30.0]);
        assert_eq!(fire.control_points.len(), 2);
        assert_eq!(fire.control_points[0].index, 1);
        assert_eq!(
            &fire.control_points[0].transform.unwrap()[12..15],
            &[5.0, 5.0, 5.0]
        );
        assert_eq!(fire.control_points[1].index, 2);
        assert_eq!(fire.control_points[1].value, Some([255.0, 128.0, 0.0]));
        assert_eq!(
            fire.seed,
            seed("particles/hideout/fire.vpcf", [10.0, 20.0, 30.0])
        );

        let candle = &effects[1];
        assert!(!candle.start_active);
        assert_eq!(candle.control_points[0].index, 3);
        assert_eq!(candle.control_points[0].value, Some([0.0; 3]));

        assert_eq!(
            issues,
            vec![
                "entityParent:door".to_string(),
                "missingControlPoint:2:nowhere".to_string(),
                "unsupportedEntity:env_beam".to_string(),
            ]
        );
    }

    fn definition(children: &[&str], renderers: Json) -> Json {
        json!({
            "_class": "CParticleSystemDefinition",
            "m_Children": children.iter().map(|child| json!({ "m_ChildRef": child })).collect::<Vec<_>>(),
            "m_Renderers": renderers,
        })
    }

    #[test]
    fn particles_walk_follows_children_and_textures() {
        let mut systems: BTreeMap<String, Json> = BTreeMap::new();
        systems.insert(
            "particles/root.vpcf".into(),
            definition(
                &[
                    "particles/Child.vpcf",
                    "particles/missing.vpcf",
                    "particles/model.vpcf",
                ],
                json!([{ "_class": "C_OP_RenderSprites", "m_vecTexturesInput": [
                    { "m_hTexture": "materials/particle/fire.vtex" },
                    { "m_hTexture": "materials\\particle\\gone.vtex" }
                ] }]),
            ),
        );
        systems.insert(
            "particles/child.vpcf".into(),
            definition(
                &["particles/root.vpcf"],
                json!([{ "_class": "C_OP_RenderTrails" }]),
            ),
        );
        systems.insert("particles/model.vpcf".into(), json!({ "_class": "CModel" }));
        let cancel = AtomicBool::new(false);
        let walk = walk_systems(
            &["particles/root.vpcf".to_string()],
            &cancel,
            |path| {
                systems
                    .get(path)
                    .map(|data| (data.clone(), Some("vpcf66".into())))
                    .ok_or(LoadError::Missing)
            },
            |path| !path.contains("gone"),
        )
        .unwrap();
        assert_eq!(
            walk.systems.keys().collect::<Vec<_>>(),
            vec!["particles/child.vpcf", "particles/root.vpcf"]
        );
        assert_eq!(
            walk.systems["particles/root.vpcf"].format.as_deref(),
            Some("vpcf66")
        );
        assert_eq!(
            walk.textures,
            vec![
                "materials/particle/base_sprite.vtex",
                "materials/particle/fire.vtex",
                "materials/particle/base_trail.vtex",
            ]
        );
        assert_eq!(
            walk.issues,
            vec![
                "missing:materials/particle/gone.vtex",
                "missing:particles/missing.vpcf",
                "notParticleSystem:particles/model.vpcf",
            ]
        );
    }

    #[test]
    fn particles_walk_limits() {
        let cancel = AtomicBool::new(false);
        // A chain 20 deep keeps the root and 12 levels below it.
        let chain = |path: &str| {
            let depth: usize = path
                .trim_start_matches("particles/c")
                .trim_end_matches(".vpcf")
                .parse()
                .unwrap();
            let child = format!("particles/c{}.vpcf", depth + 1);
            Ok((definition(&[&child], json!([])), None))
        };
        let walk =
            walk_systems(&["particles/c0.vpcf".to_string()], &cancel, chain, |_| true).unwrap();
        assert_eq!(walk.systems.len(), MAX_DEPTH + 1);
        assert_eq!(walk.issues, vec!["limit:depth"]);

        // A root with 200 children, each naming its own texture.
        let wide = |path: &str| {
            if path == "particles/wide.vpcf" {
                let children: Vec<String> =
                    (0..200).map(|n| format!("particles/w{n}.vpcf")).collect();
                let refs: Vec<&str> = children.iter().map(String::as_str).collect();
                return Ok((definition(&refs, json!([])), None));
            }
            let texture = format!(
                "materials/{}.vtex",
                path.trim_start_matches("particles/")
                    .trim_end_matches(".vpcf")
            );
            Ok((
                definition(
                    &[],
                    json!([{ "_class": "C_OP_RenderModels", "m_hTexture": texture }]),
                ),
                None,
            ))
        };
        let walk = walk_systems(&["particles/wide.vpcf".to_string()], &cancel, wide, |_| {
            true
        })
        .unwrap();
        assert_eq!(walk.systems.len(), MAX_SYSTEMS);
        assert_eq!(walk.textures.len(), MAX_TEXTURES);
        assert_eq!(walk.issues, vec!["limit:textures", "limit:systems"]);

        // Cancel stops between systems.
        let cancelled = AtomicBool::new(true);
        assert!(
            walk_systems(
                &["particles/c0.vpcf".to_string()],
                &cancelled,
                chain,
                |_| true
            )
            .is_err()
        );
    }

    #[test]
    fn particles_model_keyvalues_and_weapon() {
        let text = r#"<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:generic:version{7412167c-06e9-4698-aff2-e63eb59037e7} -->
{
    // the hero's ambients
    particle_cfg_list = [ { name = resource_name:"particles/heroes/x/idle.vpcf" config = "preview" } ]
    particles_list =
    [
        { name = "particles/heroes/x/hand.vpcf" attachment_type = "PATTACH_POINT_FOLLOW" attachment_point = "attach_hand_r" attachment_offset = [ 1.0, 0.0, -2 ] },
        { name = "particles/heroes/x/hand.vpcf" attachment_type = "PATTACH_POINT_FOLLOW" },
        { name = "../escape.vpcf" }
    ]
    CCitadelHeroModelGameData_t = { m_hAmbientParticle = resource:"particles/heroes/x/body.vpcf_c" }
    blob = #[ 0a ff ]
    note = """
multi
line
"""
}"#;
        let key_values = parse_kv3_text(text).unwrap();
        assert_eq!(key_values["blob"], json!([10, 255]));
        assert_eq!(key_values["note"], json!("multi\nline"));
        let mut found = Effects::default();
        model_keyvalue_effects(&key_values, &mut found);
        let mut issues = Vec::new();
        let weapon = json!({
            "m_PassiveWeaponParticle": "particles/weapons/glow.vpcf",
            "m_strPassiveWeaponAttachmentSource": "muzzle",
            "m_BatonFlameParticle": "particles/abilities/unicorn/flame.vpcf"
        });
        let decode = |path: &str| {
            (path == "particles/abilities/unicorn/unicorn_ambient_weapon.vpcf").then(|| {
                definition(
                    &[
                        "particles/abilities/unicorn/flame.vpcf",
                        "particles/abilities/unicorn/flame.vpcf_c",
                    ],
                    json!([]),
                )
            })
        };
        weapon_effects(&weapon, &mut found, &mut issues, &decode);
        assert!(issues.is_empty());
        let summary: Vec<(&str, &str, Option<&str>)> = found
            .effects
            .iter()
            .map(|effect| {
                (
                    effect.system.as_str(),
                    effect.source.as_str(),
                    effect.configuration.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "particles/heroes/x/idle.vpcf",
                    "model-particle-cfg",
                    Some("preview")
                ),
                ("particles/heroes/x/hand.vpcf", "model-particles-list", None),
                ("particles/heroes/x/body.vpcf", "model-ambient", None),
                ("particles/weapons/glow.vpcf", "weapon-passive", None),
                (
                    "particles/abilities/unicorn/unicorn_ambient_weapon.vpcf",
                    "weapon-passive",
                    Some("preview")
                ),
            ]
        );
        assert_eq!(found.issues, vec!["invalidSystem:../escape.vpcf"]);
        let hand = &found.effects[1];
        assert_eq!(
            hand.attachment,
            Some(EffectAttachment {
                kind: "point_follow".into(),
                point: "attach_hand_r".into(),
                offset: [1.0, 0.0, -2.0],
            })
        );
        assert_eq!(
            found.effects[3].attachment.as_ref().unwrap().point,
            "muzzle"
        );
        assert_eq!(found.effects[0].id, "ambient_0");
        assert_eq!(
            found.effects[0].seed,
            string_token("particles/heroes/x/idle.vpcf_c")
        );

        // A preview that does not place the named particle is not shown.
        let mut found = Effects::default();
        let mut issues = Vec::new();
        let renamed = json!({ "m_BatonFlameParticle": "particles/other.vpcf" });
        weapon_effects(&renamed, &mut found, &mut issues, &decode);
        assert!(found.effects.is_empty());
        assert_eq!(issues, vec!["weaponParticleUnplaced:m_BatonFlameParticle"]);
    }

    #[test]
    fn particles_attachments_parse() {
        let mesh = json!({ "m_attachments": [ { "key": "attach_hand_r", "value": {
            "m_name": "attach_hand_r", "m_bIgnoreRotation": false, "m_nInfluences": 2,
            "m_influenceNames": ["hand_R", "arm_R"],
            "m_vInfluenceOffsets": [[1, 2, 3], [0, 0, 0]],
            "m_vInfluenceRotations": [[0, 0, 0, 1], [0, 0.5, 0, 0.5]],
            "m_influenceWeights": [0.75, 0.25],
            "m_bInfluenceRootTransform": [false, true]
        } } ] });
        let attachments = parse_attachments(&mesh);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "attach_hand_r");
        assert_eq!(attachments[0].influences.len(), 2);
        assert_eq!(attachments[0].influences[0].offset, [1.0, 2.0, 3.0]);
        assert_eq!(attachments[0].influences[1].rotation, [0.0, 0.5, 0.0, 0.5]);
        assert!(attachments[0].influences[1].root_transform);
    }

    #[test]
    fn particles_paths_and_files() {
        assert!(valid_system("particles/a/b-c_d.vpcf"));
        assert!(!valid_system("particles/../b.vpcf"));
        assert!(!valid_system("materials/b.vpcf"));
        assert!(valid_texture("materials/particle/x.vtex"));
        assert!(valid_texture("dev/vgui/icon.vtex"));
        assert!(!valid_texture("x.vtex"));
        assert!(!valid_texture("materials/x.vmat"));
        assert_eq!(
            texture_file("materials/particle/x.vtex"),
            "materials_particle_x_vtex.png"
        );
        let mut used = BTreeSet::new();
        assert_eq!(unique_file("a.png", &mut used), "a.png");
        assert_eq!(unique_file("A.png", &mut used), "A_2.png");
    }

    #[test]
    fn particles_bundle_json_matches_the_contract() {
        let bundle = Bundle {
            schema: SCHEMA,
            owner: "inferno".into(),
            systems: BTreeMap::from([(
                "particles/x/y.vpcf".to_string(),
                BundleSystem {
                    data: json!({}),
                    format: Some("vpcf66".into()),
                },
            )]),
            textures: BTreeMap::from([(
                "materials/particle/x.vtex".to_string(),
                BundleTexture {
                    file: "tex/materials_particle_x_vtex.png".into(),
                    width: 256,
                    height: 256,
                    sheet: Some(Sheet {
                        sequences: vec![SheetSequence {
                            id: 0,
                            clamp: false,
                            alpha_crop: false,
                            no_color: false,
                            no_alpha: false,
                            duration: 1.0,
                            frames: vec![SheetFrame {
                                duration: 0.1,
                                images: vec![[0.0, 0.0, 0.5, 0.5]],
                            }],
                        }],
                    }),
                },
            )]),
            attachments: vec![Attachment {
                name: "attach_hand_r".into(),
                ignore_rotation: false,
                influences: vec![Influence {
                    bone: "hand_R".into(),
                    offset: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    weight: 1.0,
                    root_transform: false,
                }],
            }],
            effects: vec![Effect {
                id: "ambient_0".into(),
                system: "particles/x/y.vpcf".into(),
                source: "model-particles-list".into(),
                attachment: Some(EffectAttachment {
                    kind: "point_follow".into(),
                    point: "attach_hand_r".into(),
                    offset: [0.0; 3],
                }),
                attachment_cp1: None,
                configuration: None,
                transform: None,
                control_points: vec![ControlPoint {
                    index: 1,
                    transform: None,
                    value: Some([0.0; 3]),
                }],
                start_active: true,
                seed: 12345,
            }],
            issues: vec!["limit:systems".into()],
        };
        let value = serde_json::to_value(&bundle).unwrap();
        let keys = |value: &Json| {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        let sorted = |list: &[&str]| {
            let mut list: Vec<String> = list.iter().map(|key| key.to_string()).collect();
            list.sort();
            list
        };
        assert_eq!(
            keys(&value),
            sorted(&[
                "schema",
                "owner",
                "systems",
                "textures",
                "attachments",
                "effects",
                "issues"
            ])
        );
        assert_eq!(
            keys(&value["systems"]["particles/x/y.vpcf"]),
            sorted(&["data", "format"])
        );
        let texture = &value["textures"]["materials/particle/x.vtex"];
        assert_eq!(keys(texture), sorted(&["file", "width", "height", "sheet"]));
        let sequence = &texture["sheet"]["sequences"][0];
        assert_eq!(
            keys(sequence),
            sorted(&[
                "id",
                "clamp",
                "alphaCrop",
                "noColor",
                "noAlpha",
                "duration",
                "frames"
            ])
        );
        assert_eq!(
            keys(&sequence["frames"][0]),
            sorted(&["duration", "images"])
        );
        assert_eq!(
            sequence["frames"][0]["images"],
            json!([[0.0, 0.0, 0.5, 0.5]])
        );
        assert_eq!(
            keys(&value["attachments"][0]),
            sorted(&["name", "ignoreRotation", "influences"])
        );
        assert_eq!(
            keys(&value["attachments"][0]["influences"][0]),
            sorted(&["bone", "offset", "rotation", "weight", "rootTransform"])
        );
        let effect = &value["effects"][0];
        assert_eq!(
            keys(effect),
            sorted(&[
                "id",
                "system",
                "source",
                "attachment",
                "attachmentCp1",
                "configuration",
                "transform",
                "controlPoints",
                "startActive",
                "seed"
            ])
        );
        assert_eq!(
            keys(&effect["attachment"]),
            sorted(&["type", "point", "offset"])
        );
        assert_eq!(
            keys(&effect["controlPoints"][0]),
            sorted(&["index", "transform", "value"])
        );
        assert_eq!(effect["attachmentCp1"], Json::Null);
        assert_eq!(value["schema"], json!(1));
    }
}
