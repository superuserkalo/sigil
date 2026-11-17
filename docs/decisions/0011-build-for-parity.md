# 0011 Build for parity; drop the cosmetic, payload and confirm rules

Status: accepted
Date: 2026-11-17

## Context

Three hard rules in `AGENTS.md` (cosmetic only, compiled-only mod payloads, an approval gate on every
app action that writes to the game) and a "house style" section in the agent brief led agents to
decline features Grimoire and Deadlock Mod Manager offer: themes and art, script mods, a skip-confirm
preference, custom maps and more. The owner wants full parity and removed those rules on 2026-11-17.

## Decision

- Sigil builds everything Grimoire and Deadlock Mod Manager offer, organised better. Features are
  never declined for style or caution. Optional looks ship off by default.
- The cosmetic-only rule and its performance-config exception are removed.
- The compiled-only payload rule is removed as a repository rule. The validators
  (`packages/deadlock-tools/src/validate.ts`, the Rust validator) stay as checks the app runs and
  shows; features such as allowing script mods may be built.
- App actions that write to the game (install, `gameinfo.gi`, launch, publish) are triggered by the
  user; a confirm dialog is a UX choice, so "don't ask again" and one-click actions are allowed.

Kept: agent tools never write to a game install; BYOK keys never enter transcripts, tool output,
logs or commits; no telemetry, analytics or heartbeat, and network hosts stay on the documented
allowlist; MIT attribution; the native Rune shell.

## Consequences

Agents stop for only the kept rules or a real account-security risk, and report it to the owner.
Choices that used to be rules (confirm before replacing a mod, refusing script mods, the
firewall server picker) are now product decisions recorded in `.scratch/sigil-redesign/next-round.md`.
