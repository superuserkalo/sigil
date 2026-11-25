# 0006 Native desktop in Rust with Rune

Status: accepted
Date: 2026-11-16
Update 2026-11-25: the transition is over. The TypeScript CLI, tools and agent host were removed from the repository; Cargo is the only build toolchain for the app (ADR 0007).

## Context

The desktop shell was planned as a fork of Grimoire, an Electron app. Electron ships a browser
runtime, a Node main process and a large dependency tree for a UI that mostly lists files, draws
previews and streams a conversation. On the machines this product targets, that cost is visible.

Rune from Anomaly is a native Rust UI framework: its own GPU renderer, text, layout, springs and
platform shells for macOS, Windows, Linux and the web, with headless snapshots that agents can
check. It is pre-release and pinned to a dev revision.

The agent runtime is TinyAgent. It owns durable sessions, approvals, transcripts, provider access
and resume policy. The Rust protocol v2 port lives in `rust/crates/tinyagent`; the Rust host is
`rust/crates/agent-host`.

## Decision

- The desktop app is native Rust with Rune. No Electron renderer and no browser UI.
- The domain core is Rust in `rust/crates/deadlock-core`: game discovery, VPK reading, index,
  catalog, extraction, project manifests, baselines, compatibility checks and patch notes.
- The CLI is Rust in `rust/crates/dah-rs`.
- Rune is vendored at a pinned revision by `scripts/vendor-rune.sh`. Its patched crates are wired
  through `[patch.crates-io]` in `rust/Cargo.toml`. Do not depend on `rune = { git = ... }` alone:
  the published dependency graph does not include Rune's patched `parley_engine`.
- The Rust TinyAgent host is the target. The TypeScript host stays behind a local process boundary
  during the cutover. The UI never executes agent tools in its render loop.
- The TypeScript CLI and tools remain as a reference and a test surface while the Rust crates reach
  parity, then are removed. The agent host keeps using them in the meantime.
- Game install writes stay behind an explicit approval gate owned by the Rust app. The agent host
  returns plans, never game writes.

## Consequences

- The UI is a native binary with no Chromium, no renderer process and no Electron packaging rules.
- Rune is a pre-release dependency. Version bumps are a deliberate migration, not a lockfile drift.
- There are two build toolchains during the transition: Cargo for the app, Node for the agent host.
  Packaging must bundle or locate the sidecar. That packaging question is open.
- Rune's text input, IME and accessibility are still maturing. If a screen cannot meet the bar,
  the fallback is a small native control, not a browser view.
- The Grimoire fork is no longer the UI plan. Its Steam discovery, addon slot, VPK and preview
  knowledge remains a reference, and its MIT attribution stays in the notices.
- Headless rendering gives every screen a deterministic snapshot for review and regression checks.

## Alternatives

- Keep Electron and optimize: the renderer and packaging weight remain, and the UI work is already
  small enough to port.
- Tauri: smaller than Electron, but still a web renderer and a second language for the UI.
- egui, Slint or GPUI: viable, but Rune is the only one with a first-class headless agent loop and
  the interaction quality this product needs.
- Port the agent runtime to Rust now: no verified replacement for durable approvals, provider
  breadth and replay semantics. A sidecar is the smaller risk.

## Supersedes

- `0001-grimoire-fork-and-monorepo.md` only for the desktop shell decision. The repository layout,
  MIT attribution and Grimoire domain reference still apply.
