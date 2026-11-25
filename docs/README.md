# Docs

Status: active index.
Updated: 2026-11-25

This tree is tracked. A fresh clone should be enough for an agent or a contributor to understand the
product, the domain and the boundaries without reading the whole codebase. Sigil 0.1.42 is the
release these docs describe.

The public plan (what comes next, the mod agent, what is not planned) is
[`ROADMAP.md`](../ROADMAP.md) at the repository root.

## Read order

1. `CONTEXT.md` at the repo root: the vocabulary.
2. `docs/user-guide.md`: what the app does for a player today.
3. `docs/architecture.md`: processes, safety model, network allowlist.
4. `docs/research/deadlock-modding.md`: how mods load and what may go in a payload.
5. `docs/research/vpk-format.md` and `docs/research/patch-feeds.md`: container and update facts.
6. `docs/decisions/`: boundaries that are costly to reverse (its `README.md` lists them all).
7. `docs/runbooks/development.md`: setup, tests, troubleshooting.

## Status

Every doc starts with a status line. Values: `active` (maintained, describes the code), `historical`
(a dated record, kept but not updated; the code wins), `superseded` (replaced by the named doc).

| Doc | Status | Meaning |
| --- | --- | --- |
| `user-guide.md` | active | Player guide for 0.1.42: install, Browse, Installed, profiles, Locker, viewers, crosshair, settings pages, servers, launch, updates, privacy and feedback. |
| `privacy.md` | active | Every host Sigil contacts, what each receives, and the switches. |
| `gdpr-notes.md` | active | Open GDPR items for "Help build Sigil"; notes, not legal advice. |
| `provenance.md` | active | Original code, dependencies, references and asset rules. |
| `architecture.md` | active in part | Processes, agent runtime, safety model and network allowlist are maintained; the capability ladder, CLI list and roadmap are a 2026-11-17 record. |
| `research/deadlock-modding.md` | active | Domain facts with sources. Verify after a game update. |
| `research/vpk-format.md` | active | VPK v1/v2 layout used by the game index. |
| `research/patch-feeds.md` | active | Official Deadlock patch feed and file-change sources. |
| `research/crosshair.md` | active | Reticle convars, HUD geometry, the managed autoexec block, share codes and the community library. |
| `research/stats.md` | active | Deadlock API endpoints, derived metrics, the Stats page and its cache. |
| `research/demo-import.md` | active | Source 2 demo container parsing, local match store and merge. |
| `research/locker-and-recolor.md` | active | Hero roster, Grimoire art, project pin and recolor briefs. |
| `research/subscription-auth.md` | active | Codex and Grok sign-in flows read from fx, and the boundary. |
| `research/character-generation.md` | active | The character stage graph and its backends (Agent, experimental). |
| `research/toolchain.md` | active | Decompile and compile backends (Agent, experimental). |
| `research/stats-case-study.md` | historical | Leetify, Blitz.gg and Scope.gg patterns, 2026-11-16. |
| `research/match-recording.md` | historical | The 2026-11-16 demo and review plan; its LLM review contract is still the spec. |
| `research/meshy-reverse-engineering.md` | historical | The 2026-11-16 Meshy study the character code cites. |
| `research/llm-mesh-generation.md` | historical | Can an LLM produce a mesh? 2026-11-16 research. |
| `research/fx-libfx.md` | historical | The 2026-11-16 fx and libfx evaluation behind ADR 0004. |
| `runbooks/development.md` | active | Local setup, tests, headless checks, the CLI and agent host, troubleshooting. |
| `runbooks/windows.md` | active | The Windows build, installer, first run, launching and what is unverified on real Windows. |
| `runbooks/packaging.md` | active | One executable, the Windows zip and installer, macOS and Linux packages, the verification checklist. |
| `runbooks/releasing.md` | active | Signing key, manifest, the release steps with the installer and stable-name assets, the self-updater. |
| `runbooks/performance.md` | active | Startup, frame time, size and memory: method, measured results, remaining work. |
| `runbooks/acceptance.md` | historical | The 2026-11-16 CLI acceptance run. |
| `design/3d-viewer.md` | active | 3D viewer: asset pipeline, the `sigil-viewer` renderer, the Rune viewport and where it appears. Real-game output unverified. |
| `design/benchmark.md` | active | The in-game benchmark. Real game unverified. |
| `design/stats-live.md` | active | Live match view: console lane, live bridge HUD mod, panel and opt-in player lookup. Real game unverified. |
| `design/mod-safety.md` | active | What Deadlock loads per file type, the safety scan rule per type and the review list. |
| `design/install-robustness.md` | active | Every archive and mod shape, every install failure, the download rules and the GameBanana corpus. |
| `design/deep-links.md` | active | Install with Sigil links, OS registration, single instance, GameBanana 1-Click. |
| `design/packs.md` | active | Packs: share a set of GameBanana mods as a code, link or file. |
| `design/feature-parity.md` | historical | Grimoire and Deadlock Mod Manager feature audit as of 0.1.22 (2026-11-21), and the deferred "Sigil accounts" package. |
| `design/settings-parity.md` | historical | Settings audit as of 0.1.22 (2026-11-21). |
| `design/upstream-gaps-2026-11-21.md` | historical | Upstream gaps snapshot. |
| `design/dmm-unreleased-2026-11-21.md` | historical | Unreleased Deadlock Mod Manager work, snapshot. |
| `design/grimoire-unreleased-2026-11-21.md` | historical | Unreleased Grimoire work, snapshot. |
| `design/mod-manager-shell.md` | historical | The 2026-11-16 sidebar draft. |
| `decisions/README.md` | active | Index of ADRs 0001 to 0019 with their status. |

Removed on 2026-11-25, with their remaining facts folded into the docs that own them:
`design/locker-3d.md` (superseded by `design/3d-viewer.md`), `design/settings-social.md` (gallery
sketch now in `design/feature-parity.md`, DeadlockForge was built),
`research/install-pipeline.md` (superseded by `design/install-robustness.md` and the code), and
`research/pi-durable.md` (notes on the Pi Durable runtime, superseded by ADR 0007; no ADR linked
to it, and the TypeScript packages that used it were removed the same day).

## Updating docs

- Ship the doc change with the code change that makes it true.
- Update the affected doc and its row here.
- Record a decision instead of rewriting history when a boundary changes. Keep accepted records
  append-only; supersede them with a new record.
- Never describe a stub as working. Use the phase language in `architecture.md`.
- Local-only notes, issue drafts and handoffs go in `.scratch/<feature>/`, not here.
