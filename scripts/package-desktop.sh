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
cargo build --manifest-path rust/Cargo.toml --release -p dah-desktop -p dah-agent -p dah-rs \
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
cp "$bindir/dah-agent$exe" "$out/bin/"
cp "$bindir/dah$exe" "$out/bin/"
cp THIRD_PARTY_NOTICES.md "$out/" 2>/dev/null || true
cp LICENSE "$out/" 2>/dev/null || true
cp README.md "$out/" 2>/dev/null || true
cp docs/runbooks/packaging.md "$out/PACKAGING.md" 2>/dev/null || true

# The sidecar is looked up first beside the desktop binary, then on PATH.
cat > "$out/bin/README.txt" <<'TXT'
dah-desktop  native Rust and Rune workbench
dah-agent    TinyAgent JSONL sidecar
dah          native CLI

The desktop looks for dah-agent beside its own binary, then on PATH. Set
DAH_AGENT_BIN to override. No CSDK, game file, VPK, key or log is included.
TXT

if [[ "$host" == *darwin* && -z "$target" ]]; then
  echo "== app bundle =="
  app="$out/Deadlock Mod Manager.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "$out/bin/dah-desktop" "$app/Contents/MacOS/dah-desktop"
  cp "$out/bin/dah-agent" "$app/Contents/MacOS/dah-agent"
  cp "$out/bin/dah" "$app/Contents/MacOS/dah"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Deadlock Mod Manager</string>
  <key>CFBundleIdentifier</key><string>dev.deadlock.harness</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>dah-desktop</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
  cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/" 2>/dev/null || true
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
    for file in bin/dah-desktop$exe bin/dah-agent$exe bin/dah$exe; do
      [ -f "$file" ] || continue
      size="$(wc -c < "$file" | tr -d ' ')"
      sha="$(shasum -a 256 "$file" | cut -d' ' -f1)"
      [ $first -eq 0 ] && printf ',\n'
      first=0
      printf '    {"path": "%s", "bytes": %s, "sha256": "%s"}' "$file" "$size" "$sha"
    done
    printf '\n  ],\n'
    printf '  "includes": ["dah-desktop", "dah-agent", "dah", "THIRD_PARTY_NOTICES.md"],\n'
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
