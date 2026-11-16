# Can an LLM produce the mesh?

Status: active. Researched 2026-11-16.
Updated: 2026-11-16

Short answer: a general chat model cannot emit a production triangle mesh as
tokens, but a model plus a coding harness can produce one through code, and that
path is real. The confusion is between three different things:

1. **A general LLM writing code.** It writes Blender Python or trimesh code, runs
   it, looks at a render and iterates. This is how coding agents already build
   geometry. It is strong for props, hard-surface, kitbashed and stylized
   characters, and weak for organic likeness without reference guidance.
2. **A mesh-native generative model.** A model trained on 3D data predicts a mesh
   representation directly. MeshGPT is the canonical proof: a decoder-only
   transformer over a learned mesh vocabulary produces compact triangle meshes.
   That is a trained 3D model, not a chat model.
3. **An image-to-3D model.** TRELLIS, Hunyuan3D, Meshy, Tripo and Rodin turn a
   reference image or text into a textured mesh. This is the lane for likeness.

## Evidence

- **MeshGPT** (Siddiqui et al., 2023) generates triangle meshes with a
  decoder-only transformer and a learned latent mesh vocabulary. It demonstrates
  the approach and its limits: the vocabulary and the model are trained on mesh
  data, and the result is category-scale, not a specific hero.
  <https://arxiv.org/abs/2311.15475>
- **MCP for Blender** (MIT, ~34.6k stars) is direct evidence for lane 1. It gives
  an LLM `execute_blender_code`, a `look` tool with multi-angle and rendered
  views, scene introspection, and optional generators (Meshy, Tripo, Rodin,
  Hunyuan3D). The README states the AI writes Blender Python for modelling,
  materials, rigging and retopology, and checks its own work with renders.
  <https://github.com/ahujasid/mcp-for-blender>
- **TRELLIS** (MIT, Microsoft) is a 1.2B image-to-3D model that outputs meshes,
  Gaussians and radiance fields. It needs Linux and an NVIDIA GPU with at least
  16 GB. It is the realistic local lane for likeness, not a model we train.
  <https://github.com/microsoft/TRELLIS>
- **Meshy** is a hosted pipeline with multi-view generation, reconstruction,
  texture and PBR, remesh and rigging. Our earlier study of its client is in
  `docs/research/meshy-reverse-engineering.md`. It is an adapter, not the product.

## What the harness actually adds

The "power the model gets through the harness" is not magic geometry. It is:

- writing a script,
- running it in a contained workspace,
- rendering a turnaround,
- looking at the image,
- correcting proportions, materials and topology,
- repeating.

A coding agent with a Blender socket can do this today. A chat window cannot.

## Subscriptions versus keys

Consumer subscriptions (ChatGPT Plus, Claude Pro) are not APIs. You cannot legally
or technically plug a subscription session into a third-party app by scraping
cookies, and we will not. The supported paths are:

- an API key the user owns, stored in the OS keychain,
- a local model server (ComfyUI, TRELLIS, llama.cpp, any OpenAI-compatible
  endpoint),
- an official OAuth or device flow, only where the provider documents it.

## What this repo now ships

The stage graph in `deadlock-core::character` already separates deterministic
stages from generative ones. `dah character backends` reports the lanes and their
measured availability:

| Lane | What it is | Who runs the mesh |
| --- | --- | --- |
| `blender-code` | LLM-written Blender Python | Blender, locally |
| `local-command` | TRELLIS, Hunyuan3D or a wrapper CLI | the user's model |
| `http` | a local JSON model server | the user's model |
| `meshy` | hosted adapter, bring your own key | Meshy |

`dah character scaffold <slug>` writes `generated/characters/<slug>/mesh.py`, a
real Blender contract:

```text
inputs:  DAH_CHARACTER_DIR, DAH_STAGE_DIR, DAH_PROMPT, DAH_REFERENCE,
         DAH_TARGET_HERO, DAH_POLYCOUNT
output:  <DAH_CHARACTER_DIR>/model.glb
optional:<DAH_STAGE_DIR>/turnaround.png
```

The scaffold is a starting figure, not a finished character. An LLM edits it,
runs the local backend, inspects `turnaround.png`, and iterates. This is the
honest "LLM produces the mesh" path.

## Recommendation

1. **Do not train a mesh model.** A Meshy-class model is a research program with
   a 3D dataset, GPU training and evaluation, not a feature. The moat here is the
   mod manager, the Source 2 pipeline, approvals and verification.
2. **Be the best bring-your-own-backend harness.** Add provider adapters (Meshy,
   Tripo, Rodin) and local lanes (Blender code, TRELLIS/Hunyuan HTTP/CLI). Keys
   stay in the keychain; the transcript never sees one.
3. **Add an LLM planner stage.** Before generation, ask a model for a structured
   plan: per-view prompts, negative prompts, palette, proportions, polycount
   budget, rig template, material notes. The plan is JSON, validated, and stored
   with the task. That is where an LLM earns its place in the pipeline.
4. **Add a concept-image stage.** Text to turnaround sheets through an image
   model, then multi-image to 3D. This is the highest-quality route that does not
   need Meshy.
5. **Keep the Source 2 gate.** No generated mesh is in game until retarget, skin
   weights, `vmdl`/`vmat`, LODs, a CSDK compile and an in-game check happen. The
   stage graph already records that.

## Honest limits

- No Blender is installed on this machine, so the scaffold has not been executed
  here. The backend reports unavailable instead of pretending.
- No local TRELLIS run was performed. The HTTP and command seams are tested with
  scripted servers and fake executables.
- Open image-to-3D quality is below Meshy for a hero-quality textured character.
  It is close for stylized props and mid-detail characters, and it is private and
  free.
- Likeness rights still apply. See ADR 0005.

## Sources

- https://arxiv.org/abs/2311.15475
- https://github.com/ahujasid/mcp-for-blender
- https://github.com/microsoft/TRELLIS
- https://docs.meshy.ai
- `docs/research/meshy-reverse-engineering.md`
- `docs/research/character-generation.md`
