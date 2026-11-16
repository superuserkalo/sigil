# 0004 Embedded runtime: Pi Durable over libfx

Status: accepted. Superseded by `0007-tinyagent-host.md` for the runtime choice. The fx and libfx rejection still stands.
Date: 2026-11-16

## Context

fx from Vercel Labs is a tiny native coding agent with a JavaScript embedding library, libfx. It has
a complete built-in coding toolset, ACP, MCP, skills, sessions and permission modes. On paper it is
less work than assembling our own agent.

The product runs an Electron desktop app on Windows, macOS and Linux. It writes to a game install
only after a durable human approval, and it must support BYOK across providers.

## Decision

- Keep Pi Durable as the embedded agent runtime.
- Do not embed libfx or the fx CLI as the core.
- Treat fx as an optional external lane later: `fx acp` on macOS and Linux, an MCP server that
  exposes the same Deadlock tools, or `fx ask` for CI.
- Revisit only if Windows support and durable approvals appear in libfx, or if the product drops
  Windows.

## Consequences

- We own the coding tool surface, but Pi Durable's `CodingTools` already covers read, write, edit and
  bash.
- We keep one cross-platform runtime with no native addon beyond Node's built-in SQLite.
- Approvals are durable tasks that park on a signal, so a crash does not turn a deploy decision into
  an interrupted tool call.
- Provider breadth stays with pi-ai rather than an OpenAI-compatible gateway.
- We give up fx's ready-made permissions, subagents, skills and web tools. Some can be rebuilt or
  attached over MCP later.
- fx stays available as a reference and a possible power-user surface, not a dependency.

## Alternatives

- **Embed libfx.** Best JavaScript path, but no Windows native addon, WASM cannot run native tools,
  no durable approval primitive, and a 0.0.x API. Details in `docs/research/fx-libfx.md`.
- **Spawn the fx CLI and speak ACP.** Clean editor-style integration and a complete agent, but the
  CLI does not run on Windows and the approval UI is ours anyway.
- **Build a plain agent loop.** Maximum control, but recreates durability, approvals, replay policy,
  transcripts and multi-client views.
