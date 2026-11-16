#!/usr/bin/env bash
# Build the Windows x64 bundle from macOS or Linux with cargo-xwin.
#
# The bundle is the desktop app, the Rust agent host and the CLI, plus notices.
# It never includes CSDK 12, game files, VPKs, keys or logs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -1)"
version="${version:-0.1.0}"
target="x86_64-pc-windows-msvc"
out="$root/.scratch/package/DeadlockModManager-$version-windows-x64"
zip_path="$root/.scratch/package/DeadlockModManager-$version-windows-x64.zip"

if ! command -v cargo-xwin >/dev/null 2>&1; then
  echo "cargo-xwin is required: cargo install cargo-xwin --locked" >&2
  exit 1
fi

echo "== build windows =="
export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static"
cargo xwin build --manifest-path rust/Cargo.toml --release --target "$target" \
  -p dah-desktop -p dah-agent -p dah-rs

bindir="rust/target/$target/release"

echo "== stage =="
rm -rf "$out" "$zip_path"
mkdir -p "$out"
cp "$bindir/dah-desktop.exe" "$out/Deadlock Mod Manager.exe"
cp "$bindir/dah-agent.exe" "$out/dah-agent.exe"
cp "$bindir/dah.exe" "$out/dah.exe"
cp THIRD_PARTY_NOTICES.md "$out/" 2>/dev/null || true
cp README.md "$out/" 2>/dev/null || true

cat > "$out/README-WINDOWS.txt" <<'TXT'
Deadlock Mod Manager (Windows x64)

Run
  1. Unzip this folder anywhere you can write, for example Documents.
  2. Double-click "Deadlock Mod Manager.exe".
  3. If SmartScreen warns, choose More info, then Run anyway. The build is
     unsigned.

The app is a native window with an agent inside. It does not use a browser
view or Electron.

Agent (optional)
  The sidecar is dah-agent.exe beside the app. It starts only when the model
  environment is set. In a terminal:

    set DAH_AGENT_PROVIDER=openai
    set DAH_AGENT_MODEL=<the exact model id>
    set DAH_AGENT_API_KEY=<your key>

  For a local OpenAI-compatible server:

    set DAH_AGENT_PROVIDER=chat
    set DAH_AGENT_BASE_URL=http://127.0.0.1:8080/v1
    set DAH_AGENT_MODEL=<the model id>

  Keys are read by the host and never shown in the transcript. There is no
  telemetry. The app works without an agent for the manager screens.

Game
  Install Deadlock through Steam. The app finds the install, the addon folder
  and gameinfo.gi. It never writes to the game without an approval step.
  No CSDK 12, compiler, VPK or game file is included.

Data
  %APPDATA%\DeadlockAgentHarness holds sessions, the workspace, caches and the
  vanilla stash. A pending vanilla stash means the addon folder is empty:
  restore before deleting that folder.

Limits
  This bundle was cross-compiled and has not been launched on Windows here.
  The in-game deploy, launch and CSDK compile paths remain unverified. See
  docs/runbooks/packaging.md.

CLI
  dah.exe is the headless surface: dah doctor, dah index, dah find, dah addons,
  dah profiles, dah install, dah stats, dah demo, dah locker, dah crosshair,
  dah character, dah mod, dah watch, dah publish.
TXT

echo "== zip =="
(cd "$(dirname "$out")" && zip -qry "$zip_path" "$(basename "$out")")

echo "== result =="
size="$(wc -c < "$zip_path" | tr -d ' ')"
if command -v shasum >/dev/null 2>&1; then
  hash="$(shasum -a 256 "$zip_path" | cut -d' ' -f1)"
else
  hash="$(sha256sum "$zip_path" | cut -d' ' -f1)"
fi
printf 'zip:    %s\nbytes:  %s\nsha256: %s\n' "$zip_path" "$size" "$hash"
