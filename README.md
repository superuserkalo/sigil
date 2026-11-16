# Deadlock Agent Harness

An open-source desktop workbench where an AI agent builds, packs, validates and installs Deadlock
mods. Model access is bring-your-own-key. The app keeps your code, your game install and your
credentials on your machine.

Status: pre-alpha. Phase 1 game introspection is in place: the game VPK index, path search and
workspace extraction, plus the CLI commands and agent tools for them. Decompiling, packing,
deploying to a game install and the desktop UI are not implemented yet.

## What it should do

The workbench scales from a texture swap to a character built from a reference photo. Every tier
uses the same loop: search the game, extract, edit, build, validate, deploy with approval. The
capability ladder and current status are in [docs/architecture.md](docs/architecture.md).

A typical character request:

1. The agent searches the game index for the hero's model, materials and textures.
2. It extracts the compiled files into the mod workspace and decompiles the ones it needs.
3. It edits or generates assets, keeping the game skeleton and animations intact.
4. It compiles them with the community Windows toolchain when one is available.
5. It packs a `pakNN_dir.vpk`, validates the payload, and shows the plan.
6. It installs into a test slot and launches the game, after you approve.
7. It versions the project locally and prepares a GameBanana release, after you approve.

Reference images and likeness work follow
[ADR 0005](docs/decisions/0005-reference-images-and-likeness.md): the image stays local, the rights
basis is recorded, and publishing a real person's likeness needs a separate approval.

The shell is a fork of [Grimoire](https://github.com/Slush97/grimoire), a Deadlock mod manager, so
Steam discovery, addon slots, `gameinfo.gi`, VPK merging and previews already have a home. The
agent runtime is [Pi Durable](https://github.com/earendil-works/pi/tree/main/packages/durable).

## Architecture at a glance

```text
apps/cli                 dah doctor / index / find / extract / models / auth / chat
packages/harness         Pi Durable session, BYOK catalog, credential store
packages/deadlock-tools  Steam discovery, VPK index and extraction, payload validation, tools
docs/                    tracked project docs
vendor/                  local Grimoire checkouts for the future desktop app (gitignored)
```

Read [docs/architecture.md](docs/architecture.md) before changing a boundary:
process split, storage, build backend, safety gate or model access.

## Requirements

- Node 24 or newer, pnpm 10.
- Deadlock on Steam for the `doctor`, `index`, `find` and `extract` game checks.
- A Windows machine with community CSDK 12 for compiling assets. Packing, inspecting, merging and
  validating work on macOS and Linux.

## Quick start

```bash
pnpm install
pnpm typecheck
pnpm test
pnpm dah doctor
pnpm dah index
pnpm dah find billy --hero billy --type vmdl_c
pnpm dah extract models/heroes_wip/billy/billy.vmdl_c --out extracted
pnpm dah auth set anthropic
pnpm dah chat "find Billy's model and explain how I would recolor it"
```

`dah index`, `dah find` and `dah extract` use Steam discovery. Set `DAH_GAME_DIR` to a Deadlock
game folder, its `game` folder, or `game/citadel` for a non-Steam install or a fixture. The index
cache lives in `<workspace>/.dah/index/` and is safe to delete.

The CLI stores its SQLite database, credentials and mod workspaces under the OS application data
directory. Override with `DAH_DATA_DIR` and `DAH_WORKSPACE_DIR`.

## Safety

A mod must stay cosmetic. The payload validator rejects gameplay data (`scripts/*.vdata_c`),
skeletons (`.vnmskel_c`), executables, source assets and tool caches. Nothing writes to a game
install from an agent tool: deploy, `gameinfo.gi` edits, game launch and publishing are
main-process actions behind an approval gate. See [docs/architecture.md](docs/architecture.md) and
[SECURITY.md](SECURITY.md).

## Docs

Start at [docs/README.md](docs/README.md). The vocabulary is in [CONTEXT.md](CONTEXT.md). Agent
operating rules are in [AGENTS.md](AGENTS.md).

## License and attribution

MIT. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The desktop shell
will be a fork of Grimoire, Copyright (c) 2025 esoc, MIT. Community tool licenses are preserved
with their binaries; CSDK 12 has no license and is never redistributed.
