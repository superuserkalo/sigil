#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
owner="${GRIMOIRE_OWNER:-Slush97}"
ref="${GRIMOIRE_REF:-main}"

mkdir -p "$root/vendor"

clone() {
  local name="$1"
  local url="https://github.com/$owner/$name.git"
  local target="$root/vendor/$name"

  if [ -d "$target/.git" ]; then
    echo "vendor/$name already exists, leaving it alone"
    return
  fi

  echo "cloning $url at $ref into vendor/$name"
  git clone --depth 1 --branch "$ref" "$url" "$target"
}

clone grimoire
clone grimoire-social

cat <<'EOF'

Vendored. Next steps:

  cd vendor/grimoire-social && pnpm install --ignore-scripts
  cd ../grimoire && corepack pnpm@12 install
  corepack pnpm@12 exec electron-rebuild -f -w better-sqlite3
  corepack pnpm@12 dev

vendor/ is gitignored. Record patches to the fork in docs/decisions/ and upstream them when
possible.
EOF
