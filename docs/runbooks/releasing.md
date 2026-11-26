# Releasing Sigil

Status: active. Describes how 0.1.42 was released (2026-11-25): built, signed and published from
the maintainer's Mac, Windows only, with the installer and the stable-name assets. From the next
release, CI adds unsigned Linux and macOS downloads after publishing (see the asset table).
Updated: 2026-11-26

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

| Asset | Made by | Read by |
| --- | --- | --- |
| `Sigil-<version>-windows-x64.zip` | `scripts/package-windows.sh` (cargo-xwin) | the updater, through the signed manifest |
| `Sigil-windows-x64.zip` | a copy of the versioned zip | the stable "latest" link |
| `Sigil-Setup.exe` | `scripts/package-installer.sh` (NSIS, `scripts/installer/sigil.nsi`) | the website and README download link |
| `sigil-update.json` | `scripts/release-sign.mjs manifest`, run by the packaging script | the updater |
| `sigil-update.json.sig` | `scripts/sign-release.sh`, on the maintainer's machine only | the updater |
| `Sigil-linux-x86_64.AppImage` | the `release-assets` workflow, after publishing | the README download link; not the updater |
| `Sigil-macos.dmg` | the `release-assets` workflow, after publishing | the README download link; not the updater |

The two stable names exist because the website and the README link to
`releases/latest/download/Sigil-Setup.exe` and `releases/latest/download/Sigil-windows-x64.zip`.
Every release must carry both, or those links break the moment it becomes the latest. The updater
never reads them; it downloads the versioned zip the signed manifest names.

The last two arrive on their own: publishing the release starts
`.github/workflows/release-assets.yml`, which builds the AppImage on Ubuntu 22.04 and a universal
dmg on macOS from the release tag and uploads both with `gh release upload --clobber`. It never
replaces the five assets above. They are unsigned, not in `sigil-update.json`, and not verified
with a real game install (`docs/runbooks/packaging.md`). If a run fails, rerun it by hand:
`gh workflow run release-assets.yml -f tag=v<version>`. Until a release carries them, the
`releases/latest/download/` links for these two names return 404.

To let Linux or macOS copies update themselves instead, the signed manifest needs their entries
(`Sigil-<version>-macos-<arch>.zip`, `Sigil-<version>-x86_64.AppImage`); see "Adding macOS or
Linux" below. The CI downloads are not those entries.

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

The release is made on the maintainer's machine (macOS) with `cargo-xwin`, `makensis`
(`brew install makensis`), Node 24 and an authenticated `gh`. `vendor/rune` must be present
(`scripts/vendor-rune.sh`). The Locker art in the repository is built into `Sigil.exe`.

1. Write the player-facing notes in `.scratch/package/release-<version>/notes.md`. The release body
   is what Settings > About and updates shows as "What's new": short lines, plain Markdown.
2. Bump `version` in `rust/Cargo.toml` (`[workspace.package]`), refresh the lock file
   (`cd rust && cargo update --workspace --offline`) and commit `chore(release): <version>`.
   Pre-releases use `x.y.z-preview.N`; Sigil orders them before `x.y.z`.
3. From a clean checkout of that commit, run the tests: `cd rust && cargo test --workspace`.
4. Build and check the Windows zip and its unsigned manifest:

   ```bash
   scripts/package-windows.sh        # .scratch/package/Sigil-<version>-windows-x64.zip
                                     # .scratch/package/release-<version>/sigil-update.json
   scripts/verify-windows.sh .scratch/package/Sigil-<version>-windows-x64.zip
   ```

5. Sign. This checks the zip's size and SHA-256 against the manifest, signs, and verifies the result
   with the public key compiled into Sigil:

   ```bash
   cd .scratch/package
   ../../scripts/sign-release.sh release-<version>/sigil-update.json .
   ```

6. Push the release commit to `main`.
7. Make the stable-name assets:

   ```bash
   cp Sigil-<version>-windows-x64.zip Sigil-windows-x64.zip
   ../../scripts/package-installer.sh Sigil-<version>-windows-x64/Sigil.exe <version> "$PWD/Sigil-Setup.exe"
   ```

8. Publish. `gh release create` makes the tag at the release commit and publishes at once:

   ```bash
   gh release create v<version> Sigil-<version>-windows-x64.zip Sigil-windows-x64.zip \
     Sigil-Setup.exe release-<version>/sigil-update.json release-<version>/sigil-update.json.sig \
     --repo superuserkalo/sigil --target <release commit> --title "Sigil <version>" \
     --notes-file release-<version>/notes.md
   ```

9. Check that `gh api repos/superuserkalo/sigil/releases/latest --jq .tag_name` prints
   `v<version>` and that both stable links download.

The new tag also starts the `build-windows` workflow. It sees that the release exists and keeps its
own build as a workflow artifact only, so it never replaces a zip the signed manifest names by hash.
Pushing a tag with no release first is the older path: the workflow then attaches its zip and an
unsigned manifest to a draft release, which still needs the signature, the installer and the
stable-name assets added by hand before it is published.

A release without `sigil-update.json.sig` still shows up in the update check, with "Download it
from the release page" instead of a download.

### Adding macOS or Linux

Before step 5, add the other platforms to the same manifest, then sign once:

```bash
scripts/package-desktop.sh            # on a Mac: adds macos-<arch> to release-<version>/sigil-update.json
gh run download -n Sigil-linux-x86_64 -D .scratch/package/linux   # from the linux-build workflow
node scripts/release-sign.mjs manifest .scratch/package/release-<version>/sigil-update.json \
  --version <version> --platform linux-x86_64 \
  --zip .scratch/package/release-<version>/Sigil-<version>-x86_64.AppImage
```

Linux goes under `extra_platforms` (see above), and both files are uploaded with the release.

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

Nothing in this repository records these as checked:

- The installer on real Windows: SmartScreen on `Sigil-Setup.exe`, the per-user install into
  `%LOCALAPPDATA%\Programs\Sigil`, the shortcuts, uninstall from Windows Settings, and the updater
  replacing `Sigil.exe` inside that folder.
- The swap and rollback on real Windows (renaming the running `Sigil.exe`, a locked `assets`
  file, SmartScreen on the relaunched exe). The plan is tested on temp folders on macOS.
- The `build-windows` workflow's draft-release step, `choco install webp` and the 7-Zip archive.
- A real GitHub release download (the redirect to GitHub's asset host).
- macOS swap of a real `/Applications/Sigil.app`, and Gatekeeper on the swapped bundle.
- The `release-assets` workflow's first run, the dmg on a Mac other than the maintainer's, and
  the AppImage on a real Linux desktop.
- The Linux AppImage swap and rollback on a real desktop (renaming the running AppImage, the
  relaunch under `RUNE_RELAUNCH_AFTER`, the `.deb` hand update), and the `linux-build` workflow's
  first green run.
- A new version that crashes before `main` cannot roll itself back; the user reinstalls from the
  release page.
