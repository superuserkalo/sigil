# 0001 Grimoire fork and monorepo

Status: accepted. Superseded in part by `0006-native-rust-rune-desktop.md` for the desktop shell.
Date: 2026-11-16

## Context

Grimoire is an MIT-licensed Electron mod manager for Deadlock. It already handles Steam discovery,
addon slots, `gameinfo.gi`, VPK merge, a GameBanana catalog, and 2D/3D cosmetic previews. Building
those from scratch would delay the only differentiated part: the agent loop and the Deadlock tool
surface.

Grimoire is a single-maintainer desktop app with a companion `grimoire-social` repository. Its
workspace resolves `@grimoire/social-types` from `../grimoire-social`, so a checkout needs both
repositories side by side.

## Decision

- Fork Grimoire as the desktop shell. Keep its MIT license and attribution.
- Keep this repository as the agent and tooling home first. Do not commit a 200 MB upstream
  checkout into it.
- `pnpm vendor` clones `grimoire` and `grimoire-social` side by side into `vendor/` so the workspace
  link resolves. `vendor/` is gitignored.
- When the desktop work starts, the fork becomes its own repository. The preferred layout is either
  a pinned submodule under `apps/desktop` or a sibling checkout documented in the runbook. Record
  the choice before moving code.
- Keep the fork narrow: add the agent surface, keep mod management, hide social and stats features
  until the mod loop is proven.

## Consequences

- Upstream changes can be pulled while our patches stay small and reviewable.
- The monorepo can typecheck and test without Electron today.
- The desktop build later inherits Grimoire's packaging constraints: native modules, bundled
  main-process dependencies, i18n gates, and no em dashes.
- Two licenses and two release processes must be tracked.

## Alternatives

- Reimplement a mod manager from scratch: too much undifferentiated work.
- Vendor Grimoire into this repository: bloats the repo and obscures upstream history.
- Depend on Grimoire as a published package: it is an app, not a library.
