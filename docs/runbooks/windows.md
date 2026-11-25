# Windows build

Status: active. Windows 10 and 11 x64 is the released platform (0.1.42 on 2026-11-25). Builds are
cross-compiled on macOS with `cargo-xwin`; the installer is built there with NSIS. The list at the
end says what has not been checked on a real Windows PC by the maintainer.
Updated: 2026-11-25

## What ships

```text
Sigil-Setup.exe              per-user installer: Sigil.exe into %LOCALAPPDATA%\Programs\Sigil,
                             Start menu and desktop shortcuts, an uninstaller
Sigil-<version>-windows-x64/
  Sigil.exe                  the whole app: the Rune window, the Locker art (built in),
                             the notices, and the `cli` and `agent-host` subcommands
```

Players get the installer from
`https://github.com/superuserkalo/sigil/releases/latest/download/Sigil-Setup.exe`, or the zip from
`.../latest/download/Sigil-windows-x64.zip`. The zip's `Sigil.exe` runs from anywhere: the Desktop,
Downloads, or straight from the zip. Neither needs administrator rights. The first launch of a build
writes its art once to `%APPDATA%\Sigil\art\<build id>` and removes the art of older
builds.

The CLI and the agent host are folded into `Sigil.exe`. With the experimental Agent page on, the app
starts the host as a child process, `Sigil.exe agent-host` (or `DAH_AGENT_BIN` when set), and in-app
sign-in runs `Sigil.exe cli auth login <provider>`. From a terminal, `Sigil.exe cli doctor` prints
like the old `dah.exe doctor`: the GUI-subsystem exe attaches to the parent console for the `cli`
path. No Electron, no WebView, no bundled CSDK or game files.

## Build from macOS or Linux

```bash
cargo install cargo-xwin --locked
rustup target add x86_64-pc-windows-msvc
scripts/vendor-rune.sh && scripts/vendor-grimoire.sh   # Rune, and the Locker art source
scripts/package-windows.sh
scripts/package-installer.sh .scratch/package/Sigil-<version>-windows-x64/Sigil.exe <version> \
  .scratch/package/Sigil-Setup.exe                     # needs makensis
```

The script builds with `-C target-feature=+crt-static`, so the bundle does not need the Visual C++
redistributable. The result is `.scratch/package/Sigil-<version>-windows-x64.zip` with a SHA-256
and an unsigned update manifest. Releasing (signing, stable names, publishing) is in
`docs/runbooks/releasing.md`; the installer is described in `docs/runbooks/packaging.md`.

Verify a bundle without running it:

```bash
scripts/verify-windows.sh .scratch/package/Sigil-<version>-windows-x64.zip
```

It prints the SHA-256, the PE format and subsystem, the binary size, and checks that no stale
`dah.exe` or `dah-agent.exe`, PDB, CSDK binary or VPK is in the archive.

## Build on Windows

```powershell
git clone https://github.com/superuserkalo/sigil.git
cd sigil
bash scripts/vendor-rune.sh
cargo build --manifest-path rust/Cargo.toml --release -p dah-desktop
```

The GitHub workflow `.github/workflows/build-windows.yml` does the same on `windows-latest`, on a
`v*` tag or from the Actions tab, and uploads the zip as an artifact. The `windows-shaders` workflow
compiles every 3D viewer shader with FXC on D3D12 (WARP) whenever `rust/crates/sigil-viewer`
changes, since 0.1.21 fixed a shader that only Windows rejected.

## First run

1. Run `Sigil-Setup.exe`, or unzip `Sigil.exe` anywhere writable.
2. SmartScreen: the installer and the exe are not code signed, so choose **More info**, then
   **Run anyway**.
3. Setup has three steps: find Deadlock (Steam library folders, or `DAH_GAME_DIR`), let the game
   load mods (the `gameinfo.gi` fix, with a backup), and updates and privacy (both off until the
   player agrees).

The manager pages work without an agent. The agent host starts only with Settings > Experimental >
Agent on and a model configured, for example:

```powershell
set DAH_AGENT_PROVIDER=openai
set DAH_AGENT_MODEL=<exact model id>
set DAH_AGENT_API_KEY=<key>
```

or a local OpenAI-compatible server (`DAH_AGENT_PROVIDER=chat`, `DAH_AGENT_BASE_URL`,
`DAH_AGENT_MODEL`). Subscription sign-in (Codex, Grok) lives in Settings > Models and stores its
token in `%APPDATA%\Sigil\auth`; it has not been run on Windows here.

## Launching

Launch modded and Launch vanilla start Deadlock through Steam. Since 0.1.41 the launch steps (the
mod checks, the `gameinfo.gi` fix, setting mods aside for vanilla, the launch options) run on a
worker thread, so the window stays responsive. While a launch step is writing game files, mod
changes (toggles, deletes, load order) are refused with a short message instead of racing it.
While Deadlock runs, turning off or deleting a mod waits until the game closes.

While Deadlock runs, the sidebar's Launch modded reads Stop Deadlock (same confirm as before) and
Launch vanilla is off. Since 0.1.43 there is no Restore button: mods a vanilla launch set aside go
back by themselves, on a worker, when the launch monitor sees Deadlock close after it ran, and at
Sigil's start when a stash exists and the game is not running (so closing Sigil during a vanilla
session is covered). It never runs while the game runs, a launch is under way or a launch step
writes game files; while it runs, mod changes and launches wait. A file already in the addon
folder is kept, never overwritten. A failure is a warning toast with Try again and a banner on
Installed until a restore works. A modded launch still puts a stash back as its first step.

## Data

`%APPDATA%\Sigil` holds settings, backups, the trash, caches, the agent workspace
and sessions, and the vanilla stash. Builds up to 0.1.42 used `%APPDATA%\DeadlockAgentHarness`;
0.1.43 renames that folder to `Sigil` on its first start (see `docs/runbooks/development.md`). A pending stash means `game/citadel/addons` is empty; start Sigil
with the game closed (the mods come back by themselves) before deleting the folder. Uninstalling Sigil leaves this folder in place.

## Not verified on a real Windows PC by the maintainer

Players run every release on Windows and report problems, but none of these has a recorded check:

- The installer: SmartScreen, the per-user install, shortcuts, uninstall, and the self-updater
  replacing `Sigil.exe` inside `%LOCALAPPDATA%\Programs\Sigil`.
- The self-update swap and rollback of a running `Sigil.exe`.
- Launch modded, Launch vanilla and the stash restore on a real install, including the worker-thread
  launch steps, Stop Deadlock from the sidebar, and the automatic restore (after the game closes,
  at start, and its failure banner with Try again).
- Relay region blocking (Windows Firewall rules) and hosting a Deadworks server.
- The 3D viewer, the live match bridge and the benchmark against a real game.
- 7z and RAR archives from GameBanana on Windows paths (tested on macOS).
- CSDK 12 `resourcecompiler.exe` for the agent's compile step.
- Code signing: there is no signing identity.

`scripts/acceptance.sh` is macOS and Linux oriented; on Windows, use Git Bash and check the same
steps by hand.
