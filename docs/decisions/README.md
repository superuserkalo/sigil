# Decisions

Status: active index.
Updated: 2026-11-16

A decision record captures a boundary that is costly to reverse. Accepted records are append-only:
supersede one with a new record instead of rewriting it.

| Record | Status | Boundary |
| --- | --- | --- |
| `0001-grimoire-fork-and-monorepo.md` | accepted | Shell, license, monorepo layout |
| `0002-durable-agent-runtime.md` | accepted | Runtime, storage, secrets, process split |
| `0003-windows-compile-backend.md` | accepted | Compile is a backend with availability |

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
