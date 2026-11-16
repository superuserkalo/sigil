# 0005 Reference images, likeness and generated character mods

Status: accepted
Date: 2026-11-16

## Context

The workbench should handle mods of any size, from a texture recolor to a character whose face and
outfit are based on a reference photo. The ambitious case adds three risks that a texture swap does
not have:

1. The reference image probably came from the internet and may be copyrighted.
2. A real person's face or identity is involved, which raises consent and publicity rights.
3. A generated mesh has to fit the game's skeleton, materials, animations and compile constraints,
   so "make it look like this person" is a review-heavy pipeline, not a single prompt.

At the same time, the product already promises BYOK, local-first data, no telemetry and an approval
gate before publishing.

## Decision

- Reference images are inputs, not assets. The app never ships them in a mod and never trains a
  shared model on them.
- The user supplies the reference image, or chooses a documented image source. Agent tools do not
  fetch arbitrary URLs. Any image search provider is added to the network allowlist and the
  third-party notices before it is used.
- The harness may analyze a reference and propose a look, palette, silhouette and material plan. It
  must label generated geometry and textures as generated.
- A character mod must stay on the game skeleton. The app does not replace skeletons, gameplay data
  or animations that would change hitboxes or matchmaking behavior.
- Likeness work stays local until the user confirms they have the rights to use the reference and
  the identity. The confirmation is recorded in the project manifest.
- Publishing a mod that uses a real person's likeness requires a separate approval that names the
  reference and the permission basis. No automatic upload.
- The 3D generation step is a backend with availability. No backend is promised until one is
  verified to produce a mesh that compiles and animates in Deadlock.

## Consequences

- The simple-to-complex ladder stays honest: a recolor can ship before a generated character can.
- The project manifest needs fields for references, rights confirmation, generated assets and
  provenance.
- The network allowlist grows only by documented provider, not by arbitrary URL fetching.
- Some requests will be refused or downgraded to a concept sheet. That is a product boundary, not a
  missing feature.
- Local model support remains possible because the policy is about inputs and publishing, not about
  which model runs.

## Alternatives

- Let the agent browse and download any image: unacceptable provenance, copyright and allowlist
  risk.
- Treat a generated mesh as a drop-in hero model: it will not fit the skeleton or compile pipeline.
- Publish likeness mods without a named permission basis: exposes users and the project.
- Ban likeness mods entirely: unnecessary. Local, consent-based, non-commercial character work is a
  valid use of a modding workbench.
