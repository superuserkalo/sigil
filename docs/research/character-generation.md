# Character generation from a reference image

Status: active. Provider docs read 2026-11-16. In-game compile not verified. In the app this page sits under Agent, behind Settings > Experimental > Agent (off by default).
Updated: 2026-11-16

This document covers the hosted image-to-3D backend, what it returns, and why a
generated mesh is not a Deadlock hero model. The executable side is
`rust/crates/deadlock-core/src/character.rs`. The likeness rules come from
`docs/decisions/0005-reference-images-and-likeness.md`.

## What the pipeline does

The pipeline is a stage graph, not one provider call. The observed vendor
process it is modelled on, and the evidence for each stage, is in
`docs/research/meshy-reverse-engineering.md`. Eleven stages run in this order:

1. `prepare_reference`: deterministic. Format, dimensions, alpha channel, matte
   coverage, subject centring and alpha edge quality. No model, no key.
2. `generate_views`: generative. Multi-view or single-view image conditioning.
3. `reconstruct`: generative. Views to a mesh.
4. `cleanup`: deterministic measurement of triangle count against the LOD0
   budget, plus an optional decimation backend call when the mesh is over
   budget.
5. `uv_bake`: skipped when the mesh already carries `TEXCOORD_0`, otherwise a
   backend stage.
6. `pbr_synthesize`: skipped when the materials already reference textures,
   otherwise a backend stage.
7. `retopo`: backend stage, gated on a mesh.
8. `rig`: backend stage, gated on a mesh and on the rigging triangle limit.
9. `animate_retarget`: backend stage, gated on a succeeded `rig`.
10. `source2_map`: deterministic. Writes `source2-plan.json` with the `vmdl`,
    `vmat`, `vtex`, compiled `_c` paths, skeleton and weight requirements and the
    AnimGraph 2 binding. Always `inGameReady: false`.
11. `export`: deterministic. Copies the GLB to `export/<slug>.glb` and records
    its CRC32.

Every stage records `id`, `state` (`skipped`, `pending`, `running`, `succeeded`,
`failed`, `unsupported`), `backend`, `inputs`, `outputs`, `bytes`, `hash`,
`notes` and the evidence id from the research doc. The graph is persisted in
`task.json` and copied into `character.json`.

A generative stage with no configured backend is `unsupported` with a precise
reason. It is never skipped silently and never reported as success. A task whose
generative stages are all unsupported ends `BLOCKED`, which is an honest final
state: nothing was generated and nothing is claimed.

Backends:

| Backend | Configured by | Stages |
| --- | --- | --- |
| Local command | `DAH_MESHGEN_COMMAND` plus `DAH_MESHGEN_ARGS` (JSON array) and optional `DAH_MESHGEN_STAGES` | declared stages, default `generate_views,reconstruct` |
| HTTP JSON server | `DAH_MESHGEN_URL`, optional `DAH_MESHGEN_TOKEN` | declared stages |
| Meshy | `--provider meshy` and `MESHY_API_KEY` | the single provider task, recorded as `generate_views` plus `reconstruct` |
| Mock | `--provider mock` | deterministic offline task, for tests and demos |

`auto`, the default, tries the local command, then the HTTP server, then
nothing. Meshy is never selected implicitly.

## Meshy API

Base URL: `https://api.meshy.ai`.

Authentication: `Authorization: Bearer <api key>`. The pipeline reads
`MESHY_API_KEY`. `MESHY_API_BASE_URL` overrides the root for a local scripted
server. A missing key is an explicit error, not a silent fallback.

### Image to 3D

`POST /openapi/v1/image-to-3d` returns `202` with `{ "result": "<task id>" }`.

Fields this pipeline sends:

| Field | Value | Why |
| --- | --- | --- |
| `image_url` | Public URL or base64 data URI | Local files are encoded as `data:image/png;base64,...` |
| `should_texture` | `true` | The whole point is a textured character |
| `enable_pbr` | `true` | Base color plus metallic, roughness and normal maps |
| `pose_mode` | `a-pose` | Humanoid rigging is more likely to find a pose |
| `target_formats` | `["glb"]` | One format, one download |
| `texture_prompt` | Prompt plus style text | Guide the texture; Meshy caps it at 800 characters |

Documented input limits: JPG, JPEG, PNG or WebP, at most 20 MB, at least 32
pixels on each side, at most 178,956,970 pixels (width times height).

Documented output formats: `glb`, `fbx`, `obj` (with `mtl`), `usdz`, `stl` and
`3mf`. `target_formats` narrows the set. The task object exposes
`model_urls`, `thumbnail_url`, optional `thumbnail_urls` and `texture_urls`
with `base_color`, `metallic`, `normal`, `roughness` and sometimes `emission`.

`GET /openapi/v1/image-to-3d/:id` returns the task object.

### Text to 3D

`POST /openapi/v2/text-to-3d` with `mode: "preview"` returns an untextured mesh.
A second call with `mode: "refine"` and the preview task id textures it. Both
steps return `202` with `{ "result": "<task id>" }`, and
`GET /openapi/v2/text-to-3d/:id` reads either step.

This pipeline creates only the preview. Refine is a second paid task, and the
current CLI has no command for it, so a prompt-only character is an untextured
shape. A reference image is the path that produces a textured model in one
task.

### Polling and downloads

Task status is `PENDING`, `IN_PROGRESS`, `SUCCEEDED`, `FAILED` or `CANCELED`.
The last three are final and never change. `progress` is 0 to 100.

The documented client behavior:

- Create request timeout: 60 seconds.
- Status request timeout: 30 seconds.
- Poll interval: the `Retry-After` response header, or 5 seconds.
- Wait at least 60 minutes from `created_at` before giving up on a task.
- A failed status request does not mean a failed task. Poll again.

Model URLs are signed asset URLs. Download before `expires_at`; the API keeps
results for a limited time, documented under asset retention. The pipeline
downloads through the signed URL and does not attach the bearer token.

Request errors carry `{ "message": "..." }` with HTTP `400`, `401`, `402`,
`403`, `404`, `409`, `422`, `429` or `5xx`. Task errors carry
`task_error: { type, code, message, doc_url }`. `type` is the category
(`invalid_input`, `timeout`, `service_unavailable`, `server_error`).

### Rigging (documented, not used)

`POST /openapi/v1/rigging` accepts `input_task_id` or a GLB `model_url`. The
model must be a textured humanoid GLB, at most 380,000 triangles (a quad counts
as two), and the face must point toward `+Z`. `height_meters` defaults to 1.7.

`skeleton_template` choices: `meshyrig`, `mixamo`, `ue5_mannequin`,
`max_biped`, `unity_humanoid`, `maya_human_ik`, `blender_rigify_basic_human`,
`cc5_hd_neutral_base`, `daz_genesis9`. `include_fingers` adds hand bones.
`GET /openapi/v1/rigging/:id` returns `rigged_character_glb_url`,
`rigged_character_fbx_url` and optional walking and running clips.

None of those skeletons is the Deadlock hero skeleton. Auto-rig output is useful
for posing and retarget experiments, not as a drop-in game rig. Meshy does not
emit Source 2 skeletons, `vmdl` sources or AnimGraph 2 data.

### Convert (documented, not used)

`POST /openapi/v1/convert` with `input_task_id` or `model_url` and
`target_formats` (`glb`, `fbx`, `obj`, `usdz`, `blend`, `stl`, `3mf`) converts
an existing task. `GET /openapi/v1/convert/:id` reads it. This is the documented
way to get an FBX for Blender or Source 2 tooling without a second generation.

## What a Deadlock hero model requires

From `docs/research/deadlock-modding.md`:

- A compiled model at an exact game path: `.vmdl` source compiled to `.vmdl_c`.
- A source `.vmdl` that points at the mesh, the skeleton, material groups and
  LODs.
- The hero skeleton in the game, as `.vnmskel_c`. Mods must not ship a
  skeleton: it breaks animation and other mods.
- Materials as `.vmat` compiled to `.vmat_c`, and textures as `.vtex_c`.
- AnimGraph 2 bindings. CSDK 12 is pinned to an older build and newer formats
  compile incorrectly. `DeadlockTools` is the community tool that adds
  AnimGraph 2 references to compiled models.
- LODs for the model.
- Compile through CSDK 12 `resourcecompiler.exe` on Windows. Compile is a
  backend, not an assumption, per `docs/decisions/0003-windows-compile-backend.md`.
- Exact paths. Hero folders move between updates, so the game index is the
  source of truth, not a hard-coded hero table.

## The hard gap

| Meshy output | Deadlock requirement | Work left |
| --- | --- | --- |
| GLB, Y-up glTF convention | Source 2 Z-up world with its own forward axis | Axis and scale conversion |
| AI mesh, single LOD | Hero model with LODs | Retopology and LOD generation |
| Meshy or third-party humanoid rig | Deadlock hero skeleton and bone names | Retarget, rebind or rebuild the rig |
| Auto skin weights | Weights that match the hero skeleton | Weight transfer and cleanup |
| PBR PNG maps inside the GLB | `.vmat_c` and `.vtex_c` with Source 2 shader parameters | Material authoring, texture compile |
| Walk and run clips from the Rigging API | AnimGraph 2 state machine | Graph work, not clip import |
| No `vmdl` | `.vmdl_c` at an exact hero path | Author the model source |
| No compiler | CSDK 12 on Windows | Build backend |

So the generated GLB is a concept mesh and a base for manual work. It is not a
hero replacement, and the pipeline refuses to label it as one. This matches the
open question in `docs/architecture.md`: no image-to-3D backend is promised
until a mesh compiles and animates in game.

## Rights, likeness and privacy

ADR 0005 rules, as implemented:

- A reference image is an input, not a shipped asset. The pipeline never copies
  it into the payload or the imported record.
- A reference image is uploaded only when `allowReferenceUpload` is true
  (CLI `--allow-reference-upload`). The flag is stored in `task.json`. Without
  it, create fails before any request is sent.
- A rights note is recorded in `task.json` and `character.json`. The default
  note says rights are not confirmed.
- `publishAllowed` is always false. This code has no publish path.
- A local backend (command or HTTP on loopback) and the `mock` provider never
  upload the reference image, so consent is not required for them. Only Meshy is
  treated as remote.
- The remote provider receives the image as a base64 data URI in the request
  body. Provider terms govern retention and processing on that side. Check them
  before uploading a real person's photo.

## Implementation map

The core functions and the CLI module are implemented and tested, except for the
`stages` clap arm, which is owned by the `main.rs` change.

| Item | Location |
| --- | --- |
| Types, stage graph, backends, native checks | `rust/crates/deadlock-core/src/character.rs` |
| Core tests, including a hand-built GLB | `rust/crates/deadlock-core/src/character/tests.rs` |
| CLI module | `rust/crates/dah-rs/src/commands/character.rs` |
| Task metadata and stage graph | `<workspace>/generated/characters/<slug>/task.json` |
| Mesh | `<workspace>/generated/characters/<slug>/model.glb` |
| Source 2 plan | `<workspace>/generated/characters/<slug>/source2-plan.json` |
| Export copy | `<workspace>/generated/characters/<slug>/export/<slug>.glb` |
| Stage scratch folders | `<workspace>/generated/characters/<slug>/stages/<stage>/` |
| Import record | `<workspace>/generated/characters/<slug>/character.json` |

Backends:

- `LocalCommandBackend` runs a configured executable, one stage per call, with
  an argument array, no shell, a working directory inside the character folder,
  a timeout and a containment check on every produced file. Stage parameters are
  passed as `DAH_*` environment variables. This is the seam for TRELLIS,
  Hunyuan3D, ComfyUI CLI or a Blender script.
- `HttpBackend` posts one JSON stage request to `DAH_MESHGEN_URL` and accepts
  either base64 file data or a source path per output.
- `ProviderBackend` wraps a `CharacterProvider`. `MeshyProvider` uses the
  documented image-to-3D and text-to-3D endpoints, `Retry-After`, and a plain
  GET for signed asset URLs. `MockProvider` is deterministic and offline: each
  poll moves `PENDING` to `IN_PROGRESS` to `SUCCEEDED` and writes a minimal valid
  GLB.

A local scripted HTTP server that follows the Meshy response shape can be used
with `MESHY_API_BASE_URL=http://127.0.0.1:<port>` and any non-empty
`MESHY_API_KEY`.

## Verified and not verified

Verified on 2026-11-16:

- The base64 encoder and decoder against known vectors.
- Slug generation from a prompt, and from the reference file name when the
  prompt is empty.
- GLB header validation, including rejection of short and mismatched files.
- The GLB structural parser against a hand-built single-triangle GLB: chunk
  table, accessor bounds, triangle and vertex counts, attribute presence, bounds
  box, and a detected accessor overrun.
- PNG header and pixel analysis against a generated RGBA disc: dimensions, alpha
  channel, alpha coverage, subject box, centring and edge quality. Also a JPEG
  header read without decoding, and a too-small PNG rejected.
- Reference-only create with no backend: `prepare_reference` succeeds,
  `generate_views` is `unsupported` with the fix in the note, the task is
  `BLOCKED`, and `import` refuses.
- The full mock flow: create, two polls, download, import, status, and restoring
  a deleted GLB from the recorded task.
- Import refuses a task that is still `PENDING`.
- Create refuses a remote upload of a reference image without consent.
- The Meshy-shaped flow against a local scripted HTTP server: `POST
  /openapi/v1/image-to-3d` with an auth header and a PNG data URI, `GET
  /openapi/v1/image-to-3d/img-task-1` with `Retry-After`, then the GLB download
  from the recorded `model_urls.glb`, then import metadata with
  `source2Compile: "required"`.
- The local command backend end to end with a real executable: views output
  recorded, `model.glb` produced, cleanup measured, `source2_map` and `export`
  written.
- The HTTP backend with a scripted JSON server returning a base64 GLB.

Not verified:

- A real Meshy call. No key or credits were used.
- Any real local generative backend. No mesh model was installed, so the local
  command and HTTP seams are proven only against scripted fixtures.
- Retopology, rigging, weight transfer, LOD generation, material translation and
  AnimGraph 2 binding. Those stages are recorded as `unsupported` or skipped.
- Image-to-3D output quality, texture quality or rigging quality.
- The Rigging API and skeleton templates.
- Loading the GLB in Blender, Source 2 Viewer, CSDK 12 or the game.
- Retargeting any provider skeleton to the Deadlock skeleton.
- Compiling a `vmdl` and binding AnimGraph 2.
- In-game appearance, animation or camera behavior.

## Sources

- Meshy image to 3D API: <https://docs.meshy.ai/en/api/image-to-3d>
- Meshy text to 3D API: <https://docs.meshy.ai/en/api/text-to-3d>
- Meshy rigging API: <https://docs.meshy.ai/en/api/rigging>
- Meshy convert API: <https://docs.meshy.ai/en/api/convert>
- Meshy task lifecycle: <https://docs.meshy.ai/en/api/task-lifecycle>
- Meshy errors: <https://docs.meshy.ai/en/api/errors>
- Local domain facts: `docs/research/deadlock-modding.md`
- Likeness policy: `docs/decisions/0005-reference-images-and-likeness.md`
- Compile backend policy: `docs/decisions/0003-windows-compile-backend.md`


## Which model makes the mesh

An LLM cannot emit a production mesh as tokens, but it can write and iterate
Blender code. `dah character backends` reports the measured lanes, and
`dah character scaffold <slug>` writes the Blender script contract. The full
comparison, including MeshGPT, TRELLIS, MCP for Blender and the subscription
caveat, is in `docs/research/llm-mesh-generation.md`.
