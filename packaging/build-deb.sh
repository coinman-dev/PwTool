#!/bin/bash
# Build a Debian package: packaging/build-deb.sh [--no-build]
# Result: target/pwtool_<version>_<arch>.deb (or $OUTPUT if set)
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
VERSION=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 | python3 -I -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="pwtool"))')
# Debian orders "0.1.0~beta" before "0.1.0", matching semver "0.1.0-beta".
DEB_VERSION="${VERSION/-/\~}"
if [ -n "${DEB_REVISION:-}" ] && [ "${DEB_REVISION}" != "none" ]; then
  DEB_VERSION="${DEB_VERSION}-${DEB_REVISION}"
fi
ARCH=$(dpkg --print-architecture)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
chmod 0755 "$STAGE"
DESTDIR="$STAGE" "$HERE/install.sh" "$@" /usr
if command -v strip >/dev/null 2>&1; then
  strip --strip-unneeded "$STAGE/usr/bin/pwtool" || true
fi
mkdir -p "$STAGE/DEBIAN"
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: pwtool
Version: $DEB_VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Depends: libc6, libgcc-s1, libstdc++6, libegl1 | libgl1, libxkbcommon0
Recommends: libwayland-client0, libwayland-cursor0, libwayland-egl1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxrandr2, libxkbcommon-x11-0, xdg-desktop-portal, xdg-desktop-portal-kde | xdg-desktop-portal-gnome
Maintainer: Password Tool <https://github.com/coinman-dev/PwTool>
Homepage: https://github.com/coinman-dev/PwTool
Description: Password generator and password manager (port of Password Tech)
 Password Tool generates passwords, passphrases, formatted and phonetic
 passwords with a cryptographically secure random generator and manages
 passwords in encrypted databases (PassCube). Port of Password Tech 3.6.1
 by Christian Thoeing for Linux and Windows.
CONTROL
OUT="${OUTPUT:-$ROOT/target/pwtool_${VERSION//-/.}_${ARCH}.deb}"
mkdir -p "$(dirname "$OUT")"
dpkg-deb --root-owner-group --build "$STAGE" "$OUT"
echo "$OUT"
