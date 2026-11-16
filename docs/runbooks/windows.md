# Windows build

Status: draft. Cross-compiled bundle produced on macOS with `cargo-xwin`. The
binary has not been launched on Windows here.
Updated: 2026-11-16

## What ships

```text
DeadlockModManager-<version>-windows-x64/
  Deadlock Mod Manager.exe   native Rune window
  dah-agent.exe              TinyAgent JSONL sidecar
  dah.exe                    CLI
  README-WINDOWS.txt
  THIRD_PARTY_NOTICES.md
  README.md
```

The desktop looks for `dah-agent.exe` beside its own executable, then `PATH`, then
`DAH_AGENT_BIN`. No Electron, no WebView, no bundled CSDK or game files.

## Build from macOS or Linux

```bash
cargo install cargo-xwin --locked
rustup target add x86_64-pc-windows-msvc
scripts/package-windows.sh
```

The script builds with `-C target-feature=+crt-static`, so the bundle does not
need the Visual C++ redistributable. The result is
`.scratch/package/DeadlockModManager-<version>-windows-x64.zip` with a SHA-256.

## Build on Windows

```powershell
git clone <repo>
cd dl-harness
bash scripts/vendor-rune.sh
bash scripts/vendor-tinyagent.sh
cargo build --release -p dah-desktop -p dah-agent -p dah-rs
```

The GitHub workflow `.github/workflows/build-windows.yml` does the same and
uploads the staging folder as an artifact. It runs on `windows-latest`, either on
a tag or from the Actions tab.

## First run

1. Unzip anywhere writable.
2. Double-click `Deadlock Mod Manager.exe`.
3. SmartScreen: the build is unsigned, so choose **More info**, then **Run
   anyway**.
4. The app discovers Deadlock through the Steam library folders and `DAH_GAME_DIR`.

The agent sidecar starts only when a model is configured:

```powershell
set DAH_AGENT_PROVIDER=openai
set DAH_AGENT_MODEL=<exact model id>
set DAH_AGENT_API_KEY=<key>
```

or a local OpenAI-compatible server:

```powershell
set DAH_AGENT_PROVIDER=chat
set DAH_AGENT_BASE_URL=http://127.0.0.1:8080/v1
set DAH_AGENT_MODEL=<model id>
```

The manager screens work without an agent.

## Data

`%APPDATA%\DeadlockAgentHarness` holds sessions, the workspace, caches and the
vanilla stash. A pending stash means `game/citadel/addons` is empty; restore
before deleting the folder.

## Unverified

- Launching the app on Windows.
- In-game deploy, launch and restore on a real install.
- CSDK 12 `resourcecompiler.exe`.
- Code signing.

The acceptance script is macOS/Linux oriented; on Windows, use Git Bash and
check the same steps by hand until it is ported.
