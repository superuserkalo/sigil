# Packaging the workbench

Status: draft. Verified on macOS only. Windows is not verified.
Updated: 2026-11-16

The desktop app is a Rust binary plus a Node-free Rust sidecar (`dah-agent`) and the `dah` CLI.
There is no Electron `node_modules` allowlist and no browser shell. Packaging has to carry three
binaries and nothing else.

## What ships

| Artifact | Built from | Purpose |
| --- | --- | --- |
| `dah-desktop` | `rust/crates/desktop` | Native Rune workbench |
| `dah-agent` | `rust/crates/agent-host` | TinyAgent v2 JSONL sidecar |
| `dah` | `rust/crates/dah-rs` | CLI: doctor, index, release, deploy, stats, demo, install |
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
`Deadlock Mod Manager.app` with an `Info.plist` and the notices under `Resources`.

Output: `.scratch/package/<target>-<version>/` plus a zip beside it. The Windows
script stages `Deadlock Mod Manager.exe`, `dah-agent.exe`, `dah.exe`, the notices
and a first-run README into
`.scratch/package/DeadlockModManager-<version>-windows-x64.zip`. See
`docs/runbooks/windows.md`.

## Sidecar resolution

The desktop starts `dah-agent` from:

1. `DAH_AGENT_BIN`, when set.
2. The folder that holds the desktop binary.
3. `PATH`.

One process owns a session directory. Do not point two hosts at the same journals.

## Verification checklist

Run these on the machine that will use the build. The list is the honest state, not a claim:

| Step | macOS | Windows |
| --- | --- | --- |
| Desktop opens, sidebar renders | verified | not verified |
| Agent sidecar connects, provider set | verified headless and local SSE mock | not verified |
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

No signing identity is configured. macOS will warn on an unsigned bundle. The update check reads the
GitHub release feed only; it never downloads or executes an update automatically.

## Uninstall

Removing the bundle leaves the data directory:

- macOS: `~/Library/Application Support/DeadlockAgentHarness`
- Windows: `%APPDATA%\DeadlockAgentHarness`
- Linux: `$XDG_DATA_HOME/DeadlockAgentHarness`

It holds sessions, the workspace, cached patch notes and the vanilla stash. A pending vanilla stash
means the game's addon folder is empty: restore before deleting the data directory.
