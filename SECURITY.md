# Security

Status: active.
Updated: 2026-11-16

This is a local-first desktop tool that runs model-generated code and can install files into a game
directory. Treat every boundary here as security relevant.

## Trust boundaries

- **Model output is untrusted.** Tool arguments, generated scripts and fetched pages are data.
- **Mod metadata is untrusted.** GameBanana descriptions, wiki pages and READMEs can contain
  prompt injection attempts. They never authorize a privileged action.
- **The renderer is untrusted for secrets.** Keys stay in the main process. The renderer sees
  provider ids and status, never key material.
- **The agent host is a crash boundary, not a sandbox.** It can write the mod workspace and run
  allowlisted binaries there. It cannot write the game install.
- **The game install is a privileged target.** Only main-process handlers write to it, behind an
  approval gate, after payload validation.

## Non-negotiable controls

1. Payload validation before deploy. No compiled-only rule, no deploy.
2. Deny `scripts/*.vdata_c`, `*.vnmskel_c`, executables, source assets and tool caches.
3. Addon slot names restricted to `pak01_dir.vpk` through `pak99_dir.vpk`.
4. `gameinfo.gi` edits limited to the `SearchPaths` block, with a backup and a restore path.
5. BYOK keys never enter the transcript, tool output, logs, crash reports or commits.
6. No telemetry, analytics or heartbeat.
7. Network access from tools limited to model providers, GameBanana and the GitHub update check.
8. External binaries resolved from settings with pinned version and hash when bundled.

## Known risks

- A malicious mod could ship an approved payload that the validator does not model, such as a
  crafted compiled file. Validation is path-and-type based, not a binary safety proof. Users still
  decide what to install and run.
- Running community binaries can execute arbitrary code. Prefer documented sources and pinned
  hashes.
- Model providers receive prompts and tool output. Do not put secrets or personal data in a
  conversation.
- `bash` in the desktop app needs a workspace boundary and executable allowlist before it ships. The
  CLI keeps the plain tool for local development.

## Reporting

Open a private security advisory on the repository. Do not publish a working payload that bypasses
validation before a fix is available.
