#!/usr/bin/env bash
# Build Sigil-Setup.exe from a packaged Sigil.exe with NSIS (makensis), on macOS or Linux.
#
#   scripts/package-installer.sh <path to Sigil.exe> <version> <output Sigil-Setup.exe>
#
# The installer is per user and needs no administrator rights. It is not code signed, so Windows
# SmartScreen warns on first run, exactly as it does for the plain Sigil.exe.
set -euo pipefail

exe="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
version="$2"
out="$3"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v makensis >/dev/null 2>&1; then
  echo "makensis is required: brew install makensis" >&2
  exit 1
fi
mkdir -p "$(dirname "$out")"
out="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"

makensis -V2 -DVERSION="$version" -DEXE="$exe" -DNOTICES="$root/THIRD_PARTY_NOTICES.md" -DOUT="$out" \
  -DICON="$root/rust/crates/desktop/assets/brand/sigil.ico" "$root/scripts/installer/sigil.nsi"
size="$(wc -c < "$out" | tr -d ' ')"
printf 'installer: %s\nbytes:     %s\nsha256:    %s\n' "$out" "$size" "$(shasum -a 256 "$out" | cut -d' ' -f1)"
