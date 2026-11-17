# Third party notices

Status: active. Update when a dependency or bundled binary changes.
Updated: 2026-11-16

This project is MIT licensed. The following projects are used, forked or bundled. Their licenses and
attributions are preserved with the code or binary. If a tool is not listed here, it is not cleared
for redistribution.

## Desktop shell

**Rune**
- Source: https://github.com/anomalyco/rune
- License: MIT, Copyright (c) Anomaly
- Use: native GPU desktop UI framework. Vendored at a pinned pre-release revision by
  `scripts/vendor-rune.sh`; its patched crates are built from `vendor/rune/vendor`. Not exposed as a
  general-purpose public dependency.

**rodio, cpal and Symphonia (sound previews)**
- Sources: https://github.com/RustAudio/rodio, https://github.com/RustAudio/cpal and
  https://github.com/pdeljanov/Symphonia
- License: MIT or Apache-2.0 (rodio and cpal, Copyright (c) the RustAudio contributors); MPL-2.0
  (Symphonia, Copyright (c) Philip Deljanov). Linked as ordinary crates from crates.io; the MP3
  decoder is the only Symphonia feature enabled.
- Use: `rust/crates/desktop/src/screens/browse/audio.rs` plays GameBanana sound samples in memory.
  No sample is written to disk or redistributed.

**Grimoire**
- Source: https://github.com/Slush97/grimoire
- License: MIT, Copyright (c) 2025 esoc
- Use: domain reference for Steam discovery, addon slots, `gameinfo.gi`, VPK merge and previews. No
  longer the desktop shell. Keep this notice and the upstream license in any packaged build that
  reuses its code or assets.
- Companion: `grimoire-social` is also MIT by the same author. Its wire types are consumed only if a
  Grimoire component is reused.
- Locker art: packaged builds ship the hero renders, name plates and portraits from
  `vendor/grimoire/public`, recompressed to WebP by `scripts/pack-locker-art.sh`. This follows
  Grimoire's own distribution of the same files. The characters and their art belong to Valve
  Corporation; Sigil is not affiliated with or endorsed by Valve.
- Locker: the hero roster, codenames and face positions in `rust/crates/deadlock-core/src/locker.rs`
  are ported from `electron/main/services/heroCodenames.ts` and `src/lib/lockerUtils.ts`; the
  categories store from `src/lib/lockerCategories.ts`; the screen layout and copy from
  `src/pages/Locker.tsx` and `LockerHero.tsx`. Packaged builds also ship
  `public/locker/global-bg.webp` (recompressed) as the General backdrop.
- Autoexec premade commands: `rust/crates/deadlock-core/data/autoexec-presets.json` is ported from
  Grimoire's `src/components/autoexec/commandPresets.ts` and its English strings (every command,
  name, description and category; names recased to sentence case).
- Crosshair scenes: the two preview captures `public/crosshair/bg1.jpg` and `bg2.jpg`, recompressed
  to WebP (with small thumbnails) in `rust/crates/desktop/assets/crosshair/` and embedded in the
  desktop binary. The share code and console line follow Grimoire's crosshair page.
- Crosshair presets: the "Default" and "Game default" presets in
  `rust/crates/deadlock-core/src/crosshair.rs` carry Grimoire's `CROSSHAIR_DEFAULTS` and
  `CROSSHAIR_GAME_DEFAULTS` values from `src/lib/crosshair.ts`; the saved presets file
  (`crosshair-presets.json`: `id`, `name`, camelCase `settings`, `createdAt`), the share code shape
  check and the decode through deadlock-api follow `electron/main/ipc/crosshairPresets.ts` and
  `src/lib/crosshair.ts`. Grimoire ships no other named presets; the rest are Sigil's own.
- Performance configs: `rust/crates/deadlock-core/src/performance/generate.rs` ports the gameinfo.gi
  diff and classification from `electron/main/services/performancePresetGen.ts`, and
  `performance/classify.rs` ports the exclusion, opt-in and video tables from
  `scripts/performance-presets.json` (reasons reworded). The in-place patcher and its `was`/`added`
  markers follow `performanceConfig.ts`.
- Browse mod details: the details view in `rust/crates/desktop/src/screens/browse.rs` (header,
  gallery stack and lightbox, files, artist, about, changelog, comments, docked sidebar) is ported
  from `src/components/ModDetailsModal.tsx`, and the NSFW veil on browse cards follows its Browse
  cards. No Grimoire assets are copied.
- Settings parity (2026-11-17), ported code and protocol, no assets: the GameBanana file server
  directory, range probe and test (`rust/crates/deadlock-core/src/file_servers.rs`, from
  `electron/main/services/gamebananaFileServers.ts`); the DeadlockForge bridge protocol, ports,
  origin and host checks and limits (`rust/crates/desktop/src/screens/settings/forge.rs`, from
  `forgeBridge.ts`); the imprint entries `addoninfo.txt` and `modinfo.json`
  (`mod_data::imprint_vpk`, from `modinfoFormat.ts`); the editor picker list
  (`settings/editor.rs`, from `externalEditor.ts`); the welcome steps (`settings/onboarding.rs`,
  from `WelcomeModal.tsx`); the Preferences and Support behaviours.
- Personalize: packaged builds ship `public/locker/launch-modded-bg.webp` and
  `public/locker/launch-vanilla-bg.jpg` (copied by `scripts/pack-locker-art.sh`) as the default art
  behind the sidebar launch buttons. The two-corner ambient glow and its seven presets (`GLOWS` in
  `rust/crates/desktop/src/ui/personalize.rs`) follow `src/lib/backgroundGradient.ts`; the art
  surfaces, sources and framing follow `src/components/settings/AppearanceArtSection.tsx`. The ten
  preset themes are Sigil's own.
- Settings: the light theme neutrals and the eight accent presets in
  `rust/crates/desktop/src/ui.rs` and `prefs.rs` follow Grimoire's `src/index.css` and
  `src/lib/accentColor.ts`; the Steam launch options edit in
  `rust/crates/deadlock-core/src/steam_launch.rs` follows the approach of
  `electron/main/services/launchOptions.ts` (reimplemented in Rust).

**Lucide**
- Source: https://lucide.dev
- License: ISC, Copyright (c) 2026 Lucide Icons and Contributors
- Use: interface icons in `rust/crates/desktop/src/icons.rs`, generated by `scripts/gen-icons.mjs`.

## Agent runtime and model access

**TinyAgent**
- Source: https://github.com/superuserkalo/tinyagent
- License: MIT, Copyright (c) 2026 superuserkalo
- Use: agent host. Durable sessions, pause and resume, hooks and provider access. Vendored at a
  pinned revision by `scripts/vendor-tinyagent.sh`. The Rust implementation is not the host yet.

**Pi Durable, Pi AI, Pi Schema, Chord**
- Source: https://github.com/earendil-works/pi
- License: MIT
- Use: legacy agent host in `packages/harness` during the TinyAgent port. Not a target dependency.
  Remove this entry when the package is deleted.

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
- Use: reference for per-build file history and path migrations. No files are bundled. The
  Performance page downloads `DumpSource2/convars.txt` at runtime to mark settings the game no
  longer reads from `gameinfo.gi`; it is cached locally, not redistributed.

**OptimizationLock and OptiLock (performance configs)**
- Source: https://github.com/Sqooky/OptimizationLock and https://github.com/dacooderr/OptiLock
- License: GPL-3.0, by Sqooky, dacooder and their contributors (credited per config in the app and
  in the `gameinfo.gi` marker Sigil writes).
- Use: fetched at runtime from GitHub on the user's machine and cached under the app data
  directory. Sigil does not bundle or redistribute the config files or their values. Sigil writes a
  diff of the author's file into the user's own `gameinfo.gi` on the user's Apply.

**GameBanana**
- Use: mod catalog and publishing, with the user's own account and explicit approval. Subject to
  GameBanana's terms.

## Test and build dependencies

Development dependencies (TypeScript, tsx, Node types) keep their own licenses as declared in
`node_modules`. They are not shipped in the app runtime.
