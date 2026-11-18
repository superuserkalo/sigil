# 0013 Sigil links vpkmerge-core for compatibility repairs

Status: proposed (accepted once the integration builds and passes its tests)
Date: 2026-11-18

## Context

Game updates break older mods: skeleton bones, root attachments and cameras, movement and
footstep sound bindings, outdated vdata fields and text. Sigil only detects these
(`deadlock-core/src/mod_health.rs`). Both reference apps repair them:

- Grimoire runs the bundled `vpkmerge` binary (`compatibility heal`, `attachments heal`), then
  verifies the rewritten archive and keeps a backup.
- Deadlock Mod Manager writes repairs into a separate package without touching the mod, can fix
  before every launch, and can revert.

Repairing means decoding and re-encoding compiled Source 2 resources (KV3 in resource blocks).
vpkmerge (MIT, Rust) already does this in `vpkmerge-core` and its `morphic` KV3 crate, and is
listed in `THIRD_PARTY_NOTICES.md` as an optional external binary. ADR 0010 makes Sigil one
executable, so shelling out to a second binary is not the path.

## Decision

- `deadlock-core` depends on `vpkmerge-core` as a git dependency pinned to a full revision. The
  revision is bumped deliberately, like Rune and TinyAgent.
- `deadlock-core/src/compat_repair.rs` wraps it: a dry-run check, a repair into a staging file,
  and Grimoire's verification (same entry set, only reported entries changed, base game
  unchanged during the run, split archives refused, animation overrides from other enabled mods
  detected).
- Writing the repaired archive into `citadel/addons` stays an app action the player triggers (or
  turns on as "fix before launch"), with a backup and a revert, under the same game-running
  guard as other installs. Agent tools never call it.
- `THIRD_PARTY_NOTICES.md` changes the vpkmerge entry from an optional binary to a linked
  library and adds `morphic`.

## Consequences

- The executable grows by vpkmerge-core and its dependencies (gltf, image, quick-xml, rayon,
  valve_pak). Measure the release size before accepting; trim with features upstream if needed.
- vpkmerge's GLB soul-container import and recolor code become reachable for later waves (Mod
  Foundry, soul container and spirit urn).
- An upstream bug in repairs affects Sigil directly; the verification step and the backup are
  the safety net.
