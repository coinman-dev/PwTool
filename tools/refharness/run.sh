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
# Never show anything on the user's desktop: no display for wine, and the
# winedbg crash dialog is switched off in the prefix (crashes are reported
# as text on stderr instead).
unset DISPLAY WAYLAND_DISPLAY
if [ ! -f "$WINEPREFIX/.no-crash-dialog" ] && [ -d "$WINEPREFIX" ]; then
  wine reg add 'HKCU\Software\Wine\WineDbg' /v ShowCrashDialog /t REG_DWORD /d 0 /f >/dev/null 2>&1 \
    && touch "$WINEPREFIX/.no-crash-dialog"
fi
exec wine "$EXE" "$@"
