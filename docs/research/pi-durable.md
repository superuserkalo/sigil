# Pi Durable

Status: active. Pinned to `1.5.2`, which the package marks experimental.
Updated: 2026-11-16

Pi Durable is the durable agent harness from Earendil, shipped alongside Pi 1.0. It is MIT
licensed. It is not the Pi coding agent; it is the framework for building agentic applications,
including coding agents.

## What it provides

- One storage backend per harness: memory, SQLite, JSONL, IndexedDB, or Cloudflare Durable Object.
- Conversations as immutable transcripts, with forks, compaction, reset and handoff.
- Tasks as durable state machines with a checkpoint after every step. Interrupted work resumes when
  the storage is reopened.
- Tool calls as tasks. A tool declared `replay: "safe"` may rerun after a crash; otherwise the model
  receives an interrupted result and decides what to do.
- Extensions as named bundles of tools, prompt sections, hooks, wrappers and task definitions.
- Documents for application state, committed atomically with the transcript.
- Signals and parked tools, so a tool can wait for a human or an external job without holding a
  process.
- Hooks before and after tools, including a `beforeTool` gate that can sleep until approved.
- Per-conversation agent configuration: model, thinking level, tools, extensions, instructions and
  working directory.
- A watchable structural view for UIs, with operations suitable for a remote client, and a task
  graph.
- A model-facing code mode with a QuickJS sandbox (not used yet).

## APIs this repo uses

```ts
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { Harness, createRegistry, defineExtension, defineTool, section } from "@earendil-works/pi-durable";
import { NodeExecutionEnv } from "@earendil-works/pi-durable/env/node";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";
import { CodingTools } from "@earendil-works/pi-durable/tools";
```

- `Harness.open(storage, options, context)` opens the session. Options carry `models`, `registry`,
  `env`, `settings`, `onReport` and optionally `readOnly`.
- `harness.root(context, { agent })` creates or returns the root conversation.
- `conversation.submit({ type: "input", content }, context)` returns a submission; `wait()` resolves
  to a settled record.
- `harness.close(context)` seals storage. Reopen the same file to recover.
- `harness.resume()` enables scheduling and continues interrupted work.
- `conversation.watch(context)` gives a UI the current view and later commits.

## Models and BYOK

Model access is `@earendil-works/pi-ai` (same version, MIT). A `Models` collection holds provider
factories. This repo registers Anthropic, OpenAI, Google, OpenRouter, xAI and DeepSeek explicitly to
keep bundling predictable.

Credentials implement pi-ai's `CredentialStore`:

- `read(providerId)`
- `list()` for non-secret `{ providerId, type }` metadata
- `modify(providerId, fn)` as the only write path
- `delete(providerId)`

`Models.getAuth()` resolves the request auth. A stored credential owns its provider; environment
variables are consulted only when nothing is stored.

The CLI uses a JSON file with `chmod 600`. The desktop app will wrap the same file with Electron
`safeStorage`. Keys never enter the transcript or tool output.

## Constraints that shape this repo

- One process owns a storage at a time. Do not open the same SQLite file from the CLI and the
  desktop app concurrently.
- The API is experimental and changes without notice. Read the installed `.d.ts` files before using
  a new API. `AGENTS.md` repeats this for a reason.
- SQLite uses WAL with `synchronous = NORMAL`. Commits survive process crashes; the newest commit
  may be lost on power loss.
- A `beforeTool` gate reruns on every wake. Keep gate side effects idempotent.
- A tool that writes outside the workspace must not be replay-safe.
- Storage errors fail the harness. The host reopens and resubmits with the same `requestId` for
  exactly-once admission.

## Why this runtime

The product needs long builds, human approvals, crash recovery, resumable conversations and a UI
that can attach late. Pi Durable already models all of those as durable tasks and documents. Building
that loop from scratch would be the project.

Alternatives considered at scaffold time:

- Build a plain agent loop: less machinery, but no crash recovery, approvals or durable transcripts.
- Use Pi the coding agent directly: excellent terminal agent, but not a library shaped for an
  embedded product with its own storage and UI.
- Embed fx or libfx: a complete coding agent, but no Windows native addon, no durable approval
  primitive and a narrower provider surface. See `docs/research/fx-libfx.md` and
  `docs/decisions/0004-embedded-runtime-choice.md`.
- Use a hosted agent API: violates the BYOK and no-telemetry constraints.

## Sources

- [Pi Durable announcement](https://earendil.com/posts/pi-durable/)
- [Pi Durable README](https://github.com/earendil-works/pi/tree/main/packages/durable)
- [Pi AI README](https://github.com/earendil-works/pi/tree/main/packages/ai)
- Installed package types for `@earendil-works/pi-durable@1.5.2` and
  `@earendil-works/pi-ai@1.5.2`
