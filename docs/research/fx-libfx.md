# fx and libfx

Status: historical. The 2026-11-16 evaluation behind ADR 0004; Pi Durable, its conclusion, was itself superseded by ADR 0007, and no code runs or links fx. Not maintained; the code wins where they differ. Original status: active. Evaluated 2026-11-16 against fx CLI 0.0.9 and libfx 0.0.17.
Updated: 2026-11-16

This note records why the embedded runtime is Pi Durable rather than fx. It is not a dismissal of
fx: as a terminal coding agent it is smaller, faster and more complete than what we would build.
The mismatch is in embedding, platform support and approval semantics.

## What fx is

fx is a coding agent from Vercel Labs, written in Zig as one native binary, Apache-2.0, marked
experimental.

- Runs on macOS and Linux, x86_64 and arm64. No Windows build.
- Built-in tools: find, read, edit, shell, web search, URL read, subagents.
- Permission modes: `auto` (routine work, plus a model check for risky actions), `ask`, and
  `full-access`.
- Sessions under `~/.fx/sessions`, with compaction and full history on disk.
- Extensions: skills, MCP servers, `AGENTS.md`.
- Surfaces: interactive terminal, `fx ask --json` for scripts, `fx acp` over stdio for editors,
  and `libfx` for JavaScript applications.

## What libfx is

libfx is the same agent core as a JavaScript library.

- `createFxAgent()` returns an agent with a model, instructions, tools, provider and durability.
- `agent.session()` opens one conversation; `prompt()` returns a turn that streams events.
- Durability stores the prompt before it is accepted, each step as it runs, and the session stream
  so any server can replay or continue a turn.
- Session stores: `local()` files, `memory()`, `vercel()`, or a Workflow World for Postgres and
  similar backends.
- Tools are JavaScript functions with `inputSchema`, `idempotent` and `writes`. A call receives
  `signal`, `executionId`, `sessionId` and a copied JSON `context`. Results are limited to 64 KiB
  of text and 8 images.
- Providers: AI Gateway, a user's ChatGPT plan, a user's Grok plan, or a custom server that speaks
  the OpenAI Chat Completions API.
- Native addon for Node on macOS and Linux. WebAssembly everywhere else.
- In WebAssembly, tool calls run sequentially and the agent cannot start native processes or reach
  a native filesystem or keychain.
- libfx does not inherit the CLI's filesystem, shell or credential handling. Its tools are only what
  the host provides.
- The docs are explicit that CLI permission prompts do not apply to libfx tools. Authorization is
  the host's job, inside `execute()`.

## Fit against this product

| Requirement | Pi Durable | fx / libfx |
| --- | --- | --- |
| Windows support | Node library, Node's built-in SQLite | fx CLI has no Windows build; libfx has no Windows native addon, WASM only |
| Coding toolset out of the box | `CodingTools` (read, write, edit, bash) | libfx: none, build your own; fx CLI: yes, but not embeddable on Windows |
| BYOK provider breadth | pi-ai: Anthropic, OpenAI, Google, OpenRouter, xAI, DeepSeek and more | AI Gateway, ChatGPT, Grok, or an OpenAI-compatible endpoint |
| Approval before game writes | Signals, parked tasks and a `beforeTool` gate that survives restarts | Implement a blocking check inside `execute()`; no durable approval primitive; CLI permissions do not apply |
| Crash recovery | Tasks, documents, replay policy, storage recovery | Stored steps, `idempotent` rerun policy, session continuation |
| UI integration | Watchable conversation view, task graph, documents, multi-client | Session stream with cursors; build your own state projection |
| Maturity | 1.5.2, MIT, experimental | 0.0.17, Apache-2.0, experimental, frequent 0.0.x migrations |
| Electron packaging | Pure JS plus Node SQLite, no native addon rebuild | Native `.node` addon per OS/arch, or WASM with JSPI; verify ABI and bundling |

## Where fx is the better tool

- A complete coding agent today, with shell, file edits, web access, subagents and permissions.
- A 7.8 MiB binary with millisecond startup and no runtime to install.
- ACP makes it a good editor backend, and MCP makes its tool surface extensible.
- Skills and `AGENTS.md` support are close to what we want for Deadlock domain knowledge.
- `fx ask --json` is a clean scripting and CI surface.

## Recommendation

Keep Pi Durable as the embedded runtime. It matches the three product constraints better:

1. The desktop app must run on Windows, where most Deadlock modders are. libfx has no Windows native
   addon, and its WASM fallback cannot run native tools or expose the CLI's coding toolset.
2. Game install writes need durable, resumable human approval. Pi Durable models that as a parked
   task on a signal. libfx leaves approval inside a tool call, where a crash turns it into an
   interrupted call the model may retry.
3. BYOK means the user's provider choice, not only an OpenAI-compatible one. pi-ai covers the major
   providers directly.

Revisit fx as an optional external lane, not the core:

- `fx acp` on macOS and Linux for users who want a terminal-native agent against the same mod
  workspace.
- An MCP server that exposes the Deadlock tools to fx, so the two share the domain layer.
- A CI path using `fx ask` for batch validation or release notes.

That keeps the option open without committing the product to an experimental 0.0.x native addon.

## Open questions if fx is ever embedded

- Does the libfx native addon load in Electron after `electron-rebuild`, or does it need its own
  ABI build?
- Is JSPI available in the Electron version we ship, and what is the performance of sequential WASM
  tool calls on Windows?
- Does the Workflow World store fit a local-first desktop app, or would we write a custom store?
- Can an approval wait survive a process restart without losing the turn?

## Sources

- [fx site](https://fx.sh/)
- [fx documentation](https://fx.sh/docs)
- [fx repository and README](https://github.com/vercel-labs/fx)
- [libfx overview](https://fx.sh/docs/lib)
- [libfx durability](https://fx.sh/docs/lib/durability)
- [libfx tools](https://fx.sh/docs/lib/tools)
- [libfx providers](https://fx.sh/docs/lib/providers)
- [libfx WebAssembly](https://fx.sh/docs/lib/webassembly)
- [libfx package metadata](https://www.npmjs.com/package/libfx)
- Local FX source review in `radian-coding-agent/docs/research/2026-08-24-fx-terminus-terminal-tools.md`
