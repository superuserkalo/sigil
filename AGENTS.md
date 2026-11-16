# Deadlock Agent Harness

Desktop workbench where an AI coding agent builds, packs, validates and installs Deadlock mods.
The shell is a fork of Grimoire (MIT). The agent runtime is Pi Durable. Model access is BYOK.

Status: pre-alpha scaffold. The agent runtime, CLI, game discovery and payload validation exist.
Nothing writes to a game install yet.

## Read first

1. `CONTEXT.md` for the product vocabulary. Use those words in code, docs and UI.
2. `docs/architecture.md` for boundaries, tool catalog, safety model and roadmap.
3. `docs/research/deadlock-modding.md` for how mods actually load. This is the domain ground truth.
4. `docs/research/pi-durable.md` for the runtime this is built on.
5. `docs/decisions/` before changing a boundary that is hard to reverse.

## Repository layout

```text
apps/cli/                 dah command: doctor, models, auth, chat
packages/harness/         Pi Durable session, BYOK catalog, credential store
packages/deadlock-tools/  game discovery, VPK payload rules, agent tools
docs/                     tracked project docs (architecture, decisions, research, runbooks)
scripts/                  test dispatch and Grimoire vendoring
vendor/                   local Grimoire checkouts, gitignored
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
```

Node 24 or newer. pnpm 10. The repo has no build step yet: TypeScript runs through `tsx` or Node
type stripping.

## Hard rules

These are product constraints, not preferences.

- Never write to a game install from an agent tool. Tools produce a plan, a staging folder and
  validated output. Installing into `citadel/addons`, editing `gameinfo.gi`, launching the game,
  and publishing are main-process actions behind an explicit approval gate.
- A mod payload contains compiled `_c` files only, at exact game paths. Deny `scripts/*.vdata_c`,
  `*.vnmskel_c`, source assets, tool caches and executables. `packages/deadlock-tools/src/validate.ts`
  is the executable version of that rule. Do not weaken it in a prompt.
- Cosmetic only. No gameplay data, no skeleton replacement, no feature that gives an advantage in
  matchmaking.
- BYOK keys live in the main process. They never enter the renderer, the transcript, tool output,
  logs or a commit. The CLI store is `chmod 600`; the desktop store is wrapped with Electron
  `safeStorage`.
- No telemetry, no analytics, no heartbeat. Network calls come from a documented allowlist:
  model provider endpoints, GameBanana, and the GitHub update check.
- Preserve MIT attribution for Grimoire and every bundled community tool. See
  `THIRD_PARTY_NOTICES.md`. CSDK 12 has no license and is never redistributed.
- No em dashes in Grimoire-derived code, UI strings, comments or docs. Use a colon, period or
  parentheses.

## Agent runtime

`packages/harness` maps Pi Durable to this product:

- Harness over one SQLite storage. One process owns it.
- One conversation per mod project. The root conversation is the general workbench.
- Extensions installed in a registry: `coding` (read, write, edit, bash), `deadlock` (game status,
  payload validation, prompt rules), later `plan`, `approval`, `knowledge`.
- Every tool call is a durable task. `replay: "safe"` is allowed only for reads and pure
  computation. Anything with external effects stays `unsafe` and gets an approval gate.
- Per-conversation environment rooted at the mod workspace. A remote or Windows environment is a
  later implementation detail behind the same `env` hook.
- Credentials go through a pi-ai `CredentialStore`; `getAuth` owns provider resolution.

Adding a tool:

1. Define it with `defineTool` in the owning package.
2. Choose `replay` deliberately. Reads and pure functions are safe; writes and process launches are
   not.
3. Return model-facing `output` text. Do not stuff secrets or huge binary data into it.
4. If it mutates anything outside the workspace, route it through the approval gate and the main
   process.
5. Add a test under `packages/*/test`. Contract rules belong in code, not only in the system prompt.

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

- Pi Durable is pinned to `1.5.2` and is marked experimental. Read installed types before using a
  new API; the README can drift from the published package.
- One process owns a Pi Durable storage. Do not open the same SQLite file from the CLI and the
  desktop app at once.
- `CodingTools` includes `bash`. In the desktop app it must be wrapped with a workspace boundary and
  an executable allowlist. The CLI may keep the plain tool for now.
- External binaries are resolved from settings, never downloaded on demand from an arbitrary URL.
  Pin a version and a hash when one is added.
- Electron packaging strips `node_modules` except an allowlist. Any new main-process runtime
  dependency must be bundled or the packaged app crashes on launch. This is a known Grimoire
  failure mode.
- Game updates move hero folders and reset `gameinfo.gi`. A "worked yesterday" report is usually a
  path or SearchPaths regression, not an agent bug.
