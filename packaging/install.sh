#!/bin/bash
# Install Password Tool (port of Password Tech).
#
#   packaging/install.sh [--no-build] [PREFIX]      (default PREFIX=/usr/local)
#   DESTDIR=/tmp/stage packaging/install.sh /usr    (staging, e.g. for packages)
#
# Layout: $PREFIX/bin/pwtool, desktop file, icons, metainfo. The program data
# (translations, common passwords, script examples, license, PDF manuals) is
# embedded in the executable; the manuals are built first (packaging/manuals.sh)
# if LibreOffice is available.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
BUILD=1
if [ "${1:-}" = "--no-build" ]; then BUILD=0; shift; fi
PREFIX=${1:-/usr/local}
DEST="${DESTDIR:-}$PREFIX"
APP_ID=io.github.coinman_dev.PwTool

if [ $BUILD = 1 ]; then
  "$HERE/manuals.sh" || true
  (cd "$ROOT" && cargo build --release --locked -p pwtool)
fi
BIN="$ROOT/target/release/pwtool"
[ -x "$BIN" ] || { echo "missing $BIN (build first)" >&2; exit 1; }

install -Dm755 "$BIN" "$DEST/bin/pwtool"
install -Dm644 "$HERE/$APP_ID.desktop" "$DEST/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.metainfo.xml" "$DEST/share/metainfo/$APP_ID.metainfo.xml"
for s in 16 32 48 64 128 256; do
  install -Dm644 "$ROOT/assets/appicons/pwtool_default_$s.png" "$DEST/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"
done
echo "installed to $DEST"
