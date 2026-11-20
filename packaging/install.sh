#!/bin/bash
# Install Password Tool (Linux port of Password Tech).
#
#   packaging/install.sh [--no-build] [PREFIX]      (default PREFIX=/usr/local)
#   DESTDIR=/tmp/stage packaging/install.sh /usr    (staging, e.g. for packages)
#
# Layout: $PREFIX/bin/pwtool, $PREFIX/share/pwtool/ (translations, common
# passwords, script examples, license, manuals), desktop file, icons, metainfo.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
BUILD=1
if [ "${1:-}" = "--no-build" ]; then BUILD=0; shift; fi
PREFIX=${1:-/usr/local}
DEST="${DESTDIR:-}$PREFIX"
APP_ID=io.github.coinman_dev.PwTool

if [ $BUILD = 1 ]; then
  (cd "$ROOT" && cargo build --release -p pwtool)
fi
BIN="$ROOT/target/release/pwtool"
[ -x "$BIN" ] || { echo "missing $BIN (build first)" >&2; exit 1; }

install -Dm755 "$BIN" "$DEST/bin/pwtool"
SHARE="$DEST/share/pwtool"
install -d "$SHARE/languages" "$SHARE/script_examples"
install -m644 "$ROOT/assets/common_passwords.txt" "$ROOT/assets/license.txt" "$SHARE/"
install -m644 "$ROOT"/assets/languages/*.po "$ROOT"/assets/languages/*.lng "$ROOT"/assets/languages/*.pot "$SHARE/languages/"
install -d "$SHARE/languages/port"
install -m644 "$ROOT"/assets/languages/port/*.po "$SHARE/languages/port/"
install -m644 "$ROOT"/assets/script_examples/* "$SHARE/script_examples/"

# manuals (PDF from the original ODT files, if LibreOffice is available)
MANUALS="$ROOT/target/manuals"
if [ ! -f "$MANUALS/manual.pdf" ] && command -v soffice >/dev/null; then
  mkdir -p "$MANUALS"
  PROFILE=$(mktemp -d)
  timeout 300 soffice -env:UserInstallation=file://"$PROFILE" --headless --convert-to pdf \
    --outdir "$MANUALS" "$ROOT/upstream/manual/manual.odt" "$ROOT/upstream/manual/scripting.odt" >/dev/null 2>&1 || true
  rm -rf "$PROFILE"
fi
for f in manual.pdf scripting.pdf; do
  [ -f "$MANUALS/$f" ] && install -m644 "$MANUALS/$f" "$SHARE/$f"
done

install -Dm644 "$HERE/$APP_ID.desktop" "$DEST/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.metainfo.xml" "$DEST/share/metainfo/$APP_ID.metainfo.xml"
for s in 16 32 48 64 128 256; do
  install -Dm644 "$ROOT/assets/appicons/pwtool_default_$s.png" "$DEST/share/icons/hicolor/${s}x${s}/apps/$APP_ID.png"
done
echo "installed to $DEST"
