#!/bin/bash
# Build a Debian package: packaging/build-deb.sh [--no-build]
# Result: target/pwtool_<version>_<arch>.deb
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
VERSION=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 | python3 -I -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="pwtool"))')
ARCH=$(dpkg --print-architecture)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
DESTDIR="$STAGE" "$HERE/install.sh" "$@" /usr
mkdir -p "$STAGE/DEBIAN"
SIZE=$(du -sk "$STAGE/usr" | cut -f1)
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: pwtool
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Depends: libc6, libgtk-4-1 (>= 4.10), libglib2.0-0t64 | libglib2.0-0, glib-networking
Recommends: xdg-desktop-portal, xdg-desktop-portal-kde | xdg-desktop-portal-gnome
Maintainer: Password Tool <https://github.com/coinman-dev/PwTool>
Homepage: https://github.com/coinman-dev/PwTool
Description: Password generator and password manager (Linux port of Password Tech)
 Password Tool generates passwords, passphrases, formatted and phonetic
 passwords with a cryptographically secure random generator and manages
 passwords in encrypted databases (PassCube). Linux port of Password Tech
 3.6.1 by Christian Thoeing.
CONTROL
OUT="$ROOT/target/pwtool_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$OUT"
echo "$OUT"
