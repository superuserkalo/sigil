# Docs

Status: active index.
Updated: 2026-11-17

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
| `provenance.md` | active | Original code, dependencies, references and asset rules. |
| `architecture.md` | draft | Proposed boundaries and roadmap. Changes need a decision record. |
| `research/deadlock-modding.md` | active | Domain facts with sources. Verify after a game update. |
| `research/pi-durable.md` | superseded | Legacy Pi Durable runtime notes. |
| `research/fx-libfx.md` | active | Why fx and libfx are not the embedded runtime. |
| `research/vpk-format.md` | active | VPK v1/v2 layout used by the game index. |
| `research/patch-feeds.md` | active | Official Deadlock patch feed and file-change sources. |
| `research/subscription-auth.md` | active | Codex, Grok and Gateway sign-in flows read from fx, and the boundary. |
| `research/llm-mesh-generation.md` | active | Can an LLM produce a mesh? Code lanes, mesh models and BYO backends. |
| `research/locker-and-recolor.md` | active | Hero roster, Grimoire art, project pin and recolor briefs. |
| `research/crosshair.md` | active | Reticle convars, HUD geometry and the managed autoexec block. |
| `research/stats.md` | active | Deadlock API endpoints, derived metrics, the Stats page and its cache. |
| `research/stats-case-study.md` | active | Leetify, Blitz.gg and Scope.gg patterns for Deadlock stats. |
| `research/match-recording.md` | active | Demo import, merged API data and the linted review pipeline. |
| `research/demo-import.md` | active | Source 2 demo container parsing, local match store and merge. |
| `research/install-pipeline.md` | active | Browse download, archive extraction, payload check and slot deploy. |
| `research/meshy-reverse-engineering.md` | active | Evidence-led study of the Meshy pipeline and our own stage graph. |
| `runbooks/acceptance.md` | active | Tier 5 acceptance result and the unverified rows. |
| `runbooks/windows.md` | draft | Windows bundle, first run and unverified rows. |
| `runbooks/packaging.md` | draft | One-executable packaging (CLI and agent host folded in), with the unverified rows listed. |
| `runbooks/releasing.md` | draft | Signing key, release manifest, signing and publishing steps; the self-updater. |
| `runbooks/performance.md` | active | Startup, frame time, size and memory: method, measured results, remaining work. |
| `design/mod-manager-shell.md` | draft | Mod-manager-first sidebar, screens and real data sources. |
| `design/feature-parity.md` | current | Every Grimoire and Deadlock Mod Manager feature outside Settings, with Sigil's status, effort and priority. |
| `design/locker-3d.md` | design | What a posed 3D hero viewer needs from Rune and the game files; nothing is built. |
| `design/stats-live.md` | design | Live match strip: what `-condebug` gives, what a HUD bridge mod must add, the polling budget. |
| `design/settings-parity.md` | current | Every Grimoire and Deadlock Mod Manager setting, with Sigil's status and place. |
| `design/deep-links.md` | current | Install with Sigil links: accepted formats, OS registration, single instance, GameBanana 1-Click listing. |
| `design/settings-social.md` | current | Integrations built without a backend; the services a community gallery and remote install would need. |
| `decisions/0001-grimoire-fork-and-monorepo.md` | accepted | Fork and repo layout. |
| `decisions/0002-durable-agent-runtime.md` | accepted | Pi Durable, SQLite, secrets and process split. |
| `decisions/0003-windows-compile-backend.md` | accepted | Compile is a backend, not an assumption. |
| `decisions/0004-embedded-runtime-choice.md` | accepted | Pi Durable over fx and libfx, with fx as an optional lane. |
| `decisions/0005-reference-images-and-likeness.md` | accepted | Reference images, likeness rights and generated character mods. |
| `decisions/0006-native-rust-rune-desktop.md` | accepted | Native Rust and Rune desktop, Rust core and CLI. |
| `decisions/0007-tinyagent-host.md` | accepted | TinyAgent replaces Pi Durable as the agent host. |
| `decisions/0008-gamebanana-approved-handoff.md` | accepted | Publishing is an approved browser handoff with recorded state. |
| `decisions/0009-deadworks-servers.md` | accepted | Deadworks Servers screen: allowlisted list, checked content, confirmed join writes. |
| `decisions/0010-one-executable.md` | accepted | One shipped executable: `cli` and `agent-host` subcommands, host still a child process. |
| `decisions/0011-build-for-parity.md` | accepted | Build for parity; cosmetic-only, compiled-only payload and confirm-gate rules dropped. |
| `decisions/0012-signed-self-update.md` | accepted | Self-update from signed GitHub releases; swap, rollback and the key. |
| `runbooks/development.md` | active | Local setup and verification. |

## Updating docs

- Ship the doc change with the code change that makes it true.
- Update the affected doc and the roadmap row in `architecture.md`.
- Record a decision instead of rewriting history when a boundary changes. Keep accepted records
  append-only; supersede them with a new record.
- Never describe a stub as working. Use the phase language in `architecture.md`.
- Local-only notes, issue drafts and handoffs go in `.scratch/<feature>/`, not here.
