#!/bin/bash
# Run a command inside an invisible (virtual) KWin Wayland session with its
# own D-Bus session and runtime directory, so that windows do not appear on
# the user's desktop. Used for automatic screenshots of the GUI.
#
# Usage: tools/headless.sh <command> [args...]
set -u
RT=$(mktemp -d /tmp/pwtech-rt.XXXXXX)
chmod 700 "$RT"
export XDG_RUNTIME_DIR="$RT"
timeout -k 5 120 dbus-run-session -- bash -c '
  kwin_wayland --virtual --socket pwtest --width 1400 --height 1100 \
    --no-lockscreen --no-global-shortcuts --no-kactivities >"$XDG_RUNTIME_DIR/kwin.log" 2>&1 &
  KW=$!
  for i in $(seq 100); do [ -S "$XDG_RUNTIME_DIR/pwtest" ] && break; sleep 0.1; done
  [ -S "$XDG_RUNTIME_DIR/pwtest" ] || { echo "kwin did not start"; cat "$XDG_RUNTIME_DIR/kwin.log"; kill $KW; exit 3; }
  WAYLAND_DISPLAY=pwtest GDK_BACKEND=wayland GSETTINGS_BACKEND=memory GDK_DEBUG=no-portals GTK_USE_PORTAL=0 \
    NO_AT_BRIDGE=1 timeout -k 3 60 "$@"
  RC=$?
  kill $KW 2>/dev/null; sleep 0.5; kill -9 $KW 2>/dev/null
  exit $RC
' -- "$@" 2>&1 | grep -v "^dbus-daemon\|^$"
RC=${PIPESTATUS[0]}
rm -rf "$RT"
exit $RC
