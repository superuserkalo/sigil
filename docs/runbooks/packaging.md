# Packaging the workbench

Status: active. Windows is the released platform: `Sigil.exe` in a zip and a per-user NSIS
installer, both built on macOS. The checklist below says what has been run where. macOS and Linux
packages build but are not released.
Updated: 2026-11-25

The desktop app is one Rust executable. The `dah` CLI and the TinyAgent host are folded into it as
subcommands: `dah-desktop cli ...` (`Sigil.exe cli ...` on Windows) and `dah-desktop agent-host`.
The host still runs as its own child process with the same stdin and stdout JSONL protocol; only
the file is shared. There is no Electron `node_modules` allowlist and no browser shell. Packaging
carries one binary and its art.

## What ships

| Artifact | Built from | Purpose |
| --- | --- | --- |
| `dah-desktop` (`Sigil.exe`) | `rust/crates/desktop`, linking `rust/crates/dah-rs` and `rust/crates/agent-host` as libraries | Native Rune workbench; `cli` runs the CLI, `agent-host` the TinyAgent v2 JSONL host |
| `Sigil-<version>-windows-x64.zip` | `scripts/package-windows.sh` | `Sigil.exe` alone; the update asset and the no-installer download |
| `Sigil-Setup.exe` | `scripts/package-installer.sh` and `scripts/installer/sigil.nsi` | Per-user Windows installer (below) |
| `Sigil-<version>-x86_64.AppImage` | `scripts/package-linux.sh` | Linux workbench; updates itself from the signed manifest |
| `sigil_<version>_amd64.deb` | `scripts/package-linux.sh` | Debian/Ubuntu install (`/usr/lib/sigil/sigil`); updated by hand |
| `THIRD_PARTY_NOTICES.md` | repo root | MIT attribution for Rune, TinyAgent, Grimoire and tools |
| `manifest.json` | the script | Byte counts, SHA-256, included and excluded material |

Never included: CSDK 12, game files, VPKs, `vendor/` checkouts, API keys, journals, logs, SQLite
stores, downloaded mods.

## Build

```bash
scripts/package-desktop.sh                 # host target
scripts/package-windows.sh                 # Windows x64 via cargo-xwin
scripts/package-installer.sh <Sigil.exe> <version> <out>/Sigil-Setup.exe   # NSIS installer
scripts/package-linux.sh --deps            # Linux x64: install the build packages first (sudo)
scripts/package-linux.sh                   # Linux x64: AppImage, .deb, unsigned manifest entry
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

### Windows installer

`scripts/package-installer.sh` wraps a packaged `Sigil.exe` with NSIS (`makensis`, from Homebrew on
macOS or the distro on Linux), using `scripts/installer/sigil.nsi` and the app icon. The installer:

- installs per user, with no administrator prompt, into `%LOCALAPPDATA%\Programs\Sigil`, where
  Sigil's own updater can later replace `Sigil.exe`;
- adds Start menu and desktop shortcuts and an entry in Windows Settings > Apps, with an uninstaller
  that removes the program, the updater's `Sigil.exe.old` and `assets.old`, and the shortcuts;
- writes nothing to the game and leaves Sigil's data folder (`%APPDATA%\DeadlockAgentHarness`)
  alone on uninstall;
- has English and German pages, and is not code signed, so SmartScreen warns on first run.

It prints the output path, size and SHA-256. The release uploads it as `Sigil-Setup.exe`, a stable
name the download links rely on (`docs/runbooks/releasing.md`).

### Linux

`scripts/package-linux.sh` runs on Linux x86_64. It packs the same Locker art, builds one
`dah-desktop` binary, then wraps it with linuxdeploy (pinned tag and SHA-256) into
`.scratch/package/Sigil-<version>-x86_64.AppImage` and builds
`.scratch/package/sigil_<version>_amd64.deb`. It also writes the unsigned
`.scratch/package/release-<version>/sigil-update.json` with the AppImage under
`extra_platforms.linux-x86_64`; that is the entry the maintainer merges into the release manifest
before signing (see "Signing and updates").

macOS cannot link this build: Wayland, ALSA, OpenSSL and UnRAR's C++ need Linux headers and
libraries. Use the `linux-build` workflow (`.github/workflows/linux-build.yml`) on a manual
dispatch or a push to `main`, which builds on `ubuntu-22.04` and uploads the AppImage, the `.deb`
and the manifest entry as the `Sigil-linux-x86_64` artifact. Nothing is published and no release
is created. The workflow needs `cwebp` (installed by `--deps`), Rune and TinyAgent vendored, and
the Grimoire art checkout for `pack-locker-art.sh`; it does all three itself.

Build on the oldest glibc you want to support: the AppImage runs on that glibc or newer. The
workflow pins Ubuntu 22.04 (glibc 2.35) on purpose; a newer runner raises the floor for every
player. Bundled libraries are chosen in the script: Wayland, ALSA, fontconfig, freetype,
libstdc++ and libgcc_s stay on the system (the Vulkan driver links them), and the workflow fails
if any of them end up inside the image. Like any AppImage it mounts through FUSE 2 (`libfuse2`);
on a system without it, `APPIMAGE_EXTRACT_AND_RUN=1 ./Sigil-<version>-x86_64.AppImage` runs it
from a temporary extraction, and the `.deb` avoids FUSE entirely. Its `AppRun` still needs
`libwayland-client` and, on X11, `libxkbcommon-x11-0` from the distro.

The `.deb` installs the program in `/usr/lib/sigil/sigil` with a `/usr/bin/sigil` link, a desktop
entry and the `sigil://` handler. Settings > Updates recognises that path and points at the
release page instead of the self-updater (`rust/crates/desktop/src/package_manager.rs`).

## Helper resolution

The desktop starts the agent host from `DAH_AGENT_BIN` when set, else its own executable with
`agent-host`. It runs the CLI (publish, sign-in) from `DAH_RS_BIN` when set, else its own
executable with `cli`. A host started as `agent-host` runs its tools the same way: `DAH_RS_BIN`,
else the same executable with `cli`. The standalone `dah` and `dah-agent` binaries still build
(`cargo build -p dah-rs -p dah-agent`) for development and the CLI docs; the standalone host keeps
its old search (a sibling `dah-rs` or `dah`, the repo debug build, then `PATH`).

One process owns a session directory. Do not point two hosts at the same journals.

## Verification checklist

Run these on the machine that will use the build. The list is the honest state, not a claim. "Not
verified" means no check is recorded here, not that it is known to fail:

| Step | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Desktop opens, sidebar renders | verified | in use by players since the first releases (0.1.21 fixed a Windows-only shader failure they reported); no maintainer check recorded | not verified (no local Linux host; CI only runs `cli --help`) |
| Agent host connects, provider set | verified headless and local SSE mock | not verified | not verified |
| `cli` and `agent-host` subcommands of the one executable, host tool calls run `<exe> cli` | verified: `cli doctor`, and `agent-host` with a mock chat server running `game_status` | not verified (bundle builds and passes `verify-windows.sh`) | `cli --help` runs in the workflow; `agent-host` not verified |
| Game discovery finds the install | not verified, no install on this machine | not verified | not verified |
| Addon scan, profiles, autoexec | verified against a fake install tree | not verified | not verified |
| Launch modded and vanilla | not verified, no Steam install here | not verified | not verified |
| Deploy into `citadel/addons` | verified against a fake install tree | not verified | not verified |
| `gameinfo.gi` repair | verified against a fake install tree | not verified | not verified |
| Real `resourcecompiler.exe` compile | unavailable, no CSDK 12 | not verified | unavailable (Windows toolchain) |
| Real `.dem` import | container parser tested on synthetic files | not verified against a live demo | not verified |
| Browse install pipeline | local HTTP fixture | not verified against GameBanana | not verified |
| AppImage starts, desktop entry and `sigil://` handler register | n/a | n/a | not verified |
| `Sigil-Setup.exe` installs per user, shortcuts work, uninstall removes the program | n/a | not verified (built and published with 0.1.42) | n/a |
| Self-update: swap and rollback on a real AppImage | n/a | Windows swap: not verified (see `releasing.md`) | not verified; the plan is tested on temp folders on macOS |
| `.deb` installs, `/usr/bin/sigil` links, Settings points at the release page | n/a | n/a | not verified |

A build that fails a row above is not a release. Mark the row, do not skip it.

## Signing and updates

No code-signing identity is configured. macOS will warn on an unsigned bundle and SmartScreen on
the exe and the installer. Linux has no code signing; the AppImage is trusted the same way as the other builds, by
the update manifest's Ed25519 signature.

Every packaging script also writes an unsigned
`.scratch/package/release-<version>/sigil-update.json`: the Windows zip (no `assets` flag; the art
is inside `Sigil.exe`), on a
Mac `Sigil-<version>-macos-<arch>.zip` with `Sigil.app` at the top, and on Linux the AppImage
itself (no zip) under `extra_platforms.linux-x86_64`. Sign the merged manifest with
`scripts/sign-release.sh` before publishing.

`extra_platforms` is what keeps Windows clients updating. Sigil 0.1.x refuses a manifest with an
unknown key under `platforms`, and it never reads `extra_platforms`; a release that adds Linux
therefore still updates every 0.1.x Windows and macOS copy. `scripts/release-sign.mjs` only writes
Linux there, refuses to sign a manifest with no Windows or macOS entry, and checks the AppImage's
size and SHA-256 the same way it checks a zip. Do not move `linux-x86_64` into `platforms`: that
would make every 0.1.x client reject the whole manifest.

The AppImage updates itself. On start and on "Restart to update" the updater reads `$APPIMAGE`
(never the copy inside the read-only mount), verifies the signed entry, downloads the AppImage
itself, checks the ELF header and its x86-64 machine, copies it to `.sigil-update-new` beside the
running file, renames the running file to a hidden `.old`, moves the new one into place under the
player's own file name, and starts it. A new version that dies before drawing a frame is swapped
back on the next launch. The `.deb` and distro packages do not self-update: with `$APPIMAGE` unset
the updater says the copy was not installed from a release and points at the release page. See
`docs/runbooks/releasing.md`.

## Uninstall

Removing the program leaves the data directory:

- macOS: delete `Sigil.app`, data in `~/Library/Application Support/DeadlockAgentHarness`
- Windows: uninstall Sigil from Settings > Apps (installer), or delete `Sigil.exe` (zip); data in
  `%APPDATA%\DeadlockAgentHarness`
- Linux AppImage: delete the `.AppImage` file (and a hidden `.AppImage.old` beside it if an update
  left one), data in `$XDG_DATA_HOME/DeadlockAgentHarness` (`~/.local/share/DeadlockAgentHarness`)
- Linux `.deb`: `sudo apt remove sigil`, same data directory

It holds sessions, the workspace, cached patch notes and the vanilla stash. A pending vanilla stash
means the game's addon folder is empty: restore before deleting the data directory.
