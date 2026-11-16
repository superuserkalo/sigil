# Third party notices

Status: active. Update when a dependency or bundled binary changes.
Updated: 2026-11-16

This project is MIT licensed. The following projects are used, forked or bundled. Their licenses and
attributions are preserved with the code or binary. If a tool is not listed here, it is not cleared
for redistribution.

## Desktop shell

**Grimoire**
- Source: https://github.com/Slush97/grimoire
- License: MIT, Copyright (c) 2025 esoc
- Use: fork of the desktop mod manager shell. The full MIT text ships with the fork at
  `vendor/grimoire/LICENSE`. Keep this notice and the upstream license in any packaged build.
- Companion: `grimoire-social` is also MIT by the same author. Its wire types are consumed by the
  fork.

## Agent runtime and model access

**Pi Durable, Pi AI, Pi Schema, Chord**
- Source: https://github.com/earendil-works/pi
- Packages: `@earendil-works/pi-durable`, `@earendil-works/pi-ai`,
  `@earendil-works/pi-schema`, `@earendil-works/chord`
- License: MIT
- Use: durable agent harness, provider access, schemas and document state. Pinned to `1.5.2`.

## Deadlock tooling

**vpkmerge**
- Source: https://github.com/Slush97/vpkmerge
- License: MIT
- Use: merge and split addon VPKs. Optional external binary.

**ValveResourceFormat / Source 2 Viewer**
- Source: https://github.com/ValveResourceFormat/ValveResourceFormat
- License: MIT
- Use: inspect, export, decompile and create VPKs. Optional external binary or bundled CLI when a
  pinned release is cleared.

**VPKEdit / vpkeditcli**
- Source: https://github.com/craftablescience/VPKEdit
- License: MIT
- Use: create, inspect and extract VPKs. Optional external binary.

**DeadPacker**
- Source: https://github.com/Artemon121/DeadPacker
- License: MIT
- Use: reference for a compile, pack, copy and launch loop. Not bundled.

**DeadlockTools**
- Source: https://github.com/dotryen/DeadlockTools
- License: MIT
- Use: AnimGraph 2 references for compiled hero models. Optional external binary.

**Community CSDK 12 (Reduced CSDK 12)**
- Source: community distribution page linked from the Deadlock Modding Notes site
- License: none stated
- Use: local Windows toolchain for `resourcecompiler.exe`. Never redistributed, never downloaded by
  this app. The user installs it themselves.

## Data and services

**GameTracking-Deadlock**
- Source: https://github.com/SteamTracking/GameTracking-Deadlock
- License: not stated
- Use: reference for per-build file history and path migrations. No files are bundled.

**GameBanana**
- Use: mod catalog and publishing, with the user's own account and explicit approval. Subject to
  GameBanana's terms.

## Test and build dependencies

Development dependencies (TypeScript, tsx, Node types) keep their own licenses as declared in
`node_modules`. They are not shipped in the app runtime.
