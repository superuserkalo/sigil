#!/usr/bin/env bash
# Package the desktop workbench, the agent sidecar and the CLI.
#
# This builds release binaries and stages them next to the notices. It never
# bundles CSDK 12, game files, VPKs, credentials or logs. A macOS bundle is
# assembled when the host is macOS; other hosts get a portable folder plus a
# zip. Windows packaging is produced on Windows, because the sidecar is a native
# binary and a cross build is unverified.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

target="${1:-}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -1)"
version="${version:-0.0.0}"
host="$(rustc -vV | sed -n 's/^host: //p')"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
out="$root/.scratch/package/${target:-$host}-$version"

echo "== build =="
cargo build --manifest-path rust/Cargo.toml --release -p dah-desktop \
  ${target:+--target "$target"}

bindir="rust/target"
if [ -n "$target" ]; then
  bindir="rust/target/$target"
fi
bindir="$bindir/release"

exe=""
if [[ "$host" == *windows* || "$target" == *windows* ]]; then
  exe=".exe"
fi

echo "== stage =="
rm -rf "$out"
mkdir -p "$out/bin"
cp "$bindir/dah-desktop$exe" "$out/bin/"
cp THIRD_PARTY_NOTICES.md "$out/" 2>/dev/null || true
cp LICENSE "$out/" 2>/dev/null || true
cp README.md "$out/" 2>/dev/null || true
cp docs/runbooks/packaging.md "$out/PACKAGING.md" 2>/dev/null || true

# One executable: the CLI and the agent host are subcommands of the app.
cat > "$out/bin/README.txt" <<'TXT'
dah-desktop             native Rust and Rune workbench (Sigil)
dah-desktop cli ...     the dah CLI
dah-desktop agent-host  the TinyAgent JSONL host, started by the app as a child process

Set DAH_AGENT_BIN or DAH_RS_BIN to use a standalone build instead. No CSDK,
game file, VPK, key or log is included.
TXT

if [[ "$host" == *darwin* && -z "$target" ]]; then
  echo "== app bundle =="
  app="$out/Sigil.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$out/bin/dah-desktop" "$app/Contents/MacOS/dah-desktop"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Sigil</string>
  <key>CFBundleIdentifier</key><string>dev.deadlock.harness</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>dah-desktop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>Sigil</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <!-- "Install with Sigil" links (rust/crates/desktop/src/deeplink.rs, docs/design/deep-links.md).
       LaunchServices registers the scheme when the bundle is installed; Rune receives the link
       as a get-URL Apple event (App::on_open_url). -->
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key><string>dev.deadlock.harness.install-link</string>
      <key>CFBundleURLSchemes</key><array><string>sigil</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
  cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/" 2>/dev/null || true
  cp rust/crates/desktop/assets/brand/Sigil.icns "$app/Contents/Resources/Sigil.icns"
  "$root/scripts/pack-locker-art.sh" "$app/Contents/Resources/locker"

  # The update zip: Sigil.app at the top, for the in-app updater. Its entry goes into the
  # unsigned release manifest; sign it with scripts/sign-release.sh (docs/runbooks/releasing.md).
  case "$(uname -m)" in
    arm64) arch="arm64"; platform="macos-aarch64" ;;
    *) arch="x86_64"; platform="macos-x86_64" ;;
  esac
  update_zip="$root/.scratch/package/Sigil-$version-macos-$arch.zip"
  rm -f "$update_zip"
  (cd "$out" && zip -qry "$update_zip" Sigil.app)
  manifest="$root/.scratch/package/release-$version/sigil-update.json"
  mkdir -p "$(dirname "$manifest")"
  node "$root/scripts/release-sign.mjs" manifest "$manifest" --version "$version" \
    --platform "$platform" --zip "$update_zip"
  echo "update zip: $update_zip"
  echo "manifest:   $manifest (unsigned)"
fi

echo "== manifest =="
(
  cd "$out"
  {
    printf '{\n'
    printf '  "product": "deadlock-mod-manager",\n'
    printf '  "version": "%s",\n' "$version"
    printf '  "host": "%s",\n' "$host"
    printf '  "target": "%s",\n' "${target:-$host}"
    printf '  "packagedAt": "%s",\n' "$stamp"
    printf '  "binaries": [\n'
    first=1
    for file in bin/dah-desktop$exe; do
      [ -f "$file" ] || continue
      size="$(wc -c < "$file" | tr -d ' ')"
      sha="$(shasum -a 256 "$file" | cut -d' ' -f1)"
      [ $first -eq 0 ] && printf ',\n'
      first=0
      printf '    {"path": "%s", "bytes": %s, "sha256": "%s"}' "$file" "$size" "$sha"
    done
    printf '\n  ],\n'
    printf '  "includes": ["dah-desktop (cli and agent-host subcommands)", "THIRD_PARTY_NOTICES.md"],\n'
    printf '  "excludes": ["CSDK 12", "game files", "VPKs", "credentials", "logs", "vendor checkouts"],\n'
    printf '  "unverified": ["Windows install and launch path", "in-game deploy", "real resourcecompiler run", "real Meshy or local model backend"]\n'
    printf '}\n'
  } > manifest.json
  cat manifest.json
)

echo "== archive =="
if command -v zip >/dev/null 2>&1; then
  (cd "$out" && zip -qry "../$(basename "$out").zip" .)
  echo "archive: $(dirname "$out")/$(basename "$out").zip"
else
  echo "zip is not available; the folder is the artifact"
fi

echo "staged: $out"
