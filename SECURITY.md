# Security policy

Status: active.
Updated: 2026-11-25

Sigil is a desktop app that downloads archives from the internet, writes into a game folder,
updates itself and talks to a small server of its own. Reports about any of that are welcome.

## Supported versions

Fixes go into the latest release only. Sigil updates itself from signed GitHub releases, so a fix
reaches players as a new version; there are no back-ported patch releases. Check that you can
reproduce a problem on the [latest release](https://github.com/superuserkalo/sigil/releases/latest)
before reporting it.

## Reporting a vulnerability

Please do not post details of a vulnerability in a public issue, discussion or pull request.

- If the repository's **Security** tab offers **Report a vulnerability**, use it. That opens a
  private advisory only the maintainer can see.
- Otherwise, open an issue that says only that you have a security report, with no details, and the
  maintainer will reply with a private way to send it.

Include the Sigil version, what an attacker controls (a mod archive, a link, a network position),
what they can achieve, and the steps or a proof of concept. Do not publish a working exploit before
a fix is released. You will get an answer as soon as the maintainer can; this is a one-person
project, so give it a few days.

## In scope

- **Mod archive handling.** Downloading, extracting (ZIP, 7z, RAR) and installing mods from
  GameBanana, a pasted link, a profile or pack import, a server join or a 1-click link: path
  traversal, links, writing outside the addons folders, decompression bombs, or a mod running code
  through Sigil.
- **The updater.** The release manifest's Ed25519 signature check, the download and its size and
  SHA-256 checks, unpacking, and the swap and rollback of `Sigil.exe`
  (`rust/crates/desktop/src/updater.rs`, `docs/runbooks/releasing.md`).
- **Sigil's Worker** (`services/telemetry`): the opt-in telemetry, crash and diagnostics routes, the
  mod hash lookup, the catalog search index and the community crosshair library. That includes
  ways to read or delete someone else's data, bypass the limits, or get personal data past the
  scrubbing.
- **Game folder writes.** `gameinfo.gi` edits, the vanilla stash, autoexec and launch options, and
  anything that lets an input write where it should not.
- **Local listeners and handlers.** The `sigil://` link handler, the opt-in DeadlockForge loopback
  listener, and the map download server and UPnP mapping used when hosting a server.
- **Secrets.** Model keys, the proxy password and GameBanana tokens leaking into logs, crash
  reports, diagnostics or the agent transcript.

Out of scope: what a mod does inside the game once installed (report that to its creator on
GameBanana), Deadlock or Steam themselves, and reports that need an attacker who already controls
the player's Windows account.

## How Sigil is meant to behave

- Mod content is untrusted. Sigil lists an archive's files before writing them, flags programs and
  scripts, and installs mods only as `pakNN_dir.vpk` files in the addons folders. The check is based on
  paths and file types; it is not a proof that a compiled file is harmless.
- `gameinfo.gi` edits are limited to the `SearchPaths` lines Sigil manages, plus the tagged lines of
  a Performance config or a server join, each with a backup.
- An update is installed only when its manifest carries a valid signature by a key compiled into
  Sigil, and its file matches the signed size and hash. A failed start rolls back.
- Nothing about the player leaves the PC without their yes: usage events, crash reports, match data
  sharing and product data are opt-ins, off by default. There is no third-party analytics and no
  heartbeat. Every host is listed in [docs/privacy.md](docs/privacy.md).
- Model keys stay in the OS keychain and never enter the agent transcript, tool output or logs.
  Agent tools never write to the game install, and the agent host has no general shell tool.
- Mod descriptions, GameBanana metadata and other fetched text are data, never instructions to the
  agent.
