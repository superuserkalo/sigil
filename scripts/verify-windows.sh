#!/usr/bin/env bash
# Inspect a Windows bundle without running it. Checks the PE format, the
# subsystem, the sidecar placement and the hashes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
zip="${1:-$(ls -t "$root"/.scratch/package/DeadlockModManager-*-windows-x64.zip 2>/dev/null | head -1)}"
if [ -z "${zip:-}" ] || [ ! -f "$zip" ]; then
  echo "No Windows bundle found. Run scripts/package-windows.sh first." >&2
  exit 1
fi

echo "bundle: $zip"
if command -v shasum >/dev/null 2>&1; then
  shasum -a 256 "$zip"
else
  sha256sum "$zip"
fi
echo

tmp="$(mktemp -d "${TMPDIR:-/tmp}/dah-win-verify.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
unzip -q "$zip" -d "$tmp"
dir="$(find "$tmp" -maxdepth 1 -mindepth 1 -type d | head -1)"

for exe in "Deadlock Mod Manager.exe" "dah-agent.exe" "dah.exe"; do
  path="$dir/$exe"
  if [ ! -f "$path" ]; then
    echo "MISSING $exe" >&2
    exit 1
  fi
  printf '%-28s ' "$exe"
  file -b "$path"
  printf '%-28s %s bytes\n' "" "$(wc -c < "$path" | tr -d ' ')"
done

if [ -f "$dir/THIRD_PARTY_NOTICES.md" ]; then
  echo "notices: present"
else
  echo "notices: MISSING" >&2
fi
if find "$dir" -iname "*.pdb" -o -iname "resourcecompiler.exe" -o -iname "*.vpk" | grep -q .; then
  echo "unexpected build or game file in the bundle" >&2
  exit 1
fi
echo "sidecar: beside the app"
