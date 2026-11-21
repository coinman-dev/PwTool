#!/bin/bash
# Build the PDF manuals from doc/manual/*.odt with LibreOffice into
# target/manuals. Run it before `cargo build`: the build embeds the PDFs
# into the executable (crates/pwtool-app/build.rs); without them "Open
# Manual" has nothing to open.
#
#   packaging/manuals.sh [--required]
#
# --required fails if LibreOffice is missing or the conversion fails
# (release builds).
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
OUT="$ROOT/target/manuals"
REQUIRED=0
[ "${1:-}" = "--required" ] && REQUIRED=1

newer() { [ ! -f "$OUT/$1.pdf" ] || [ "$ROOT/doc/manual/$1.odt" -nt "$OUT/$1.pdf" ]; }
if ! newer manual && ! newer scripting; then
  echo "manuals up to date in $OUT"
  exit 0
fi
if ! command -v soffice >/dev/null; then
  echo "LibreOffice (soffice) not found: manuals not built" >&2
  exit $REQUIRED
fi
mkdir -p "$OUT"
PROFILE=$(mktemp -d)
trap 'rm -rf "$PROFILE"' EXIT
timeout 300 soffice -env:UserInstallation=file://"$PROFILE" --headless --convert-to pdf \
  --outdir "$OUT" "$ROOT/doc/manual/manual.odt" "$ROOT/doc/manual/scripting.odt" >/dev/null 2>&1 || true
for f in manual scripting; do
  if [ ! -s "$OUT/$f.pdf" ]; then
    echo "conversion of $f.odt failed" >&2
    exit $REQUIRED
  fi
done
ls -l "$OUT"
