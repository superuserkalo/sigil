#!/usr/bin/env bash
# Build the Windows x64 bundle from macOS or Linux with cargo-xwin.
#
# The bundle is one file, Sigil.exe. The Locker art and the notices are built into it, and the
# dah CLI (`Sigil.exe cli ...`) and the agent host (`Sigil.exe agent-host`) are folded in.
# It never includes CSDK 12, game files, VPKs, keys or logs.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -1)"
version="${version:-0.1.0}"
target="x86_64-pc-windows-msvc"
out="$root/.scratch/package/Sigil-$version-windows-x64"
zip_path="$root/.scratch/package/Sigil-$version-windows-x64.zip"

if ! command -v cargo-xwin >/dev/null 2>&1; then
  echo "cargo-xwin is required: cargo install cargo-xwin --locked" >&2
  exit 1
fi

echo "== art =="
# Packed once, then built into the executable by rust/crates/desktop/build.rs.
art="$root/.scratch/locker-art"
rm -rf "$art"
"$root/scripts/pack-locker-art.sh" "$art"
export SIGIL_ART_DIR="$art"

echo "== build windows =="
export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static"
# The RAR reader builds RARLAB's UnRAR (C++). cargo-xwin compiles it with the host clang, older
# than the MSVC STL expects, and clang (unlike cl.exe) wants the SSSE3 and AES intrinsics UnRAR
# uses enabled. Every CPU Deadlock runs on has both. A build on Windows with cl.exe needs none
# of this.
export CL="${CL:-} /D_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH -mssse3 -maes"
cargo xwin build --manifest-path rust/Cargo.toml --release --target "$target" \
  -p dah-desktop

bindir="rust/target/$target/release"

echo "== stage =="
rm -rf "$out" "$zip_path"
mkdir -p "$out"
cp "$bindir/dah-desktop.exe" "$out/Sigil.exe"
# Everything (art, notices, the CLI and the agent host) lives inside Sigil.exe.

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

# The update manifest for this version: the zip's name, size and SHA-256. Unsigned here; sign it with scripts/sign-release.sh
# (docs/runbooks/releasing.md).
manifest="$root/.scratch/package/release-$version/sigil-update.json"
mkdir -p "$(dirname "$manifest")"
node "$root/scripts/release-sign.mjs" manifest "$manifest" --version "$version" \
  --platform windows-x86_64 --zip "$zip_path"
echo "manifest: $manifest (unsigned)"
