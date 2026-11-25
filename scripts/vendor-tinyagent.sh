#!/usr/bin/env bash
set -euo pipefail

# TinyAgent is not published to npm. Pin a revision and vendor it: its TypeScript implementation
# is the reference that the Rust port in rust/crates/tinyagent is kept in sync with. Nothing builds
# or links it.

cd "$(dirname "${BASH_SOURCE[0]}")/.."

REPO="${TINYAGENT_REPO:-https://github.com/superuserkalo/tinyagent.git}"
REV="${TINYAGENT_REV:-95ba029be92511bd59929aa21dd56d20559468d4}"
DEST="vendor/tinyagent"

if [[ -d "$DEST/.git" ]]; then
  current="$(git -C "$DEST" rev-parse HEAD)"
  if [[ "$current" == "$REV" ]]; then
    echo "tinyagent already at $REV"
  else
    rm -rf "$DEST"
  fi
fi

if [[ ! -d "$DEST/.git" ]]; then
  mkdir -p vendor
  git init -q "$DEST"
  git -C "$DEST" remote add origin "$REPO"
  git -C "$DEST" fetch --depth 1 origin "$REV"
  git -C "$DEST" checkout -q FETCH_HEAD
  echo "tinyagent vendored at $REV"
fi

