#!/usr/bin/env bash
# Tier 5 acceptance: the release loop, the manager writes and the data surfaces,
# against a disposable fake install. It never touches a real game directory.
#
#   scripts/acceptance.sh           # offline surfaces
#   scripts/acceptance.sh --network # also calls api.deadlock-api.com
#
# Exit code is the number of failed checks.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

network=0
[ "${1:-}" = "--network" ] && network=1

bin="${DAH_BIN:-$root/rust/target/debug/dah}"
if [ ! -x "$bin" ]; then
  echo "Building the CLI first..."
  cargo build -q --manifest-path rust/Cargo.toml -p dah-rs || exit 1
  bin="$root/rust/target/debug/dah"
fi

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
base="$root/.scratch/acceptance/$stamp"
game="$base/Deadlock"
workspace="$base/workspace"
project="$workspace/mods/acceptance"
export DAH_GAME_DIR="$game"
export DAH_DATA_DIR="$base/data"
export DAH_WORKSPACE_DIR="$workspace"
export DAH_LOCKER_ASSETS="$root/rust/crates/desktop/assets/locker"

pass=0
fail=0
declare -a failures=()

ok() { pass=$((pass + 1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail + 1)); failures+=("$1"); printf '  FAIL %s\n' "$1"; [ -n "${2:-}" ] && printf '       %s\n' "$2"; }

run() {
  local name="$1"
  shift
  local out
  if out="$("$@" 2>&1)"; then ok "$name"; else bad "$name" "$(printf '%s' "$out" | tail -3)"; fi
}

expect_file() {
  if [ -f "$2" ]; then ok "$1"; else bad "$1" "missing $2"; fi
}

expect_refusal() {
  local name="$1" needle="$2"
  shift 2
  local out
  if out="$("$@" 2>&1)"; then
    bad "$name" "expected a refusal, command succeeded"
  elif printf '%s' "$out" | grep -qF "$needle"; then
    ok "$name"
  else
    bad "$name" "refusal did not mention: $needle"
  fi
}

expect_contains() {
  local name="$1" needle="$2"
  shift 2
  local out
  if out="$("$@" 2>&1)"; then
    if printf '%s' "$out" | grep -qF "$needle"; then ok "$name"; else bad "$name" "output did not contain: $needle"; fi
  else
    bad "$name" "$(printf '%s' "$out" | tail -3)"
  fi
}

echo "Acceptance: $base"
mkdir -p "$game/game/citadel/addons" "$game/game/citadel/cfg" "$workspace"

cat > "$game/game/citadel/gameinfo.gi" <<'GI'
SearchPaths
{
	Game citadel/addons
	Game citadel
}
GI

echo
echo "1. discovery and project"
run "doctor finds the install" "$bin" doctor
expect_contains "doctor reports capabilities" "Capabilities" "$bin" doctor
run "create a project" "$bin" mod new acceptance --workspace "$workspace" --name "Acceptance Recolor"
python3 - "$project/project.json" <<'PYTARGETS'
import json, sys
path = sys.argv[1]
data = json.load(open(path))
data["targets"] = [
    "materials/models/heroes_wip/punkgoat/punkgoat_body.vmat_c",
    "models/heroes_wip/punkgoat/punkgoat.vmdl_c",
]
json.dump(data, open(path, "w"), indent=2)
PYTARGETS
mkdir -p "$project/staging/materials/models/heroes_wip/punkgoat" "$project/staging/models/heroes_wip/punkgoat"
printf 'Layer0\n{\n\tshader "csgo_complex"\n}\n' > "$project/staging/materials/models/heroes_wip/punkgoat/punkgoat_body.vmat_c"
printf 'fixture texture\n' > "$project/staging/materials/models/heroes_wip/punkgoat/punkgoat_head.vtex_c"
printf 'fixture model\n' > "$project/staging/models/heroes_wip/punkgoat/punkgoat.vmdl_c"
run "validate the staging tree" "$bin" mod validate --workspace "$project"
run "pack the addon" "$bin" mod pack --workspace "$project" --slot 05
run "pack the game catalog seed" "$bin" mod pack --workspace "$project" --slot 01
cp "$project/builds/pak01_dir.vpk" "$game/game/citadel/pak01_dir.vpk"
cp "$project/builds/pak01_000.vpk" "$game/game/citadel/pak01_000.vpk" 2>/dev/null || true
cp "$project/builds/pak05_dir.vpk" "$game/game/citadel/addons/pak05_dir.vpk"
cp "$project/builds/pak05_000.vpk" "$game/game/citadel/addons/pak05_000.vpk" 2>/dev/null || true

# A real PNG with alpha for the character reference check.
python3 - "$base/reference.png" <<'PYPNG'
import struct, sys, zlib
w = h = 96
rows = b''.join(
    b'\x00' + b''.join(
        (bytes([120, 90, 70, 235]) if (x-48)**2 + (y-52)**2 < 1200 else bytes([0, 0, 0, 0]))
        for x in range(w)
    )
    for y in range(h)
)
def chunk(tag, data):
    return struct.pack('>I', len(data)) + tag + data + struct.pack('>I', zlib.crc32(tag + data) & 0xffffffff)
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b'')
open(sys.argv[1], 'wb').write(png)
PYPNG

echo
echo "2. catalog and addon slots"
run "index the game" "$bin" index --refresh
expect_contains "find Billy's material" "punkgoat_body.vmat_c" "$bin" find punkgoat
run "record the baseline" "$bin" mod baseline --workspace "$project"
run "write a release" "$bin" mod release --workspace "$project" --slot 07 --note "acceptance build"
expect_contains "addons list sees the slot" "pak05_dir.vpk" "$bin" addons list
run "disable slot 05" "$bin" addons disable 5 --approve
expect_contains "slot 05 is disabled" "disabled" "$bin" addons list
run "enable slot 05" "$bin" addons enable 5 --approve

echo
echo "3. manager writes"
run "capture a profile" "$bin" profiles save "Acceptance loadout"
expect_contains "list the profile" "Acceptance loadout" "$bin" profiles list
run "apply the profile" "$bin" profiles apply acceptance-loadout --approve
printf 'rate 128000\ncl_showfps 1\n' > "$base/autoexec.cfg"
run "write autoexec" "$bin" autoexec set --file "$base/autoexec.cfg" --approve
expect_contains "autoexec has the line" "rate 128000" "$bin" autoexec show
run "apply the crosshair" "$bin" crosshair apply --approve
expect_contains "crosshair block landed" "dah:crosshair begin" "$bin" autoexec show
run "clear the crosshair block" "$bin" crosshair clear --approve
run "create a character task" "$bin" character create "" --reference "$base/reference.png" --hero billy
run "print the character stages" "$bin" character stages

echo
echo "4. release, deploy and publish"
run "plan the deploy" "$bin" mod deploy --workspace "$project" --slot 07
run "apply the deploy" "$bin" mod deploy --workspace "$project" --slot 07 --approve
expect_file "slot 07 exists after apply" "$game/game/citadel/addons/pak07_dir.vpk"
expect_contains "gameinfo lists addons" "citadel/addons" "$bin" doctor
run "prepare the publish folder" "$bin" publish prepare --workspace "$project"
expect_refusal "publish status refuses before an upload" "No upload record" "$bin" publish status --workspace "$project"

echo
echo "5. locker, demo and install"
expect_contains "locker lists Billy" "Billy" "$bin" locker list
run "record a recolor brief" "$bin" locker recolor billy --palette frost --note "acceptance"
expect_file "recolor brief" "$workspace/generated/recolors/punkgoat.json"
run "demo backends" "$bin" demo backends
expect_contains "demo reports container only" "container" "$bin" demo backends
run "watch the projects" "$bin" watch --limit 5
perl -e 'alarm 7; exec @ARGV' "$bin" watch --watch --interval 5 >/dev/null 2>&1 || true
expect_file "watch state" "$workspace/.dah/watch-state.json"

# A compiled tree, so the install pipeline packs the slot itself.
mkdir -p "$base/install/game/citadel/materials/models/heroes_wip/punkgoat"
printf 'fixture install material\n' > "$base/install/game/citadel/materials/models/heroes_wip/punkgoat/punkgoat_alt.vmat_c"
(cd "$base/install" && zip -qr "$base/acceptance-mod.zip" game)
plan_id="$("$bin" install plan "$base/acceptance-mod.zip" --slot 09 --json 2>/dev/null | python3 -c "import sys,json;print(json.load(sys.stdin)['plan']['id'])" 2>/dev/null)"
if [ -n "$plan_id" ]; then ok "plan an install"; else bad "plan an install" "no plan id"; fi
run "apply the install" "$bin" install apply "$plan_id" --approve
expect_file "slot 09 installed" "$game/game/citadel/addons/pak09_dir.vpk"
expect_contains "install journal" "pak09_dir.vpk" "$bin" install list

echo
echo "6. launch and stats"
expect_contains "launch status" "stash" "$bin" launch status
if [ "$network" = "1" ]; then
  expect_contains "stats player" "Rank:" "$bin" stats player 138006262 --recent 5
  expect_contains "stats heatmap" "Kill" "$bin" stats heatmap 138006262
else
  echo "  skip stats (add --network to call the public API)"
fi

echo
echo "------------------------------------------------------------"
printf 'pass %d  fail %d\n' "$pass" "$fail"
if [ "$fail" -gt 0 ]; then
  printf 'failed: %s\n' "${failures[*]}"
  exit "$fail"
fi
echo "Tier 5 acceptance passed. Artifacts: $base"
