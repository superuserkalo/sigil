#!/usr/bin/env bash
# Sign a Sigil release manifest with the local update signing key.
#
#   scripts/sign-release.sh <sigil-update.json> [folder-with-the-zips]
#
# Checks every zip the manifest lists (size and SHA-256) when the folder is given or sits next
# to the manifest, writes <manifest>.sig, then verifies the signature against the public key
# compiled into Sigil (rust/crates/desktop/src/updater.rs). The private key stays in its file:
# SIGIL_SIGNING_KEY, else ~/.config/sigil/update-signing.key (mode 600). See
# docs/runbooks/releasing.md.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${1:?usage: sign-release.sh <sigil-update.json> [folder-with-the-zips]}"
dir="${2:-$(dirname "$manifest")}"
key="${SIGIL_SIGNING_KEY:-$HOME/.config/sigil/update-signing.key}"

if ! command -v node >/dev/null 2>&1; then
  echo "node is required (Node 24 or newer)" >&2
  exit 1
fi

node "$root/scripts/release-sign.mjs" sign "$manifest" --key "$key" --dir "$dir"
node "$root/scripts/release-sign.mjs" verify "$manifest"
