# Provenance

Status: active. What is original here, what is a dependency, and what is a
reference.
Updated: 2026-11-16

This is not a Grimoire fork. The desktop, the domain core, the CLI and the agent
host in `rust/` are a native Rust implementation written for this project. No
Electron renderer, React component or Grimoire TypeScript module is compiled into
the app.

That is not the same as "no prior art". The project stands on MIT and Apache-2.0
work, and some of it is read or studied rather than copied.

## Original in this repository

- `rust/crates/deadlock-core`: discovery, VPK read/write, game index, catalog,
  addons, profiles, autoexec, launch stash, deploy, migration, release, publish
  state, install pipeline, stats, demo import and merge, review linter, Locker,
  recolor briefs, crosshair, toolchain, character stage graph, subscription auth.
- `rust/crates/dah-rs`: the `dah` CLI over that core.
- `rust/crates/agent-host`: the TinyAgent v2 JSONL host, tools and approvals.
- `rust/crates/desktop`: the native Rune workbench, every screen and the shell.
- The docs, acceptance script and packaging scripts.

## Dependencies

| Project | License | Use |
| --- | --- | --- |
| Rune | MIT | Native GPU UI framework, vendored at a pinned revision |
| TinyAgent | MIT | Agent runtime, vendored at a pinned revision |
| Rust crates | permissive | see `Cargo.lock` and `THIRD_PARTY_NOTICES.md` |

## Reference only, no code compiled

| Project | License | What was used |
| --- | --- | --- |
| Grimoire | MIT | Desktop and mod-manager behavior, Steam discovery, addon slot rules, `gameinfo.gi`, VPK merge ideas, sidebar order, crosshair convar surface, hero asset layout, server browser design |
| fx | Apache-2.0 | Codex and Grok OAuth clients, endpoints, scopes, headers and current model hints |
| DeadlockTools | MIT | AnimGraph 2 references for compiled hero models |
| DeadPacker | MIT | Compile, pack, copy and launch loop ideas |
| ValveResourceFormat / VPKEdit / vpkmerge | MIT | Optional external tools, resolved from settings, never downloaded |
| CSDK 12 | none stated | Local Windows compiler, never bundled or redistributed |

Rewritten in Rust from the reference: the Grimoire crosshair convar names and
geometry rules, the hero art naming rules, and the fx OAuth protocol values.
Those are data and protocol facts, not copied source files.

## Assets and data

- Grimoire's `public/locker` hero renders and name art are read at runtime from a
  local checkout. They are not stored in this repository and not redistributed in
  the packages.
- Game paths come from the user's own Deadlock install.
- API data comes from `api.deadlock-api.com`, Steam news, and GameBanana under
  their terms.
- No CSDK, game VPK, game asset or credential is in a package.

## History

ADR 0001 planned a Grimoire fork for the desktop shell. ADR 0006 superseded that
for the shell: the app is native Rust with Rune. The fork was never checked in,
and `vendor/grimoire` remains an upstream reference checkout.

## Rule

If a future change copies source or assets from another project, update
`THIRD_PARTY_NOTICES.md` and this file in the same commit. If it is a new
boundary, record a decision.
