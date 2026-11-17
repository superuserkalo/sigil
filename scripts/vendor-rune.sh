#!/usr/bin/env bash
set -euo pipefail

# Rune is pre-release and not on crates.io. Pin the dev revision and vendor its
# patched crates so the Rust workspace builds reproducibly without a fork.

cd "$(dirname "${BASH_SOURCE[0]}")/.."

RUNE_REPO="${RUNE_REPO:-https://github.com/anomalyco/rune.git}"
RUNE_REV="${RUNE_REV:-631e8a67cb3c40225efa3f6c4f57b80e7b18f190}"
DEST="vendor/rune"

# Local fixes on top of the pinned revision, applied in order. Each is skipped when it is
# already in the checkout. Drop a patch once the pinned revision contains it upstream.
apply_patches() {
  local patch
  for patch in scripts/patches/rune/*.patch; do
    [[ -e "$patch" ]] || continue
    if git -C "$DEST" apply --reverse --check "../../$patch" 2>/dev/null; then
      echo "patch already applied: $(basename "$patch")"
    else
      git -C "$DEST" apply "../../$patch"
      echo "patch applied: $(basename "$patch")"
    fi
  done
}

if [[ -d "$DEST/.git" ]]; then
  current="$(git -C "$DEST" rev-parse HEAD)"
  if [[ "$current" == "$RUNE_REV" ]]; then
    echo "rune already at $RUNE_REV"
    apply_patches
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
apply_patches
echo "rune vendored at $RUNE_REV"
