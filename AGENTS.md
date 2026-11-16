# Deadlock Agent Harness

Desktop workbench where an AI coding agent builds, packs, validates and installs Deadlock mods.
The desktop UI is native Rust with Rune. The agent host is TinyAgent. Model access is BYOK.

Status: pre-alpha. The native Rust core, Rune app and CLI are in progress. The TinyAgent host, game
discovery, VPK indexing and payload validation exist. Nothing writes to a game install yet.

## Read first

1. `CONTEXT.md` for the product vocabulary. Use those words in code, docs and UI.
2. `docs/architecture.md` for boundaries, tool catalog, safety model and roadmap.
3. `docs/research/deadlock-modding.md` for how mods actually load. This is the domain ground truth.
4. `docs/research/pi-durable.md` for the runtime this is built on.
5. `docs/decisions/` before changing a boundary that is hard to reverse.

## Repository layout

```text
apps/cli/                 dah command: doctor, models, auth, chat, index, find, extract
rust/crates/deadlock-core  Rust domain core: discovery, VPK, catalog, project, patches, impact
rust/crates/dah-rs        native Rust CLI
rust/crates/tinyagent     Rust TinyAgent v2 port
rust/crates/agent-host    Rust TinyAgent JSONL host (dah-agent)
rust/crates/desktop       native Rune desktop workbench
packages/harness/         legacy Pi Durable session, BYOK catalog, credential store
packages/deadlock-tools/  game discovery, VPK payload rules, agent tools (during the port)
docs/                     tracked project docs (architecture, decisions, research, runbooks)
scripts/                  test dispatch, Grimoire and Rune vendoring
vendor/                   local Rune and Grimoire checkouts, gitignored
.scratch/                 local specs, issues, worktrees, throwaway output, gitignored
```

The desktop app is not checked in yet. `pnpm vendor` clones Grimoire and grimoire-social side by
side into `vendor/` so their workspace link resolves. Treat those checkouts as upstream until the
fork is created; do not edit them in place without recording the patch.

## Commands

```bash
pnpm install
pnpm typecheck                 # every workspace package
pnpm test                      # typecheck then node --test over packages/*/test
pnpm dah doctor                # game install, gameinfo, addon slots, providers
pnpm dah models                # configured providers and available models
pnpm dah auth set anthropic   # prompts for a key, stores it outside the repo
pnpm dah chat "recolor ..."    # one agent run in the local workspace
pnpm vendor                    # clone Grimoire and grimoire-social into vendor/
pnpm vendor:rune               # clone the pinned Rune revision into vendor/rune
pnpm rust:check                # cargo check the Rust workspace
pnpm rust:app                  # run the Rune desktop workbench
pnpm rust:dah -- doctor        # run the native CLI
pnpm rust:agent               # run the Rust TinyAgent host
```

Node 24 or newer. pnpm 10. Rust 1.99.1 for the native workspace. The repo has no TypeScript build
step yet: it runs through `tsx` or Node type stripping.

## Hard rules

These are product constraints, not preferences.

- Never write to a game install from an agent tool. Tools produce a plan, a staging folder and
  validated output. Installing into `citadel/addons`, editing `gameinfo.gi`, launching the game,
  and publishing are app actions behind an explicit approval gate.
- The desktop UI is native Rust with Rune. Do not introduce an Electron renderer or a browser web
  view as the app shell. See `docs/decisions/0006-native-rust-rune-desktop.md`.
- TinyAgent remains the agent host in a Node sidecar until the Rust implementation reaches protocol
  v2 with durable sessions, pause and resume. The UI never runs agent tools in its render loop.
- A mod payload contains compiled `_c` files only, at exact game paths. Deny `scripts/*.vdata_c`,
  `*.vnmskel_c`, source assets, tool caches and executables. `packages/deadlock-tools/src/validate.ts`
  is the executable version of that rule. Do not weaken it in a prompt.
- Cosmetic only. No gameplay data, no skeleton replacement, no feature that gives an advantage in
  matchmaking.
- BYOK keys live in the Rust app and never enter the UI transcript, tool output, logs or a commit.
  The CLI store is `chmod 600`; the desktop store uses the OS keychain.
- No telemetry, no analytics, no heartbeat. Network calls come from a documented allowlist:
  model provider endpoints, `api.steampowered.com` for the official patch feed, GameBanana, and the
  GitHub update check.
- Preserve MIT attribution for Rune, TinyAgent, Grimoire and every bundled community tool. See
  `THIRD_PARTY_NOTICES.md`. CSDK 12 has no license and is never redistributed.
- No em dashes in code, UI strings, comments or docs. Use a colon, period or parentheses.

## Agent runtime

`rust/crates/agent-host` runs TinyAgent v2 as a local sidecar (`dah-agent`):

- One durable session per mod project. The root session is the general workbench. Sessions are
  JSONL journals under the app data directory.
- Tools call the Rust domain core through `dah-rs`. The payload validator is also implemented in the
  host for pure checks.
- Mark a tool `idempotent: true` only for reads and pure computation. Writes, process launches and
  external calls are not idempotent.
- Approval gates use `beforeTool` and `pause`. The Rust app answers with a resume result. Deploy,
  launch, `gameinfo.gi` repair and publish are gated.
- Providers are OpenAI Responses, Anthropic Messages and OpenAI-compatible Chat Completions. The
  last one is the local-model path.
- Keys come from the app or environment and never enter the journal.
- `apps/agent-host` is the TypeScript reference implementation during the cutover.
  `packages/harness` is the legacy Pi Durable host; do not add features to it. See
  `docs/decisions/0007-tinyagent-host.md`.

Adding a tool:

1. Define it in `rust/crates/agent-host` with a JSON Schema and an explicit `idempotent` value.
2. Return compact model-facing text. Do not put secrets or large binaries in it.
3. If it leaves the mod workspace, route it through the Rust app and pause for approval.
4. Add a contract test in the owning crate or package. Rules belong in code, not only in the prompt.

## Deadlock domain rules

- Engine name is `citadel`. Mods load from `game/citadel/addons`.
- A mod is `pakNN_dir.vpk`, NN from `01` to `99`. Other names and subfolders are ignored. Lower
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
  in sync by reading the TypeScript implementation; run `scripts/vendor-tinyagent.sh` after a
  revision bump.
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
- The desktop app is a Rust binary plus a Node sidecar. Packaging must bundle or locate the sidecar;
  there is no Electron `node_modules` allowlist to lean on.
- Game updates move hero folders and reset `gameinfo.gi`. A "worked yesterday" report is usually a
  path or SearchPaths regression, not an agent bug.
