# 0010 Sigil ships as one executable

Status: accepted
Date: 2026-11-17

## Context

The bundle shipped three programs: `Sigil.exe` (the Rune app), `dah-agent.exe` (the TinyAgent
JSONL host the app spawns) and `dah.exe` (the CLI that the host's tools, publish and sign-in run).
A user who moved `Sigil.exe` alone lost the agent and sign-in, and the three programs each carried
their own copy of `deadlock-core`, serde, ureq, rustls and the standard library. The owner approved
folding them into one file on 2026-11-17.

## Decision

`dah-desktop` (Sigil) is the only shipped binary. Its `main` checks the first argument before any
Rune or window state exists:

- `Sigil cli <args...>` runs the `dah-rs` CLI entry (`dah_rs::run_from`). On Windows it first calls
  `AttachConsole(ATTACH_PARENT_PROCESS)` so a terminal shows the output of the GUI-subsystem exe.
- `Sigil agent-host` runs the TinyAgent host (`dah_agent::run`). It is still a separate child
  process with the unchanged stdin and stdout JSONL protocol; only the file is shared.

`rust/crates/dah-rs` and `rust/crates/agent-host` are libraries with thin `dah` and `dah-agent`
binaries, which still build for development and the CLI docs.

Resolution, everywhere a helper is started: an explicit variable first (`DAH_AGENT_BIN` for the
host, `DAH_RS_BIN` for the CLI), else `std::env::current_exe()` with the `agent-host` or `cli`
subcommand (`rust/crates/desktop/src/helpers.rs`). A host started as `agent-host` runs its tools as
`<same exe> cli ...` unless `DAH_RS_BIN` is set. The standalone `dah-agent` keeps its old search.

## Consequences

- One file to ship, move and verify. Measured with fat LTO and one codegen unit, same sources:
  on Windows Sigil.exe grows by 8.6 MB and the bundle shrinks by 12.1 MB (59.5 MB to 47.4 MB); on
  macOS Sigil grows by 6.3 MB and the bundle shrinks by 10.1 MB (51.8 MB to 41.7 MB), because the
  helpers no longer carry their own copies of shared crates. See `docs/runbooks/performance.md`.
- The window path pays one argument comparison; no static initializers were added (the binary has
  no `__mod_init_func` section before or after). Paired startup runs show no measurable change.
- A CLI call through Sigil starts about 2 ms slower than the 8.7 MB `dah` (a larger image to load).
  The host spawns the CLI per tool call, which is small next to a model turn.
- The agent host's tokio, reqwest and jsonschema now live in the app binary. They run only in the
  `agent-host` child.

## Alternatives

- Keep three binaries: rejected by the owner for the reasons above.
- Run the agent host on a thread inside the app process: rejected. AGENTS.md keeps the host a
  separate process, so a host crash or hang never takes the window down and one process owns the
  session journals.
