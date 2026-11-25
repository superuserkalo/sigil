# Sigil (Deadlock Agent Harness)

Sigil is a desktop mod manager for Deadlock: Browse and install from GameBanana, Installed with load
order and overlaps, launch modded or vanilla with the `gameinfo.gi` fix, Locker, Crosshair,
Profiles, Autoexec, Performance, Stats and Servers. The desktop UI is native Rust with Rune. An
experimental agent workbench (TinyAgent, BYOK models) sits behind Settings > Experimental > Agent.
"Deadlock Agent Harness" is the original project name; it survives in crate names and the data
folder (`DeadlockAgentHarness`).

Status: released for Windows 10 and 11 x64 (0.1.42 on 2026-11-25), with a per-user installer and
signed self-updates. The app writes to the game install on the player's actions (install, toggle,
load order, `gameinfo.gi`, autoexec, launch). Agent tools still never do. Rows not yet verified on a
real Windows install are listed in `docs/runbooks/windows.md`.

## Read first

1. `CONTEXT.md` for the product vocabulary. Use those words in code, docs and UI.
2. `docs/architecture.md` for boundaries, tool catalog, safety model and roadmap.
3. `docs/research/deadlock-modding.md` for how mods actually load. This is the domain ground truth.
4. `docs/decisions/0007-tinyagent-host.md` for the agent runtime.
5. `docs/decisions/` before changing a boundary that is hard to reverse.

## Repository layout

```text
rust/crates/desktop       the Sigil app (Rune); also runs as `Sigil cli` and `Sigil agent-host`
rust/crates/deadlock-core Rust domain core: discovery, VPK, installs, load order, launch, catalog,
                          crosshair, stats, servers, telemetry client
rust/crates/sigil-viewer  3D viewer (wgpu)
rust/crates/dah-rs        native Rust CLI (folded into the app as `cli`)
rust/crates/tinyagent     Rust TinyAgent v2 port
rust/crates/agent-host    Rust TinyAgent JSONL host (dah-agent; folded in as `agent-host`)
services/telemetry/       Sigil's Cloudflare Worker: opt-in telemetry, catalog index, crosshairs
site/                     the sigilmods.app website (static, deployed on Vercel)
docs/                     tracked project docs (architecture, decisions, research, runbooks)
scripts/                  vendoring, packaging, installer, release signing
vendor/                   local Rune, TinyAgent and Grimoire checkouts, gitignored
.scratch/                 local specs, issues, worktrees, throwaway output, gitignored
```

`pnpm vendor` clones Grimoire and grimoire-social into `vendor/`; Grimoire is a domain reference
only: nothing is built from that checkout. The Locker art is Sigil's own copy in
`rust/crates/desktop/assets/locker`. Do not edit the vendored checkouts in place without recording
the patch.

## Commands

```bash
pnpm vendor:rune               # clone the pinned Rune revision into vendor/rune (needed to build)
pnpm vendor                    # clone Grimoire and grimoire-social into vendor/ (Locker art)
pnpm rust:check                # cargo check the Rust workspace
pnpm rust:app                  # run the Rune desktop app
pnpm rust:dah -- doctor        # run the native CLI
pnpm rust:agent                # run the Rust TinyAgent host
cd rust && cargo test -p deadlock-core -p dah-desktop    # the domain core and the app (what CI runs)
cd services/telemetry && npm test                        # the Worker tests (vitest, what CI runs)
scripts/package-windows.sh     # Windows x64 Sigil.exe and zip via cargo-xwin
scripts/package-installer.sh <Sigil.exe> <version> <out>  # Sigil-Setup.exe via NSIS
```

Rust 1.99.1 (pinned in `rust/rust-toolchain.toml`). Node 24 for the release scripts and the Worker
(npm, `services/telemetry/package-lock.json`). pnpm 10 is optional: the root `package.json` only
holds the `pnpm vendor*` and `pnpm rust:*` aliases above.

## Hard rules

These are product constraints, not preferences.

- Build for parity. Sigil's goal is everything Grimoire and Deadlock Mod Manager offer, organised
  better. Build what they have; never decline a feature for style or caution. Stop only for the
  two rules on keys and telemetry below or a real account-security risk, and tell the owner.
  Optional looks (themes, glow, art) are built and left off by default.
- Never write to a game install from an agent tool. Tools produce a plan, a staging folder and
  validated output. Installing into `citadel/addons`, editing `gameinfo.gi`, launching the game,
  and publishing are app actions the user triggers. A confirm dialog is a UX choice, not a rule:
  "don't ask again" and one-click actions are fine.
- The desktop UI is native Rust with Rune. Do not introduce an Electron renderer or a browser web
  view as the app shell. See `docs/decisions/0006-native-rust-rune-desktop.md`.
- TinyAgent is the agent host and runs as its own child process: the Rust protocol v2 host
  (`Sigil agent-host`), not a Node sidecar (ADR 0007, ADR 0010). The UI never runs agent tools in
  its render loop.
- BYOK keys live in the Rust app and never enter the UI transcript, tool output, logs or a commit.
  The CLI store is `chmod 600`; the desktop store uses the OS keychain.
- Nothing leaves the machine without the player's yes (for product data: the Share answer or the
  Help build Sigil switch, ADR 0017). No third-party analytics (no Google
  Analytics, no Sentry SaaS), no heartbeat. Network calls come from a documented allowlist: model
  provider endpoints, `api.steampowered.com` for the official patch feed and Valve's relay list, GameBanana, and the
  GitHub update check. Owner-approved opt-ins, off by default: match data sharing to
  `api.deadlock-api.com` (`/v1/matches/salts`), Discord Rich Presence over the local Discord IPC
  socket (no network) (2026-11-17), and Sigil's own telemetry Worker on Cloudflare (2026-11-18):
  usage events, scrubbed crash reports, player-sent diagnostics and unknown-mod hash lookups, each
  off until the player agrees (`docs/decisions/0014-opt-in-telemetry.md`, `docs/privacy.md`), and
  "Help build Sigil" product data to the same Worker (2026-11-25): GameBanana mod ids and actions,
  mod health, Browse use with scrubbed search text, top heroes and rank tier, performance and
  hardware class, app use; asked in onboarding, off by default, switchable and deletable at any
  time (`docs/decisions/0017-opt-in-product-data.md`). Only the desktop app records it; the CLI and
  the agent host never do. Catalog search index (2026-11-25, owner approved): the
  Worker, not the app, reads public GameBanana text and builds an index the app downloads by
  default and matches locally: typed text never leaves the PC. Two optional steps, off by default,
  send public GameBanana content only to Cloudflare Workers AI and TypeSafe (`api.typesafe.ai`);
  the key is a Worker secret and never ships in the app
  (`docs/decisions/0018-typesafe-catalog-tags.md`). Community crosshair library (2026-11-25,
  owner approved): on the player's own Publish, a crosshair's name and settings go to the same
  Worker and become public; browsing and applying send nothing about the player. No accounts, no
  install id, no IP stored; a daily network hash for limits and dedupe is deleted after 2 days, and
  a per-crosshair report hash after 90 days (`docs/decisions/0019-community-crosshairs.md`).
- Preserve MIT attribution for Rune, TinyAgent, Grimoire and every bundled community tool. See
  `THIRD_PARTY_NOTICES.md`. CSDK 12 has no license and is never redistributed.
- No em dashes in code, UI strings, comments or docs. Use a colon, period or parentheses.

## Agent runtime

`rust/crates/agent-host` runs TinyAgent v2 as a local child process (`dah-agent`):

- One durable session per mod project. The root session is the general workbench. Sessions are
  JSONL journals under the app data directory.
- The host runs as `Sigil agent-host` (the app's own executable, still a separate child process
  speaking the same JSONL protocol); its tools run `Sigil cli ...`. `DAH_AGENT_BIN` and `DAH_RS_BIN`
  point at standalone builds instead.
- Tools call the Rust domain core through `dah-rs`. The payload validator is also implemented in the
  host for pure checks.
- Mark a tool `idempotent: true` only for reads and pure computation. Writes, process launches and
  external calls are not idempotent.
- Approval gates use `beforeTool` and `pause`. The Rust app answers with a resume result. Deploy,
  launch, `gameinfo.gi` repair and publish are gated.
- Providers are OpenAI Responses, Anthropic Messages and OpenAI-compatible Chat Completions. The
  last one is the local-model path.
- Keys come from the app or environment and never enter the journal.
- The Rust host is the only host; see `docs/decisions/0007-tinyagent-host.md`.

Adding a tool:

1. Define it in `rust/crates/agent-host` with a JSON Schema and an explicit `idempotent` value.
2. Return compact model-facing text. Do not put secrets or large binaries in it.
3. If it leaves the mod workspace, route it through the Rust app and pause for approval.
4. Add a contract test in the owning crate. Rules belong in code, not only in the prompt.

## Deadlock domain rules

- Engine name is `citadel`. Mods load from `game/citadel/addons`.
- A mod is `pakNN_dir.vpk`, NN from `01` to `99` per folder (Sigil adds `citadel/sigil-addons2` to
  `sigil-addons10` for slots 100 to 990; `deadlock_core::addons`). Other names and subfolders are ignored. Lower
  number wins when two mods ship the same path.
- The game only sees the `addons` folder when `gameinfo.gi` lists `Game citadel/addons` above
  `Game citadel`. Game updates and Steam file verification reset that file.
- A mod replaces files by exact path. One wrong folder or letter means the game silently uses its
  own file.
- Compiled names end in `_c`; the game ignores source files.
- Compiling requires the Windows CSDK 12 toolchain (`resourcecompiler.exe`). Packing, inspecting,
  merging and validating are cross-platform. Do not promise a one-click compile on macOS or Linux
  until a verified backend exists.
- Upstream path moves break mods. Use `GameTracking-Deadlock` history and a path migration step
  instead of guessing.
- Use the game index (`dah index`, `deadlock_search_paths`) for exact paths. Hero folders move
  between updates, so do not hard-code a hero folder table.

## Docs

`docs/` is tracked here on purpose: this is a new project and agents need the context in a fresh
clone. Keep it that way. `docs/README.md` is the index; every doc carries a status line.

- Local-only work goes in `.scratch/<feature>/`: specs, issue notes, handoffs, experiments.
  `.scratch/worktrees/<producer>/<slug>` for worktrees. Never use OS temp directories for work you
  want to keep.
- After a behavior change, update the affected doc and the roadmap status in `docs/README.md`.
- Record a decision in `docs/decisions/` when a boundary changes: process split, storage, build
  backend, safety gate, model access.
- Do not let docs claim a feature works when only a stub exists. Mark phase status honestly.

## Git

- Stage explicit paths. Never `git add -A`.
- Conventional subject: `feat(cli): add doctor`, `fix(tools): reject vdata payloads`,
  `docs: record build backend decision`.
- Do not commit `vendor/`, `.scratch/`, game files, VPKs, credentials, SQLite stores or logs.
- Never run `git reset --hard`, `git checkout .`, `git clean -fd`, `git stash`, or
  `git commit --no-verify` in a shared checkout.
- Do not push, open a PR, create a release or publish a mod unless asked.

## Gotchas

- TinyAgent is pinned and vendored. The Rust port in `rust/crates/tinyagent` is protocol v2. Keep it
  in sync by reading the TypeScript implementation in `vendor/tinyagent`; run
  `scripts/vendor-tinyagent.sh` after a revision bump.
- Rune is pre-release. `rune = { git = ... }` alone fails because its patched `parley_engine` is not
  published. Run `scripts/vendor-rune.sh` and keep the `[patch.crates-io]` entries in
  `rust/Cargo.toml`. Bump the vendored revision deliberately.
- Do not swap the agent runtime to fx or libfx without superseding ADR 0007. fx remains a possible
  external ACP or MCP lane, not a dependency.
- One process owns the TinyAgent session directory. Do not point two hosts at the same journals.
- The agent host must not expose a general shell tool by default. Tools call `dah-rs` with argument
  arrays from the workspace.
- External binaries are resolved from settings, never downloaded on demand from an arbitrary URL.
  Pin a version and a hash when one is added.
- The desktop app is one Rust executable: the CLI (`cli`) and the agent host (`agent-host`) are
  subcommands of it. The Locker art (`rust/crates/desktop/build.rs`) and the notices are built
  into that executable, so packaging ships the one file.
- Game updates move hero folders and reset `gameinfo.gi`. A "worked yesterday" report is usually a
  path or SearchPaths regression, not an agent bug.
