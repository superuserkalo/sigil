# Releasing Sigil

Status: active. The signing flow and the updater are tested locally against a fake release.
0.1.1 is the first published release; its Windows swap is the first run on Windows.
Updated: 2026-11-17

Sigil updates itself from GitHub releases of `superuserkalo/sigil`. A release is trusted only when
its manifest carries a valid Ed25519 signature by the key compiled into Sigil. CI builds and
uploads; only the maintainer's machine signs.

## The signing key

| Item | Where |
| --- | --- |
| Private key | `~/.config/sigil/update-signing.key` (PKCS#8 PEM, mode 600, folder mode 700) |
| Public key | `UPDATE_PUBLIC_KEY` in `rust/crates/desktop/src/updater.rs` |
| Key id | `0cd4b3b09aeb594e` (first 8 bytes of SHA-256 over the raw public key) |

Rules:

- The private key never enters the repository, CI, a log, a chat or a release. `release-sign.mjs`
  never prints it, and refuses a key file other users can read.
- Back it up once, offline: copy the file to an encrypted USB drive or a password manager's file
  attachment, and keep a second copy somewhere else. Without it no installed Sigil will accept a
  new update; users would have to download a release by hand.
- If it leaks: generate a new key, ship one release signed by the old key whose
  `TRUSTED_KEYS` lists the new key, then a release that drops the old key. Say so in the notes.
- Rotation follows the same two steps. `TRUSTED_KEYS` may hold more than one key; the signature
  file names the key id it was made with.

A new key (only for a new project or a rotation; it refuses to overwrite):

```bash
node scripts/release-sign.mjs keygen            # writes ~/.config/sigil/update-signing.key
node scripts/release-sign.mjs pubkey            # prints the public key and key id again
```

## What a release carries

| Asset | Made by |
| --- | --- |
| `Sigil-<version>-windows-x64.zip` | the `build-windows` workflow on a tag, or `scripts/package-windows.sh` |
| `Sigil-<version>-macos-arm64.zip` | `scripts/package-desktop.sh` on a Mac (`Sigil.app` at the top) |
| `Sigil-<version>-x86_64.AppImage` | the `linux-build` workflow, or `scripts/package-linux.sh` on Linux |
| `sigil-update.json` | the same scripts (`release-sign.mjs manifest`), unsigned |
| `sigil-update.json.sig` | `scripts/sign-release.sh`, on the maintainer's machine only |

The manifest:

```json
{
  "format": 1,
  "product": "sigil",
  "version": "0.2.0",
  "published": "2026-11-20T10:00:00Z",
  "platforms": {
    "windows-x86_64": { "asset": "Sigil-0.2.0-windows-x64.zip", "size": 41234567, "sha256": "<hex>" },
    "macos-aarch64": { "asset": "Sigil-0.2.0-macos-arm64.zip", "size": 39876543, "sha256": "<hex>" }
  }
}
```

A Windows release is one file, `Sigil.exe` (the art is built in), so its entry has no
`assets` flag. Older installs that kept an `assets/` folder beside the executable leave it in
place; the new executable no longer reads it.

Linux goes under a separate top-level key, `extra_platforms`, and its asset is the AppImage
itself, not a zip:

```json
{
  "extra_platforms": {
    "linux-x86_64": { "asset": "Sigil-0.2.0-x86_64.AppImage", "size": 89234567, "sha256": "<hex>" }
  }
}
```

Sigil 0.1.x refuses an unknown key under `platforms` and ignores `extra_platforms`, so adding
Linux still updates every older Windows and macOS copy. A manifest with no Windows or macOS entry
cannot be signed (`scripts/release-sign.mjs` refuses it); the Linux-only manifest a Linux build
writes stays unsigned and is merged into the release manifest first. See
`docs/runbooks/packaging.md`.

The signature covers `sigil-update-manifest v1\n` followed by the exact manifest bytes, so any
edit after signing (even whitespace) needs a new signature.

## Steps

1. Bump `version` in `rust/Cargo.toml` (`[workspace.package]`). Pre-releases use
   `x.y.z-preview.N`; Sigil orders them before `x.y.z`.
2. Commit, then tag `v<version>` and push the tag. The `build-windows` workflow builds the zip,
   writes the unsigned `sigil-update.json`, and attaches both to a draft release (marked
   pre-release when the tag has a `-`). Nothing is public yet.
3. On a Mac, build the macOS update zip and add its entry to the same manifest:

   ```bash
   gh release download v<version> -p sigil-update.json -p 'Sigil-*-windows-x64.zip' \
     -D .scratch/package/release-<version>
   scripts/package-desktop.sh            # adds macos-<arch> to .scratch/package/release-<version>/sigil-update.json
   mv .scratch/package/Sigil-<version>-macos-*.zip .scratch/package/release-<version>/
   ```

   Skip this step for a Windows-only release.
4. Add Linux from the `linux-build` workflow's `Sigil-linux-x86_64` artifact (manual dispatch or a
   push to `main`; nothing is published there). Download the artifact, then add its entry:

   ```bash
   gh run download -n Sigil-linux-x86_64 -D .scratch/package/linux
   mv .scratch/package/linux/Sigil-<version>-x86_64.AppImage .scratch/package/release-<version>/
   node scripts/release-sign.mjs manifest .scratch/package/release-<version>/sigil-update.json \
     --version <version> --platform linux-x86_64 \
     --zip .scratch/package/release-<version>/Sigil-<version>-x86_64.AppImage
   ```

   This writes `extra_platforms.linux-x86_64`; `platforms` is untouched. Skip this step for a
   release without Linux.
5. Sign. This checks each listed zip and AppImage's size and SHA-256 against the manifest, signs,
   and verifies the result with the public key compiled into Sigil:

   ```bash
   scripts/sign-release.sh .scratch/package/release-<version>/sigil-update.json
   ```

6. Upload and publish:

   ```bash
   cd .scratch/package/release-<version>
   gh release upload v<version> sigil-update.json sigil-update.json.sig \
     Sigil-*-macos-*.zip Sigil-*-x86_64.AppImage --clobber
   gh release edit v<version> --draft=false --notes-file notes.md
   ```

   The release body is what Settings > About and updates shows as "What's new". Write it for
   players: short lines, plain Markdown.

A release without `sigil-update.json.sig` still shows up in the update check, with "Download it
from the release page" instead of a download.

## What the updater does

`rust/crates/desktop/src/updater.rs`:

- Check (Settings, at startup when "Check for Sigil updates" is on, or on the chosen interval):
  `GET api.github.com/repos/superuserkalo/sigil/releases/latest` (Stable) or
  `releases?per_page=10` (Preview), then the manifest and signature from
  `github.com/superuserkalo/sigil/releases/download/...`. Offline mode blocks all of it.
- Refuses: a bad or unknown-key signature, a manifest whose version is not the release tag, any
  version not newer than the running one (no downgrades), a platform the manifest does not list.
- Download: streams the asset to `<data>/updates/download/`, stops at the signed size, checks size
  and SHA-256, unpacks a zip into `<data>/updates/staged/<version>/tree` (no paths outside it, no
  links) or moves the AppImage there as is, checks the tree holds `Sigil.exe`, a `Sigil.app`
  reporting that version, or an x86-64 AppImage, and writes `<data>/updates/pending.json`.
- Install ("Restart to update", or the next launch): copies the tree to `.sigil-update-new`
  beside the install (same volume), renames `Sigil.exe` to `Sigil.exe.old` (and `assets` to
  `assets.old`), `Sigil.app` to `.Sigil.app.old`, or the running AppImage to a hidden
  `.AppImage.old` under the player's file name, moves the new files in, writes
  `<data>/updates/applied.json`, starts the new version and quits. A failed step undoes the
  renames before it.
- Rollback: the new version marks `applied.json` confirmed a second after its first frame. A
  launch that finds the record unconfirmed from an earlier launch restores the `.old` files and
  starts the previous version; Settings then says the update was undone. A confirmed update
  deletes the `.old` files on the next start.
- On Linux it only runs for the AppImage: the layout comes from `$APPIMAGE`, never from the
  program inside the read-only mount. A `.deb`, a distro package or a dev build is told to
  download from the release page. It does not run for a development build (not `Sigil.exe`, not
  inside `Sigil.app`, not an AppImage), or a macOS app still in App Translocation (opened from
  Downloads; move it to Applications).

Debug builds read `SIGIL_UPDATE_API` to point the check at a local server; release builds ignore
it. The tests in `updater.rs` use it that way.

## Not verified yet

- The swap and rollback on real Windows (renaming the running `Sigil.exe`, a locked `assets`
  file, SmartScreen on the relaunched exe). The plan is tested on temp folders on macOS.
- The `build-windows` workflow's release step, `choco install webp` and the 7-Zip archive.
- A real GitHub release download (the redirect to GitHub's asset host).
- macOS swap of a real `/Applications/Sigil.app`, and Gatekeeper on the swapped bundle.
- The Linux AppImage swap and rollback on a real desktop (renaming the running AppImage, the
  relaunch under `RUNE_RELAUNCH_AFTER`, the `.deb` hand update), and the `linux-build` workflow's
  first green run.
- A new version that crashes before `main` cannot roll itself back; the user reinstalls from the
  release page.
