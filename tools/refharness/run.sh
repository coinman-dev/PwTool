#!/usr/bin/env bash
# Runs the reference harness under wine (console program, opens no windows).
# Paths given to the harness may be Linux paths (absolute -> Z:\..., or
# relative to the current directory); the harness converts them itself.
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
export WINEPREFIX="${WINEPREFIX:-$ROOT/.ref/wineprefix}"
export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:-mscoree,mshtml=}"
export WINEDEBUG="${WINEDEBUG:--all}"
EXE="${REFHARNESS_EXE:-$ROOT/.ref/refharness/refharness.exe}"
if [ ! -f "$EXE" ]; then
  echo "refharness.exe not found - run tools/refharness/build.sh first" >&2
  exit 2
fi
exec wine "$EXE" "$@"
