# tinyagent (Rust)

A Rust port of TinyAgent protocol v2, forked from
[superuserkalo/tinyagent](https://github.com/superuserkalo/tinyagent) at revision
`95ba029be92511bd59929aa21dd56d20559468d4` (MIT).

Protocol version 2 is implemented in this crate: durable sessions with fenced appends, resume
after a crash, pause and resume for approvals, `beforeTool` and `afterTool` hooks, typed output,
streaming providers with usage reporting, run limits, and events that replay from the journal. The
CLI, JSONL RPC mode and HTTP server all use the same agent loop and the same `Session`.

The TypeScript host in `apps/agent-host` is still the supported one until this crate is exercised
against a real provider on every platform. This crate is the long-term host.

## Shape

| Type | Role |
| --- | --- |
| `Agent<P>` | One model and tool loop. `Agent::builder` sets limits, hooks, tracing and the ID source. |
| `Session<P>` | A durable conversation. `new`/`with_directory` create or continue; `run`, `resume`, `abandon`, `reconcile` and `compact` write. |
| `Storage` | `load`, fenced `append`, `list`, `release`. `MemoryStorage` and `FileStorage` implement it. |
| `Tool` | JSON Schema arguments, an optional timeout with a grace period, an idempotency flag and an optional `beforeTool` hook. |
| `RunHooks` | How a session journals a run and describes the unfinished part of an interrupted one. A bare `Agent::run` works without it. |
| `AgentEvent` | One ordered event: run start, model request lifecycle, text, tool start and end, compaction, pause, run end with usage. |

A run's events go to one sink (`EventFn`), so delivery applies backpressure and an error stops the
run. `Session::events` rebuilds the same event stream from the journal, page by page, with the
`cursor` of the record each event came from.

## Journal

`FileStorage` keeps one append-only JSONL file per session in a directory. Each line is
`{"payload": "<record json>", "sha256": "<hex>"}`; a torn final line is discarded and the file is
truncated back to the last complete record. A per-session lock file makes a second writer fail
fast, and the fenced append (`sequence` must be `stored count + 1`) is what keeps the journal
correct. Records are `session`, `configuration`, `run_start`, `item`, `tool_intent`, `tool_rerun`,
`tool_resumed`, `tool_paused`, `tool_result`, `run_paused`, `run_commit`, `run_failed`,
`checkpoint` and `reconciled`.

The journal format is version 1 and belongs to this crate. It is not yet interchangeable with the
TypeScript, Python or other journals.

## Resume

A run is durable once its `run_start` record is written. Reopening a session with an open run
exposes `resumable()`, or `recovery_required()` when the interrupted call is not safe to repeat and
the session was opened with `UncertainPolicy::Reconcile`. `resume` continues the run: a finished
tool call is kept, an interrupted idempotent call runs again with the same execution ID
(`<session>:<intent sequence>`), and an interrupted call that is not idempotent is settled as
interrupted and the model is told. A paused run needs `options.result` or `options.error`.

## Not implemented

Each of these is a deliberate gap, not a bug in progress:

- **Image content.** Message content and tool output are text (or JSON serialized to text). Image
  parts are not represented, so a multimodal tool result would be flattened.
- **`extra` request fields.** `RunOptions.extra` reaches providers as `ProviderRequest.extra`; the
  built-in OpenAI and Anthropic providers do not merge it into their request bodies yet.
- **Provider retries for real providers.** The loop retries only `Error::ProviderRetry` (with an
  optional `retry_after`). A provider that reports a plain `Error::Provider` gets one attempt.
- **A default token counter.** Compaction needs a `ContextBudget` with a host-supplied
  `count_tokens`. There is no built-in tokenizer.
- **A Rust `Stream`.** Events are delivered through a callback sink. Wrap it in a channel if you
  need a stream.
- **`inspect` paging.** The RPC state view is assembled by the adapter from `history`, `records`,
  `events` and the getters; there is no single `inspect` method here.
- **Parallel tool calls.** One tool call at a time, in the order the model asked.
- **Stopping host code.** A tool handler that ignores its cancellation token is abandoned after the
  grace period; it is not killed.
- **Journal migration.** A record written in a newer `JOURNAL_VERSION` is refused with
  `Error::Unsupported` instead of guessed at.

## Commands

```sh
cargo check --manifest-path rust/Cargo.toml -p tiny-agent
```

The tests for this port live in `src/storage.rs`, `src/file_storage.rs` and `src/session.rs`, and
cover the storage contract, the torn-record and second-writer rules, pause and resume, reopening a
paused session, an interrupted idempotent rerun, an interrupted non-idempotent settlement, and a
repeated message ID. The provider, CLI, RPC and HTTP test modules are still being ported to the v2
shapes, so `cargo test -p tiny-agent` does not build them yet; run the three modules above once
they do.
