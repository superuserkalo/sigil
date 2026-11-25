# Decompile and compile toolchain

Status: active. Backend code exists; no real game asset has been round-tripped on Windows yet. In the app this page sits under Agent, behind Settings > Experimental > Agent (off by default).
Updated: 2026-11-16

This document explains the two external tools the harness drives, what the Rust backend does
today, what it refuses to do, and what has actually been verified. Domain facts live in
`docs/research/deadlock-modding.md`; the boundary decision is
`docs/decisions/0003-windows-compile-backend.md`.

## In plain language

Source 2 games do not load the files a modder edits. They load compiled versions whose names end in
`_c`. A `.png` becomes a `.vtex_c`, a `.vmat` becomes a `.vmat_c`, a `.wav` becomes a `.vsnd_c`.

- **Decompile** means turning a compiled `_c` file back into something a person or an agent can
  edit: a PNG, a text material, a WAV. It is reverse engineering, not a perfect restore. Some data
  is reconstructed, some is lost.
- **Compile** means turning the edited source back into the `_c` file the game reads. It needs
  Valve's resource compiler, which ships inside the Windows CSDK 12 community toolset.

The workbench does not edit `_c` bytes in place. It extracts the compiled file, decompiles it,
edits the source, compiles it, validates the result and packs it into a VPK.

## Which tool does what

| Step | Tool | Platform | License | Role here |
| --- | --- | --- | --- | --- |
| Decompile | `Source2Viewer-CLI` | macOS, Linux, Windows | MIT | Run `-i <compiled> -o <folder> -d` and collect the source files it writes |
| Compile | `resourcecompiler.exe` | Windows | none stated for CSDK 12 | Run `-i <source> -game <gameinfo.gi>` |
| Pack, inspect | `vpkeditcli` | macOS, Linux, Windows | MIT | Detected for the pack and inspect path |
| Merge, split | `vpkmerge` | macOS, Linux, Windows | MIT | Detected for the addon merge path |

`Source2Viewer-CLI` is the command-line build of Source 2 Viewer / ValveResourceFormat. Its
arguments are not a stable public API; the decompile call can need updating after a tool upgrade.
Attribution stays in `THIRD_PARTY_NOTICES.md`.

CSDK 12 is a community adaptation of Valve's Counter-Strike 2 workshop toolset. It has no license.
It is never bundled, downloaded, mirrored or redistributed by this project. The user installs it
themselves.

## Configuration

Each tool is resolved from an explicit environment variable first, then from `PATH`. An explicit
value is authoritative: if it points at a missing or unusable file, the status says so and does not
quietly fall back to a different binary.

| Variable | Meaning |
| --- | --- |
| `DAH_SOURCE2VIEWER_CLI` | Path to the `Source2Viewer-CLI` binary |
| `DAH_VPKEDITCLI` | Path to `vpkeditcli` |
| `DAH_VPKMERGE` | Path to `vpkmerge` |
| `DAH_RESOURCECOMPILER` | Direct path to `resourcecompiler.exe` |
| `DAH_CSDK_PATH` | CSDK 12 root; the backend looks under `game/bin_tools/win64/resourcecompiler.exe` and nearby layouts |
| `DAH_WORKSPACE_DIR` | Fallback workspace root for decompile containment when no `project.json` is above the output |

There is no downloader and no bundled binary. A future bundled tool must have a clear license, a
pinned version and a pinned hash, and an entry in the notices.

## What the Rust backend does

The module is `deadlock_core::toolchain`. It exposes:

- `toolchain_status()` and `format_toolchain_status()`
- `ToolKind`, `ToolStatus`, `ToolchainStatus`
- `plan_decompile()` and `decompile()` with `DecompileRequest`, `DecompilePlan`, `DecompileResult`
- `plan_compile()` and `compile()` with `CompileRequest`, `CompilePlan`, `CompileResult`

Detection reports four tools: Source 2 Viewer CLI, `resourcecompiler.exe`, `vpkeditcli` and
`vpkmerge`. Each status carries a label, availability, the resolved path, a reason when unavailable,
and a best-effort version line. Version probing runs `--version` with a three second cap. The
compiler is not probed with a version flag because no inert flag is documented.

A tool is available only when the resolved file is a real file, has an execute bit on Unix, and is
not a Windows PE binary on macOS or Linux. A PE file found on `PATH` on macOS is reported as
unavailable with that reason instead of pretending it can run. A script wrapper configured through
`DAH_RESOURCECOMPILER` is accepted, which is the only supported non-Windows path today and is not
the same as claiming Wine works.

Decompile:

1. Requires an available `Source2Viewer-CLI`. Otherwise it refuses with the detection reason.
2. Requires the input file to exist and resolve inside a mod workspace. The workspace is the
   nearest ancestor with `project.json`, else `DAH_WORKSPACE_DIR` when the output is below it.
3. Requires the output folder to resolve inside the same workspace.
4. Creates the output folder, snapshots it, runs `Source2Viewer-CLI` with an argument array and no
   shell, and captures stdout and stderr.
5. Returns the files created or changed under the output folder, plus the captured log.
6. Caps one run at 120 seconds. A timeout or a non-zero exit is an error with the log.

Compile:

1. Requires an available `resourcecompiler.exe`. On macOS or Linux with no runnable configured
   compiler it returns an explicit error naming the platform limitation.
2. Requires the source file and a real `gameinfo.gi`. A folder is accepted and `gameinfo.gi` is
   appended.
3. Creates the output folder and uses it as the child process working directory.
4. Runs `resourcecompiler.exe -i <source> -game <gameinfo>` with no shell and captures output.
5. Returns the `_c` files created or changed in the output folder and next to the source, plus the
   log. It does not predict a game path.
6. Caps one run at 300 seconds. A timeout or a non-zero exit is an error with the log.

`resourcecompiler.exe` chooses where it writes compiled files. For a source outside a CSDK content
folder it can write next to the source, and it can also write into the CSDK game tree. The harness
reports what it observed, and a later staging step still has to place each `_c` at its exact game
path. Do not read `CompileResult.outputs` as "the mod is staged".

## CLI surface

The command module is `rust/crates/dah-rs/src/commands/toolchain.rs`. It is not wired into
`main.rs` yet. When wired:

```text
dah toolchain status
dah mod decompile <input> [--out DIR] [--workspace DIR] [--approve] [--json]
dah mod compile <source> [--gameinfo FILE] [--out DIR] [--workspace DIR] [--approve] [--json]
```

Both mod commands are dry runs by default. They resolve the same plan the real run uses and print
the tool path, argument array, input, output and workspace. `--approve` is required to start a
process. `--json` returns the plan in a `plan` object, or `outputs` and `log` after a run.

`dah toolchain status` is always a report and exits zero. It never runs a compile or a decompile.

`dah mod compile` defaults `--gameinfo` to `$DAH_CSDK_PATH/gameinfo.gi`. There is no guess from the
installed game, because the compiler needs the CSDK content tree, not the shipping game's file.

## Platform limits

- Decompile, pack, inspect, merge and validate are cross-platform.
- Compile requires a Windows host with CSDK 12, or a runnable configured wrapper. No Wine or Proton
  backend has been verified, so the app does not promise one.
- CSDK 12 is pinned to an older game build. Newer formats such as AnimGraph 2 can compile
  incorrectly. A successful compile is not proof that the asset works in the current game.
- A model can decompile into several files. A texture decompile is not guaranteed to reproduce the
  original authoring file or its exact compression settings.

## Safety rules

- No agent tool writes to the game install. Compile and decompile write inside the workspace only.
  Deploy, launch, `gameinfo.gi` repair and publish remain app actions behind approval.
- Decompile refuses an input or output outside the workspace, including through symlink resolution
  of the deepest existing ancestor.
- No shell is used. Arguments go to the process as an array, so a path with spaces or shell
  metacharacters is one argument.
- Tool paths come from settings or `PATH`, never from a URL. Nothing is downloaded.
- A missing tool disables the step with a reason. The backend never substitutes a different
  compiler.
- Captured output is capped at 64 KiB. Logs contain paths and tool output, never provider keys.
- CSDK 12 stays local. The app never copies it into a release or a VPK.

## What is verified

Verified on this macOS host:

- `cargo check` for the standalone module tree passes.
- With no tools configured, status reports all four tools unavailable with honest reasons:
  not found on `PATH`, and for the compiler, Windows only plus never bundled.
- Fake executable scripts under `.scratch/fake-tools` prove env-based detection, `PATH` detection,
  version capture, `DAH_CSDK_PATH` resolution of `game/bin_tools/win64/resourcecompiler.exe`, and a
  full fake decompile and compile run that produced output files and captured logs.
- A fake executable with an `MZ` header is rejected on macOS with "Windows PE binary" rather than
  reported as available.
- Decompile refuses an output outside the project and an input outside the project with explicit
  messages.
- CLI dry runs print the resolved command and do not start a process. `--approve` starts it.

Not verified:

- A real `Source2Viewer-CLI` decompile of a Deadlock asset. The real CLI was not installed on this
  host.
- A real `resourcecompiler.exe` compile on Windows. No Windows host or CSDK 12 install was used.
- Whether the CSDK compiler writes next to the source or into its own game tree for a given Deadlock
  source type. The backend reports observed files instead of assuming.
- A Wine or Proton wrapper that actually runs `resourcecompiler.exe`. Do not promise one.
- Timeout cleanup of grandchild processes. The direct child is killed; a grandchild that inherited
  the output pipes can outlive it.
- Wiring in `dah-rs/src/main.rs` and `commands/mod.rs`, which are owned outside this change.

## References

- [Source 2 Viewer command-line utility](https://s2v.app/ValveResourceFormat/guides/command-line.html)
- [ValveResourceFormat repository](https://github.com/ValveResourceFormat/ValveResourceFormat)
- [CSDK 12 guide, Divine Skins Wiki](https://wiki.divineskins.gg/en/docs/deadlock/tools/csdk-12)
- [How Deadlock mods work](https://wiki.divineskins.gg/en/docs/deadlock/basics/how-mods-work)
