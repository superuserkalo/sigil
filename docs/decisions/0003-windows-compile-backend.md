# 0003 Windows compile backend

Status: accepted
Date: 2026-11-16

## Context

CSDK 12 compiles Deadlock assets through `resourcecompiler.exe`. It is Windows only and has no
license. Source 2 Viewer, `vpkeditcli`, `vpkmerge` and the rest of the pack and inspect path are
cross-platform.

The harness runs on macOS, Linux and Windows. Pretending compile works everywhere would produce
silent failures and bad output.

## Decision

- Model compile as a `BuildBackend` with an explicit `available` state and a reason.
- Ship `UnavailableBackend` first. It reports the missing toolchain and disables the compile step.
- Add `LocalWindowsBackend` using a configured CSDK 12 path and `resourcecompiler.exe`.
- Add a remote backend over a Pi execution environment when a Windows host is available.
- Do not download a compiler from an arbitrary URL. Do not redistribute CSDK 12.
- Packing, inspecting, merging, validating and deploy planning remain cross-platform.

## Consequences

- macOS and Linux users can inspect, edit, pack, validate and deploy existing compiled assets, but
  cannot compile new ones until a backend exists.
- The UI must show capability, not hope. A disabled button with a reason beats a failed run.
- The project workspace and tool contracts stay backend-agnostic.
- A future Wine or Proton backend can be measured against the same interface.

## Alternatives

- Assume Wine or Proton works: unverified and likely fragile with Valve's tooling.
- Require Windows for the whole app: loses the macOS and Linux audience Grimoire already serves.
- Bundle a compiler: license and redistribution problems.
