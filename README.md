# Deadlock Agent Harness

An open-source desktop workbench where an AI agent builds, packs, validates and installs Deadlock
mods. Model access is bring-your-own-key. The app keeps your code, your game install and your
credentials on your machine.

Status: pre-alpha. The native release loop works: validate, pack, versioned release, deploy plan
and approved deploy, path migration, cross-project patch watch and GameBanana handoff. The Rune
desktop app, Rust CLI and TinyAgent v2 host are connected and the mod manager is usable against a
game install: installed slots toggle, profiles capture and apply, autoexec edits behind approval,
Browse downloads and installs with a validated plan, and Launch handles modded, vanilla and restore.

Stats read the public Deadlock API and local demos, including a real kill and death heatmap and a
coaching review whose numbers are linted against the verified facts. The Locker resolves hero art
from a local root. Character generation runs a studied stage graph: deterministic stages work with
no key, generative stages need a configured backend, and nothing claims to be in game.

Still blocked on hardware and accounts: real in-game deploy, a Windows CSDK compile, real
`resourcecompiler.exe` runs, a real Meshy or local model backend, and 7z or rar archives when those
tools are absent. Direct GameBanana API upload stays out of scope by decision.

## Native desktop

The desktop shell is native Rust with [Rune](https://github.com/anomalyco/rune), not Electron. The
domain core lives in `rust/crates/deadlock-core`, the CLI in `rust/crates/dah-rs`, and the app in
`rust/crates/desktop`.

```bash
./scripts/vendor-rune.sh      # pinned Rune checkout for the Rust build
./scripts/vendor-tinyagent.sh # pinned TinyAgent SDK for the agent host
cargo run --manifest-path rust/Cargo.toml -p dah-desktop
cargo run --manifest-path rust/Cargo.toml -p dah-rs -- doctor
cargo run --manifest-path rust/Cargo.toml -p dah-agent  # Rust TinyAgent host
```

Rune is pre-release. The revision is pinned and its patched crates are vendored. See
[ADR 0006](docs/decisions/0006-native-rust-rune-desktop.md).

The release loop is native:

```bash
pnpm rust:dah -- mod validate --workspace <project>
pnpm rust:dah -- mod release --workspace <project> --slot 6
pnpm rust:dah -- mod deploy --workspace <project> --slot 6          # plan
pnpm rust:dah -- mod deploy --workspace <project> --slot 6 --approve
pnpm rust:dah -- vpk inspect <project>/builds/<version>/pak06_dir.vpk --verify
pnpm rust:dah -- publish prepare --workspace <project>
pnpm rust:dah -- publish upload --workspace <project> --approve --open
pnpm rust:dah -- publish confirm --workspace <project> --url https://gamebanana.com/mods/<id>
pnpm rust:dah -- publish status --workspace <project>
pnpm rust:dah -- watch                 # add --refresh for patches, --watch for a loop
```

The deploy plan reports slot conflicts, replacement and `gameinfo.gi` repair before anything is
written. Approved deploy backs up replaced files. Publish prepares a ZIP, records the approved
handoff, opens the GameBanana submission page, then records and verifies the submission URL. It
never handles your account or uploads files.

The agent host is TinyAgent. The Rust app owns the window, game discovery, VPK work, approvals UI and
game writes. TinyAgent owns durable sessions, pause and resume, provider access and tools. There is
no browser renderer. The Rust protocol v2 crate is landed; the Rust host is being completed, and the
TypeScript host remains the reference during the cutover. See
[ADR 0007](docs/decisions/0007-tinyagent-host.md).

A Windows x64 bundle is built with `scripts/package-windows.sh` (cargo-xwin) or
the GitHub Actions workflow. It contains the app, the sidecar and the CLI. See
[docs/runbooks/windows.md](docs/runbooks/windows.md).

One command proves the loop against a disposable fake install:

```bash
scripts/acceptance.sh
```

Subscription sign-in works for Codex and Grok, with the first-party OAuth flows
documented from the Apache-2.0 fx source. Use `dah auth login codex`, or the
Settings panel, then `dah auth use codex --model <id>`. Tokens live in the app
data auth folder, never in a transcript. See
[docs/research/subscription-auth.md](docs/research/subscription-auth.md).

## Where it stands

| Tier | State | Evidence |
| --- | --- | --- |
| 0 Inspect | Works | `dah index`, `find`, `extract` against a real install |
| 1 Asset swap | Pack, validate, release, deploy work; decompile and compile need external tools | Acceptance run and deploy backup |
| 2 Material and effect | Planned | Not claimed |
| 3 Character | Stage graph and reference checks; no compile | `dah character stages` |
| 4 Multi-asset | Batch watch, versioned releases | `dah watch` |
| 5 Release and survive updates | Works | `scripts/acceptance.sh`: 44 checks pass |
| 6 Living workbench | Works | Watch state, stats, demos, linted review, crosshair, profiles, installs |

Not verified on this machine: a Windows install and launch, a real CSDK compile, a real Meshy or local
model, positions from a real `.dem`, 7z and rar archives, and signed bundles. The Deadworks server
browser is not shipped because it needs a third-party relay and content sync; see
[docs/design/mod-manager-shell.md](docs/design/mod-manager-shell.md).

## The manager

The default screen is Installed, because this is a mod manager with an agent inside, not an agent
demo. Sidebar groups:

- Library: Installed, Browse, Conflicts, Updates.
- Modding: Profiles, Autoexec, Stats, Locker, Launch.
- Workbench: Agent, Projects, Game Index, Patches, Impact, Release.
- Foundry: Toolchain, Character.

A screen ships only when core can back it with real data or a gated action. When a backend is
missing, the screen names it, says why, and keeps every action that does not need it.

```bash
pnpm rust:dah -- addons list                  # installed slots and shared paths
pnpm rust:dah -- addons set 6 off --approve   # one rename, behind approval
pnpm rust:dah -- profiles save "Ranked"       # capture the current slot set
pnpm rust:dah -- crosshair show               # reticle convars and preview geometry
pnpm rust:dah -- install plan 715100          # download, extract, validate, plan a slot
pnpm rust:dah -- install apply <plan> --approve
pnpm rust:dah -- launch modded                # or vanilla, restore, status
pnpm rust:dah -- locker list                  # heroes, Grimoire art and catalog paths
pnpm rust:dah -- locker recolor billy --palette frost   # a recorded work order
pnpm rust:dah -- stats player <account-id>    # rank, career, heroes, form
pnpm rust:dah -- stats heatmap <account-id>   # real kill and death cells
pnpm rust:dah -- demo import match.dem        # local store and merged summary
pnpm rust:dah -- demo review <id> --file r.md --write   # linted coaching review
pnpm rust:dah -- demo review <id> --model local --base-url http://127.0.0.1:8080/v1 --write
pnpm rust:dah -- toolchain status             # what can decompile and compile
pnpm rust:dah -- auth login codex             # ChatGPT subscription sign-in
pnpm rust:dah -- auth use codex --model <id>  # point the host at it
pnpm rust:dah -- character stages             # the generation stage graph
pnpm rust:dah -- character backends           # Blender code, local models, Meshy
pnpm rust:dah -- character scaffold billy     # a Blender script an LLM can edit
```

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
Steam discovery, addon slots, `gameinfo.gi`, VPK merging and previews already have a home. The agent
host is [TinyAgent](https://github.com/superuserkalo/tinyagent).

## Architecture at a glance

```text
rust/crates/desktop       native Rune workbench (Rust)
rust/crates/deadlock-core game, VPK, index, project, patch and impact logic (Rust)
rust/crates/dah-rs        native CLI (Rust)
rust/crates/tinyagent     Rust TinyAgent v2 port
rust/crates/agent-host    Rust TinyAgent JSONL host (dah-agent)
apps/agent-host           TinyAgent v2 local agent host (Node sidecar)
apps/cli                  legacy TypeScript CLI during the port
packages/harness          legacy Pi Durable host, kept for reference during the port
packages/deadlock-tools   TypeScript domain core and tools during the port
docs/                     tracked project docs
vendor/rune               pinned Rune checkout (gitignored, scripts/vendor-rune.sh)
vendor/tinyagent          pinned TinyAgent checkout (gitignored, scripts/vendor-tinyagent.sh)
vendor/                   local Grimoire checkouts for domain reference (gitignored)
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
