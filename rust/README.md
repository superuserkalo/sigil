# Rust workspace

The whole of Sigil: the Rune desktop app, the domain core, the CLI, the agent host and the 3D
viewer. The app is one executable that also runs as `Sigil cli` and `Sigil agent-host`.

```text
crates/deadlock-core   domain core: discovery, VPK, installs, load order, launch, catalog,
                       crosshair, stats, servers, telemetry client
crates/dah-rs          native CLI (folded into the app as `cli`)
crates/tinyagent       Rust TinyAgent protocol v2 port
crates/agent-host      Rust TinyAgent JSONL host (dah-agent; folded in as `agent-host`)
crates/desktop         the Sigil app (dah-desktop, Rune)
crates/sigil-viewer    3D viewer (wgpu)
```

## Build

```bash
../scripts/vendor-rune.sh
cargo check --workspace
cargo run -p dah-desktop
cargo run -p dah-rs -- doctor
cargo run -p dah-agent
cargo test -p deadlock-core -p dah-desktop
```

The toolchain is pinned in `rust-toolchain.toml` to 1.99.1. Rune is pre-release and not on
crates.io. It is vendored at a pinned revision, and `Cargo.toml` repeats Rune's patched crates
because a dependency's `[patch]` does not apply to this workspace.

## Headless UI checks

Rune runs the same renderer without a window:

```bash
cargo run -p dah-desktop -- --snap .scratch/desktop.png --scene installed
cargo run -p dah-desktop -- --tree .scratch/tree.json --scene index
cargo run -p dah-desktop -- --list-scenes
```

Scenes are registered in `crates/desktop/src/main.rs`.

## Boundary

The agent host is TinyAgent. The Rust host is `rust/crates/agent-host` (`dah-agent`) on top of the
v2 crate, run as `Sigil agent-host`. These crates do not run a browser renderer. The desktop app owns the window, game discovery, VPK work,
approvals UI and game writes. See `docs/decisions/0006-native-rust-rune-desktop.md` and
`docs/decisions/0007-tinyagent-host.md`.
