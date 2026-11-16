# Meshy reverse engineering: their process, our pipeline

Status: active. Static analysis of the shipped web bundle on 2026-11-16. No
Meshy account, key or credit was used. Nothing here executes their client.

Updated: 2026-11-16

This document answers one question: how does Meshy actually turn an image into a
game-ready character, and what of that process belongs in `dah`? It is not a
Meshy integration guide. The executable side is
`rust/crates/deadlock-core/src/character.rs`, which implements our own stage
graph. `docs/research/character-generation.md` remains the doc for the public
API surface and the Deadlock requirements.

## Method and what was analyzable

Tools:

- `rea` 6.4.0 at `/opt/homebrew/bin/rea`. Commands run:
  `rea --help`, `rea doctor`, `rea analyze-javascript-application`.
- `curl` for the web bundle and `docs.meshy.ai`.
- `python3` for byte-level greps of the minified chunks and for the doc HTML to
  text pass.

`rea doctor` on this machine:

| Check | Result |
| --- | --- |
| node | ok, 26.7.0 |
| host | ok, 26.0 |
| hopper | missing analysis engine (`HOPPER_LAUNCHER_PATH` unset) |
| ghidra | missing analysis engine (`GHIDRA_INSTALL_DIR` unset) |
| ida-registration | missing analysis engine |
| skill:identity | stale |
| 19 agent registrations | stale |

So: no native analysis engine was available and none was installed. That is a
real limit, not a workaround: there is no native binary in scope anyway. Meshy
is a web product, so JavaScript analysis is the right lane and it was available.

Artifacts captured under `.scratch/rea/meshy/` (gitignored, not committed):

| Path | What |
| --- | --- |
| `raw/index.html` | 1,010,191 bytes, the production app document |
| `chunks.txt` | 153 script URLs |
| `app/_next/static/chunks/*.js` | all 153 chunks, 10 MB of minified JS |
| `analysis.json` | REA application Evidence, 57 MB |
| `docs/*.txt` | 25 API doc pages converted from HTML to text |

REA run:

```
rea analyze-javascript-application ./app --artifact-format directory \
  --analysis-timeout-ms 15000 --format json
```

Result envelope:

| Field | Value |
| --- | --- |
| evidence id | `ev_67bd5b0e87a535df496ea936390ac1d34d507ca1df57677a50337428b4f3fe36` |
| confidence / authority | `derived` / `shipped-artifact` |
| root artifact sha256 | `e7404fa0e486731a9f89db61056a75b8ecdf88f774832105141336c0d4525f7a` |
| application graph id | `jag_a598150de4a7dbcabe247be31acca70c93c6b4f99ce630e5dabd14614da3ad5a` |
| semantic graph id | `jsrg_a7df4de29109a88827a418b4c985f2f2ad8657c252e663993c7678daaa1615a3` |
| parsed files | 153 / 153, 0 parse failures, 10,130,463 text bytes |
| AST nodes visited | 2,613,591 |
| semantic nodes / relations | 25,697 / 37,663 |
| findings | 452 |
| inventory graph | 497 nodes: 153 javascript-asset, 34 javascript-module, 34 endpoint, 6 storage, 4 worker, 1 artifact |

REA's own declared limits, quoted from the Evidence:

- `coverage.status` is `partial`, `truncated: true`.
- Per-module worker timeout 15,000 ms and a result budget; oversized modules
  kept file-level facts only. Several chunks are named in `limitations`.
- "CommonJS and ESM binding relationships were recovered from inert syntax and
  resolved only within the inventoried artifact container."
- "JavaScript and HTML were parsed as inert text; bundle bootstrap code was
  never executed."

Consequence: call-flow inside very large chunks is incomplete, so a claim that
depends on a single call chain is INFERRED, not OBSERVED. Everything below that
is tagged OBSERVED was read directly out of a chunk, a doc page or the REA
inventory.

What REA did establish on its own, without grep: 4 web workers, 1 of which is
constructed from `/resource/converter/converter-loader.min.js`, plus the WebGL
extension inventory (`WEBGL_compressed_texture_s3tc`, `EXT_texture_filter_anisotropic`,
`KHR_parallel_shader_compile`, ...). That is a three.js style in-browser viewer
with a client-side converter worker, not a server-rendered mesh preview.

## Evidence table

Tags: OBSERVED (read directly), INFERRED (follows from code but not proven by
one symbol), UNKNOWN (not determinable from what was captured).

| ID | Claim | Tag | Locator |
| --- | --- | --- | --- |
| E1 | The production app references 153 JS chunks from `cdn.meshy.ai/webapp-build-assets/production/_next/static/chunks/` | OBSERVED | `raw/index.html`, 153 unique `*.js` hrefs |
| E2 | Client is Next.js/Turbopack | OBSERVED | `chunk: turbopack-1ou5qfbip2g5i.js`, `_next/static` paths |
| E3 | Web app calls an internal JSON API under `/v2/tasks`, not `/openapi` | OBSERVED | `chunk 34zrff-alaid0.js` offset 14410 (`method:"POST",url:"/v2/tasks"`), `chunk 08n5prwcdjt2r.js` offset ~1563 (`TasksV2="/v2/tasks"`) |
| E4 | Task creation body is `{phase, task_type, args:{draft:{...}}, workspaceId, batch_id?, generation_group_id?, number_of_generations?, rescue_source_task_id?, model_type?, ultra_mode?, ultra_resolution?}` | OBSERVED | `34zrff-alaid0.js` offsets 14410 and 14925 |
| E5 | The web app has named task phases: `draft`, `generate`, `texture`, `auto-uv`, `remesh`, `stylize`, `model-edit`, `rigging`, `animate`, `upload`, `relief` | OBSERVED | `3kq60jin6d1al.js` (`formatTaskV2` failure tags), `2u_riupanqyx3.js`, `1lnfb3q1szdu0.js` |
| E6 | Image-to-3D draft form schema is `aiModel`, `poseMode`, `license`, `shouldTransferImageStyle?`, `ultraMode?`, `ultraResolution?` (`2k`\|`4k`), `multiView?`, `modelType`, `topology?` (`quad`\|`triangle`), `lowPolyAiModel?`, `targetPolycount?`, `autoSplit?`, `autoTexture?`, `seed?` (1..2^32-1) | OBSERVED | `34zrff-alaid0.js` offset 3592 |
| E7 | `IMAGE_TO_3D_DEFAULT_VALUES` = `{aiModel:"avocado", poseMode:"", modelType:"standard", name:"", license:"private", image:null, multiViewOpen:false, multiView:{images:[]}, shouldTransferImageStyle:true, autoSplit:false, ultraMode:false}` | OBSERVED | `3kq60jin6d1al.js` |
| E8 | Internal ai model ids: `blueberry` (Meshy 7), `avocado` (Meshy 6), `meshy-6-lite`, `meshy-5.3`, `meshy-5.1`, `meshy-5`, `meshy-4`, `meshy-3`, `meshy-3-turbo` | OBSERVED | `3tzv8ewlukwta.js` module 660562 |
| E9 | Model types: `standard`, `lowpoly`, `venus`, `venus-t2p5`; smart topology is surfaced as `meshy-t2` / `meshy-t2.5` | OBSERVED | `3tzv8ewlukwta.js`; `docs/image-to-3d.txt` lines 250-261 |
| E10 | Client polycount bounds: DEFAULT 10000, MIN 100, MAX 20000, STEP 100, ABSOLUTE_MAX 100000 | OBSERVED | `3tzv8ewlukwta.js` (`{DEFAULT:1e4,MIN:100,MAX:2e4,STEP:100,ABSOLUTE_MAX:1e5}`) |
| E11 | Public API polycount ranges differ per path: remesh 100..300000 default 30000, `meshy-t2` 100..15000 default 4000, `meshy-t2.5` 100..30000 default 15000 | OBSERVED | `docs/image-to-3d.txt` lines 565-586 |
| E12 | Remesh form defaults: `{topology: triangle, decimationMode: DEFAULT, targetPolycount: 10000, isUseTexture: true, enablePBR: true}` and the standalone remesh panel uses `decimationMode: FIXED` | OBSERVED | `2u_riupanqyx3.js` |
| E13 | `decimation_mode` is a 1..4 level enum (1 ultra, 2 high, 3 medium, 4 low) and overrides `target_polycount` | OBSERVED | `docs/image-to-3d.txt` lines 506-521 |
| E14 | Licence choices `cc0`, `cc-by-4.0`, `private`; default private | OBSERVED | `3tzv8ewlukwta.js`; `3kq60jin6d1al.js` |
| E15 | Quad output is not only a mesh: the generate result carries `quadJsonUrl` beside `modelUrl` and optional `previewGlbUrl` | OBSERVED | `3kq60jin6d1al.js` (`quadJsonUrl`) |
| E16 | Export format sets: all mesh `fbx,obj,glb,usdz,stl,blend,3mf,dxf`; animation `fbx,glb,usdz`; smart rig `glb`; 3D print `stl,3mf,obj,glb`; `dxf` is print-only | OBSERVED | `1lnfb3q1szdu0.js` module 507956 |
| E17 | Animation download defaults to 30 fps, with `allAnimation`, `includeAnimation`, `includeRiggedCharacter`, `withoutSkin`, `singleFile` switches | OBSERVED | `1lnfb3q1szdu0.js` module 647142 |
| E18 | Skeleton templates, with beta flags: `meshyrig` (default, 24 bones, centimetres), `mixamo`, `ue5_mannequin`, `max_biped`, `unity_humanoid`, `maya_human_ik`, `blender_rigify_basic_human`, `cc5_hd_neutral_base`, `daz_genesis9` | OBSERVED | `1lnfb3q1szdu0.js` module 430120; `docs/rigging.txt` |
| E19 | Rig kinds: `biped`, `quadruped`, `quadruped-cat`, `smart` | OBSERVED | `3v69eweeytop9.js` module 10505 |
| E20 | Animation clips are an enum of roughly 200 named actions (`Base`, `Walking`, `Running`, `Idle`, `Attack`, `Dead`, dance and combat sets, ...) | OBSERVED | `3v69eweeytop9.js` module 10505 |
| E21 | Public rigging constraints: at most 380,000 triangles (quad counts 2), GLB only, face toward +Z, `height_meters` default 1.7, 400 MB URL / 100 MB data URI, no Draco above 100 MB | OBSERVED | `docs/rigging.txt` lines 118-160 |
| E22 | Client polling loop: overall timeout 300,000 ms; wait 2,000 ms for the first 30,000 ms then 5,000 ms; 5,000 ms when nothing is pending; statuses `PENDING`,`IN_PROGRESS`,`SUCCEEDED`,`FAILED`,`CANCELED`; `FAILED`/`CANCELED` abort the whole batch | OBSERVED | `1oyatefi437x3.js` offset 37015 |
| E23 | The web app prefers an SSE stream `/v2/tasks/{id}/stream` with bearer token, 35,000 ms idle watchdog and a `holdUntil` hint, and falls back to polling | OBSERVED | `02a66wws1xnhb.js` offset 3443 |
| E24 | Background removal is a separate file operation: `POST /v1/files/images/{id}/remove-background`, exposed as uploader and viewer buttons; the agent platform has `POST /v4/agent/ops/remove-background` | OBSERVED | `33bkqnqysna79.js` offset 11905; `33zbx71__2tof.js` offset 596400; `0imi_u27i-w_e.js` test ids |
| E25 | Background removal is not a parameter of the image-to-3D request; the public API has no such field | OBSERVED | `docs/image-to-3d.txt` parameter list |
| E26 | Client-side format conversion runs in a worker built from `/resource/converter/converter-loader.min.js` | OBSERVED | REA worker node `jag` graph; `raw` chunk with `converter-loader.min.js` |
| E27 | The app's agent ships 38 tools: `text-to-image`, `image-to-image`, `image-to-3d`, `smart-topology`, `multi-image-to-3d`, `remesh`, `split-model-into-parts`, `retexture`, `retry-generation`, `texture-multiview`, `native-texture`, `image-to-relief`, `multi-color-print`, `convert-model-formats`, `export-to-slicer`, `repair-printability`, `analyze-printability`, `inspect-artifact`, `view-model-thumbnail`, `retrieve-printing-guide`, `web-search`, `web-fetch`, `search-image`, `search-artifacts`, `show-artifacts`, `browse-artifacts`, `trace-artifact-lineage`, `search-animate-action`, `ask-user`, `canvas-add`, `rigging`, `animate`, `read-skill`, `create-skill`, `update-skill`, `manage-user-preferences`, `read-user-preferences`, `spawn-agent` | OBSERVED | `33zbx71__2tof.js` offset 41777 |
| E28 | Tools are grouped by output media: image, model, rigging; three tools are gated for approval (`rigging`, `create-skill`, `update-skill`) | OBSERVED | `33zbx71__2tof.js` (`tH=["rigging","create-skill","update-skill"]`, `i$` media switch) |
| E29 | The agent platform has first-class artifacts and skills: `/v4/agent/artifacts`, `/v4/agent/artifacts/register`, `/v4/agent/artifacts/upload`, `/v4/agent/skills`, `/v4/agent/skills/market`, `/v4/agent/threads`, `/v4/agent/projects`, `/v4/agent/template-runs`, `/v4/agent/usage`, `/v4/agent/pricing` | OBSERVED | `33zbx71__2tof.js` and neighbours |
| E30 | API playground exposes exactly the post-processing operations `convert`, `uv-unwrap`, `resize`, `remesh`, and 3D print `repair`, `analyze`, `multi-color` | OBSERVED | `4544pkktkc1m4.js` offset 42528 |
| E31 | Credit prices: image-to-3D mesh only 20, +2K texture 30, +4K 30, +8K 35; Ultra geometry +5; remesh 5; UV unwrap 5; convert 1; resize 1; rigging 5; animation 3 per action up to 10; text to motion 10 or 3 | OBSERVED | `docs/pricing.txt` |
| E32 | Public task object fields: `id`, `type`, `model_urls{glb,fbx,obj,usdz,stl,pre_remeshed_glb}`, `thumbnail_url`, `thumbnail_urls{front,right,back,left}`, `texture_prompt`, `progress`, `started_at`, `created_at`, `expires_at`, `finished_at`, `status`, `texture_urls[{base_color,metallic,normal,roughness,emission}]`, `preceding_tasks`, `task_error`, `consumed_credits` | OBSERVED | `docs/image-to-3d.txt` lines 931-1058 |
| E33 | Public image-to-3D accepts `input_task_id` as an alternative to `image_url`, so a generated image can feed the 3D step without a download | OBSERVED | `docs/image-to-3d.txt` lines 174-197 |
| E34 | `image_enhancement` (default true) preprocesses the input image, and `remove_lighting` (default true) strips highlights and shadows from the base colour | OBSERVED | `docs/image-to-3d.txt` lines 650-690 |
| E35 | `auto_size`, `origin_at` (`bottom`\|`center`) and `multi_view_thumbnails` exist; multi-view thumbnails need `auto_size: true` and add about 5 s | OBSERVED | `docs/image-to-3d.txt` lines 721-767 |
| E36 | Texture models are separate from geometry models: `meshy-7` (PBR, to 8K), `meshy-n1` (2K base colour only, reference driven), `meshy-6-lite` (2K, cheapest) | OBSERVED | `docs/image-to-3d.txt` lines 322-359 |
| E37 | `meshy-n1` fails a task above 7,999,999 triangles and refunds the credits | OBSERVED | `docs/image-to-3d.txt` line 353 |
| E38 | Their remesh stage is optional and lossy: `should_remesh` default false except `meshy-6-lite`, and `save_pre_remeshed_model` exists to keep the pre-decimation GLB | OBSERVED | `docs/image-to-3d.txt` lines 463-531 |
| E39 | Their UV stage is a separate paid endpoint `POST /openapi/v1/uv-unwrap` | OBSERVED | `docs/uv-unwrap.txt` endpoint set |
| E40 | Their rig, animation, remesh, retexture, resize, convert and auto-split are each separate async endpoints with their own `:id` and `:id/stream` | OBSERVED | `docs/*.txt` endpoint sets |
| E41 | Meshy emits no Source 2 artifact of any kind: no `vmdl`, `vmat`, `vtex`, `_c` file, skeleton or AnimGraph node appears in any doc page or chunk | OBSERVED (absence across 25 doc pages and 153 chunks) | `docs/*.txt`, `app/_next/static/chunks` |
| E42 | The web app tracks a `srMode` value of `"weak"` next to `enablePBR` on the generate panel | OBSERVED | `2u_riupanqyx3.js` |
| E43 | What `srMode` changes, and whether a super-resolution pass exists server side | UNKNOWN | no doc page, no schema field |
| E44 | Whether `draft` and `generate` are two model passes or one pass with a preview artifact | UNKNOWN | phase names are OBSERVED, the split is not visible without server code |
| E45 | Their server-side meshing, texturing and rigging implementations | UNKNOWN | server side, not shipped to the client |

## Their stage graph, as observed

Ordering below is the order the client drives it. Each row names the observable
trigger. "Paid" means a credit charge is documented. "Async" means a task id and
a poll or stream.

| # | Stage | Input | Output | Optional | Paid | Async | Lossy |
| --- | --- | --- | --- | --- | --- | --- | --- |
| M1 | `prepare_reference` | image file, or an image task id (E33) | uploaded file id, or the image task | image path only; prompt-only uses text-to-3D | no | no | `image_enhancement` and `remove_lighting` are documented lossy preprocessing (E34) |
| M2 | `draft` | draft args (E6) | preview frames, `quadJsonUrl` on completion (E15) | yes, batch and multi-view use it | yes, inside the task price | yes | preview only |
| M3 | `generate` | accepted draft | `modelUrl`, `previewGlbUrl` | no | 20 credits base (E31) | yes | geometry is a single LOD |
| M4 | `remesh` | model + `topology`, `target_polycount` or `decimation_mode` | decimated mesh, optional `pre_remeshed_glb` | yes, `should_remesh` default false except `meshy-6-lite` (E38) | 5 credits as a standalone call (E31) | yes | yes, this is decimation |
| M5 | `auto-uv` / `uv-unwrap` | mesh | UV layout | yes | 5 credits standalone | yes | yes, re-layout discards the original UVs |
| M6 | `texture` | mesh, `texture_model`, `texture_resolution`, `texture_prompt` or `texture_image_url` | base colour plus optional metallic, roughness, normal, emission | yes, `should_texture` (E36) | 10 for 2K/4K, 15 for 8K (E31) | yes, in the same task or via `retexture` | yes, baked lighting removal is explicit |
| M7 | `smart-topology` | image, `meshy-t2` / `meshy-t2.5`, `target_polycount` | triangle mesh at target count, no remesh | alternative to M3+M4 | same as image-to-3D (E31) | yes | yes, generative retopo |
| M8 | `rigging` | textured humanoid GLB, `skeleton_template`, `include_fingers`, `height_meters` | rigged GLB and FBX, optional walk and run clips | yes | 5 credits | yes | yes, auto weights |
| M9 | `animate` | rig task id plus `action_id`/`action_ids`/`motion_task_id`, optional `post_process.change_fps` | animation GLB and FBX, `processed_animation_fps_fbx_url` | yes | 3 credits per action | yes | yes, retarget onto the chosen skeleton |
| M10 | `export` / `convert` | model or task id plus `target_formats` | `glb, fbx, obj, usdz, stl, blend, 3mf, dxf` (E16) | yes | 1 credit for convert | yes for convert, client worker for some formats (E26) | format only |

Cross-cutting, and worth copying because it is good design:

- Every async stage is `create`, then `stream` or `poll`, then `download`, and
  each has a `DELETE` that refunds only while `PENDING`.
- The client keeps the pre-lossy artifact whenever a later stage is destructive
  (`save_pre_remeshed_model`, `quadJsonUrl` beside the GLB, `withoutSkin` on
  animation export). Reproducibility is a product feature, not an afterthought.
- Pricing is attached to the stage, not to the run (E31). A retry is a new
  charge and `rescue_source_task_id` exists for that case.

## Their client architecture, as observed

- The workspace is a chat-style orchestrator with 38 tools, not a form wizard
  (E27). The same actions are available as UI panels and as agent tools.
- Tools are grouped by output media and three are approval gated (E28). Their
  gated set is `rigging`, `create-skill`, `update-skill`: expensive or
  irreversible things pause.
- Artifacts are first class and addressable (`artifact_id`, `artifact_type`,
  `url`), with lineage queries (`trace-artifact-lineage`) and a canvas
  (`canvas-add`) (E29).
- Progress arrives over SSE with a 35 s idle watchdog; polling is the fallback
  (E22, E23).
- Background removal is a user action in the uploader, not an implicit step
  (E24, E25). That is a useful contrast with our rule that a reference image is
  an input the user controls.

## The economics we are not copying

The client's polycount UI caps at 20,000 with an absolute ceiling of 100,000
(E10) while the public API allows up to 300,000 for remesh (E11). The web UI cap
is a product and cost decision, not a technical one. Our default budget comes
from the Deadlock side, not from Meshy.

## The gap that decides our design

Meshy's last stage is a file download: a GLB, FBX or USDZ plus PNG maps. There
is no Source 2 stage at all (E41). Every deterministic step between "a GLB
exists" and "a hero model loads in game" is missing from their pipeline:

- axis and unit conversion into Source 2 Z-up,
- retopology to a hero-appropriate edge flow at a fixed budget,
- LOD chain generation,
- skeleton binding onto the Deadlock hero skeleton,
- material translation into `.vmat` and `.vtex`,
- `.vmdl` authoring plus CSDK 12 compile,
- AnimGraph 2 bindings.

So a Meshy call can only ever be one node in our graph. That is why the new
implementation is a stage graph with a pluggable generative seam and native
deterministic stages, and why `MeshyProvider` is now one adapter among several
instead of the pipeline.

## Our stage graph

Ids are stable because they are written into `task.json`. The `evidence` column
is the id from the table above that justifies the stage's existence.

| Stage | Kind | Native in Rust | Backend required | Evidence |
| --- | --- | --- | --- | --- |
| `prepare_reference` | deterministic | yes: format, dimensions, alpha channel, matte coverage, centering, edge quality | no | M1, E24, E25, E34 |
| `generate_views` | generative | no | yes | M2, E6, E15 |
| `reconstruct` | generative | no | yes | M3 |
| `cleanup` | deterministic analysis, generative surgery | analysis yes, decimation no | only when over budget | M4, E38 |
| `uv_bake` | generative | detection only | yes when UVs are absent | M5 |
| `pbr_synthesize` | generative | detection only | yes when maps are absent | M6, E36 |
| `retopo` | generative | budget check only | yes | M7 |
| `rig` | generative | requirement check only | yes | M8, E18, E21 |
| `animate_retarget` | generative | no | yes, needs `rig` | M9, E20 |
| `source2_map` | deterministic | yes: artifact plan | no | E41, `docs/research/deadlock-modding.md` |
| `export` | deterministic | yes: hash and place the GLB | no | M10, E16 |

Rules the implementation enforces:

1. A native stage never claims to have changed geometry it did not touch.
2. A generative stage with no backend is `unsupported` with a precise reason,
   never `skipped` and never `succeeded`.
3. `source2_map` always runs when a mesh exists and always says
   `inGameReady: false`, `source2Compile: "required"`.
4. `publishAllowed` has no code path to `true`.

## What we deliberately did not copy

- Their phase names, wire format and `/v2/tasks` shape. Those are their private
  API and are not a contract we can rely on.
- Their credit model. We have no billing.
- Their asset retention. Their URLs expire; ours are local files.
- Their skeleton list as a rig target. Only `meshyrig` and `mixamo` are even
  interesting as an intermediate, and neither is a Deadlock skeleton.

## Honest gaps

- No Meshy task was created. Nothing here proves their output quality, timing
  distribution or failure rates.
- Server-side stage boundaries are inferred from client phases and endpoint
  names, not observed (E44, E45).
- `srMode` is unexplained (E43).
- No native analysis engine was available, so no non-JavaScript target could be
  checked. Not needed here, but it means `rea doctor` is not green.
- REA coverage is partial, so per-function call graphs inside the largest chunks
  are missing. The stage graph above rests on names, schemas and endpoints,
  which were all captured at file level.
- Our own generative seam is unproven: no local mesh backend was configured on
  this machine, so every generative stage is exercised only as `unsupported`
  plus a fake backend in tests.

## Sources

- Web bundle: `https://www.meshy.ai/`, chunks under
  `https://cdn.meshy.ai/webapp-build-assets/production/_next/static/chunks/`
- `https://docs.meshy.ai/en/api/image-to-3d`
- `https://docs.meshy.ai/en/api/multi-image-to-3d`
- `https://docs.meshy.ai/en/api/rigging`
- `https://docs.meshy.ai/en/api/animation`
- `https://docs.meshy.ai/en/api/animation-library`
- `https://docs.meshy.ai/en/api/remesh`
- `https://docs.meshy.ai/en/api/retexture`
- `https://docs.meshy.ai/en/api/uv-unwrap`
- `https://docs.meshy.ai/en/api/resize`
- `https://docs.meshy.ai/en/api/auto-split`
- `https://docs.meshy.ai/en/api/convert`
- `https://docs.meshy.ai/en/api/pricing`
- `https://docs.meshy.ai/en/api/rate-limits`
- `https://docs.meshy.ai/en/api/task-lifecycle`
- `https://docs.meshy.ai/en/api/webhooks`
- Local domain facts and the compile gap: `docs/research/deadlock-modding.md`
- Public API surface and Deadlock requirements: `docs/research/character-generation.md`
- Likeness policy: `docs/decisions/0005-reference-images-and-likeness.md`
