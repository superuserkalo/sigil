#!/usr/bin/env bash
set -euo pipefail

# Rune is pre-release and not on crates.io. Pin the dev revision and vendor its
# patched crates so the Rust workspace builds reproducibly without a fork.

cd "$(dirname "${BASH_SOURCE[0]}")/.."

RUNE_REPO="${RUNE_REPO:-https://github.com/anomalyco/rune.git}"
RUNE_REV="${RUNE_REV:-631e8a67cb3c40225efa3f6c4f57b80e7b18f190}"
DEST="vendor/rune"

if [[ -d "$DEST/.git" ]]; then
  current="$(git -C "$DEST" rev-parse HEAD)"
  if [[ "$current" == "$RUNE_REV" ]]; then
    echo "rune already at $RUNE_REV"
    exit 0
  fi
  echo "rune is at $current, expected $RUNE_REV; re-vendoring"
  rm -rf "$DEST"
fi

mkdir -p vendor
git init -q "$DEST"
git -C "$DEST" remote add origin "$RUNE_REPO"
git -C "$DEST" fetch --depth 1 origin "$RUNE_REV"
git -C "$DEST" checkout -q FETCH_HEAD
echo "rune vendored at $RUNE_REV"
