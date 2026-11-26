#!/usr/bin/env bash
# Package the desktop workbench, the agent sidecar and the CLI.
#
# This builds release binaries and stages them next to the notices. It never
# bundles CSDK 12, game files, VPKs, credentials or logs. A macOS bundle is
# assembled when the host is macOS; other hosts get a portable folder plus a
# zip. Windows packaging is produced on Windows, because the sidecar is a native
# binary and a cross build is unverified.
#
#   scripts/package-desktop.sh                          # host target
#   scripts/package-desktop.sh universal-apple-darwin   # macOS arm64 + x86_64 in one binary (lipo)
#   scripts/package-desktop.sh [target] --dmg <path>    # macOS: also write Sigil.app into a dmg
#
# On macOS the bundle is signed ad hoc (`codesign -s -`): no Developer ID, no notarization, so
# Gatekeeper still blocks a downloaded copy until the player allows it (docs/runbooks/packaging.md).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

target=""
dmg=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dmg) dmg="${2:?--dmg needs a path}"; shift 2 ;;
    *) target="$1"; shift ;;
  esac
done
version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -1)"
version="${version:-0.0.0}"
host="$(rustc -vV | sed -n 's/^host: //p')"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
out="$root/.scratch/package/${target:-$host}-$version"

echo "== build =="
if [ "$target" = "universal-apple-darwin" ]; then
  # One binary for Apple silicon and Intel: build each slice, then join them with lipo.
  for slice in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --manifest-path rust/Cargo.toml --release -p dah-desktop --target "$slice"
  done
  mkdir -p "rust/target/$target/release"
  lipo -create -output "rust/target/$target/release/dah-desktop" \
    rust/target/aarch64-apple-darwin/release/dah-desktop \
    rust/target/x86_64-apple-darwin/release/dah-desktop
  lipo -info "rust/target/$target/release/dah-desktop"
else
  cargo build --manifest-path rust/Cargo.toml --release -p dah-desktop \
    ${target:+--target "$target"}
fi

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
                        (only in a build with the agent feature)

Set DAH_AGENT_BIN or DAH_RS_BIN to use a standalone build instead. No CSDK,
game file, VPK, key or log is included.
TXT

if [[ "$host" == *darwin* && ( -z "$target" || "$target" == *apple-darwin ) ]]; then
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
  cp -R rust/crates/desktop/assets/locker "$app/Contents/Resources/locker"

  # Ad-hoc signature: Apple silicon refuses to run an unsigned binary, and the bundle seal covers
  # Info.plist and Resources. It is not a Developer ID, so it does not satisfy Gatekeeper.
  codesign --force --deep -s - "$app"
  codesign --verify --deep --strict "$app"

  # The update zip: Sigil.app at the top, for the in-app updater. Its entry goes into the
  # unsigned release manifest; sign it with scripts/sign-release.sh (docs/runbooks/releasing.md).
  case "${target:-$host}" in
    universal-apple-darwin) arch="universal"; platforms=(macos-aarch64 macos-x86_64) ;;
    aarch64-*) arch="arm64"; platforms=(macos-aarch64) ;;
    *) arch="x86_64"; platforms=(macos-x86_64) ;;
  esac
  update_zip="$root/.scratch/package/Sigil-$version-macos-$arch.zip"
  rm -f "$update_zip"
  (cd "$out" && zip -qry "$update_zip" Sigil.app)
  manifest="$root/.scratch/package/release-$version/sigil-update.json"
  mkdir -p "$(dirname "$manifest")"
  for platform in "${platforms[@]}"; do
    node "$root/scripts/release-sign.mjs" manifest "$manifest" --version "$version" \
      --platform "$platform" --zip "$update_zip"
  done
  echo "update zip: $update_zip"
  echo "manifest:   $manifest (unsigned)"

  if [ -n "$dmg" ]; then
    echo "== dmg =="
    # Sigil.app beside an Applications link, compressed. hdiutil now and then reports a busy
    # resource on CI runners, so it gets three tries.
    stage="$root/.scratch/package/dmg-$version"
    rm -rf "$stage"
    mkdir -p "$stage" "$(dirname "$dmg")"
    ditto "$app" "$stage/Sigil.app"
    ln -s /Applications "$stage/Applications"
    rm -f "$dmg"
    for try in 1 2 3; do
      if hdiutil create -volname "Sigil $version" -srcfolder "$stage" -ov -format UDZO "$dmg"; then
        break
      fi
      [ "$try" -lt 3 ] || exit 1
      sleep 5
    done
    rm -rf "$stage"
    hdiutil verify "$dmg"
    printf 'dmg: %s\n  bytes:  %s\n  sha256: %s\n' "$dmg" "$(wc -c < "$dmg" | tr -d ' ')" \
      "$(shasum -a 256 "$dmg" | cut -d' ' -f1)"
  fi
elif [ -n "$dmg" ]; then
  echo "--dmg needs a macOS host and a macOS target" >&2
  exit 1
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
