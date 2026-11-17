# 0012 Sigil updates itself from signed GitHub releases

Status: accepted
Date: 2026-11-17

## Context

Sigil ships as `Sigil.exe` plus `assets/` in a zip on Windows and as `Sigil.app` on macOS. The
update check only told the user a release existed. Grimoire uses electron-updater (no
auto-download, changelog first, manual on its ad-hoc signed macOS build) and Deadlock Mod Manager
uses the Tauri updater (a minisign key in the app, a `latest.json` per channel, a fixture endpoint
for its end-to-end runs). Sigil has neither runtime, and replacing its own executable is a write
outside the app data directory, so the boundary needs a record.

## Decision

- A release carries `sigil-update.json` (version, and per platform the zip name, size, SHA-256
  and whether `assets/` is replaced) and `sigil-update.json.sig`: Ed25519 over
  `sigil-update-manifest v1\n` plus the manifest bytes, in a three-line format with a key id.
  The public key is compiled in (`TRUSTED_KEYS` in `updater.rs`); verification uses `ring`, which
  rustls already links. The private key lives only on the maintainer's machine
  (`~/.config/sigil/update-signing.key`); `scripts/release-sign.mjs` signs with Node's crypto. CI
  builds and attaches the unsigned manifest to a draft release; it never sees the key.
- Sigil refuses an unsigned or wrongly signed manifest, a manifest whose version is not the
  release tag, any version not newer than the running one, a zip whose size or hash differs, and
  archive paths that leave the staging folder or are links.
- The download and staging go to the app data directory. The swap runs only on "Restart to
  update" or at the next launch, renames the current program aside (`Sigil.exe.old`,
  `.Sigil.app.old`), and keeps it until the new version has drawn a frame and started once more.
  A launch that finds the new version unconfirmed from an earlier launch rolls back.
- Only `api.github.com` and the repository's release download path (with GitHub's redirect to
  its asset host) are contacted. Offline mode blocks them. No identifier is sent.

## Consequences

- Users get updates without a second tool, and a compromised GitHub account alone cannot push
  code to them: it would also need the signing key.
- Losing the key strands installed copies on their version; the runbook requires an offline
  backup. Rotation needs one release signed by the old key that trusts the new one.
- A version that crashes before `main` cannot roll itself back.
- The Windows swap and relaunch rely on Rune's relaunch wait (`RUNE_RELAUNCH_AFTER`); a Rune bump
  that renames it must update `updater.rs`.
- No Developer ID or Authenticode signing yet: SmartScreen and Gatekeeper behaviour on the
  swapped binary is unverified.
