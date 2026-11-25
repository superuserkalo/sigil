# dah-agent

The Rust TinyAgent v2 host for the Deadlock workbench, shipped inside the app as
`Sigil agent-host`: one process, one durable session at a time, JSONL over stdin and stdout.

Status: the host is complete against the Rust `tiny-agent` protocol v2 crate and is the only host.
The earlier TypeScript host was removed from the repository on 2026-11-25 (ADR 0007).

## Run

```sh
cargo build --manifest-path rust/Cargo.toml -p dah-agent
rust/target/debug/dah-agent
```

Or, from `rust/`:

```sh
cargo build -p dah-agent
```

The parent process writes one JSON command per line to stdin and reads one JSON envelope per line
from stdout. stdout carries protocol envelopes only. Diagnostics, including a one-line ready
message, go to stderr. Blank lines are ignored.

The host exits on `shutdown`, on stdin EOF, or when stdin closes. A run is cancelled and its
session released before exit.

## Environment

| Variable | Required | Meaning |
| --- | --- | --- |
| `DAH_AGENT_MODEL` | yes | Exact model id for the configured provider. No default. |
| `DAH_AGENT_PROVIDER` | no | `openai` (default), `anthropic` or `chat`. |
| `DAH_AGENT_API_KEY` | for openai and anthropic | Provider key. Falls back to `OPENAI_API_KEY` or `ANTHROPIC_API_KEY`. `chat` may run without one. |
| `DAH_AGENT_BASE_URL` | no | API root for a gateway or local server. The adapter appends its path. |
| `SIGIL_DATA_DIR` | no | Base data directory (`DAH_DATA_DIR` still works when it is unset). Default: the OS application data directory plus `Sigil`, moved once from the old `DeadlockAgentHarness` or `deadlock-agent-harness`. |
| `SIGIL_WORKSPACE_DIR` | no | Same as `DAH_WORKSPACE_DIR`, which is still read when it is unset. |
| `DAH_WORKSPACE_DIR` | no | Mod workspace and the `dah-rs` working directory. Default: `<data>/workspace`. |
| `DAH_RS_BIN` | no | `dah-rs` binary. Default: a sibling of this executable, then `rust/target/debug/dah-rs` (or `dah`, the current crate binary name) above the workspace, then `dah-rs` on `PATH`. |

Provider keys are read once at startup. They never enter an envelope, the journal, a log line or a
child process environment. `dah-rs` children run with `DAH_AGENT_API_KEY`, `OPENAI_API_KEY` and
`ANTHROPIC_API_KEY` removed.

A missing key or model is a startup failure: stdout gets
`{"id":null,"ok":false,"error":{"code":"config_error","message":"..."}}` and the process exits 1.

## Protocol

Every command is a JSON object with an optional string `id` and a `type`. Every response echoes the
`id` it was given (`null` when absent).

### Commands

```json
{"id":"1","type":"prompt","text":"recolor Haze's coat","sessionId":"s_..."}
{"id":"2","type":"resume","result":{"approved":true},"sessionId":"s_..."}
{"id":"3","type":"resume","error":"The user declined the deploy.","sessionId":"s_..."}
{"id":"4","type":"abort"}
{"id":"5","type":"state"}
{"id":"6","type":"list_sessions"}
{"id":"7","type":"new_session"}
{"id":"8","type":"shutdown"}
```

- `prompt` needs a non-empty `text`. Without `sessionId` it continues the open session, or creates a
  fresh one when none is open. With a `sessionId` it opens that stored session (`session_not_found`
  when it does not exist) and refuses to run a session with an interrupted or paused run
  (`resume_required`) or one that needs reconciliation (`recovery_required`).
- `resume` accepts exactly one of `result` or `error`, and only for a paused run. `result:null` is a
  result. A run interrupted by a crash has no paused call, so it resumes without either field.
- `abort` cancels the active run. It answers immediately with `aborted:true` and the run reports
  `aborted` when it settles.
- `state` reports the open session and the active run without changing anything.
- `list_sessions` lists the journal ids in the session directory.
- `new_session` stops the active run, closes the current session and creates a fresh one.
- `shutdown` stops the active run, closes the session, answers with `stopped:true` and exits.

### Result envelope

```json
{
  "id": "1",
  "ok": true,
  "sessionId": "s_...",
  "text": "The assistant's final reply.",
  "pause": null,
  "usage": {"inputTokens": 1200, "outputTokens": 300, "requests": 2}
}
```

`pause` is `{"toolCallId":"...","toolName":"deploy","payload":{"kind":"approval",...}}` when the run
stopped for approval.

### Event envelope

While a run executes, each TinyAgent event is forwarded unchanged:

```json
{"event":{"runId":"r_...","sequence":1,"type":"agent_start"}}
{"event":{"runId":"r_...","sequence":2,"type":"text_delta","delta":"The "}}
{"event":{"runId":"r_...","sequence":3,"type":"tool_execution_start","toolCallId":"call_1","toolName":"game_status","arguments":{},"executionId":"s_...:4"}}
{"event":{"runId":"r_...","sequence":4,"type":"tool_execution_end","toolCallId":"call_1","toolName":"game_status","output":"{...}","isError":false}}
{"event":{"runId":"r_...","sequence":5,"type":"agent_end","status":"completed","text":"...","usage":{...}}}
```

### Error envelope

```json
{"id":"1","ok":false,"error":{"code":"busy","message":"A run is already active. Send abort first."}}
```

Codes: `config_error`, `workspace_error`, `invalid_json`, `invalid_utf8`, `line_too_large`,
`invalid_command`, `busy`, `session_closed`, `session_not_found`, `session_failed`, `session_fenced`,
`storage_error`, `resume_required`, `recovery_required`, `nothing_to_resume`, `provider_error`,
`tool_error`, `configuration_error`, `aborted`, `internal_error`, plus a run failure code such as
`limit_exceeded`, `repeated_tool_errors`, `hook_error` or `output_missing`.

A fatal session code (`session_failed`, `storage_error`, `session_fenced`) discards the open session
object. The journal stays on disk and can be reopened by id.

## Approval flow

`beforeTool` pauses an exact dangerous action instead of matching a substring: `deploy_apply`,
`publish_upload`, `game_launch`, `gameinfo_repair`, `install_addon`, and any tool name ending in
`_apply` or `_upload` (case-insensitive). Reads and workspace writes such as `deploy_plan`,
`project_migrate`, `project_release` and `publish_prepare` pass through. The pause payload is:

```json
{"kind":"approval","tool":"deploy_apply","arguments":{...}}
```

The result envelope carries the same payload in `pause`. The UI shows it, then answers with
`resume` (`result` on approval, `error` on refusal). The pause is durable: a restarted host can open
the session, see `state.paused`, and resume it. A resumed call is settled with the app's answer; its
hooks do not decide again.

No tool in this host installs into the game, applies a migration, launches the game, repairs
`gameinfo.gi` or uploads a release. `deploy_plan` and `project_migrate` return a plan only, and
`publish_prepare` writes a folder. The app performs the approved action.

## Tools

| Tool | Idempotent | Runs |
| --- | --- | --- |
| `game_status` | yes | `dah-rs doctor --json` |
| `game_index` | no | `dah-rs index [--refresh] --json` |
| `search_paths` | yes | `dah-rs find <query> [--hero] [--type] [--folder] [--limit] --json` |
| `extract_paths` | no | `dah-rs extract <paths...> [--out] [--overwrite] --json` |
| `project_check` | yes | `dah-rs mod check --json` |
| `project_validate` | yes | `dah-rs mod validate --json` |
| `project_pack` | no | `dah-rs mod pack [--slot NN] --json` |
| `project_release` | no | `dah-rs mod release [--slot NN] [--note TEXT] --json` |
| `deploy_plan` | yes | `dah-rs mod deploy --json` (never `--approve`) |
| `project_migrate` | yes | `dah-rs mod migrate --json` (never `--apply`) |
| `watch_projects` | yes | `dah-rs watch [--limit N] --json` |
| `publish_prepare` | no | `dah-rs publish prepare --json` |
| `patch_notes` | no | `dah-rs patches [--limit] [--refresh] --json` |
| `validate_payload` | yes | in-process path rules |

`project_pack` and `project_release` take the addon slot from `project.json` unless the model passes
`slot`. No tool accepts `--workspace` or `--out`, so every write lands in the workspace that
`DAH_WORKSPACE_DIR` names. `project_release` requires a recorded baseline and refuses to overwrite an
existing version folder.

`dah-rs` is spawned with an argument array, never a shell string, from `DAH_WORKSPACE_DIR`, with a
120 second timeout and no provider credentials. `validate_payload` mirrors
`deadlock_core::payload`: it rejects absolute, backslash, empty or parent segments,
staging-folder prefixes, tool caches, `*.vnmskel_c`, `scripts/*.vdata_c`, source assets, archives and
executables, and reports duplicate paths case-insensitively. Entries that do not end in `_c` warn.

There is no general shell tool and no arbitrary process tool.

## Storage

Journals live under `<data>/agent-sessions/<sessionId>.jsonl`, one append-only TinyAgent v2 JSONL
file per session with fenced appends and a per-session writer lock. Reopening a session by id after a
restart continues the conversation. One host process owns the directory; do not point two hosts at
it.

The journal format is the Rust crate's version 1. It is not interchangeable with a journal written
by the TypeScript TinyAgent, and this host refuses an unknown version with `storage_error` rather than
guessing.

The host keeps one session open at a time. `new_session` and a `prompt` for another id close the
current one first. `shutdown`, stdin EOF and a fatal error all release the lock.

## Redaction

Every outbound envelope, every stderr diagnostic and every tool result pass through the same
redaction: `sk-...`, GitHub and Slack tokens, AWS access key ids, bearer tokens, and
`api_key`/`token`/`password` assignments. The `afterTool` hook replaces a tool result only when
redaction changed it. Provider keys are never printed at all.

## Limits worth knowing

- One tool call at a time, in the order the model asked.
- One active run at a time; other commands answer `busy`.
- No compaction budget is configured, so a long session grows until the provider rejects it. Add a `ContextBudget` when a host-side token counter exists.
- Image content and provider retries for plain provider errors are TinyAgent limitations, not host
  ones. See `rust/crates/tinyagent/README.md`.
