#!/bin/bash
# Run a command inside an invisible (virtual) KWin Wayland session with its
# own D-Bus session and runtime directory, so that windows do not appear on
# the user's desktop. Used for automatic screenshots of the GUI.
#
# Everything runs in its own process group; services started through D-Bus
# activation inside the session are killed together with it at the end
# (otherwise they keep the output open and the script never returns).
#
# Usage: tools/headless.sh <command> [args...]
set -u
RT=$(mktemp -d /tmp/pwtech-rt.XXXXXX)
chmod 700 "$RT"
export XDG_RUNTIME_DIR="$RT"
LOG="$RT/output.log"
setsid timeout -k 5 120 dbus-run-session -- bash -c '
  exec </dev/null
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
' -- "$@" >"$LOG" 2>&1 &
PID=$!
wait $PID
RC=$?
# kill what is left in the process group (D-Bus activated services)
kill -TERM -- -$PID 2>/dev/null
sleep 0.3
kill -KILL -- -$PID 2>/dev/null
grep -v "^dbus-daemon\|^$\|^qt\.qpa\." "$LOG"
# the document portal leaves its FUSE mount behind
if [ -e "$RT/doc" ] || grep -q " $RT/doc " /proc/mounts; then
  fusermount3 -uz "$RT/doc" 2>/dev/null || fusermount -uz "$RT/doc" 2>/dev/null
fi
rm -rf "$RT"
exit $RC
