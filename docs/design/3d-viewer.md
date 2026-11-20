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

A key is 24 hex characters of SHA-256 over: the pipeline version (`sigil-viewer-1`), the pinned
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
  "clips": ["shop_menu_base", "primary_stand_reload"],
  "idleClip": "shop_menu_base",
  "sources": [
    { "slot": 3, "file": "addons/pak03_dir.vpk", "name": "Ember Infernus", "kind": "addon", "enabled": true, "used": 14 },
    { "slot": null, "file": "pak01_dir.vpk", "name": "Deadlock", "kind": "base", "enabled": true, "used": 52 },
    { "slot": null, "file": "core/pak01_dir.vpk", "name": "Deadlock core", "kind": "core", "enabled": true, "used": 1 }
  ],
  "modelEntry": "models/heroes_wip/inferno/inferno.vmdl_c",
  "clipInfo": [
    { "name": "shop_menu_base", "durationSeconds": 2.97, "fps": 30.0, "frameCount": 90,
      "playback": "hold", "action": "idle", "origin": "clipFile",
      "resource": "models/heroes_wip/inferno/clips/inferno_shop_menu_base.vnmclip_c" }
  ],
  "cloth": "inferno.cloth.json",
  "units": "meters",
  "up": "+Y",
  "forward": "+Z",
  "key": "3f0c...",
  "issues": [],
  "pipeline": "sigil-viewer-1"
}
```

| Field | Meaning |
| --- | --- |
| `schema` | File contract version, 1. |
| `hero` | Class codename from `scripts/heroes.vdata_c`, without `hero_`. |
| `displayName` | Roster display name. |
| `variant` | The look the glb shows. `original` when no installed mod touches the hero. |
| `clips` | glTF animation names in the glb, idle first. Same names in the modded and original export, so a before/after view can play both in step. |
| `idleClip` | The clip to start on, or null when the model offers none (bind pose). |
| `sources` | The stack the export read, highest precedence first. `slot` null for game files. `used` counts files read from that VPK. |
| `modelEntry` | The compiled body model inside the VPKs. |
| `clipInfo` | Per clip: length, rate, frame count, `playback` (`loop` or `hold`: play once and hold the last frame), `action` (`idle`, `relaxedIdle`, `heroPose`, `running`, `reload`, `dash`, `heavyMelee`), `origin` (`model`: embedded in the shown model; `base`: the game's own model at the same path, bound by bone name, used when a skin has no clip files; `clipFile`: a `.vnmclip_c`). |
| `cloth` | File beside the glb with the model's `PHYS.m_pFeModel` as JSON (vpkmerge's `export_femodel_json` shape: nodes, rods, capsules, integrator), or null. Source space. |
| `issues` | `changesNotShown:<file>`: a mod in the stack none of whose files were read (Deadlock Mod Manager's warning). `clipRig:<name>` / `clipSkeleton:<name>`: a clip file left out because it does not fit the rig. |

Clip choice (Grimoire's `heroAnimationCatalog.ts`): the reviewed per-hero recipe (for example
Infernus `shop_menu_base` held, Haze `shop_menu_base` plus a held reload), else a standing or
relaxed idle, else the menu pose vpkmerge would bake, held. Additive clips and names with a layer
or transition token (`aim`, `add`, `delta`, `layer`, `intro`, `death`, ...) never qualify. At most
eight clips and 10,000 frames. Clip files beside the model (`<model dir>/clips/*.vnmclip_c`,
also named `<codename>_<clip>`) win over a same-name embedded copy, because the game's graph
plays the files. A skin that ships no clip files takes its motions from the game's model.

## Hideout scene: `scene.json` / `scene-<hero key>.json`

```json
{
  "schema": 1,
  "map": "dl_hideout",
  "variant": "original",
  "key": "a91e...",
  "units": "meters",
  "up": "+Y",
  "models": [
    { "glb": "models/5b1c....glb", "transform": [16 floats], "tint": [1, 1, 1, 1], "primitive": null, "animation": null },
    { "glb": "models/77ad....glb", "transform": [16 floats], "tint": [0.8, 0.8, 0.8, 1], "primitive": 3, "animation": null },
    { "glb": "models/c2f0....glb", "transform": [16 floats], "tint": [1, 1, 1, 1], "primitive": null, "animation": "idle_loop" }
  ],
  "lights": [
    { "kind": "spot", "color": [1, 0.85, 0.7], "intensity": 80.0, "position": [x, y, z], "direction": [x, y, z],
      "range": 12.7, "cone": [20, 45], "classname": "light_barn", "realtime": true,
      "temperature": null, "shadowChannel": 1, "fogStrength": null }
  ],
  "fog": { "color": [r, g, b], "strength": 1.0, "indirectStrength": 1.0, "drawDistance": 60.0,
           "fadeIn": [0, 2], "transform": [16 floats], "min": [x, y, z], "max": [x, y, z] },
  "sky": { "cubemap": ["sky/px.hdr", "sky/nx.hdr", "sky/py.hdr", "sky/ny.hdr", "sky/pz.hdr", "sky/nz.hdr"],
           "exposureBias": 0.0, "tint": [1, 1, 1], "position": null },
  "reflection": { "cubemap": ["reflection/px.hdr", "..."], "exposureBias": 0.0, "tint": [1, 1, 1], "position": [x, y, z] },
  "lightmap": { "file": "lightmap/irradiance.hdr", "uvScale": [u, v] },
  "camera": { "position": [x, y, z], "target": [x, y, z], "fov": 60.0 },
  "heroAnchor": [16 floats],
  "hero": { "hero": "inferno", "glb": "../../heroes/<key>/inferno.glb", "sidecar": "../../heroes/<key>/inferno.json",
            "transform": [16 floats], "key": "<hero key>" },
  "spawn": { "position": [x, y, z], "yawDegrees": 90.0 },
  "sources": [ { "slot": null, "file": "maps/dl_hideout.vpk", "name": "Hideout", "kind": "map", "enabled": true, "used": 412 } ],
  "failures": [],
  "omissions": ["autoExposure", "colorCorrection", "directionalLightmap", "fogIrradiance", "fogVolumes",
                "lightCookies", "lightProbes", "particles", "staticBundle", "tonemap"],
  "pipeline": "sigil-viewer-1"
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
| `fog` | The master `env_volumetric_fog_controller`: tint, strengths, distances in meters, and its box (placement plus local min and max in meters, glTF axes). Null when absent or disabled. |
| `sky` | The enabled `env_sky` cube as six Radiance `.hdr` faces (linear). Faces are named in Source axes as vpkmerge writes them: `px` looks along Source +X, which is glTF +Z; `pz` along Source +Z (glTF +Y). `exp2(exposureBias) * tint` scales it. |
| `reflection` | The `env_cubemap` nearest the spawn, same face layout, with its capture `position`. |
| `lightmap` | The baked `directional_irradiance_sh2_dc` lightmap as Radiance `.hdr`, capped at 2048, with the world's `m_vLightmapUvScale`. Whether a room glb carries lightmap UVs depends on the morphic export; treat it as optional. |
| `camera` | Behind the spawn as the game's third-person camera: 180 in back, eye 62 in up, target 42 in up, 60 degree vertical field (Grimoire's measured oracle cameras). |
| `heroAnchor` | Where the hero stands: the player spawn (`info_team_spawn` named `*player_spawn*`) turned 180 degrees to face the camera. Null when the build has no spawn. |
| `hero` | Present in `scene-<hero key>.json`: the hero export's files, relative to the scene folder, and the same transform as `heroAnchor`. |
| `variant` | The look of the room. A mod changes the room only when it ships a file the unmodded room read (`deps.json`) or anything under `maps/dl_hideout`, so with only hero skins installed the room is `original` while the hero is modded. |
| `failures` | Resources that failed to export; their instances are left out. |
| `omissions` | Parts of the game's scene not in this file, so the renderer does not guess. |

Static props and world objects are baked into their starting pose (`ref` for world objects; the
entity's `StartingAnim`, `IdleAnim` or `defaultanim` for props). Props that loop their starting
animation keep their rig and that one clip. Props parented to another entity are skipped
(`entityAttachments`). Room textures are capped at 256 px (Grimoire's room cap), the hero at
1024.

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
  (`heroAnimationCatalog.ts`); the Hideout world-node, aggregate-fragment, entity-lump, light,
  spawn, prop, animated-prop, sky and fog reading (`hideoutScene.ts`, `hideoutAtmosphere.ts`,
  `source2Entity.ts`, `hideoutTransform.ts`); the room and hero texture caps; the camera.
- Deadlock Mod Manager (GPL, behaviour only, no code): per-source dependency reporting and the
  `changesNotShown` warning when a mod's files are never read (Foundry `model_assets.rs`); one
  model decode at a time; cancellable jobs; the before/after comparison playing the stock clip
  at the selected clip's normalized phase (`comparison-playback.ts`), which this pipeline
  supports by giving both exports the same clip names.
- Sigil's own: any number of stacked mods resolved across open VPKs instead of a merged temp VPK;
  disabled-slot preview; dropping mods that cannot change the hero so the modded and original
  previews share an entry; the room keyed on the mods that touch what the room reads.

## Not done yet

- Grimoire's UI AnimGraph2 reader (picking the shop or hero-select state from the compiled
  graph). The recipes it falls back to are ported; heroes without a recipe use a standing idle.
- Hideout probes (`probes.bin`), fog irradiance volume, directional lightmap channels, color
  correction LUT, tone curve and bloom, light cookies, fog shadow atlas, map particles, the
  meshopt static bundle, distant-instance culling. Each is listed in `omissions`.
- Hero ambient particles and effects (`heroAmbientDiscovery.ts`), material groups and skins per
  hero form, and WIP heroes whose pose lives only in a loose NM pose clip.
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
`viewer-hideout`.

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
  them, glass and additive drawn blended.
- Lighting is Grimoire's Citadel NPR port: stepped wrapped diffuse, ambient bands at fixed
  luminance targets, the upward silhouette rim with its hero depth test, NPR-scaled reflections,
  the lit inverted-hull outline (0.1 game units times the outline mask). The studio rig, its
  calibration and the hero inspect post curve (Caldera) come from Grimoire; a scene uses the
  base Deadlock curve, its realtime lights (directional, point, spot), the fog, the sky cube as
  backdrop and the reflection cube for reflections and ambient.
- Frame: hero depth prepass, 4x MSAA HDR pass per shown model, bloom (threshold, five-level
  chain), post into an sRGB target, read back.

Not consumed yet: `lightmap` (room glbs carry no lightmap UVs today), `cloth`, the fog box and
its `indirectStrength`, light `temperature` and `shadowChannel`, shadows of any kind (the studio
draws a contact shadow instead), particles. Point and spot `intensity` is used as authored
(candela over distance squared) with no calibration against the game, since no export has been
seen. The cube face orientation (each face's up) follows glTF/OpenGL convention and is unverified
against a real vpkmerge cube.

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
arrows orbit, + and - zoom, Space plays, Home resets, F toggles full screen and Escape leaves it.
The strip has play and pause, seek (pauses), speed, the clip, With mods / Without mods / Split
(the split line drags), camera reset, a PNG capture into `Pictures/Sigil`, and full screen
(`viewer::overlay`, placed by `app::view`). Below 720 points wide the strip folds into two rows
and drops the clock, speed and clip (full screen has them). Headless snapshots render one fixed
frame on the calling thread, at the size a scene gives with `viewer::headless_points`.

### Cost

Measured on an Apple M3 (release, the stand-in hero, 31k triangles):

| Size | Frame | Without readback | Readback (copy, map, unpad) |
| --- | --- | --- | --- |
| 1280 x 800 | 2.9 ms | 2.5 ms | 0.17 ms |
| 2560 x 1600 | 5.4 ms | 3.5 ms | 1.5 ms |

The UI side then copies each frame into Rune's image atlas and uploads it (one more 4 to 16 MB
copy a frame on the UI's render pass). That copy, not the GPU, is the ceiling at full-screen
Retina sizes. The zero-copy path is Rune's `NativeSurface` (`vendor/rune/crates/rune/src/native_surface/`,
used by video): render into an IOSurface-backed Metal texture on macOS or a D3D12 shared texture
on Windows and hand the handle to the view, so no frame crosses the CPU. It needs the viewer's
device to create exportable textures (wgpu-hal access to the Metal and D3D12 resources) and is
not built.
