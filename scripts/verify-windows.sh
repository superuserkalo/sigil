#!/usr/bin/env bash
# Inspect a Windows bundle without running it. Checks the PE format, the
# subsystem, that the helpers and art are inside Sigil.exe, and the hashes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
zip="${1:-$(ls -t "$root"/.scratch/package/Sigil-*-windows-x64.zip 2>/dev/null | head -1)}"
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

for exe in "Sigil.exe"; do
  path="$dir/$exe"
  if [ ! -f "$path" ]; then
    echo "MISSING $exe" >&2
    exit 1
  fi
  printf '%-28s ' "$exe"
  file -b "$path"
  printf '%-28s %s bytes\n' "" "$(wc -c < "$path" | tr -d ' ')"
done

for stale in "dah-agent.exe" "dah.exe"; do
  if [ -f "$dir/$stale" ]; then
    echo "unexpected $stale: the CLI and the agent host are folded into Sigil.exe" >&2
    exit 1
  fi
done

extra="$(find "$dir" -mindepth 1 ! -name Sigil.exe | sed "s|$dir/||")"
if [ -n "$extra" ]; then
  echo "unexpected files beside Sigil.exe (it ships as one file):" >&2
  echo "$extra" >&2
  exit 1
fi
if LC_ALL=C grep -aq "heroes/Abrams_Render.webp" "$dir/Sigil.exe"; then
  echo "art: built in"
else
  echo "art: MISSING from Sigil.exe (set SIGIL_ART_DIR or run scripts/pack-locker-art.sh)" >&2
  exit 1
fi
if LC_ALL=C grep -aqF "# Third party notices" "$dir/Sigil.exe"; then
  echo "notices: built in"
else
  echo "notices: MISSING" >&2
fi
if find "$dir" -iname "*.pdb" -o -iname "resourcecompiler.exe" -o -iname "*.vpk" | grep -q .; then
  echo "unexpected build or game file in the bundle" >&2
  exit 1
fi
echo "helpers: folded into Sigil.exe (cli, agent-host)"
