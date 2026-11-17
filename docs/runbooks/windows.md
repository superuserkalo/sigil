# Windows build

Status: draft. Cross-compiled bundle produced on macOS with `cargo-xwin`. The
binary has not been launched on Windows here.
Updated: 2026-11-17

## What ships

```text
Sigil-<version>-windows-x64/
  Sigil.exe                  the whole app: the Rune window, the Locker art (built in),
                             the notices, and the `cli` and `agent-host` subcommands
```

`Sigil.exe` runs from anywhere: the Desktop, Downloads, or straight from the zip. The first
launch of a build writes its art once to `%APPDATA%\DeadlockAgentHarness\art\<build id>` and
removes the art of older builds.

The CLI and the agent host are folded into `Sigil.exe`. The app starts the host as a child
process, `Sigil.exe agent-host` (or `DAH_AGENT_BIN` when set), and in-app sign-in runs
`Sigil.exe cli auth login <provider>`. From a terminal, `Sigil.exe cli doctor` prints like the old
`dah.exe doctor`: the GUI-subsystem exe attaches to the parent console for the `cli` path. No
Electron, no WebView, no bundled CSDK or game files.

## Build from macOS or Linux

```bash
cargo install cargo-xwin --locked
rustup target add x86_64-pc-windows-msvc
scripts/package-windows.sh
```

The script builds with `-C target-feature=+crt-static`, so the bundle does not
need the Visual C++ redistributable. The result is
`.scratch/package/Sigil-<version>-windows-x64.zip` with a SHA-256.

Verify a bundle without running it:

```bash
scripts/verify-windows.sh .scratch/package/Sigil-0.1.0-windows-x64.zip
```

It prints the SHA-256, the PE format and subsystem, the binary size, and checks
that no stale `dah.exe` or `dah-agent.exe`, PDB, CSDK binary or VPK is in the
archive.

## Build on Windows

```powershell
git clone <repo>
cd dl-harness
bash scripts/vendor-rune.sh
bash scripts/vendor-tinyagent.sh
cargo build --release -p dah-desktop
```

The GitHub workflow `.github/workflows/build-windows.yml` does the same and
uploads the staging folder as an artifact. It runs on `windows-latest`, either on
a tag or from the Actions tab.

## First run

1. Unzip anywhere writable.
2. Double-click `Sigil.exe`.
3. SmartScreen: the build is unsigned, so choose **More info**, then **Run
   anyway**.
4. The app discovers Deadlock through the Steam library folders and `DAH_GAME_DIR`.

The agent host starts only when a model is configured:

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

## Subscriptions

Sign-in lives in Settings > Models. Sign in starts `Sigil.exe cli auth login <provider>`
as a child process, which opens the browser and stores the login; Settings shows the
pending state and the sign-in link, and picks the login up when the child exits. With
a model id set, the agent switches to the new login at once (`host.json`). There are
no batch files.

The token lives in `%APPDATA%\DeadlockAgentHarness\auth`. It is never rendered,
logged or put in a command line. `Sigil.exe cli auth logout codex` revokes it.

The browser login is the first-party OAuth flow documented in
`docs/research/subscription-auth.md`. It has not been run on Windows here, so
treat the first sign-in as a test.

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
