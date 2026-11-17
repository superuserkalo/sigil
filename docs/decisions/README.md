# Decisions

Status: active index.
Updated: 2026-11-17

A decision record captures a boundary that is costly to reverse. Accepted records are append-only:
supersede one with a new record instead of rewriting it.

| Record | Status | Boundary |
| --- | --- | --- |
| `0001-grimoire-fork-and-monorepo.md` | accepted | Shell, license, monorepo layout |
| `0002-durable-agent-runtime.md` | accepted | Runtime, storage, secrets, process split |
| `0003-windows-compile-backend.md` | accepted | Compile is a backend with availability |
| `0004-embedded-runtime-choice.md` | accepted | Pi Durable stays the runtime; fx is optional |
| `0005-reference-images-and-likeness.md` | accepted | Reference images and likeness stay local and consent-based |
| `0006-native-rust-rune-desktop.md` | accepted | Native Rune desktop; Rust core and CLI |
| `0007-tinyagent-host.md` | accepted | TinyAgent host; durable sessions and approvals |
| `0008-gamebanana-approved-handoff.md` | accepted | Approved GameBanana handoff and verification |
| `0009-deadworks-servers.md` | accepted | Deadworks servers: third-party list, untrusted content, join writes |
| `0010-one-executable.md` | accepted | The CLI and the agent host fold into Sigil as `cli` and `agent-host` subcommands |
| `0011-build-for-parity.md` | accepted | Build for parity with Grimoire and Deadlock Mod Manager; the cosmetic, payload and confirm rules are dropped |
| `0012-signed-self-update.md` | accepted | Sigil updates itself from Ed25519-signed GitHub releases, with rollback |

Use this template:

```markdown
# NNNN Title

Status: proposed | accepted | superseded by NNNN
Date: YYYY-MM-DD

## Context

What forces a decision.

## Decision

What we chose.

## Consequences

What becomes easier, what becomes harder, what must be revisited.

## Alternatives

What we rejected and why.
```
