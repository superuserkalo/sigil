# Rust workspace

The native side of the Deadlock Agent Harness: a Rune desktop app, a domain core and a CLI.

```text
crates/deadlock-core   game discovery, VPK, catalog, project, patches, impact
crates/dah-rs          native CLI
crates/tinyagent       Rust TinyAgent protocol v2 port
crates/agent-host      Rust TinyAgent JSONL host (dah-agent)
crates/desktop         Rune workbench
```

## Build

```bash
../scripts/vendor-rune.sh
cargo check --workspace
cargo run -p dah-desktop
cargo run -p dah-rs -- doctor
cargo run -p dah-agent
```

The toolchain is pinned in `rust-toolchain.toml` to 1.99.1. Rune is pre-release and not on
crates.io. It is vendored at a pinned revision, and `Cargo.toml` repeats Rune's patched crates
because a dependency's `[patch]` does not apply to this workspace.

## Headless UI checks

Rune runs the same renderer without a window:

```bash
cargo run -p dah-desktop -- --snap .scratch/desktop.png --scene workbench
cargo run -p dah-desktop -- --tree .scratch/tree.json --scene index
```

Scenes are listed in `crates/desktop/src/main.rs`.

## Boundary

The agent host is TinyAgent. The Rust host is `rust/crates/agent-host` (`dah-agent`) on top of the
v2 crate. The TypeScript host in `apps/agent-host` is the reference during the cutover. These
crates do not run a browser renderer. The desktop app owns the window, game discovery, VPK work,
approvals UI and game writes. See `docs/decisions/0006-native-rust-rune-desktop.md` and
`docs/decisions/0007-tinyagent-host.md`.
