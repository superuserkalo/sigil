# dah-desktop

Native Rust desktop workbench for the Deadlock modding harness, built with Rune. The UI is a
keyed element tree over plain Rust state. No browser, no web view, no telemetry.

## Run

From `rust/`:

```sh
cargo run -p dah-desktop
```

Headless snapshots use the same renderer and a fixed virtual clock:

```sh
cargo run -p dah-desktop -- --snap out.png --scene index
cargo run -p dah-desktop -- --snap out.png --scene impact --size 1440x960 --scale 2
cargo run -p dah-desktop -- --list-scenes
```

Scenes are declared in `app.rs` and exposed through `AppSpec::scenes`:

| Scene | What it shows |
| --- | --- |
| `workbench` | Chat transcript, composer, provider status and a running tool timeline (demo fixture) |
| `projects` | Project list and a selected manifest (demo fixture) |
| `index` | Search, hero/type/VPK filters and result rows (demo fixture) |
| `patches` | Official patch list, new badges, note body and target summary (demo fixture) |
| `impact` | Compatibility cards, gameinfo and slot warnings, patch correlation (demo fixture) |
| `settings` | BYOK provider status, local endpoint, paths, safety and host state (demo fixture) |
| `empty` | The honest first-run workbench: no project, plus the real host config or its explicit error |
| `local` | The real startup state and a real `dah-agent` child, for checking a machine with a game install |
| `local-projects` | The real project read from `DAH_WORKSPACE` |
| `local-patches` | The real cached Steam news read from the data directory |

Demo scenes render a gold `DEMO DATA` chip in the header. They never claim to have read a game
install. The default launch and the `empty` scene use only real data or explicit empty states.

## Data wiring

`data.rs` performs reads at startup:

- `deadlock_core::discovery::read_game_status` detects the install, `gameinfo.gi` SearchPaths and
  addon slots. The sidebar and status bar report the result.
- `deadlock_core::patch_notes::load_patch_note_cache` reads `patch-notes.json` from the data
  directory. Cached patch notes fill the Patches screen with their real titles and body text.
- `deadlock_core::project::read_project` reads `project.json` when `DAH_WORKSPACE` names a
  workspace. Targets, slot, references and history come from the manifest.

Paths:

- `SIGIL_DATA_DIR` (or the older `DAH_DATA_DIR`) overrides the data directory. The default is
  `~/Library/Application Support/Sigil` on macOS, `%APPDATA%\Sigil` on Windows, and
  `$XDG_DATA_HOME/Sigil` (or `~/.local/share/Sigil`) on Linux. The app sets `SIGIL_DATA_DIR` for
  the CLI and agent host it starts.
- `SIGIL_WORKSPACE_DIR` (or `DAH_WORKSPACE_DIR`, `DAH_WORKSPACE`) points at a project workspace.

The Game Index and Impact screens do not load the whole catalog yet. Filling 150,000 VPK entries
or running a baseline check on the render thread would stall the window; both need the background
job path through `setup` and `poll`. Until then those actions say so in the status bar and leave the
fixture or empty state in place. The payload rules and the write boundary are still visible.

## Structure

```text
src/main.rs       AppSpec, setup, window size, scenes, poll and key handlers
src/agent.rs      dah-agent process, JSONL client, reader threads, lifecycle state
src/app.rs        State, shell (sidebar, header, status bar), scene table, event mapping
src/model.rs      Render model, demo fixtures and empty defaults
src/data.rs       Read-only deadlock-core adapter used at startup
src/ui.rs         Palette, type scale and primitives (panels, chips, actions, toggles, tables)
src/screens/      Workbench, Projects, Index, Patches, Impact, Settings
```

State is plain Rust: current screen, selected project and note, the index query and filters,
loaded project/catalog/patch data, composer edits, provider status, the agent link and a status
line. Handlers are closures on keyed elements. There is no speculative store or reducer.

## Agent host

The window connects to the Rust TinyAgent host, `dah-agent`, as a child process. There is no Node
sidecar and no second protocol: the app speaks the JSONL commands in
`rust/crates/agent-host/README.md` (`prompt`, `resume`, `abort`, `state`, `list_sessions`,
`new_session`, `shutdown`) over the child's stdin and reads one envelope per line from stdout.

Environment read at `AppSpec::setup` (the key is never stored or rendered):

| Variable | Required | Meaning |
| --- | --- | --- |
| `DAH_AGENT_MODEL` | yes | Exact model id. A missing value shows a configuration card. |
| `DAH_AGENT_PROVIDER` | no | `openai` (default), `anthropic` or `chat`. |
| `DAH_AGENT_API_KEY` | for openai and anthropic | Read by the host, never shown. |
| `DAH_AGENT_BASE_URL` | no | Gateway or local server root. |
| `DAH_AGENT_BIN` | no | Host binary. Default: a sibling of the app, `rust/target/debug/dah-agent` above the working directory, then PATH. |

Lifecycle: `starting`, `ready`, `running`, `paused`, `error`, `stopped`. The reader threads wake
Rune through the `Waker` from `setup`; `poll` drains the channel without blocking and returns true
only when visible state changed. Sending a prompt clears the composer, appends the user row and
moves to `running`. `text_delta` appends to the streaming assistant row, `tool_execution_start`
and `tool_execution_end` add timeline and transcript rows, and `agent_end` settles status and
usage. A pause envelope renders an approval card with the tool name and payload: Approve sends
`resume` with `"approved by user"`, Deny sends `resume` with `"denied by user"`.

Process handling: the child stays alive for the app's lifetime. `rune::run` has no close hook, so
the quit chords (⌘Q and ⌘W) send `shutdown` before the window closes. Otherwise the host exits on
stdin EOF when this process ends, and `Drop for AgentLink` sends `shutdown`, waits briefly for the
journal to settle and then kills the child.

Demo scenes use a labelled fixture link and spawn no process. The `empty` and `local` scenes use
the real host; without `DAH_AGENT_MODEL` they show the explicit configuration error.

## Rune APIs used

- `AppSpec` with `scenes`, `poll` and `on_key`, plus `rune::run` for the window and headless tools.
- Keyed `El` builders: `col`, `row`, `el`, `text`, `spacer`, `rule`, `scroll`, `absolute`, `fill`,
  `border`, `shadow`, `radius`, `opacity`, `motion`, `hover`, `press`, `press_scale`.
- `text_input` bound to `TextEdit` with `on_edit`, `on_submit` and `on_input_key`, for the prompt
  composer, the index search and the local endpoint.
- `tok::theme`, the semantic palette, and a small local type scale on `TextStyle` with
  `Family::SystemMono` for paths and tool names.
- Springs from `tok::motion` (`SNAPPY`) and a frame-rate pulse driven by `AppSpec::poll` for the
  running tool row.
- Accessibility roles, labels, switch state and row labels on the interactive elements. Decorative
  glyphs are hidden from the tree.

## Boundaries

- Nothing writes to a game install. Deploy, `gameinfo.gi` repair, launch and publish remain
  approval-gated app actions and are not in this crate. The host has no shell tool; its tools call
  `dah-rs` with argument arrays from the workspace.
- The workbench never runs agent tools in its render loop. Tools run in the host process; the
  window only renders events and answers approvals.
- Keys are never rendered. Providers show configured state, model name and endpoint only.
- The Game Index and Impact screens still do not load the whole catalog on the render thread. A
  future background job will use `setup` and `poll` like the agent link does.
