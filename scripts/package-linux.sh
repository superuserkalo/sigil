#!/usr/bin/env bash
# Build the Linux x86_64 AppImage and .deb on a Linux host (or in CI: .github/workflows/linux-build.yml).
#
#   scripts/package-linux.sh            # build, stage, AppImage, .deb, unsigned manifest entry
#   scripts/package-linux.sh --deps     # first install the Ubuntu or Debian build packages (sudo)
#
# The program is one file, `sigil` (the dah-desktop crate): the Locker art and the notices are
# built into it, and the CLI (`sigil cli ...`) and the agent host (`sigil agent-host`) are
# subcommands. It never includes CSDK 12, game files, VPKs, keys or logs.
#
# Outputs, in .scratch/package/:
#   Sigil-<version>-x86_64.AppImage   updates itself (rust/crates/desktop/src/updater.rs)
#   sigil_<version>_amd64.deb         /usr/lib/sigil/sigil, /usr/bin/sigil; updated by hand
#   release-<version>/sigil-update.json gains `extra_platforms.linux-x86_64` (unsigned)
#
# Build on the oldest glibc you want to support: the AppImage runs on that glibc or newer. CI uses
# Ubuntu 22.04 (glibc 2.35). macOS cannot link this build (Wayland, ALSA, OpenSSL and UnRAR's
# C++ need Linux headers and libraries); use the workflow or a Linux machine. See
# docs/runbooks/packaging.md.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
  echo "package-linux.sh runs on Linux x86_64. From macOS, run the linux-build workflow" >&2
  echo "(gh workflow run linux-build.yml) or build on a Linux machine." >&2
  exit 1
fi

if [ "${1:-}" = "--deps" ]; then
  # Headers for the -sys crates (Wayland, xkbcommon, ALSA, fontconfig, OpenSSL), the C++ compiler
  # for UnRAR, cwebp for the Locker art, and what linuxdeploy needs to inspect the binary.
  sudo apt-get update
  sudo apt-get install -y --no-install-recommends \
    build-essential pkg-config libwayland-dev libxkbcommon-dev libasound2-dev \
    libfontconfig1-dev libssl-dev libxcb1-dev webp file zip dpkg-dev desktop-file-utils
  shift
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' rust/Cargo.toml | head -1)"
version="${version:-0.1.0}"
target="x86_64-unknown-linux-gnu"
pkg="$root/.scratch/package"
tools="$root/.scratch/linux-pkg/tools"
appdir="$root/.scratch/linux-pkg/AppDir"
debroot="$root/.scratch/linux-pkg/deb"
appimage="$pkg/Sigil-$version-x86_64.AppImage"
deb="$pkg/sigil_${version}_amd64.deb"

# linuxdeploy bundles the libraries the binary links that a distro may lack (OpenSSL 3,
# xkbcommon) and wraps the AppDir with its built-in appimage plugin. Pinned by tag and hash:
# nothing runs that does not match.
linuxdeploy_url="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
linuxdeploy_sha="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"

sha256() { sha256sum "$1" | cut -d' ' -f1; }

echo "== art =="
art="$root/.scratch/locker-art"
rm -rf "$art"
"$root/scripts/pack-locker-art.sh" "$art"
export SIGIL_ART_DIR="$art"

echo "== build linux =="
cargo build --manifest-path rust/Cargo.toml --release --target "$target" -p dah-desktop
bin="rust/target/$target/release/dah-desktop"

desktop_entry() {
  # $1: the Exec program. The sigil:// handler is declared here so the AppImage integrators
  # (AppImageLauncher, Gear Lever) and the .deb register it; Sigil also writes its own
  # per-user entry when links are turned on (rust/crates/desktop/src/deeplink.rs).
  cat <<DESKTOP
[Desktop Entry]
Type=Application
Name=Sigil
GenericName=Deadlock mod manager
Comment=Install, build and manage Deadlock mods
Exec=$1 %u
Icon=sigil
Terminal=false
Categories=Game;Utility;
MimeType=x-scheme-handler/sigil;
StartupWMClass=sigil
DESKTOP
}

echo "== AppDir =="
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/512x512/apps" "$appdir/usr/share/doc/sigil"
install -m 755 "$bin" "$appdir/usr/bin/sigil"
desktop_entry sigil > "$appdir/usr/share/applications/sigil.desktop"
cp rust/crates/desktop/assets/brand/app-icon.png "$appdir/usr/share/icons/hicolor/512x512/apps/sigil.png"
cp THIRD_PARTY_NOTICES.md "$appdir/usr/share/doc/sigil/"
cp LICENSE "$appdir/usr/share/doc/sigil/" 2>/dev/null || true
if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$appdir/usr/share/applications/sigil.desktop"
fi

echo "== linuxdeploy =="
mkdir -p "$tools"
ld="$tools/linuxdeploy-x86_64.AppImage"
if [ ! -f "$ld" ] || [ "$(sha256 "$ld")" != "$linuxdeploy_sha" ]; then
  rm -f "$ld"
  curl -fsSL --retry 3 -o "$ld.part" "$linuxdeploy_url"
  if [ "$(sha256 "$ld.part")" != "$linuxdeploy_sha" ]; then
    rm -f "$ld.part"
    echo "linuxdeploy does not match its pinned SHA-256; refusing to run it" >&2
    exit 1
  fi
  mv "$ld.part" "$ld"
fi
chmod 755 "$ld"

rm -f "$appimage"
# Left to the system on purpose:
# - libwayland-*: the Vulkan driver links the system copy; two copies in one process break.
# - libasound: a bundled one looks for its PipeWire and Pulse plugins in the build distro's path.
# - libfontconfig, libfreetype: an older copy misreads a newer system's font configuration.
# - libstdc++, libgcc_s: Mesa's Vulkan drivers need the system's newer libstdc++.
# glibc, libGL and libvulkan are on linuxdeploy's own exclude list.
# The patterns are full regexes (`https://github.com/linuxdeploy/linuxdeploy` reads them with
# std::regex), so each ends in `.*`; a bare `libasound*` would only match `libasound`.
# No FUSE in containers and CI runners: run linuxdeploy extracted.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 LINUXDEPLOY_OUTPUT_VERSION="$version" OUTPUT="$appimage" \
  "$ld" --appdir "$appdir" \
  --executable "$appdir/usr/bin/sigil" \
  --desktop-file "$appdir/usr/share/applications/sigil.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/512x512/apps/sigil.png" \
  --exclude-library 'libwayland-.*' \
  --exclude-library 'libasound\..*' \
  --exclude-library 'libfontconfig\..*' \
  --exclude-library 'libfreetype\..*' \
  --exclude-library 'libstdc\+\+.*' \
  --exclude-library 'libgcc_s\..*' \
  --output appimage
test -s "$appimage"
chmod 755 "$appimage"

echo "== deb =="
# The program lives in /usr/lib/sigil with a link in /usr/bin: Settings > Updates recognises
# that folder and points at the release page instead of the self-updater
# (rust/crates/desktop/src/package_manager.rs, Manager::SigilDeb).
rm -rf "$debroot"
mkdir -p "$debroot/DEBIAN" "$debroot/usr/lib/sigil" "$debroot/usr/bin" \
  "$debroot/usr/share/applications" "$debroot/usr/share/icons/hicolor/512x512/apps" \
  "$debroot/usr/share/doc/sigil"
install -m 755 "$bin" "$debroot/usr/lib/sigil/sigil"
ln -s ../lib/sigil/sigil "$debroot/usr/bin/sigil"
desktop_entry /usr/bin/sigil > "$debroot/usr/share/applications/sigil.desktop"
cp rust/crates/desktop/assets/brand/app-icon.png "$debroot/usr/share/icons/hicolor/512x512/apps/sigil.png"
cp THIRD_PARTY_NOTICES.md "$debroot/usr/share/doc/sigil/"
cp LICENSE "$debroot/usr/share/doc/sigil/copyright" 2>/dev/null || cp THIRD_PARTY_NOTICES.md "$debroot/usr/share/doc/sigil/copyright"
size_kb="$(du -sk "$debroot/usr" | cut -f1)"
cat > "$debroot/DEBIAN/control" <<CONTROL
Package: sigil
Version: $version
Section: games
Priority: optional
Architecture: amd64
Installed-Size: $size_kb
Depends: libc6 (>= 2.35), libssl3 | libssl3t64, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libasound2 | libasound2t64, libfontconfig1, libvulkan1, libstdc++6
Recommends: xdg-utils, xdg-desktop-portal
Maintainer: Sigil <noreply@users.noreply.github.com>
Homepage: https://github.com/superuserkalo/sigil
Description: Deadlock mod manager and mod workbench
 Sigil installs, builds, packs and validates Deadlock mods. It never writes
 to the game install without the player's action.
CONTROL
rm -f "$deb"
dpkg-deb --root-owner-group --build "$debroot" "$deb" > /dev/null

echo "== result =="
for file in "$appimage" "$deb"; do
  printf '%s\n  bytes:  %s\n  sha256: %s\n' "$file" "$(wc -c < "$file" | tr -d ' ')" "$(sha256 "$file")"
done

# The update manifest entry: the AppImage itself under `extra_platforms`, which Sigil 0.1.x
# never reads (scripts/release-sign.mjs). Unsigned here; sign the merged manifest with
# scripts/sign-release.sh once the Windows entry is in it (docs/runbooks/releasing.md).
manifest="$pkg/release-$version/sigil-update.json"
mkdir -p "$(dirname "$manifest")"
node "$root/scripts/release-sign.mjs" manifest "$manifest" --version "$version" \
  --platform linux-x86_64 --zip "$appimage"
echo "manifest: $manifest (unsigned)"
