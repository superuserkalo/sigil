# 0002 Durable agent runtime

Status: accepted
Date: 2026-11-16

## Context

The agent runs long builds, waits for human approvals, survives crashes, and needs a UI that can
attach late. Model access must be BYOK. Keys must never reach the renderer or a transcript.

Pi Durable provides durable conversations, tasks, documents, signals, approvals and a watchable
view. It is experimental, but it models exactly the hard parts. It shares pi-ai with the Pi coding
agent, so provider support is broad.

## Decision

- Use Pi Durable, pinned to an exact version. Read installed types before using a new API.
- One SQLite storage per app data directory. One process owns it.
- Run the harness in a Node utility process, not the Electron main process and not the renderer.
- The main process owns decrypted credentials and all game-install writes. The agent host receives
  provider auth through a narrow handshake.
- One conversation per mod project. The root conversation is the general workbench.
- Tools are durable tasks. Only reads and pure computation are `replay: "safe"`.
- Approval gates are `beforeTool` hooks that park on a signal. Deploy, launch, `gameinfo.gi` repair
  and publish require a decision.

## Consequences

- Crash recovery, long waits and resumable approvals are runtime features, not bespoke code.
- The app inherits an experimental API. Version bumps need a migration pass and type review.
- Two processes must agree on storage ownership. The CLI uses a separate data directory by default.
- A wedged agent can be restarted without losing the transcript.
- Some Pi Durable concepts leak into the UI model: conversations, tasks, views, approvals.

## Alternatives

- Plain in-process agent loop: no durable approvals or crash recovery.
- Pi coding agent as a subprocess: good terminal UX, wrong storage and UI boundary.
- Hosted agent service: breaks BYOK and no-telemetry.
