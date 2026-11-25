# Development runbook

Status: active.
Updated: 2026-11-25

## Prerequisites

- git and Rust through rustup. `rust/rust-toolchain.toml` pins 1.99.1 with rustfmt and clippy; it
  installs on the first build.
- Node 24 for the release scripts and the Worker in `services/telemetry` (npm). pnpm 10 is optional:
  the root `package.json` only holds the `pnpm vendor*` and `pnpm rust:*` shortcuts.
- Linux: the system packages the app links (Wayland, ALSA, OpenSSL and friends).
  `scripts/package-linux.sh --deps` installs them on Debian and Ubuntu.
- Optional: Deadlock through Steam for real game checks. Tests never need it.
- Optional: `cargo-xwin` and `makensis` for Windows builds, Windows
  with community CSDK 12 for the agent's compile step.

## First run

```bash
./scripts/vendor-rune.sh                                    # or: pnpm vendor:rune
cargo run --manifest-path rust/Cargo.toml -p dah-desktop    # or: pnpm rust:app
```

Rune is pinned pre-release. The workspace repeats its `[patch.crates-io]` entries in
`rust/Cargo.toml`; a plain git dependency fails on the patched text stack.

The Locker's hero art is in `rust/crates/desktop/assets/locker` and is always built in.

Without it the build embeds no art and the app falls back to art it finds on disk.

## Tests

```bash
cd rust && cargo test -p deadlock-core -p dah-desktop       # the domain core and the app
cd rust && cargo test -p sigil-viewer                        # when the 3D viewer changed
cd rust && cargo test --workspace                            # everything, as a release does
cd services/telemetry && npm ci && npm test                  # the Worker (vitest, local workerd)
```

Tests build synthetic VPKs and fake install trees under `.scratch/`; no game install is needed.
CI (`.github/workflows/ci.yml`) runs the domain core, the app and the Worker tests on every pull
request and every push to main.

## Headless UI checks

Rune runs the same renderer without a window:

```bash
cargo run --manifest-path rust/Cargo.toml -p dah-desktop -- --snap .scratch/desktop.png --scene installed
cargo run --manifest-path rust/Cargo.toml -p dah-desktop -- --tree .scratch/tree.json --scene browse
```

Scenes are registered in `rust/crates/desktop/src/app.rs` and the `*_scenes.rs` files.

## Data locations

The app and the CLI share one data directory:

- macOS: `~/Library/Application Support/DeadlockAgentHarness`
- Windows: `%APPDATA%\DeadlockAgentHarness`
- Linux: `$XDG_DATA_HOME/DeadlockAgentHarness`, defaulting to `~/.local/share/DeadlockAgentHarness`

It holds settings, backups, the trash, caches, the vanilla stash, `auth/` sign-in tokens, the agent
workspace and `agent-sessions/`. Model keys and the proxy password are in the OS keychain. Override
the folders with `DAH_DATA_DIR` and `DAH_WORKSPACE_DIR`. Use a separate data directory when running
the CLI or a second app beside your usual one: one process owns the TinyAgent session directory.

## The CLI

The CLI is `rust/crates/dah-rs`, also shipped inside the app as `Sigil cli`.

```bash
pnpm rust:dah -- doctor                      # game install, gameinfo.gi, addon slots
pnpm rust:dah -- index                       # build the game catalog
pnpm rust:dah -- find billy --hero billy --type vmdl_c
pnpm rust:dah -- extract models/heroes_wip/billy/billy.vmdl_c --out extracted
pnpm rust:dah -- addons list                 # installed slots and shared paths
pnpm rust:dah -- --help                      # every command
```

On a machine without Steam discovery, point at the game with `DAH_GAME_DIR` (the Deadlock folder,
its `game` folder or `game/citadel`). `index` caches metadata under `<workspace>/.dah/index/`; the
cache invalidates when the VPK path, size or modification time changes, and is safe to delete.

## The agent host

The agent is behind Settings > Experimental > Agent. Its host is TinyAgent v2 in Rust
(`rust/crates/agent-host` over `rust/crates/tinyagent`), started by the app as `Sigil agent-host`.
It speaks JSONL over stdio and can run alone:

```bash
cargo run --manifest-path rust/Cargo.toml -p dah-agent     # or: pnpm rust:agent
```

Configuration: `DAH_AGENT_PROVIDER` (`openai`, `anthropic`, `chat`, `codex` or `grok`),
`DAH_AGENT_MODEL`, `DAH_AGENT_BASE_URL` for a local OpenAI-compatible server, and
`DAH_AGENT_API_KEY`. Tools call the CLI with argument arrays; there is no general shell tool. Do not
point two hosts at the same session directory.

The Rust port follows the upstream TypeScript TinyAgent, which `scripts/vendor-tinyagent.sh` checks
out into `vendor/tinyagent` as a reference; nothing builds or runs it.

## Windows compile toolchain (agent workbench)

1. Download community CSDK 12 from the Deadlock Modding Notes page. Use the "Reduced" build.
2. Unzip to a short path such as `C:\Reduced_CSDK_12`, out of OneDrive and the game folder.
3. Start Steam, then `csdkcfg.exe`.
4. Create an addon and confirm `game/bin_tools/win64/resourcecompiler.exe` exists.
5. Point Sigil at it from the Toolchain page.

Sigil never redistributes CSDK 12. It has no license.

## Troubleshooting

**The game does not load any mods.** `gameinfo.gi` must list `Game citadel/addons` above
`Game citadel`. A game update or Steam file verification resets it; the app's banner fixes it, and
`pnpm rust:dah -- doctor` reports it.

**The install was not found.** Steam discovery found no library. Set `DAH_GAME_DIR`.

**The index is stale after a game update.** `pnpm rust:dah -- index --refresh`, or delete
`<workspace>/.dah/index/`.

**The mod loads but the change is missing.** The path inside the VPK is wrong, the file is not
compiled, or a lower-numbered mod ships the same path. Check Overlap, or inspect the VPK with
`pnpm rust:dah -- vpk inspect <file>` and compare paths with the game index.

**The build fails in `vendor/rune`.** Run `./scripts/vendor-rune.sh` again; the pinned revision
may have changed.

**The storage is locked.** Another Sigil owns the data directory. Close it or set `DAH_DATA_DIR`.

**A test wrote outside the repo.** It should not. Tests use `.scratch/` and clean up after
themselves.
