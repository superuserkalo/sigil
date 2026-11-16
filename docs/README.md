# Docs

Status: active index.
Updated: 2026-11-16

This tree is tracked. A fresh clone should be enough for an agent to understand the product, the
domain and the boundaries without reading the whole codebase.

## Read order

1. `CONTEXT.md` at the repo root: the vocabulary.
2. `docs/architecture.md`: processes, tool catalog, safety model, roadmap.
3. `docs/research/deadlock-modding.md`: how mods load and what may go in a payload.
4. `docs/research/vpk-format.md` and `docs/research/patch-feeds.md`: container and update facts.
5. `docs/decisions/`: boundaries that are costly to reverse.
6. `docs/runbooks/development.md`: setup, toolchain, troubleshooting.

## Status

Every doc starts with a status line. Values: `active`, `draft`, `planned`, `superseded`.

| Doc | Status | Meaning |
| --- | --- | --- |
| `architecture.md` | draft | Proposed boundaries and roadmap. Changes need a decision record. |
| `research/deadlock-modding.md` | active | Domain facts with sources. Verify after a game update. |
| `research/pi-durable.md` | superseded | Legacy Pi Durable runtime notes. |
| `research/fx-libfx.md` | active | Why fx and libfx are not the embedded runtime. |
| `research/vpk-format.md` | active | VPK v1/v2 layout used by the game index. |
| `research/patch-feeds.md` | active | Official Deadlock patch feed and file-change sources. |
| `research/stats-case-study.md` | active | Leetify, Blitz.gg and Scope.gg patterns for Deadlock stats. |
| `research/match-recording.md` | proposed | Local demo recording, stats and LLM review pipeline. |
| `research/demo-import.md` | active | Source 2 demo container parsing, local match store and merge. |
| `research/install-pipeline.md` | active | Browse download, archive extraction, payload check and slot deploy. |
| `research/meshy-reverse-engineering.md` | active | Evidence-led study of the Meshy pipeline and our own stage graph. |
| `runbooks/packaging.md` | draft | Desktop, sidecar and CLI packaging, with the unverified rows listed. |
| `design/mod-manager-shell.md` | draft | Mod-manager-first sidebar, screens and real data sources. |
| `decisions/0001-grimoire-fork-and-monorepo.md` | accepted | Fork and repo layout. |
| `decisions/0002-durable-agent-runtime.md` | accepted | Pi Durable, SQLite, secrets and process split. |
| `decisions/0003-windows-compile-backend.md` | accepted | Compile is a backend, not an assumption. |
| `decisions/0004-embedded-runtime-choice.md` | accepted | Pi Durable over fx and libfx, with fx as an optional lane. |
| `decisions/0005-reference-images-and-likeness.md` | accepted | Reference images, likeness rights and generated character mods. |
| `decisions/0006-native-rust-rune-desktop.md` | accepted | Native Rust and Rune desktop, Rust core and CLI. |
| `decisions/0007-tinyagent-host.md` | accepted | TinyAgent replaces Pi Durable as the agent host. |
| `decisions/0008-gamebanana-approved-handoff.md` | accepted | Publishing is an approved browser handoff with recorded state. |
| `runbooks/development.md` | active | Local setup and verification. |

## Updating docs

- Ship the doc change with the code change that makes it true.
- Update the affected doc and the roadmap row in `architecture.md`.
- Record a decision instead of rewriting history when a boundary changes. Keep accepted records
  append-only; supersede them with a new record.
- Never describe a stub as working. Use the phase language in `architecture.md`.
- Local-only notes, issue drafts and handoffs go in `.scratch/<feature>/`, not here.
