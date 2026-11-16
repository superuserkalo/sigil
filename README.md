# Deadlock Agent Harness

An open-source desktop workbench where an AI agent builds, packs, validates and installs Deadlock
mods. Model access is bring-your-own-key. The app keeps your code, your game install and your
credentials on your machine.

Status: pre-alpha scaffold. Game discovery, payload validation, the agent runtime and a CLI exist.
Deploying to a game install and the desktop UI are not implemented yet.

## What it should do

You describe a mod. The agent:

1. Finds the Deadlock install and the exact game path for the file you care about.
2. Extracts and decompiles that file into a mod workspace.
3. Edits it with normal coding tools (images, audio, materials, UI, models).
4. Compiles it with the community Windows toolchain when one is available.
5. Packs a `pakNN_dir.vpk` with cross-platform tools.
6. Validates the payload against the community rules before anything touches the game.
7. Installs it into a test slot and launches the game, after you approve.
8. Prepares a release for GameBanana, after you approve.

The shell is a fork of [Grimoire](https://github.com/Slush97/grimoire), a Deadlock mod manager, so
Steam discovery, addon slots, `gameinfo.gi`, VPK merging and previews already have a home. The
agent runtime is [Pi Durable](https://github.com/earendil-works/pi/tree/main/packages/durable).

## Architecture at a glance

```text
apps/cli                 dah doctor / models / auth / chat
packages/harness         Pi Durable session, BYOK catalog, credential store
packages/deadlock-tools  Steam discovery, gameinfo, addon slots, payload validation, tools
docs/                    tracked project docs
vendor/                  local Grimoire checkouts for the future desktop app (gitignored)
```

Read [docs/architecture.md](docs/architecture.md) before changing a boundary:
process split, storage, build backend, safety gate or model access.

## Requirements

- Node 24 or newer, pnpm 10.
- Deadlock on Steam for the `doctor` game checks.
- A Windows machine with community CSDK 12 for compiling assets. Packing, inspecting, merging and
  validating work on macOS and Linux.

## Quick start

```bash
pnpm install
pnpm typecheck
pnpm test
pnpm dah doctor
pnpm dah auth set anthropic
pnpm dah chat "find Billy's model and explain how I would recolor it"
```

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
