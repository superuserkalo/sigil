# Development runbook

Status: active.
Updated: 2026-11-16

## Prerequisites

- Node 24 or newer.
- pnpm 10 (`corepack enable` is enough).
- Deadlock installed through Steam for the game checks.
- Optional: Windows with community CSDK 12 for compiling assets.
- Optional: Source 2 Viewer, `vpkeditcli`, `vpkmerge` on PATH for packing and inspection. None are
  required by the scaffold.

## First run

```bash
pnpm install
pnpm typecheck
pnpm test
pnpm dah doctor
```

`pnpm test` runs every package typecheck and then `node --test` over `packages/*/test/*.test.ts`.

## Data locations

The CLI resolves its data directory as:

- macOS: `~/Library/Application Support/DeadlockAgentHarness`
- Windows: `%APPDATA%/DeadlockAgentHarness`
- Linux: `$XDG_DATA_HOME/deadlock-agent-harness`, defaulting to
  `~/.local/share/deadlock-agent-harness`

Contents:

```text
agent-sessions/   TinyAgent session journals, one per conversation
credentials.json  provider keys, chmod 600
workspace/        default mod workspace root
```

Override with `DAH_DATA_DIR` and `DAH_WORKSPACE_DIR`. Use a separate data directory when running the
CLI beside the desktop app: one process owns the TinyAgent session directory.

## Providers

```bash
pnpm dah auth set anthropic
pnpm dah auth list
pnpm dah models
```

The prompt reads the key without echoing it. Keys are stored in `credentials.json` with mode `600`.
Environment variables are a fallback when no credential is stored (`ANTHROPIC_API_KEY`,
`OPENAI_API_KEY`, and so on, depending on the provider).

Never commit `credentials.json` or paste a key into a conversation. The agent does not need to see
it.

## One agent run

```bash
pnpm dah chat "Find the game install, report the addon slots, and validate a payload containing \
models/heroes_wip/punkgoat/punkgoat.vmdl_c"
```

The agent runs in the workspace directory and can read, write, edit and use bash there. Game install
writes are not implemented yet.

## Game introspection

```bash
pnpm dah index
pnpm dah index --refresh
pnpm dah find billy --hero billy --type vmdl_c
pnpm dah find vtex_c --folder panorama/images --limit 20
pnpm dah extract models/heroes_wip/billy/billy.vmdl_c --out extracted
```

`index` parses `game/citadel/pak01_dir.vpk` and caches metadata in
`<workspace>/.dah/index/citadel-pak01.json`. The cache invalidates when the VPK path, size or
modification time changes. `find` searches that cache. `extract` re-reads the VPK, checks the entry
CRC32 and writes under the workspace with the original game paths.

On a machine without Steam discovery, point at the game:

```bash
DAH_GAME_DIR="/path/to/Deadlock" pnpm dah index
DAH_GAME_DIR="/path/to/Deadlock/game/citadel" pnpm dah index
```

The same override works for the agent tools. Tests build synthetic VPKs, so no game install is
needed for the test suite.

Decompiling extracted `_c` files is not implemented yet. Use Source 2 Viewer until a decompile
backend lands.

## Acceptance

One command exercises the tier 5 loop against a disposable fake install. It never reads or writes a
real game directory.

```bash
scripts/acceptance.sh              # offline surfaces
scripts/acceptance.sh --network    # also calls api.deadlock-api.com
```

It builds a project, validates and packs it, seeds a fake game catalog, baselines and releases,
toggles slots, saves and applies a profile, writes autoexec and a crosshair block, records a
character task and a recolor brief, deploys, prepares a publish folder, imports a demo, plans and
applies an install, and checks launch status and the watch state. Artifacts land under
`.scratch/acceptance/<stamp>/`. The script exits with the number of failed checks.

## Native app and agent host

```bash
./scripts/vendor-rune.sh
./scripts/vendor-tinyagent.sh
cargo run --manifest-path rust/Cargo.toml -p dah-desktop
cargo run --manifest-path rust/Cargo.toml -p dah-rs -- doctor
```

Rune is pinned pre-release. The workspace repeats its `[patch.crates-io]` entries; a plain git
dependency fails on the patched text stack. Headless checks:

```bash
cargo run --manifest-path rust/Cargo.toml -p dah-desktop -- --snap .scratch/desktop.png --scene workbench
cargo run --manifest-path rust/Cargo.toml -p dah-desktop -- --tree .scratch/tree.json --scene index
```

The agent host is TinyAgent v2. The Rust host is `rust/crates/agent-host` (`dah-agent`) over
`rust/crates/tinyagent`. It speaks JSONL over stdio:

```bash
cargo run --manifest-path rust/Cargo.toml -p dah-agent
```

The Rune app spawns `dah-agent`. The TypeScript host in `apps/agent-host` is reference only.

Configuration: `DAH_AGENT_PROVIDER` (`openai`, `anthropic` or `chat`), `DAH_AGENT_MODEL`,
`DAH_AGENT_BASE_URL` for a local OpenAI-compatible server, and `DAH_AGENT_API_KEY`. Sessions live
under the app data directory. The Rune app spawns `dah-agent`, which calls `dah-rs` for game and
project operations.

Do not point two hosts at the same session directory.

## Vendoring Grimoire

```bash
pnpm vendor
```

This clones `grimoire` and `grimoire-social` side by side into `vendor/`. Override the owner or ref
when the fork exists:

```bash
GRIMOIRE_OWNER=your-handle GRIMOIRE_REF=main pnpm vendor
```

Grimoire needs pnpm 12 and native module rebuilds:

```bash
cd vendor/grimoire-social && pnpm install --ignore-scripts
cd ../grimoire && corepack pnpm@12 install
corepack pnpm@12 exec electron-rebuild -f -w better-sqlite3
corepack pnpm@12 dev
```

Do not commit `vendor/`. Do not edit it in place without writing down the patch.

## Windows compile toolchain

1. Download community CSDK 12 from the Deadlock Modding Notes page. Use the "Reduced" build.
2. Unzip to a short path such as `C:\Reduced_CSDK_12`. Keep it out of OneDrive and out of the game
   folder.
3. Start Steam, then `csdkcfg.exe`.
4. Create an addon and confirm `game/bin_tools/win64/resourcecompiler.exe` exists.
5. Record the paths in settings when the build backend lands.

The app will never redistribute CSDK 12. It has no license.

## Packing and inspection tools

The pack layer will call external binaries resolved from settings:

- `vpkeditcli` for create, inspect and extract.
- `Source2Viewer-CLI` for decompile, export and VPK create with chunks.
- `vpkmerge` for merging addon slots.

Pin a version and a hash when one is bundled. Otherwise let the user point at an existing install.

## Troubleshooting

**The game does not load any mods.** Check `pnpm dah doctor`: `gameinfo.gi` must list
`Game citadel/addons` above `Game citadel`. A game update or Steam verification resets it.

**`dah index` says the install was not found.** Steam discovery found no library. Set
`DAH_GAME_DIR` to the Deadlock folder, its `game` folder, or `game/citadel`. Run `pnpm dah doctor`
to see which roots were searched.

**The index is stale after a game update.** `pnpm dah index --refresh`, or delete
`<workspace>/.dah/index/`. The cache also invalidates on its own when the VPK size or modification
time changes.

**An extracted file is missing or has the wrong path.** Run `pnpm dah find` with the exact name.
A payload needs the exact game path, so copy the path from the index instead of typing it.

**The mod loads but the change is missing.** The path inside the VPK is wrong, the file is not
compiled, or a lower-numbered mod ships the same path. Inspect the VPK and compare paths with the
game index.

**The agent says no model is configured.** Run `pnpm dah auth set <provider>` or export the provider
key. `pnpm dah models` shows what resolved.

**The storage is locked or stale.** Another process owns it. Close the desktop app or point
`DAH_DATA_DIR` somewhere else.

**The CLI cannot import workspace packages.** Run through `pnpm dah`, which uses `tsx`. Node's type
stripping does not run TypeScript inside `node_modules` symlinks.

**A test wrote outside the repo.** It should not. Tests use `.scratch/tmp/` and clean up after
themselves.
