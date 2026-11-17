# Locker 3D hero viewer

Status: design only, 2026-11-17. Nothing here is built. This records what Rune would need
before a posed hero can spin in the Locker, so the next round starts from facts instead of a
webview.

## The decision in one line

A 3D viewer needs a mesh path, a material path and a depth-tested 3D pass. Rune has none of
the three today: it draws an instanced quad renderer with text, images and rounded rects.
The honest options are to extend Rune with a scene layer, or to render a still on a worker.
The webview route is closed by `docs/decisions/0006-native-rust-rune-desktop.md`.

## What the reference apps do

- Grimoire draws the hero with three.js in the renderer process: a glTF asset (or a
  converted Source 2 model) is loaded by a small scene, lit with a fixed key and rim light,
  and posed from an animation clip. The camera orbits on drag and returns to a fixed
  three-quarter view.
- Deadlock Mod Manager uses the same idea with its own asset set. Both ship converted models
  with the app or download them; neither reads the user's VPK at view time.

Sigil cannot ship those assets: they are Valve's, and the CSDK has no licence that allows
redistribution. Any viewer must read the user's own install.

## Rune today

From the vendored revision:

- Paint is an instanced wgpu uber-quad: one vertex buffer of quads, a texture atlas and a
  glyph atlas. There is no vertex format for arbitrary meshes, no index buffer, no depth
  attachment and no camera uniform.
- Layout and hit testing are 2D. `El::canvas` is a 2D paint hook, not a 3D surface.
- Images arrive as straight RGBA8 (`ImageCache`), decoded on worker threads. There is no
  texture compression in the pipeline.
- The single render pass is ordered by paint position; alpha is blended, depth is not tested.

That means a mesh cannot be an `El` without a new primitive, and a naive painter's-order
draw of a skinned model will show wrong faces as soon as the camera moves.

## What a viewer needs, in order

### 1. Mesh data

A Deadlock hero is a compiled Source 2 model. Inside `citadel` VPKs that is a chain:

- `vmdl_c`: model header, body groups, material groups, attachments, hitboxes;
- `vmesh_c` or embedded mesh blocks: vertex streams, index buffer, per-LOD ranges;
- `vmat_c`: material graph with texture slots;
- `vtex_c`: compiled texture (BC1 to BC7, or RGBA), plus mip chains.

Two ways to get triangles:

1. Parse the user's files in `deadlock-core`: a Source 2 mesh reader that understands the
   vertex stream declarations, index size and the material table. That is a new binary
   format reader with its own tests and a lot of edge cases (LODs, morphs, flexes).
2. Convert once, locally, through Source2Viewer CLI, the way recolor already plans to
   decompile: the user's own machine produces a glTF or OBJ and Sigil reads that. No Valve
   bytes leave the machine and no asset is redistributed. The cost is an external tool the
   user must install, and a conversion that can fail per hero.

Option 2 is the smaller first step and matches the recolor brief model: record the intent,
hand the compile step to a tool, never claim an asset exists until it does.

### 2. Materials and textures

- Read `vmat_c` for the base colour, normal, mask and blend slots.
- Decompress `vtex_c` on a worker to RGBA8. BC7 and BC6H are the common formats; a decoder
  crate or a hand-written block decoder is needed. Reuse the image cache's mip chain so the
  GPU upload is one texture per material.
- Fall back to a flat colour per material when a texture is missing: a solid, readable
  silhouette is better than a black model.
- Lighting: one key light, one fill, a rim light and a cheap ambient, all in the shader.
  No PBR environment map in the first version.

### 3. Rig, pose and animation

A T-pose is not the reference look. The Locker wants the hero's select pose:

- skeleton and inverse bind matrices from the model;
- skinning weights and joints per vertex;
- one animation clip (the hero's idle or select), or a single baked pose exported by the
  converter.

For the first version, a static select pose is enough. Animation adds a sampler, an
animation clock and a pause when the page is not visible.

### 4. A depth-tested 3D pass in Rune

The real blocker. Rune needs a scene primitive with:

- a vertex format: position, normal, UV, joints, weights, colour;
- an index buffer and per-draw uniform (model matrix, base colour, texture handle);
- a depth texture and depth test in the pass, cleared per scene;
- a camera uniform (view, projection, light directions);
- a clip rectangle and rounded-corner mask so the scene sits inside the Locker's card or
  hero page without bleeding;
- a way to composite the scene under the existing 2D layer, or an offscreen texture that
  the 2D layer draws as an image;
- event routing: drag to orbit, wheel to zoom, and a focus ring for the keyboard.

The offscreen route is the smaller change: render the scene to a texture on the frame, then
draw that texture with the existing image path. It costs one copy but keeps the 2D layer
untouched. It still needs the 3D pipeline, depth pass and camera.

Rough size: a `rune::scene3d` module with its own tests and a small shader, plus an asset
upload lifetime bound to the screen. It is a Rune change, so it needs a pinned revision bump
and a note in `docs/provenance.md`.

### 5. Loading and memory

- Parse and upload on a worker, never on the layout thread.
- One hero at a time: the grid keeps the render images it already has. A viewer is opened
  from the hero page, not the grid.
- Budget: under 60 MB of GPU memory per hero at 1024 texture edges, under 250 ms to first
  frame on an M1 or a mid Windows laptop.
- Cache the converted mesh under `<data>/cache/locker-models/<hero>-<hash>/` with the source
  file's size and modified time in the key. Safe to delete.
- A missing model, an unreadable VPK or a failed conversion shows the render image and a
  short line: the viewer is available when the game files are indexed.

### 6. Controls and settings

- Drag orbits, wheel zooms, double press resets to the three-quarter view.
- The last camera per hero is kept in `locker-prefs.json` only if it is useful; the default
  is the reference angle.
- The viewer is off by default in the sense that the hero page still opens on the render
  image; a "3D" control on the card/hero page opens the scene.

## Dependencies a build would add

- `glam` or `nalgebra` for math (small, MIT).
- A BC texture decoder, or a hand-written BC1 to BC7 decoder in `deadlock-core`.
- A glTF reader if the converter route is used (`gltf` crate, MIT/Apache).
- No three.js, no webview, no remote asset fetch.

Every one is a new crate, so the owner approves it first.

## Milestones

| Step | Result | Effort |
| --- | --- | --- |
| M0 | One converted hero draws as a still in a Locker fixture, depth tested | L |
| M1 | Orbit, zoom, reset, focus ring, clip rounded to the card | M |
| M2 | Select pose from a converted clip; idle animation paused off screen | M |
| M3 | Materials and textures from the user's VPK, flat-colour fallback | L |
| M4 | Every roster hero, cache, budgets, error states | M |

M0 is the gate. Until a triangle with a depth buffer draws inside a rounded clip, the rest
is speculation.

## What the repository can do meanwhile

- Keep the render art on the hero page and the grid (already built).
- Keep the recolor brief honest: it names the files, it never claims a rendered edit.
- Revisit this doc when Rune gains a scene or 3D primitive, and record the decision in
  `docs/decisions/` before adding the pipeline.
