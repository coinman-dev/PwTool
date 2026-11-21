#!/bin/bash
# Build the Windows installer of Password Tool with NSIS (on Linux or in Git
# Bash on Windows) from the exe made by packaging/windows/build.sh:
#
#   packaging/windows/build-installer.sh [pwtool.exe]
#
# Needs: makensis 3 (Debian/Ubuntu: apt install nsis), python3.
#
# Environment:
#   OUTPUT  installer file (default: target/pwtool_<version>_windows_x64_setup.exe)
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

VERSION=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
[ -n "$VERSION" ] || { echo "no version in Cargo.toml" >&2; exit 1; }
EXE="${1:-$ROOT/target/pwtool_${VERSION//-/.}_windows_x64.exe}"
[ -f "$EXE" ] || { echo "missing $EXE (run packaging/windows/build.sh first)" >&2; exit 1; }
command -v makensis >/dev/null || { echo "makensis not found (apt install nsis)" >&2; exit 1; }
OUT="${OUTPUT:-$ROOT/target/pwtool_${VERSION//-/.}_windows_x64_setup.exe}"
mkdir -p "$(dirname "$OUT")"

# 0.1.0-beta -> 0.1.0.0 (VIProductVersion needs four numbers)
NUMERIC=$(python3 -I -c 'import re,sys; n=[int(x) for x in re.findall(r"\d+", sys.argv[1].split("-")[0])][:3]; print(".".join(map(str, n + [0]*(4-len(n)))))' "$VERSION")

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
# icon of the installer and uninstaller: the program icon, PNG images in an
# ICO file as in crates/pwtool/build.rs
python3 -I - "$ROOT/assets/appicons" "$WORK/pwtool.ico" <<'PY'
import struct, sys
src, dst = sys.argv[1], sys.argv[2]
images = []
for s in (16, 32, 48, 64, 128, 256):
    data = open(f"{src}/pwtool_default_{s}.png", "rb").read()
    w, h = struct.unpack(">II", data[16:24])
    images.append((w, h, data))
out = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
for w, h, data in images:
    out += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
out += b"".join(d for _, _, d in images)
open(dst, "wb").write(out)
PY

makensis -V2 -INPUTCHARSET UTF8 \
  "-DVERSION=$VERSION" "-DVERSION_NUMERIC=$NUMERIC" \
  "-DSOURCE_EXE=$EXE" "-DROOT=$ROOT" "-DICON=$WORK/pwtool.ico" "-DOUTPUT=$OUT" \
  "$HERE/pwtool.nsi"
ls -l "$OUT"
echo "$OUT"
