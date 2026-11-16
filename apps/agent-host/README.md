# @dah/agent-host

Status: reference. The production host is the Rust `dah-agent` in `rust/crates/agent-host`.
This Node sidecar remains as the TypeScript reference during the cutover.

The Rust and Rune desktop app spawns this process and speaks newline-delimited JSON over its
stdin and stdout. The host owns one TinyAgent session at a time, runs the model, executes the
Deadlock tools and journals every run. The app owns the window, credentials, file dialogs,
approvals and every game install write.

This is the process boundary recorded in `docs/decisions/0007-tinyagent-host.md`. The host is
temporary: it stays until the Rust TinyAgent implementation reaches protocol v2 with durable
sessions, pause and resume.

## Run

```sh
pnpm install
DAH_AGENT_PROVIDER=openai \
DAH_AGENT_MODEL=your-model-id \
DAH_AGENT_API_KEY=... \
pnpm --filter @dah/agent-host dev
```

`pnpm agent:host` at the repository root runs the same entry.

Node 24 or newer. The package depends on the vendored `tiny-agent-sdk` at
`vendor/tinyagent/tinyagents/typescript`. Run `pnpm vendor:tinyagent` after a revision bump.

## Protocol

- UTF-8 JSONL: one complete JSON object per line, LF terminated.
- stdout carries only protocol envelopes. Diagnostics go to stderr with an `[agent-host]` prefix.
- Every command is answered. Responses and events may interleave.
- One run at a time. A second `prompt` or `resume` while one is active is a `busy` error. There is
  no prompt queue.

### Result envelope

```json
{"id":"1","ok":true,"sessionId":"s_6076...","text":"The index has 214883 paths.","pause":null,"usage":{"inputTokens":2214,"outputTokens":188,"requests":2}}
```

`id` is the command's `id`, or `null` when the command omitted it. `pause` is `null` or a pending
pause object. `usage` is the last run's token counts, or `null` when no run has finished.

Some commands add fields:

| Command | Extra fields |
|---|---|
| `prompt`, `resume` | none |
| `abort` | `aborted`, `runId` |
| `state` | `state` |
| `list_sessions` | `sessionIds` |
| `new_session` | `previousSessionId` |
| `shutdown` | `stopped` |

### Event envelope

```json
{"event":{"type":"text_delta","runId":"r_a6aa...","sequence":3,"delta":"Checking. "}}
```

Every TinyAgent run event is forwarded as `{"event": ...}`: `agent_start`, `text_delta`,
`tool_execution_start`, `tool_execution_end`, `compaction_start`, `compaction_end` and
`agent_end`. `prompt` and `resume` send their events as they happen and their result or error
envelope last. `agent_end` is the last event of a run. `abort` answers immediately; the interrupted
command ends with an error envelope and an `agent_end` whose status is `aborted`.

### Error envelope

```json
{"id":"2","ok":false,"error":{"code":"resume_required","message":"Session s_6076... is paused on deploy. Send resume with a result or error."}}
```

Codes come from TinyAgent where it has one: `busy`, `invalid_command`, `resume_required`,
`nothing_to_resume`, `recovery_required`, `session_not_found`, `session_failed`,
`session_fenced`, `storage_error`, `session_corrupt`, `provider_error`, `limit_exceeded`,
`repeated_tool_errors`, `hook_error`, `aborted`. The host adds `invalid_json`, `line_too_large`,
`session_closed`, `workspace_error` and `internal_error`. A startup failure with no usable config
answers `config_error` and exits non-zero.

### Commands

| Command | Fields | Meaning |
|---|---|---|
| `prompt` | `text`, optional `sessionId` | Run one prompt on the current session. |
| `resume` | optional `result` or `error`, optional `sessionId` | Continue an interrupted or paused run. |
| `abort` | none | Request cancellation of the active run. |
| `state` | none | Report run and session state without changing either. |
| `list_sessions` | none | List stored session IDs. |
| `new_session` | none | Abort any run, close the session and create a new one. |
| `shutdown` | none | Abort any run, close the session, answer and exit. |

`prompt` with no open session creates one and reports its ID in the result. `prompt` or `resume`
with a `sessionId` reopens that stored session first. `resume` is the way to continue a run after a
host restart or a paused approval:

```jsonl
{"id":"1","type":"resume","sessionId":"s_6076a1","result":"approved by Ana"}
{"id":"2","type":"prompt","text":"Use search_paths for billy.vmdl_c."}
```

`text` must be a non-empty string. `result` may be any JSON value, including `null`; `error` must be
a string. Send exactly one of them, and only to a paused run. For an interrupted run with no pause,
send `resume` without either. A prompt while a run is paused or interrupted answers
`resume_required`. A resume with no open run answers `nothing_to_resume`.

### State

```json
{"id":"3","type":"state"}
{"id":"3","ok":true,"sessionId":"s_6076...","text":"","pause":null,"usage":null,"state":{"status":"idle","runId":null,"sessionId":"s_6076...","messageCount":0,"resumable":false,"paused":null,"recoveryRequired":false,"usage":null}}
```

`status` is `idle` or `running`. `messageCount` counts committed history items. `resumable` means
the journal holds a run that `resume` can continue. `paused` carries the pending tool call. The host
uses TinyAgent's default `onUncertain: "tell_model"`, so `recoveryRequired` is always false and an
interrupted call is reported to the model with the standard interrupted message.

## Sessions

Sessions are TinyAgent journals under `<data dir>/agent-sessions/`. The host takes `<data dir>`
from `DAH_DATA_DIR`, or from the platform data directory plus the leaf `DeadlockAgentHarness`
(`deadlock-agent-harness` on Linux, matching `rust/crates/dah-rs/src/paths.rs`).

```text
<data dir>/agent-sessions/<sessionId>.jsonl
```

Each journal is append-only, checksummed and locked by its writer. One process owns the directory.
Do not point two hosts at the same journals.

The platform data directory is `~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
and `$XDG_DATA_HOME` or `~/.local/share` on Linux.

A host starts with no open session. The first `prompt` or `new_session` creates one. The host
reopens a stored session by ID and can resume an interrupted or paused run in it. A session ID looks
like `s_` followed by up to 100 letters, digits, `_` or `-`. Listing and reopening use the same
`FileStorage` instance, so a second writer fails fast.

`new_session` closes the current journal without deleting it, including a paused or interrupted run.
That journal can be reopened by ID later and resumed.

## Provider configuration

| Variable | Meaning |
|---|---|
| `DAH_AGENT_PROVIDER` | `openai` (default), `anthropic` or `chat`. `chat` is the local OpenAI-compatible path. |
| `DAH_AGENT_MODEL` | Required. The exact model ID. The host does not pick a default model. |
| `DAH_AGENT_BASE_URL` | Provider API root. Use it for a gateway or a local server. |
| `DAH_AGENT_API_KEY` | The key. When unset, `OPENAI_API_KEY` or `ANTHROPIC_API_KEY` is used for the matching provider. |
| `DAH_DATA_DIR` | Overrides the platform data directory. |
| `DAH_WORKSPACE_DIR` | Mod workspace. Defaults to `<data dir>/workspace`, matching `dah-rs`. |
| `DAH_RS_BIN` | The Rust CLI. Defaults to `rust/target/debug/dah-rs` or the `dah` binary the current crate builds, then `dah-rs` on PATH. |

`DAH_AGENT_API_KEY` is not required for `chat`, so a local server can run without one. Keys are read
from the environment when the process starts. They are never written to a journal, an event, a log,
a tool result or an error. Tool children do not inherit the key variables. Every outbound value also
passes the redactor, which replaces key-shaped strings.

The host resolves the provider, model and key before it accepts commands. An invalid provider, a
missing model or a missing key for `openai` or `anthropic` produces a `config_error` envelope on
stdout, a diagnostic on stderr, and exit code 1.

## Tools

Each tool returns compact JSON text. The tools call `dah-rs` with an argument array through
`execFile`, from the mod workspace, with `--json`. No tool builds a shell string and no tool exposes
a general shell.

| Tool | Invocation | Effect |
|---|---|---|
| `game_status` | `dah-rs doctor --json` | Reads install, `gameinfo.gi` and addon slots. |
| `game_index` | `dah-rs index [--refresh] --json` | Reads the game VPKs and updates the workspace index cache. |
| `search_paths` | `dah-rs find <query> [--hero] [--type] [--folder] [--limit] --json` | Reads exact game paths and sizes from the index. |
| `extract_paths` | `dah-rs extract <paths...> [--out] [--overwrite] --json` | Writes extracted game files inside the mod workspace. |
| `project_check` | `dah-rs mod check --json` | Reads the project baseline against current game files. |
| `patch_notes` | `dah-rs patches [--limit] [--refresh] --json` | Fetches `api.steampowered.com` on demand and caches the response in the workspace. |
| `validate_payload` | in process | Pure port of the payload path rules. No file access. |

`validate_payload` lives in `src/payload.ts` and mirrors
`packages/deadlock-tools/src/validate.ts` until the Rust core owns the rule. It enforces compiled
files only, safe relative paths, no skeletons, no `scripts/*.vdata_c`, no tool caches, no source
assets, no archives and no executables.

The tools above are wired to the Rust CLI. If `dah-rs` is missing, or a subcommand is not built in
the binary yet, the tool returns an error that names the failed command and the binary path. It does
not fake a result. `game_status` needs a Deadlock install. `search_paths` and `project_check` need
an indexed install. `extract_paths` needs a Deadlock install and writes only under
`DAH_WORKSPACE_DIR`.

## Approval semantics

`beforeTool` pauses any tool whose name contains `deploy`, `publish`, `launch`, `install` or
`gameinfo`, including future tools. It returns:

```json
{"kind":"approval","tool":"deploy","arguments":{...}}
```

The run ends with `agent_end` status `paused` and a result envelope whose `pause` is the pending
call. The app shows the approval UI. `resume` with `result` approves and continues; `resume` with
`error` refuses and tells the model why. Neither the handler nor `beforeTool` runs again for that
call.

No deploy, publish, launch, install or `gameinfo.gi` repair tool exists yet. The hook is the gate
for them when they are added. Game install writes stay in the Rust app even after approval: the host
prepares and validates, the app writes.

## Not here

- No general shell and no arbitrary filesystem tool. The tool set is the list above.
- No game install writes, no deploy, no launch and no publish.
- No reconcile command. Interrupted calls use TinyAgent's `tell_model` policy.
- No telemetry, analytics or heartbeat. The only network calls are the selected model provider and
  the user-requested Steam patch feed through `dah-rs`.

## Files

```text
src/main.ts      process entry, JSONL framing, shutdown and signals
src/host.ts      session lifecycle, run management, envelopes
src/agent.ts     TinyAgent Agent, provider, hooks and redaction
src/tools.ts     Deadlock tools and JSON Schemas
src/dah-rs.ts    execFile wrapper for the Rust CLI
src/payload.ts   pure payload validation port
src/prompt.ts    system prompt and domain rules
src/config.ts    environment and data directory resolution
src/redact.ts    key-shaped string redaction
test/            one protocol smoke test
```
