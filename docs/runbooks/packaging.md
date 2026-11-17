# Packaging the workbench

Status: draft. Verified on macOS only. Windows is not verified.
Updated: 2026-11-17

The desktop app is one Rust executable. The `dah` CLI and the TinyAgent host are folded into it as
subcommands: `dah-desktop cli ...` (`Sigil.exe cli ...` on Windows) and `dah-desktop agent-host`.
The host still runs as its own child process with the same stdin and stdout JSONL protocol; only
the file is shared. There is no Electron `node_modules` allowlist and no browser shell. Packaging
carries one binary and its art.

## What ships

| Artifact | Built from | Purpose |
| --- | --- | --- |
| `dah-desktop` (`Sigil.exe`) | `rust/crates/desktop`, linking `rust/crates/dah-rs` and `rust/crates/agent-host` as libraries | Native Rune workbench; `cli` runs the CLI, `agent-host` the TinyAgent v2 JSONL host |
| `THIRD_PARTY_NOTICES.md` | repo root | MIT attribution for Rune, TinyAgent, Grimoire and tools |
| `manifest.json` | the script | Byte counts, SHA-256, included and excluded material |

Never included: CSDK 12, game files, VPKs, `vendor/` checkouts, API keys, journals, logs, SQLite
stores, downloaded mods.

## Build

```bash
scripts/package-desktop.sh                 # host target
scripts/package-windows.sh                 # Windows x64 via cargo-xwin
```

The script refuses nothing silently: a missing binary fails the stage step, and the manifest records
the target, the host, the timestamp and the itemised exclusions. On macOS it also assembles
`Sigil.app` with an `Info.plist`, the notices and the Locker art under `Resources`.

Output: `.scratch/package/<target>-<version>/` plus a zip beside it. The Windows
script packs the Locker art into `.scratch/locker-art`, builds it into the executable
(`SIGIL_ART_DIR`, read by `rust/crates/desktop/build.rs`) and zips `Sigil.exe` alone into
`.scratch/package/Sigil-<version>-windows-x64.zip`. The first launch of a build unpacks its art
once to `<app data>/art/<build id>` (`rust/crates/desktop/src/art.rs`); the notices are in
Settings. See
`docs/runbooks/windows.md`.

## Helper resolution

The desktop starts the agent host from `DAH_AGENT_BIN` when set, else its own executable with
`agent-host`. It runs the CLI (publish, sign-in) from `DAH_RS_BIN` when set, else its own
executable with `cli`. A host started as `agent-host` runs its tools the same way: `DAH_RS_BIN`,
else the same executable with `cli`. The standalone `dah` and `dah-agent` binaries still build
(`cargo build -p dah-rs -p dah-agent`) for development and the CLI docs; the standalone host keeps
its old search (a sibling `dah-rs` or `dah`, the repo debug build, then `PATH`).

One process owns a session directory. Do not point two hosts at the same journals.

## Verification checklist

Run these on the machine that will use the build. The list is the honest state, not a claim:

| Step | macOS | Windows |
| --- | --- | --- |
| Desktop opens, sidebar renders | verified | not verified |
| Agent sidecar connects, provider set | verified headless and local SSE mock | not verified |
| `cli` and `agent-host` subcommands of the one executable, host tool calls run `<exe> cli` | verified: `cli doctor`, and `agent-host` with a mock chat server running `game_status` | not verified (bundle builds and passes `verify-windows.sh`) |
| Game discovery finds the install | not verified, no install on this machine | not verified |
| Addon scan, profiles, autoexec | verified against a fake install tree | not verified |
| Launch modded and vanilla | not verified, no Steam install here | not verified |
| Deploy into `citadel/addons` | verified against a fake install tree | not verified |
| `gameinfo.gi` repair | verified against a fake install tree | not verified |
| Real `resourcecompiler.exe` compile | unavailable, no CSDK 12 | not verified |
| Real `.dem` import | container parser tested on synthetic files | not verified against a live demo |
| Browse install pipeline | local HTTP fixture | not verified against GameBanana |

A build that fails a row above is not a release. Mark the row, do not skip it.

## Signing and updates

No code-signing identity is configured. macOS will warn on an unsigned bundle and SmartScreen on
the exe. Updates are signed separately: both packaging scripts also write an unsigned
`.scratch/package/release-<version>/sigil-update.json` (the Windows zip with `assets: true`; on a
Mac, `Sigil-<version>-macos-<arch>.zip` with `Sigil.app` at the top). Sign it with
`scripts/sign-release.sh` before publishing. Sigil downloads and verifies an update when a check
finds one and installs it only on "Restart to update" or the next launch. See
`docs/runbooks/releasing.md`.

## Uninstall

Removing the bundle leaves the data directory:

- macOS: `~/Library/Application Support/DeadlockAgentHarness`
- Windows: `%APPDATA%\DeadlockAgentHarness`
- Linux: `$XDG_DATA_HOME/DeadlockAgentHarness`

It holds sessions, the workspace, cached patch notes and the vanilla stash. A pending vanilla stash
means the game's addon folder is empty: restore before deleting the data directory.
