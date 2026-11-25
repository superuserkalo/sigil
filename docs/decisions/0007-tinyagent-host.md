# 0007 TinyAgent replaces Pi Durable

Status: accepted
Date: 2026-11-16
Update 2026-11-25: the cutover is complete. The Rust host (`Sigil agent-host`) is the only host, and the TypeScript reference host (`apps/agent-host`) and the Pi Durable harness (`packages/harness`) were removed from the repository.

## Context

The desktop app is moving to native Rust with Rune. The agent host was Pi Durable, a capable but
experimental TypeScript harness. It carries a large runtime surface for this product: documents,
task graphs, watchable views and a specific storage model.

TinyAgent is a small MIT agent SDK owned by this project. The TypeScript v2 implementation already
has the primitives that actually gate the product:

- Durable sessions in an append-only JSONL storage contract with fenced appends.
- Resume after a crash, with a recorded tool call and an explicit idempotent rerun policy.
- Pause and resume for approvals, questions and client-executed tools.
- `beforeTool` and `afterTool` hooks.
- OpenAI Responses, Anthropic Messages and OpenAI-compatible Chat Completions, including local
  servers.
- Streaming events and a replayable journal.
- A protocol with shared conformance tests against a second implementation.

The Rust TinyAgent implementation was protocol v1: in-memory sessions only, no durable pause or
resume. It has since been ported to protocol v2 in `rust/crates/tinyagent`, with durable storage,
resume, pause, hooks and providers.

## Decision

- TinyAgent is the agent host. Pi Durable is no longer the runtime.
- The Rust protocol v2 crate (`rust/crates/tinyagent`) and Rust host (`rust/crates/agent-host`,
  `dah-agent`) are the production host. The TypeScript v2 host remains a reference implementation
  only.
- The TypeScript reference host is vendored at a pinned revision and built by
  `scripts/vendor-tinyagent.sh`; the Rust port is tracked in-repo.
- The host runs as a local Node sidecar and speaks newline-delimited JSON over stdio. The Rust/Rune
  app spawns it and owns the window, credentials, file dialogs, approvals UI and game writes.
- Sessions use TinyAgent's `FileStorage` under the app data directory. One app instance owns the
  session directory.
- Agent tools call the Rust domain core (through `dah-rs`) so there is one implementation of game
  discovery, VPK, index and project rules during the port.
- Approval-gated actions (deploy, launch, `gameinfo.gi` repair, publish, likeness publish) pause the
  run in `beforeTool`. The UI answers with `resume({ result })` or `resume({ error })`.
- BYOK keys come from the environment or the app's credential handshake, never the transcript.
  Chat Completions is the local-model path.
- The Rust crate is forked into `rust/crates/tinyagent` and ported to protocol v2. The Rust host
  lives in `rust/crates/agent-host` (`dah-agent`) and is spawned by the Rune app. The TypeScript host
  in `apps/agent-host` is reference only.
- The Node sidecar is out of the product path. `apps/agent-host` and `packages/harness` stay only
  as references until removed.

## Consequences

- The agent surface shrinks to tools, instructions, sessions and a small RPC protocol.
- Durable approvals and resume are first-class instead of rebuilt around a larger harness.
- TinyAgent's real-provider and Windows support are not yet verified. The host must be exercised
  against a scripted provider first, then a real key, then Windows.
- Two languages remain during the transition. The Node process is headless and has no renderer, so
  it does not carry the Electron performance problem.
- Journals are TinyAgent-specific and not portable across its language implementations yet. A
  later Rust port needs a migration or a stable journal contract.
- The existing `packages/harness` Pi Durable code is legacy until removed. Do not add features to it.

## Alternatives

- Keep Pi Durable: more machinery than the product needs, and it keeps the runtime tied to an
  experimental package with a different durability model.
- Adopt Rust TinyAgent v1 now: no durable sessions, pause or resume. It would regress the approval
  and crash-safety promises.
- Write a new Rust agent loop: recreates storage fencing, resume, pause, providers, hooks and event
  replay. TinyAgent already has them with conformance tests.
- Keep the agent in the Rust UI process: a crashed or wedged model request would take the workbench
  with it, and credentials would cross into the UI process.
