#!/usr/bin/env bash
# Pack the Locker hero art from the Grimoire checkout into an app asset root.
#
#   scripts/pack-locker-art.sh <out-root>
#
# Writes <out-root>/heroes (renders), <out-root>/names (name plates) and
# <out-root>/hero_portraits (with chip-icons for Stats) as lossy WebP with alpha, sized for the Locker grid
# and detail view. Source: vendor/grimoire/public (MIT; the art follows
# Grimoire's precedent, see THIRD_PARTY_NOTICES.md). Needs cwebp.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
public="$root/vendor/grimoire/public"
out="${1:?usage: pack-locker-art.sh <out-root>}"

if ! command -v cwebp >/dev/null 2>&1; then
  echo "cwebp is required (brew install webp)" >&2
  exit 1
fi
if [ ! -d "$public/locker/heroes" ]; then
  echo "No Grimoire checkout at $public. Run pnpm vendor." >&2
  exit 1
fi

pack() {
  local from="$1" to="$2" width="$3"
  mkdir -p "$to"
  for src in "$from"/*.png; do
    [ -e "$src" ] || continue
    local name
    name="$(basename "${src%.png}")"
    cwebp -quiet -q 82 -alpha_q 90 -m 6 -resize "$width" 0 "$src" -o "$to/$name.webp"
  done
}

pack "$public/locker/heroes" "$out/heroes" 1100
pack "$public/locker/names" "$out/names" 720
pack "$public/heroes" "$out/hero_portraits" 256
# Stats hero chips: small round icons beside hero names and match rows.
pack "$public/heroes/chip-icons" "$out/hero_portraits/chip-icons" 64
# The General tile backdrop in the Locker grid.
cp "$public/locker/global-bg.webp" "$out/global-bg.webp"
# Personalize: the default art behind the sidebar launch buttons.
cp "$public/locker/launch-modded-bg.webp" "$out/launch-modded-bg.webp"
cp "$public/locker/launch-vanilla-bg.jpg" "$out/launch-vanilla-bg.jpg"
du -sh "$out"
