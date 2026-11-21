# 3D viewer: asset pipeline and file contract

Status: active, 2026-11-20. The asset half is built (`deadlock_core::viewer`, `dah viewer`). The
renderer half is built (`rust/crates/sigil-viewer`, `rust/crates/desktop/src/viewer/`, see
"Renderer" below). The app places it in the Locker, Installed, a mod's details and a full-screen
Hideout (see "Where it appears"); every placement and state is checked in headless scenes on a
stand-in hero and room. No real-game export has been run yet (no Deadlock install on the build
Mac): what a real hero or the real `dl_hideout.vpk` looks like in the app is unverified, and the
Hideout's entity and world-node parsing is verified only against Grimoire's schema.

## Goal

A player sees a hero exactly as the game draws it with the mods they have installed: enabled
mods, or a disabled one to preview before switching it on. Animated, orbitable, with a before and
after switch against the unmodded hero, in Locker, Installed and a mod's details. Then the
Hideout (`dl_hideout`) with those mods applied and the hero standing at the player spawn.

## Split

Two crates talk through files, so neither links the other:

- `deadlock_core::viewer` (pure Rust, no GPU) reads the game's VPKs read only, decodes models with
  vpkmerge's `morphic` (linked library, MIT) and writes glbs plus JSON into a cache folder.
- `sigil-viewer` (wgpu, offscreen) opens those files and draws them. It never reads a VPK.

The app passes `<data>/viewer-cache` (`desktop/src/data.rs` `data_dir()`) as the cache folder.
Every call is blocking, meant for a worker thread, and takes a cancel flag.

## Units and axes

Everything in every file is glTF space: **Y up, meters, right handed**. Source 2 is Z up, X
forward, Y left, in inches. morphic wraps each model in VRF's `TRANSFORMSOURCETOGLTF`, which
works out to:

```text
gltf = 0.0254 * (source.y, source.z, source.x)
```

- Source forward (+X) is glTF **+Z**. Source up (+Z) is glTF **+Y**. Source left (+Y) is glTF +X.
- A hero glb stands on `y = 0` at the origin, facing **+Z**.
- A scene placement is a Source matrix `M` emitted as `S * M * S^-1`: the renderer multiplies a
  glb (already in glTF space) by the emitted `transform` and is done.
- Matrices are 4x4 **column major**, 16 floats, the glTF order.
- Colors are **sRGB 0 to 1** unless a field says linear.
- Angles in `cone` are degrees as authored. `yawDegrees` is a Source yaw (about glTF +Y).

## The VPK stack

`viewer::sources(game_root, variant, include_disabled_slots)` returns the stack in game
precedence, first wins:

1. `citadel/grimoire/pakNN_dir.vpk`, the Global priority folder, when `gameinfo.gi` mounts it.
2. Enabled addon slots in load order, lowest slot first (`citadel/addons`, then
   `citadel/sigil-addons2..10` for slots 100 to 990). A disabled slot passed in
   `include_disabled_slots` joins at the position it would load from.
3. `citadel/pak01_dir.vpk`.
4. `core/pak01_dir.vpk`.

`Original` is 3 and 4 only. The Hideout puts `citadel/maps/dl_hideout.vpk` above 3.

For a hero, mods that cannot change it are dropped before keying: a mod stays when it ships the
body model, anything in the model's folder, or a `models/heroes*` or `materials/models/heroes*`
path that belongs to the hero. When none stay, a `modded` request is the `original` export (same
key, same files, `variant: "original"` in the sidecar). The UI can compare the two keys to say
"no installed mod changes this hero".

## Cache layout and keys

```text
<cache>/heroes/<key>/<codename>.glb
<cache>/heroes/<key>/<codename>.json          sidecar
<cache>/heroes/<key>/<codename>.cloth.json    when the model has cloth
<cache>/hideout/<key>/room.json               the room without a hero (internal)
<cache>/hideout/<key>/deps.json               every resource the room read (internal)
<cache>/hideout/<key>/scene.json              scene without a hero
<cache>/hideout/<key>/scene-<hero key>.json   scene with that hero export at the spawn
<cache>/hideout/<key>/models/<hash>.glb
<cache>/hideout/<key>/lightmap/irradiance.hdr
<cache>/hideout/<key>/sky/{px,nx,py,ny,pz,nz}.hdr
<cache>/hideout/<key>/reflection/{px,nx,py,ny,pz,nz}.hdr
```

A key is 24 hex characters of SHA-256 over: the pipeline version (`sigil-viewer-4`), the pinned
vpkmerge revision, the hero codename and model entry (heroes), the texture cap, the effective
variant, and for every VPK in the stack its kind, path, size and modification time (mods also
their chunk files). A game update, a new exporter, a changed or reordered mod all make a new key;
nothing else does. Names are not in the key: a renamed mod refreshes the sidecar in place.

An entry is written into `.tmp-*` and renamed into place when complete, so a reader never sees
half an entry and two exports of one key cannot interleave. A hit writes `.used`; `prune_cache`
removes the least recently used entries past a byte budget, never one used in the last five
minutes, and clears `.tmp-*` folders over an hour old. Only one heavy export runs at a time in a
process (Deadlock Mod Manager's single decode slot); a waiting call re-checks the cache first.

## Hero preview: `<codename>.glb`

Binary glTF 2.0 written by `morphic::model::to_glb_textured`:

- LOD0 meshes, skin and skeleton. Bone local transforms stay in Source space under the wrapper
  node, so clips retarget by bone name.
- PBR materials with embedded PNG textures, capped at 1024 px on the long edge by default
  (`max_texture_edge`; `--full-textures` lifts it). Material `extras.morphic` (schema 4) carries
  the Source 2 shader name, texture reflectivity and the remaining parameters; see vpkmerge
  `docs/hero-glb-material-data.md` for the channel packing (pbr.vfx metalness in B, roughness in
  G, AO in R).
- Outline and jitter shells are left out.
- glTF animations, idle first, named as `clips` lists them. The `root_motion` bone's
  translation is removed so the hero idles in place.

## Hero sidecar: `<codename>.json`

```json
{
  "schema": 1,
  "hero": "inferno",
  "displayName": "Infernus",
  "variant": "modded",
  "clips": ["ui_shop_idle"],
  "idleClip": "ui_shop_idle",
  "sources": [
    { "slot": 3, "file": "addons/pak03_dir.vpk", "name": "Ember Infernus", "kind": "addon", "enabled": true, "used": 14 },
    { "slot": null, "file": "pak01_dir.vpk", "name": "Deadlock", "kind": "base", "enabled": true, "used": 52 },
    { "slot": null, "file": "core/pak01_dir.vpk", "name": "Deadlock core", "kind": "core", "enabled": true, "used": 1 }
  ],
  "modelEntry": "models/heroes_wip/inferno/inferno.vmdl_c",
  "clipInfo": [
    { "name": "ui_shop_idle", "durationSeconds": 4.0, "fps": 30.0, "frameCount": 121,
      "playback": "loop", "action": "idle", "origin": "clipFile",
      "resource": "models/heroes_wip/inferno/clips/ui_shop_idle.vnmclip_c",
      "label": "Idle", "group": "idle", "speed": 1.0,
      "showcase": { "graph": "models/heroes_wip/inferno/inferno_ui.vnmgraph_c", "state": "shop" },
      "floor": null }
  ],
  "cloth": "inferno.cloth.json",
  "units": "meters",
  "up": "+Y",
  "forward": "+Z",
  "key": "3f0c...",
  "issues": [],
  "pipeline": "sigil-viewer-4"
}
```

| Field | Meaning |
| --- | --- |
| `schema` | File contract version, 1. |
| `hero` | Class codename from `scripts/heroes.vdata_c`, without `hero_`. |
| `displayName` | Roster display name. |
| `variant` | The look the glb shows. `original` when no installed mod touches the hero. |
| `clips` | glTF animation names in the glb, in selection order (the showcase first when there is one). Same names in the modded and original export, so a before/after view can play both in step (unless a mod's clips change the staged-pose answer below, which can keep the standing idle on one side only). |
| `idleClip` | The clip to start on (the default clip, not always an idle): the standing idle when the showcase is staged (below), else the showcase, else the hero's first recipe clip present, else a standing or relaxed idle, else `hero_pose` or `ui_hero_pose`, else the first clip. Null when the model offers none (bind pose). |
| `sources` | The stack the export read, highest precedence first. `slot` null for game files. `used` counts files read from that VPK. |
| `modelEntry` | The compiled body model inside the VPKs. |
| `clipInfo` | Per clip: length, rate, frame count, `playback` (`loop` or `hold`: play once and hold the last frame), `action` (`idle`, `relaxedIdle`, `heroPose`, `running`, `reload`, `dash`, `heavyMelee`), `origin` (`model`: embedded in the shown model; `base`: the game's own model at the same path, bound by bone name, used when a skin has no clip files; `clipFile`: a `.vnmclip_c`), `resource` (the `.vnmclip_c` path for a clip file), `label` (English picker name: `Idle`, `Relaxed idle`, `Hero pose`, `Run`, `Reload`, `Dash`, `Heavy melee` from the recipe, else the clip's own words, `Ui shop idle`), `group` (`idle`, `movement`, `combat` or `other`, Grimoire's `groupHeroClips` token rules), `speed` (playback rate multiplier: the graph clip node's `m_flSpeedMultiplier` for a showcase, else 1), `showcase` (null, or `{ "graph", "state" }` with `state` `shop` or `heroSelect` when the game's UI graph plays this clip on that screen; its `playback` is already the clip's), `floor` (null, or how far the clip lowers the hero's floor contact below the rest pose's, in meters, negative: Grimoire's `grimoireFloor`; the studio floor goes down with it while the clip plays; absent in older files). |
| `cloth` | File beside the glb with the model's `PHYS.m_pFeModel` as JSON (vpkmerge's `export_femodel_json` shape: nodes, rods, capsules, integrator), or null. Source space. |
| `issues` | `changesNotShown:<file>`: a mod in the stack none of whose files were read (Deadlock Mod Manager's warning). `clipRig:<name>` / `clipSkeleton:<name>`: a clip file left out because it does not fit the rig. |

Clip choice (Grimoire's `heroAnimationCatalog.ts`, `heroRiggedClips.ts` and
`heroAnimationMetadata.ts`). The model's DATA block names its AnimGraph2 graphs
(`m_animGraph2Refs`); every graph's `m_resources` are candidate clip files, and the `ui` graph is
read for a showcase: a state of the root state machine whose path ends in `/shop`, `/hero_shop`,
`/select` or `/hero_select`, with no layer weight, root motion weight or bone mask, not an off
state, whose child is a plain clip node (a data slot, no reverse or reset input, a positive
speed). Its clip plays looped when the node allows looping, else held, at the node's speed. A shop
state wins over hero select for the same clip. A showcase clip authored for a skeleton the model
does not list in `m_vecNmSkeletonRefs` is ignored. The selection, in order:

1. the showcase (shop before hero select, at most 2,400 frames),
2. the reviewed per-hero recipes (for example Infernus `shop_menu_base` held, Haze
   `shop_menu_base` plus a held reload), or the standing idle, relaxed idle and hero pose
   recipes for a hero without one; beside a showcase only the first idle recipe hit is kept, as a
   standing fallback,
3. beside a showcase with no idle recipe hit, the first standing or relaxed idle,
4. with nothing selected, the first neutral idle,
5. clips the compiled data marks standalone, by name, while under eight clips,

all within 10,000 frames; with nothing at all, the menu pose vpkmerge would bake, held. A clip
qualifies only as a complete full-body action: more than one frame and over 1 ms long, not
additive, hidden, delta, base-dependent, a transition, flagged non-standalone, or moving the hero
(root motion: average linear speed over 0.01 or turn rate over 0.001 rad/s, so a running clip
that travels is left out), and no layer or transition token in its name (`aim`, `add`, `delta`,
`layer`, `intro`, `death`, ...). Embedded clips carry no flags of their own: the first eight
recipe clips borrow `additive` and root motion from the clip file of the same name, or its
reviewed alias (`PRODUCTION_CLIP_ALIASES`, for example Infernus `shop_menu_base` from
`ui_shop_idle`), when frame counts match and durations agree within half a frame. Clip files come
from the graphs first, then `<model dir>/clips/*.vnmclip_c` sorted; a showcase keeps its file
name, any other file is read only when a recipe could pick it (also named
`<codename>_<clip>`). A showcase or a complete clip file wins over a same-name embedded copy,
because the game's graph plays the files. A skin that ships no clip files takes its motions from
the game's model. The `root_motion` bone's translation is still removed from every clip, for
embedded clips no file vouches for.

Staged poses (Grimoire's `heroStagedPose.ts` and `writeHeroAnimationPolicy`, in
`viewer/staged.rs`). Some UI states are staged for the game's own menu set and look wrong alone:
Graves' shop pose lays the spectral hand on the shop floor, Rem's sits him on a seat the preview
does not have. When the export carries a showcase and a standing fallback (the first other clip
whose action is `idle` or `relaxedIdle`), the written glb is sampled: nine poses per clip, from
start to end, bone origins only. Lengths are fractions of the hero's height, the rest-pose height
span of the bones some vertex weighs over 5%. The showcase is staged when

- a weighted bone whose parent is a joint moves more than 25% of the height further from that
  parent than at rest and comes to rest under 10% of the height above the floor (but not more
  than one height below it, an authored hide), in any sample;
- the weighted bones never span half the rest height (the hero lies flat; Viscous on Kelvin's
  pelvis-less skeleton); or
- the floor contacts (weighted bones resting within -2% to +5% of the height of the origin) stay
  more than 8% of the height higher than the standing idle holds them (only a positive idle
  offset counts, so a hero that hovers in every clip keeps its showcase).

A staged showcase stays in `clips` but `idleClip` names the standing idle; an unstaged one keeps
`idleClip` and the standing idle is dropped from the glb, `clips` and `clipInfo`. A glb the test
cannot read counts as not staged. Every remaining clip also gets its floor offset: the lowest its
floor contacts come relative to their rest heights over the samples (lifts below -25% of the
height are a hide and ignored), snapped to 0 within 0.5% of the height. Only a lowering is
written (`floor`); the studio floor sits at the hero's origin (or its lowest point when the pose
lifts it all above) plus the playing clip's `floor`, so a shop idle that sinks the feet keeps
them on the floor.

## Hideout scene: `scene.json` / `scene-<hero key>.json`

```json
{
  "schema": 1,
  "map": "dl_hideout",
  "variant": "original",
  "key": "a91e...",
  "units": "meters",
  "up": "+Y",
  "exposure": 1.0,
  "post": {
    "tonemap": { "exposureBias": 0.8992, "shoulderStrength": 0.3538, "linearStrength": 0.3258, "linearAngle": 0.2528,
                 "toeStrength": 0.6966, "toeNum": 0.0, "toeDenom": 0.7819, "whitePoint": 3.9996 },
    "bloom": { "blendMode": "add", "bloomStrength": 0.5, "screenBloomStrength": 0.0, "blurBloomStrength": 0.0,
               "threshold": 1.0, "thresholdWidth": 0.999, "blurWeights": [5 floats], "blurTints": [[r, g, b] x 5] },
    "colorCorrection": { "file": "post/color-correction.bin", "dimension": 32 }
  },
  "models": [
    { "glb": "models/5b1c....glb", "transform": [16 floats], "tint": [1, 1, 1, 1], "primitive": null, "animation": null },
    { "glb": "models/77ad....glb", "transform": [16 floats], "tint": [0.8, 0.8, 0.8, 1], "primitive": 3, "animation": null },
    { "glb": "models/c2f0....glb", "transform": [16 floats], "tint": [1, 1, 1, 1], "primitive": null, "animation": "idle_loop" }
  ],
  "lights": [
    { "kind": "spot", "color": [1, 0.85, 0.7], "intensity": 80.0, "position": [x, y, z], "direction": [x, y, z],
      "range": 12.7, "cone": [20, 45], "classname": "light_barn", "realtime": true,
      "temperature": null, "shadowChannel": 1, "fogStrength": null,
      "brightness": 1.0, "skirt": 0.1, "skirtNear": 0.0, "fogOnly": false,
      "cookie": "cookies/fan_cookie_1a2b3c4d.png", "basis": [[x, y, z], [x, y, z], [x, y, z]],
      "fogShadow": { "tiles": [[w, h, x, y]], "size": [x, y, z], "shear": [x, y] } }
  ],
  "fog": { "color": [r, g, b], "strength": 1.0, "indirectStrength": 1.0, "drawDistance": 60.0,
           "fadeIn": [0, 2], "transform": [16 floats], "min": [x, y, z], "max": [x, y, z],
           "irradiance": { "file": "fog/irradiance.vtv", "worldToLocal": [16 floats] },
           "volumes": [ { "shape": "box", "worldToLocal": [16 floats], "strength": 1.0, "falloff": 1.0,
                          "indirectStrength": null, "tint": null } ],
           "shadows": "fog/shadows.vtv" },
  "sky": { "cubemap": ["sky/px.hdr", "sky/nx.hdr", "sky/py.hdr", "sky/ny.hdr", "sky/pz.hdr", "sky/nz.hdr"],
           "exposureBias": 0.0, "tint": [1, 1, 1], "position": null },
  "reflection": { "cubemap": ["reflection/px.hdr", "..."], "exposureBias": 0.0, "tint": [1, 1, 1], "position": [x, y, z] },
  "lightmap": { "file": "lightmap/irradiance.hdr", "uvScale": [u, v], "uvSet": 3,
                "direction": "lightmap/direction.png", "shadows": "lightmap/shadows.png" },
  "probes": [ { "file": "probes/0.vtv", "shadows": "probes/0-shadows.vtv", "worldToLocal": [16 floats],
                "size": [8, 8, 4], "offset": [0, 0, 0], "atlasSize": [w, h, d], "fade": [4, 0, 4],
                "priority": 0, "encoding": "linear" } ],
  "camera": { "position": [x, y, z], "target": [x, y, z], "fov": 60.0 },
  "heroAnchor": [16 floats],
  "hero": { "hero": "inferno", "glb": "../../heroes/<key>/inferno.glb", "sidecar": "../../heroes/<key>/inferno.json",
            "transform": [16 floats], "key": "<hero key>" },
  "spawn": { "position": [x, y, z], "yawDegrees": 90.0 },
  "sources": [ { "slot": null, "file": "maps/dl_hideout.vpk", "name": "Hideout", "kind": "map", "enabled": true, "used": 412 } ],
  "failures": [],
  "omissions": ["particles", "staticBundle"],
  "pipeline": "sigil-viewer-4"
}
```

| Field | Meaning |
| --- | --- |
| `models[].glb` | Relative to the scene file's folder. One glb per distinct model, pose, skin and material override set; many instances share it. |
| `models[].transform` | Placement in glTF space, column major. Multiply the glb's scene by it. |
| `models[].tint` | sRGB multiplier and opacity (`rendercolor`/`renderamt` for props, `m_vTintColor` for world objects and fragments), or null for white. Grimoire multiplies it with the draw call's own `extras.morphic_tint` in sRGB. |
| `models[].primitive` | For an aggregate fragment: draw only the glTF primitive with this index within its mesh (Grimoire matches `parser.associations` primitive index). Null draws the whole glb. |
| `models[].animation` | Name of the glTF animation to loop for an animated prop, or null for a static (baked) one. |
| `lights[]` | Lights not wholly baked into the lightmap: realtime lights, and baked ones that still light fog (`realtime: false`, `fogStrength` set). `intensity` is candela for point and spot, the authored brightness for directional. `direction` is the way the light shines. `range` meters. `cone` inner and outer angle in degrees for `light_barn` (and `[0, 90]` for `light_rect`), null otherwise. `temperature` kelvin when the light uses color temperature. |
| `lights[].brightness` | Linear brightness the game's light shader takes: `2^brightness * brightnessscale` for omni, barn and rect lights, `brightness * brightnessscale` for `light_environment`. |
| `lights[].skirt`, `skirtNear` | Range falloff skirts (`skirt`, default 0.1; `skirt_near`, default 0, Grimoire's defaults). Null for `light_environment`. |
| `lights[].fogOnly` | True for a baked light kept only because it lights fog (the inverse of `realtime`). |
| `lights[].cookie` | `lightcookie` of an omni or barn light as an sRGB PNG, at most 512 px, named `cookies/<stem>_<sha8>.png` (sha256 of the compiled path), or null. A cookie that fails to export leaves null and keeps `lightCookies` in `omissions`. |
| `lights[].basis` | The light entity's own axes in glTF space, unit length: Source local +X (forward), +Y (left), +Z (up), from its angles. Orients cookies: barn cookies in the light's local frame, omni cookies by the local ray. |
| `exposure` | The master `post_processing_volume`'s fixed exposure (Grimoire's `fixedMapExposure`: exposure off gives `2^exposurecompensation`; equal min and max (linear, else log2) give `min * 2^exposurecompensation`). 1 when the map has no master volume. Left out when the exposure adapts, and `autoExposure` stays in `omissions`. |
| `post` | The master volume's `.vpost`, or null when it has none or it fails to decode. `tonemap` is `m_toneMapParams` (`whitePoint` in stops, as stored), null unless `m_bHasTonemapParams`. `bloom` is `m_bloomParams` (`blendMode` `add`, `screen` or `blur`), null unless `m_bHasBloomParams`. `colorCorrection` is `m_colorCorrectionVolumeData` written raw: RGBA8, `dimension^3 * 4` bytes, x fastest, then y, then z; null when the dimension is 0 or 1. |
| `fog` | The master `env_volumetric_fog_controller`: tint, strengths, distances in meters, and its box (placement plus local min and max in meters, glTF axes). Null when absent or disabled. |
| `lights[].fogShadow` | A fog light's tiles in `fog.shadows` (Grimoire's `lightFogShadowTiles`): `precomputed_shadowmap_atlas_rect_0..5` as `[w, h, x, y]` in atlas uv, in order until the first missing one, kept only as one tile (barn) or six (omni faces forward, -up, up, left, -left, -forward); `size` is `size_params` and `shear` the entity's `shear`, Source units. Null without tiles, or when the atlas did not export. |
| `fog.shadows` | The world's `m_lightMaps` entry `.../precomputed_shadowmap_atlas.vtex` as a VTV (R16, depth 1): per texel `1 - ndc.z` of the nearest surface in the light's own frustum. Null when no fog light has tiles or the atlas fails (it is then a failure and `fogShadows` stays in `omissions`). |
| `fog.irradiance` | The controller's `fogirradiancevolume` as a VTV file, with `worldToLocal` over the controller's box. Null when the controller names none or it fails. |
| `fog.volumes[]` | Enabled `env_volumetric_fog_volume` entities, at most 16 (`fogVolumeLimit` past that): `shape` `box` (0) or `sphere` (1), `strength` (`FogStrength`), `falloff` (`FalloffExponent`), `indirectStrength` and `tint` (0 to 1) only when their override flag is set. Shape 2 (height fog) is skipped and adds `heightFogVolumes`. |
| `worldToLocal` | On probes, fog irradiance and fog volumes: column major, takes a glTF world point (meters) to the box's normalized `[0, 1]^3` (Grimoire's `(inverse(matrix) * p - min) / (max - min)` with the glTF to Source conversion folded in). The local axes are the entity's Source axes (x forward, y left, z up), not glTF axes, matching the atlas layout. |
| `probes[]` | Light probe volumes (entities with `lightprobetexture` and `light_probe_size_x/y/z`), all on one atlas like Grimoire (`multipleProbeAtlases` when another is named). `file` is the irradiance atlas (VTV; its depth is six equal bands, one per cube face, +X +Y +Z -X -Y -Z in Source axes), `shadows` the `lightprobetexture_dlshd` atlas or null. `size` probes per axis, `offset` the atlas texel offset (`light_probe_atlas_x/Y/Z`), `atlasSize` the atlas texture's full width, height and depth. `fade` is box size over `edge_fade_dists` per axis, 0 on an axis without a fade (no edge fade). `priority` is `indoor_outdoor_level`. `encoding` is `rgbm` when the world's `m_nLightmapGameVersionNumber` is 0, else `linear`. |
| VTV files | Uncompressed: u32 LE magic `0x31565456` (`VTV1`), width, height, depth, component (1 RGBA8, 2 RGBA16F, 3 R16), then mip 0 texels tightly packed, x fastest, then y, then z, no color conversion. Grimoire's VTV1 without the gzip. |
| `sky` | The enabled `env_sky` cube as six Radiance `.hdr` faces (linear). Faces are named in Source axes as vpkmerge writes them: `px` looks along Source +X, which is glTF +Z; `pz` along Source +Z (glTF +Y). `exp2(exposureBias) * tint` scales it. |
| `reflection` | The `env_cubemap` nearest the spawn, same face layout, with its capture `position`. |
| `lightmap` | The baked `directional_irradiance_sh2_dc` lightmap as Radiance `.hdr`, capped at 2048, with the world's `m_vLightmapUvScale`. `uvSet` is 3: morphic writes every vertex TEXCOORD stream as `TEXCOORD_<Source semantic index>`, so room glbs carry lightmap UVs as `TEXCOORD_3` (Grimoire samples `uv3`); a primitive without it is not lightmapped (Grimoire lights it from the probes). `direction` (`directional_irradiance_sh2_lum`) and `shadows` (`direct_light_shadows`) are UNORM RGBA8 PNGs, no sRGB conversion, at most 2048, written only for `m_nLightmapVersionNumber` 8 with `m_nLightmapGameVersionNumber` 5; null otherwise. |
| `camera` | Behind the spawn as the game's third-person camera: 180 in back, eye 62 in up, target 42 in up, 60 degree vertical field (Grimoire's measured oracle cameras). |
| `heroAnchor` | Where the hero stands: the player spawn (`info_team_spawn` named `*player_spawn*`) turned 180 degrees to face the camera. Null when the build has no spawn. |
| `hero` | Present in `scene-<hero key>.json`: the hero export's files, relative to the scene folder, and the same transform as `heroAnchor`. |
| `variant` | The look of the room. A mod changes the room only when it ships a file the unmodded room read (`deps.json`) or anything under `maps/dl_hideout`, so with only hero skins installed the room is `original` while the hero is modded. |
| `failures` | Resources that failed to export; their instances are left out. |
| `omissions` | Parts of the game's scene not in this file, so the renderer does not guess. Always `particles` (map particles) and `staticBundle` (Grimoire's merged meshopt bundle). Others appear when something is missing or failed: `autoExposure`, `tonemap`, `colorCorrection`, `directionalLightmap`, `lightCookies`, `lightProbes`, `fogVolumes`, `fogIrradiance`, `fogShadows`, `multipleProbeAtlases`, `heightFogVolumes`, `fogVolumeLimit`, `entityAttachments`, `instanceStreams`, `animatedProps`, `playerSpawn`. |

Static props and world objects are baked into their starting pose (`ref` for world objects; the
entity's `StartingAnim`, `IdleAnim` or `defaultanim` for props). Props that loop their starting
animation keep their rig and that one clip. Props parented to another entity are skipped
(`entityAttachments`). Room textures are capped at 256 px (Grimoire's room cap), the hero at
1024.

`HideoutExportRequest::progress` takes a `Fn(f32) + Sync` that hears 0.0 to 1.0, never going
back, over real stages: the hero (the first 30% when one is asked, scaled from the hero
export's own progress), then the world read, lightmap, entities, lights and post,
probe and fog volumes, reflection, world nodes, and every model exported counted over the
total. A cache hit reports 1.0 at once.

## API

```rust
use deadlock_core::viewer::{self, Variant};

viewer::heroes(game_root) -> Result<Vec<HeroEntry>>          // codename, displayName, modelEntry
viewer::find_hero(game_root, "Infernus") -> Result<HeroEntry>
viewer::sources(game_root, Variant::Modded, Some(&[7])) -> Vec<Source>
viewer::heroes_in_vpk(&vpk) -> Result<Vec<String>>           // codenames a mod changes
viewer::export_hero(game_root, cache, "inferno", Variant::Modded, &[], &cancel) -> Result<Exported>
viewer::export_hero_with(&HeroExportRequest { library, max_texture_edge, extra_slots, exclude_slots, .. }, &cancel)
viewer::export_hideout(game_root, cache, Variant::Modded, Some("inferno"), &cancel) -> Result<PathBuf>
viewer::export_hideout_with(&HideoutExportRequest { .. }, &cancel)
viewer::prune_cache(cache, keep_bytes) -> Result<PruneReport>
viewer::is_cancelled(&error) -> bool
```

`Exported { glb, sidecar, cloth, key, variant, cached }`. A second call with unchanged inputs
returns `cached: true` without opening a model.

`extra_slots` adds switched-off slots (preview a mod before turning it on); `exclude_slots` leaves
enabled slots out (the hero without one mod, the "before" side of a mod's preview while it is on).
Both change the stack and so the key.

## CLI

```text
Sigil cli viewer heroes [--json]
Sigil cli viewer mod-heroes <pakNN_dir.vpk> [--json]
Sigil cli viewer export-hero <hero> [--original] [--with-slot N]... [--out DIR] [--full-textures] [--json]
Sigil cli viewer export-hideout [--hero H] [--original] [--with-slot N]... [--out DIR] [--json]
Sigil cli viewer prune [--keep-mb 2048] [--out DIR] [--json]
```

`--out` is the cache folder to write into (default `<data>/viewer-cache`); the command prints
the file paths. The game comes from discovery or `DAH_GAME_DIR`. The real-game test is
`SIGIL_GAME_ROOT=<Deadlock> cargo test -p deadlock-core viewer -- --ignored` (output in
`.scratch/viewer-samples/cache`, or `SIGIL_VIEWER_OUT`; hero from `SIGIL_VIEWER_HERO`).

## Where it comes from

- Grimoire (MIT), ported: the content-addressed hero cache over resolved source VPKs and the
  reviewed model entries (`heroPoseModels.ts`); skins without clip files taking the game's
  motions (`recoverSkinAnimations`); the clip recipes, standalone filter and bounds
  (`heroAnimationCatalog.ts`); the staged-pose test and per-clip floor offsets
  (`heroStagedPose.ts`, `writeHeroAnimationPolicy` in `heroAnimationExport.ts`); the Hideout
  world-node, aggregate-fragment, entity-lump, light, spawn, prop, animated-prop, sky and fog
  reading (`hideoutScene.ts`, `hideoutAtmosphere.ts`,
  `source2Entity.ts`, `hideoutTransform.ts`); the room and hero texture caps; the camera.
- Deadlock Mod Manager (GPL, behaviour only, no code): per-source dependency reporting and the
  `changesNotShown` warning when a mod's files are never read (Foundry `model_assets.rs`); one
  model decode at a time; cancellable jobs; the before/after comparison playing the stock clip
  at the selected clip's normalized phase (`comparison-playback.ts`), which this pipeline
  supports by giving both exports the same clip names.
- Sigil's own: any number of stacked mods resolved across open VPKs instead of a merged temp VPK;
  disabled-slot preview; dropping mods that cannot change the hero so the modded and original
  previews share an entry; the room keyed on the mods that touch what the room reads.

## Particle bundle: `particles/bundle.json`

Written by `viewer/particles.rs` beside a hero's sidecar (`"particles": "particles/bundle.json"`)
and the Hideout scene (`"particles"`), read by `sigil_viewer::particles`. Everything inside is
Source space (inches, Z up); the renderer applies Source to glTF and the placement.

```json
{
  "schema": 1, "owner": "inferno",
  "systems": { "particles/x/y.vpcf": { "data": { "...": "the .vpcf_c DATA as JSON" }, "format": "vpcf66" } },
  "textures": { "materials/particle/x.vtex": { "file": "tex/materials_particle_x_vtex.png", "width": 256, "height": 256,
                "sheet": { "sequences": [ { "id": 0, "clamp": false, "alphaCrop": false, "noColor": false, "noAlpha": false,
                           "duration": 1.0, "frames": [ { "duration": 0.1, "images": [[0, 0, 0.5, 0.5]] } ] } ] } } },
  "attachments": [ { "name": "attach_hand_r", "ignoreRotation": false,
                     "influences": [ { "bone": "hand_R", "offset": [0, 0, 0], "rotation": [0, 0, 0, 1], "weight": 1.0, "rootTransform": false } ] } ],
  "effects": [ { "id": "ambient_0", "system": "particles/x/y.vpcf", "source": "model-particles-list",
                 "attachment": { "type": "point_follow", "point": "attach_hand_r", "offset": [0, 0, 0] }, "attachmentCp1": null,
                 "configuration": null, "transform": null, "controlPoints": [], "startActive": true, "seed": 12345 },
               { "id": "anim_1", "system": "particles/x/burst.vpcf", "source": "anim-event",
                 "attachment": { "type": "point_follow", "point": "head", "offset": [0, 0, 0] }, "attachmentCp1": null,
                 "configuration": null, "transform": null, "controlPoints": [], "startActive": false, "seed": 6789,
                 "clip": { "name": "hero_shop", "start": 0.25, "stops": [ { "time": 0.75, "immediately": true, "endCap": false } ],
                           "stopOnClipChange": true } } ],
  "issues": [],
  "bones": [ { "name": "hand_R", "bind": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1.5, -10.8, 31, 1] } ],
  "snapshots": { "particles/x/y.vsnap": { "numParticles": 2, "strings": ["hand_R"], "columns": [
                 { "name": "position", "type": "float3", "data": "vector", "values": [0, 0, 1, 0, 0, 2] },
                 { "name": "skinning", "type": "skinning", "data": "skinning",
                   "values": { "joints": [0, 0, 0, 0, 0, 0, 0, 0], "weights": [1, 0, 0, 0, 1, 0, 0, 0] } } ] } }
}
```

Textures are top-mip PNGs (straight alpha, at most 1024 px); sheet rects are uv `[u0, v0, u1, v1]`.
`source` is one of `model-particles-list`, `model-particle-cfg`, `model-ambient`, `hero-ambient`
(kept with `startActive: false`, as Grimoire leaves it out of the showcase), `weapon-passive`,
`anim-event` and `map-entity` (with `transform`, the entity matrix, and `controlPoints` from
`cpointN`, `tint_cp`, `data_cp`). Limits: 160 systems 12 levels deep and 128 textures a bundle
when written; the renderer loads at most 256 systems an effect, 512 a bundle and 32 children a
system, sheets at most 1024 sequences of 1024 frames of 16 images (65,536 in all), textures at
most 4096 x 4096.

`anim-event` effects (Grimoire's `discoverAnimationEvents`) come from the `AE_CL_CREATE_PARTICLE_EFFECT`
and `AE_CL_CREATE_PARTICLE_EFFECT_CFG` events of the clips the glb plays (their `.vnmclip_c`
`m_events[]`, or for an embedded clip the model's legacy `ANIM` block `m_eventArray[]` of that
name), then the rest of the `ui` graph's clips (named by file stem), at most 48 clip files. Only
`clip` effects carry `clip` (the key is absent otherwise): `name` is the glb animation name,
`start` and each stop's `time` are normalized clip time (0 to 1), `stops` are the same clip's
`AE_CL_STOP_PARTICLE_EFFECT` events naming the same system at or after the start
(`stop_instantly` or `stop_immediately` set `immediately`, `play_endcap` sets `endCap`), and
`stopOnClipChange` is `stop_on_seq_change`. The attachment comes from `attachment_type` (default
`absorigin_follow`, lowercased, `pattach_` dropped) and `attachment_point`, `attachmentCp1` from
the `_cp1` keys; a `_CFG` event names a `configuration` and no attachment. They are written with
`startActive: false` and started by the renderer as the clip passes them.

`bones` (hero bundles, present only when the model has a skeleton) are the model's bones with
their model-space bind poses (Source space, column major), which skinned snapshots deform by.
`snapshots` holds every `.vsnap_c` a system names in `m_hSnapshot`, decoded from its DATA block
and block compressed SNAP block; `strings` is the DATA block's string list (joint and hitbox
names), `values` are numbers for `float`, `int` and `vector` (three a row, flattened), indices
into `strings` for `string`/`bone` (-1 for one outside it), and `{ joints, weights }` with four of
each a row for `skinning`. Names are never copied per row, so a small file cannot decode into a
large one. Limits: 4 MB a compiled file, 65,536 rows, 4,096 strings of at most 256 bytes, a SNAP
block that inflates to at most 16 MB and holds exactly the rows its attributes ask for (up to 16
bytes of padding), and 32 MB of decoded rows and strings for all of a bundle's snapshots. Both
keys are absent when empty.

## Not done yet

- The per-clip `floor` moves only the studio floor. In the Hideout the hero stands at the
  spawn on the room's own floor; Grimoire lowers its scaled room by the offset, and Sigil does
  not move the hero or the room for it yet.
- Compiled `hidden`, `delta`, `requiresBase`, `transition` and `standalone` flags for embedded
  clips: the pinned vpkmerge does not report them, so only clip file metadata (additive, root
  motion) feeds the filter, and step 5 of the clip choice finds nothing yet.
- The Hideout's meshopt static bundle and height fog volumes. Each is listed in `omissions`.
- Particle models (`C_OP_RenderModels`): the bundle carries no meshes yet and the renderer has
  no model draw path (bind-pose meshes per particle need a mesh export into the bundle and an
  instanced lit pipeline), so the class stays in `Viewer::particle_diagnostics`. Also not ported:
  `C_OP_MaintainEmitter` (so its snapshot count), the other snapshot functions, a snapshot that
  `info_particle_system` overrides with `snapshot_file`, and per-effect model bones other than
  control point 0's hero. Animation-event effects keep at most two playing instances each
  (Grimoire makes one per event); a third event restarts the oldest.
- Material groups and skins per hero form; WIP heroes whose pose lives only in a loose NM pose
  clip.
- A real-game run. The ignored test and the CLI are ready for the first machine with Deadlock.

## Where it appears

The screens never open files themselves. They ask `desktop/src/viewer/prepare.rs` for a view under
a key; it runs the export on a worker thread into `<data>/viewer-cache`, shows the stage while it
runs ("Preparing Haze with your mods", step 1 of 2, Cancel), and opens the viewport when the files
are ready. A cache hit opens at once. Every ask has an owner; when the owner goes (the player leaves
the Locker's hero page, closes the dialog, leaves full screen) or another hero is picked under the
same key, the export is cancelled through its flag and the view closed. When the installed mods
change under an open view (a skin turned on in the Locker, Turn on in the dialog), it is asked
again, so the view follows the game. About 30 seconds after launch the cache is pruned once, on a
worker, to 2 GB (`prune_cache`, least recently used first).

| Place | How a player gets there | What it shows |
| --- | --- | --- |
| Locker, hero page | Rail: **3D view** (a switch, kept across heroes, as Grimoire's 3D toggle) | The hero with every enabled mod against the unmodded game (With mods / Without mods / Split). It takes the 2D art's place right of the sections; the sections pane narrows to 380 points to keep the view about 440 wide, and below 300 points of room the view replaces the pane. Previous and next hero re-ask under the same key. |
| Locker, skin rows | The cube button on each skin, on or off | Preview in 3D for that one skin on that hero (below). |
| Locker, Hideout | Rail: **Hideout** | The Hideout full screen with the player's mods and this hero at the spawn (Grimoire puts the Hideout in its hero viewer as a scene; Sigil gives it the whole window). The strip's last button closes it. |
| Installed, card menu | ... > **Preview in 3D** ("while off" on a switched-off mod) | Preview in 3D dialog. Offered only when `heroes_in_vpk` says the mod changes a hero model: every installed slot is read once per addon scan on a worker and kept by slot, file and size. |
| Installed, a multi-file mod's file list | The cube button on each file | The same, for that file. |
| Mod details (Browse) | **Preview in 3D** beside "Installed in slot NN" | The same, for the installed slot. |

The **Preview in 3D** dialog (`viewer/dialog.rs`) shows the mod's hero with the mod's slot in the
stack, switched on or not (`extra_slots`), against the hero as it is now without it
(`exclude_slots` when it is on). The compare buttons read With this mod / Without this mod. A mod
that changes several heroes gets a hero picker in the header. A switched-off mod says so and offers
Turn on (the same switch as the card). Escape closes it; full screen works from the strip.

Notes under the view's title: "No installed mod changes Haze." when the modded and original exports
share a key (no compare buttons then), "Neon does not change how Haze looks." for a mod whose stack
equals the one without it, and "Neon changes nothing this view draws." for each `changesNotShown`
issue in the sidecar (Deadlock Mod Manager's warning).

States: no game folder ("Deadlock was not found", Open Settings), the export failed (the error,
Try again, Send feedback through `notify::feedback`), no GPU (the hero's Locker art as the still
with a one-line note, or the message where a screen has no picture). Demo mode uses the stand-in
hero and room.

Headless scenes (`cargo run -p dah-desktop -- --snap out.png --scene <name>`): `viewer-locker`,
`viewer-locker-loading`, `viewer-locker-failed`, `viewer-locker-nogame`, `viewer-locker-nogpu`,
`viewer-locker-hideout`, `viewer-hideout-loading`, `viewer-installed-menu`, `viewer-mod-dialog`,
`viewer-mod-dialog-loading`, `viewer-details`, and the bare viewport's `viewer`, `viewer-split`,
`viewer-hideout`, `viewer-controls`, `viewer-backdrop-ember`, `viewer-backdrop-studio-gradient`,
`viewer-backdrop-none`, `viewer-transparent`, `viewer-gif-review` and `viewer-recording`. The
stand-in Hideout (`fixture_room::upgrade_hideout`) bakes its own lightmap (DC, direction, a
stationary light's shadow mask) with `TEXCOORD_3` UVs, a probe volume, a colour grade, a cookie,
a fog volume and map particles, so `viewer-hideout` exercises every Hideout path.

What a tester on Windows with Deadlock should check first: the Locker's 3D view on a hero with an
enabled skin (first export time, the hero standing and animating, With mods against Without mods),
a switched-off skin through Installed > ... > Preview in 3D, the Hideout from the Locker rail, and a
second open of each (it should come from the cache at once).

## Renderer

`rust/crates/sigil-viewer` draws the files above on its own wgpu device (wgpu `~30.0.1`, the
version and per-platform backend Rune links: Metal, D3D12 with FXC, Vulkan). It never reads a VPK
and never touches the UI.

```rust
let mut v = sigil_viewer::Viewer::new()?;              // Err(ViewerError::NoGpu) without an adapter
v.load_hero_file(&glb, Some(&sidecar))?;               // or set_hero(model::load_path(..)?, &Sidecar::read(..))
v.set_before(Some(stock_model), &stock_sidecar);       // the unmodded side
v.set_scene(sigil_viewer::load_scene(&scene_json)?);   // Hideout: props, lights, fog, sky, probe
v.compare = Compare::Split(0.5);                       // After, Before or Split(fraction)
v.camera.orbit(dx, dy); v.camera.zoom(0.9); v.camera.pan(dx, dy, h); v.reset_camera();
v.set_clip("shop_menu_base"); v.seek(1.0); v.playback.speed = 0.5; v.playback.playing = false;
let rgba: &[u8] = v.render(width, height, dt);         // sRGB RGBA8 rows, top first
let png = v.capture_png(width, height)?;
```

- Loading: glTF 2.0 through the `gltf` crate (the one morphic links), every node, skin, clip
  (linear, step, cubic) and embedded image; tangents are generated when missing.
- Skinning on the GPU from one matrix palette per model; a clip plays by name, loops or holds
  per the sidecar's `clipInfo.playback`; the stock side plays the modded clip's phase (Deadlock Mod
  Manager's comparison playback).
- Materials honour vpkmerge's contract: base color sRGB, metalness in MR blue, roughness in
  green, AO red, `pbr.vfx` NPR normals decoded hemi-octahedrally, `g_vColorTint1` decoded from
  sRGB once and applied through the tint mask, color correction about `texture_reflectivity`,
  self-illumination gated on its scale, authored shells hidden unless Grimoire's rule reveals
  them, glass and additive drawn blended. `pbr.vfx`'s alt albedo (`g_tAltColor_Reveal`: RGB a
  second albedo, alpha a reveal mask) wipes over the color by `g_flAltTextureAmount` with
  `g_flAltTextureRevealSoftness` edges; a decompiled amount expression is evaluated for a hero
  at rest (`$AmmoPercent` 1, `$ReloadProgress` 0, `time()` 0), so Violet's brush shows inked.
  An expression outside the small evaluator keeps the authored amount (`material/alt_reveal.rs`).
- Lighting is Grimoire's Citadel NPR port: stepped wrapped diffuse, ambient bands at fixed
  luminance targets, the upward silhouette rim with its hero depth test, NPR-scaled reflections,
  the lit inverted-hull outline (0.1 game units times the outline mask). The studio rig, its
  calibration and the hero inspect post curve (Caldera) come from Grimoire; a scene uses the
  base Deadlock curve, its realtime lights (directional, point, spot), the fog, the sky cube as
  backdrop and the reflection cube for reflections and ambient.
- Frame: hero depth prepass, 4x MSAA HDR pass per shown model, bloom (threshold, five-level
  chain), post into an sRGB target, read back.
- Bounds (`load_scene_with`, `set_scene_with`): each distinct glb is read and uploaded once and
  its placements share it (an aggregate is placed once per fragment; reading it per placement
  ran the app out of memory and is what hung the first Windows Hideout). At most 2,048 files and
  20,000 placements, files over 512 MB unread, 2 GB of decoded models. On the GPU every buffer is
  checked against the device's `max_buffer_size`, textures against `max_texture_dimension_2d`,
  and a viewer keeps to 1 GB of textures and 1.5 GB of geometry (half on integrated and software
  adapters): past that textures lose their top mips, then are dropped, and meshes are left out,
  each recorded in `Viewer::usage`. Prop bounds are measured once and still props posed once.
  The load reports progress and stops when the view closes.
- Errors: the device's uncaptured-error and lost-device handlers record the first error instead
  of panicking; the pipelines are built inside error scopes, and when the driver rejects them
  (FXC on D3D12 is the risk) the software adapter (WARP) is tried before the view fails with the
  reason, which says whether WARP was tried. WARP compiles with the same FXC, so the shaders
  themselves must suit it: no write through a dynamic index into a local vector, matrix or
  struct array (FXC's X3500 and X3511, which broke 0.1.20 on Windows) and no implicit-LOD
  sampling in loops. `src/shaders/fxc.rs` checks naga's HLSL for both on every platform, and
  `.github/workflows/windows-shaders.yml` builds every pipeline on D3D12 WARP. A frame waits at most 20 seconds on the GPU. The render thread runs under
  `catch_unwind`, so a panic is a "could not open" state with its message, never a view left on
  Loading. Adapter, counts, sizes, timings and what was left out go to Sigil's log.

- Shadows: the studio key and the Hideout's nearest realtime directional or spot light cast a
  2048 shadow map (3x3 PCF, slope bias, normal offset) onto the hero, the room and the studio
  floor, which is Grimoire's 0.25 shadow catcher. Alpha-tested parts stay out of the map; a
  stationary light (baked shadow channel) shadows only the hero, its static shadow being baked.
  Point lights cast none (Grimoire's map lights cast none at all).
- Hideout lighting (Grimoire's `source2Lightmap`, `source2ProbeVolume`, `source2DynamicLights`,
  `source2LightCookies`, `source2Atmosphere`, `sourcePostProcess`): the SH2 lightmap read through
  `TEXCOORD_3` with the lightmap reflection blend on rough baked surfaces; the light probe volumes
  as the hero's NPR ambient cube and the ambient of props without a lightmap, with their shadow
  volume; Source light units (linear brightness, 10000 / d^2 in inches, skirt); cookies (barn
  framed in its cone, omni wrapped, luminance only); fog-only lights kept off surfaces; the
  baked shadow mask squared for point and spot lights; the map's fixed exposure, tone curve,
  bloom (VRF threshold window, per-level weights and tints by blend constant, add, screen, blur)
  and colour correction volume (indexed by the display-encoded colour); the volumetric fog
  marched through the fog volumes (32 midpoint steps, 0.01 per inch, the irradiance volume and
  the fog lights, each shadowed by its tiles in the precomputed shadow atlas: a 2x2 compare of
  the stored `1 - ndc.z` in the barn's frustum or the omni's cube face). As Grimoire, the march
  runs at half resolution (`Look::half_res_fog`, on): the opaque depth drawn at half size, one
  march per block to it, and each opaque surface blends the four nearest blocks bilinearly,
  dropping blocks whose depth is more than 5 % off its own (the closest block when none is near).
  Blended surfaces march per fragment. The sky is left clear of fog, as in Grimoire.
- Distant props (Grimoire's room clearance, `sigil_viewer::clearance`): placements whose bounds
  lie over 65 m from the spawn are kept only when the room opens toward them. Once per scene the
  near room's opaque surfaces are drawn depth only into six 32 x 32 views from a metre above the
  spawn; a far prop stays when a point of a 5 x 5 x 5 grid over its box is nearer than the free
  distance in its direction (99 %) or looks out through an opening (the 1000 m reach).
  `SceneReport::culled` counts what went.
- Cloth (`sigil_viewer::cloth`, Grimoire's solver): the hero's `<codename>.cloth.json` stepped at
  1/120 s on the clip's own clock (at most 12 substeps, 8 ms), reset on a clip change or seek.
- Particles (`sigil_viewer::particles`, a subset of Grimoire's port of VRF's): the hero's bundle
  on its posed attachments and the map's effects lit by its sun, simulated in Source space,
  drawn as instanced spritecards (and trails) in the scene pass after the surfaces. Classes the
  port lacks are listed in `Viewer::particle_diagnostics`, never guessed. A new bundle gets a
  second of simulation before its first frame so a still shows it. Ropes (`C_OP_RenderRopes`,
  Grimoire's `renderRopes.ts`) are Catmull-Rom ribbons through the particles, cut into a power of
  two of pieces from the rope's screen coverage, V from world arc length over
  `m_flTextureVWorldSize` with scroll, offset and the carried V when the first particle dies,
  closed loops and split segments; each piece is a strip quad (instance mode 3) in the same
  spritecard pipeline. Animation-event effects start and stop as the playing clip passes their
  events (`ParticleGpu::set_clip`, called from `Viewer::step_particles` with the clip's glb name
  and `time / duration`): the events in (last, now] pass, a loop passes both ends, a clip change
  stops the effects with `stopOnClipChange`, and a finished one goes idle (no auto restart).
  Skinned snapshots (`C_INIT_InitSkinnedPositionFromCPSnapshot`, `C_OP_SnapshotRigidSkinToBones`,
  snapshot counts on the instantaneous and continuous emitters, `PF_TYPE_SNAPSHOT_COUNT`) bind a
  system's `m_hSnapshot` to its control point and deform the points by control point 0's bones:
  each frame the host builds a palette of `posed bone * inverse bind` from the bundle's `bones`
  and the attachment lookup (a bone name resolves to the bone, unless an attachment has that
  name).
- Looks (`Look`): Grimoire's backgrounds (None, Transparent, a picture, the ten soul gradients),
  Deadlock Mod Manager's studio gradient, Midtown dusk lighting, bloom, shadows, cloth and
  particles switches; `auto_rotate` turns the hero at 0.25 rad/s (Grimoire's turntable).

Not consumed yet: light `temperature`. Point and spot lights without `brightness` (older exports) use `intensity` as
candela over distance squared. The cube face orientation (each face's up) follows glTF/OpenGL
convention and is unverified against a real vpkmerge cube; none of the Hideout data has been
seen from a real export yet.

### Viewport

`rust/crates/desktop/src/viewer/` owns one render thread per viewport. The thread renders into
a `rune::Pixels` at the element's device size (capped at about 3.2 million pixels), only while
the element is on screen and a clip plays or the camera moved, paced to 60 frames a second;
otherwise it sleeps on its channel. The UI thread only forwards input.

```rust
viewer::open_hero(s, key, title, Source::Hero { glb, sidecar }, Some(Source::Hero { .. }));
viewer::open_scene(s, key, title, scene_json, None);   // the scene's own hero, or Some((glb, sidecar))
viewer::view(s, &u, key) -> El                          // fills its parent
viewer::set_still(s, key, pixels);                      // shown when the machine has no GPU
viewer::close(s, key);
```

Drag orbits, the wheel zooms, Shift-drag pans, double-click resets, and with the view focused the
arrows orbit, + and - zoom, Space plays, Home resets, F toggles full screen. Escape leaves full
screen from anywhere in the window (and closes the Hideout, which lives only there). Every state
has a way out: Loading shows what it does ("Reading the room's models: 137 of 412"), the time so
far and Cancel or Close, which stops the load; Failed shows the reason with Try again, Send
feedback and Close; full screen keeps a close button in the corner whatever the state.
The loader holds the preview until it is whole (Grimoire's `PreviewLoader`): the hero, the
compare side, the room and its upload all load before the view turns Ready, and Ready waits for
the first frame ("Drawing the first frame"), with a 20 second fallback when no frame comes.
The strip has play and pause, seek (pauses), speed, the clip, With mods / Without mods / Split
(the split line drags), camera reset, a PNG capture into `Pictures/Sigil`, and full screen
(`viewer::overlay`, placed by `app::view`). Below 720 points wide the strip folds into two rows
and drops the clock, speed and clip (full screen has them). Headless snapshots render one fixed
frame on the calling thread, at the size a scene gives with `viewer::headless_points`.

The strip also has (`viewer/controls.rs`) Auto-rotate, the Controls panel, Record a GIF and Record
a video. The Controls panel (Grimoire's, anchored to its button by `viewer::popover`) holds the
clip picker grouped Idle, Movement, Combat, Other with the sidecar's friendly names, Lighting
(Studio, Midtown dusk), Background (Studio, None, Transparent, Deadlock Mod Manager's studio
gradient, Grimoire's ten soul gradients, a picture of the player's up to 12 MB) and switches for
Auto-rotate, Cloth, Particles, Bloom, Shadows and (where the device can draw lines) Deadlock Mod
Manager's Wireframe, its exposure slider (plus
or minus two stops in quarter stops over the scene's exposure; the value resets it) and "Ask
where to save", kept across sessions in `<data>/viewer.json`. The studio camera's angle (yaw,
pitch and zoom over the framing) is kept per hero in `<data>/viewer-camera.json` when a view
closes and restored when that hero opens again; Reset camera returns to the framing. A Hideout
keeps its own camera.
GIF (Grimoire's `viewerGif.ts`): up to four seconds at 12 frames a second and 480 px, then a review
that loops the take, trims start and end on a strip of six thumbnails (arrows step a frame, Shift
twelve), and saves, retakes or cancels. Video (Deadlock Mod Manager's): up to ten seconds at 30
frames a second, a REC pill with the clock, saved as WebM on every platform. The VP8 encoder and
WebM writer are Sigil's own pure Rust (`sigil_viewer::capture`, written from RFC 6386): 16x16
intra and whole-pixel motion compensated inter frames, a key frame every 3 seconds, no loop
filter, encoded on its own thread with the long edge capped at 1280 (about 25 ms a frame there,
under 5 ms at 640x400). ffmpeg's decode matches the encoder's reconstruction bit for bit. Motion
JPEG `.mov` and `.avi` remain as fallback formats in the library. Recordings and pictures go to
`Pictures/Sigil`; with "Ask where to save" on, a picture and a GIF open a save dialog instead
(a video still lands in `Pictures/Sigil`). Taking a picture flashes the stage until it is saved
(Deadlock Mod Manager's capture flash). A one-mod view (Preview in 3D) has "Use this view as
the mod's cover": the frame is saved into `<data>/mod-images` and becomes the mod's picture,
Deadlock Mod Manager's rendered cover for local mods, done by the player's framing instead of
a background render. The Locker's In-game card dialog has "Use the 3D view" per card variant
while the hero page's 3D view is open: its frame, posed and framed by the player, is centre
cropped to the variant like a chosen image (Grimoire's 3D card snapshots; the panorama card as
backdrop and the pose-cycle arrows are not built). Grimoire's keys join: W A S D orbit, Q and E zoom, R resets, T turns the
turntable. The export panel shows a real percentage from the pipeline's progress callback.

### Cost

Measured on an Apple M3 (release, the stand-in hero, 31k triangles):

| Size | Frame | Without readback | Readback (copy, map, unpad) |
| --- | --- | --- | --- |
| 1280 x 800 | 2.9 ms | 2.5 ms | 0.17 ms |
| 2560 x 1600 | 5.4 ms | 3.5 ms | 1.5 ms |

With everything on (release, `cargo test --release -p sigil-viewer --test parity_bench -- --ignored
--nocapture`, the same M3, 1280 x 800):

| Scene | Frame |
| --- | --- |
| Stand-in hero: cloth, ember plume (45 particles), shadow map, bloom | 2.7 ms |
| The same without particles and shadows | 2.5 ms |
| Stand-in Hideout: lightmap, probes, fog volume march with shadows (half resolution, 32 steps), cookies, map post, 108 particles | 6.6 ms |
| The same with the fog marched per surface fragment (32 steps) | 19.1 ms |

Cloth alone: 0.005 ms a frame for the stand-in (21 nodes), 0.12 ms for a synthetic 401-node
rig. Particles alone: 0.2 ms a frame to step 2,000 particles and build their cards. The
Hideout used to cost 11.5 ms with a 16 step march per surface fragment; the half resolution
march (twice the steps, with fog shadows) brought it to 6.6 ms. Grimoire measured 16.5 to 12.0 ms
for the same change.

The UI side then copies each frame into Rune's image atlas and uploads it (one more 4 to 16 MB
copy a frame on the UI's render pass). That copy, not the GPU, is the ceiling at full-screen
Retina sizes. The zero-copy path is Rune's `NativeSurface` (`vendor/rune/crates/rune/src/native_surface/`,
used by video): render into an IOSurface-backed Metal texture on macOS or a D3D12 shared texture
on Windows and hand the handle to the view, so no frame crosses the CPU. It needs the viewer's
device to create exportable textures (wgpu-hal access to the Metal and D3D12 resources) and is
not built.
