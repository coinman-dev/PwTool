#!/bin/bash
# Cross-compile Password Tool for Windows x64 on Linux: one pwtool.exe that
# needs only system DLLs (static C runtime, data and manuals embedded).
#
#   packaging/windows/build.sh [--tests]
#
# --tests also builds the workspace test executables into target/windows/tests
# (they run on Windows without anything next to them).
#
# Needs: rustup target x86_64-pc-windows-gnullvm and llvm-mingw (UCRT).
# The PDF manuals are embedded if target/manuals holds them
# (packaging/manuals.sh).
#
# Environment:
#   LLVM_MINGW  llvm-mingw directory (default: ~/.local/opt/llvm-mingw-*-ucrt-*)
#   OUTPUT      exe file (default: target/pwtool_<version>_windows_x64.exe)
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TARGET=x86_64-pc-windows-gnullvm
TESTS=0
for a in "$@"; do
  case "$a" in
    --tests) TESTS=1 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

if [ -z "${LLVM_MINGW:-}" ]; then
  LLVM_MINGW=$(ls -d "$HOME"/.local/opt/llvm-mingw-*-ucrt-* 2>/dev/null | sort | tail -1 || true)
fi
[ -x "${LLVM_MINGW:-}/bin/x86_64-w64-mingw32-clang" ] || { echo "llvm-mingw not found (set LLVM_MINGW)" >&2; exit 1; }

export PATH="$LLVM_MINGW/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang
export CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang
export CXX_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang++
export AR_x86_64_pc_windows_gnullvm=llvm-ar
export WINDRES=x86_64-w64-mingw32-windres
# static C runtime (no vcruntime/ucrt redistributable needed); libc++ of
# zxcvbn linked statically too, else libc++.dll would be required
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUSTFLAGS="-C target-feature=+crt-static -L native=$LLVM_MINGW/x86_64-w64-mingw32/lib"
export CXXSTDLIB_x86_64_pc_windows_gnullvm=static=c++

(cd "$ROOT" && cargo build --release --locked --target $TARGET -p pwtool)
if [ $TESTS = 1 ]; then
  rm -rf "$ROOT/target/windows/tests"
  mkdir -p "$ROOT/target/windows/tests"
  (cd "$ROOT" && cargo test --workspace --locked --target $TARGET --no-run --message-format=json) |
    python3 -I -c '
import json, shutil, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["profile"]["test"]:
        shutil.copy(m["executable"], sys.argv[1])
        print("test executable:", m["executable"].rsplit("/", 1)[-1], file=sys.stderr)
' "$ROOT/target/windows/tests"
fi
EXE="$ROOT/target/$TARGET/release/pwtool.exe"
[ -f "$EXE" ] || { echo "missing $EXE" >&2; exit 1; }

VERSION=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 | python3 -I -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="pwtool"))')
OUT="${OUTPUT:-$ROOT/target/pwtool_${VERSION//-/.}_windows_x64.exe}"
mkdir -p "$(dirname "$OUT")"
cp "$EXE" "$OUT"
llvm-strip --strip-unneeded "$OUT" 2>/dev/null || true

# only system DLLs may be imported
python3 -I - "$OUT" <<'PY'
import subprocess, sys
allowed = {"kernel32", "user32", "gdi32", "shell32", "ole32", "oleaut32", "advapi32", "ws2_32", "bcrypt", "ntdll",
           "opengl32", "imm32", "comctl32", "comdlg32", "dwmapi", "uxtheme", "shlwapi", "winmm", "crypt32", "secur32",
           "userenv", "propsys", "dbghelp", "iphlpapi", "netapi32", "uiautomationcore", "version", "setupapi", "hid",
           "powrprof", "msimg32", "d3d11", "dxgi", "dcomp", "winspool", "mswsock", "rpcrt4", "synchronization", "oleacc",
           "bcryptprimitives", "combase", "psapi"}
out = subprocess.run(["llvm-readobj", "--coff-imports", sys.argv[1]], capture_output=True, text=True, check=True).stdout
dlls = sorted({l.split(":", 1)[1].strip().lower() for l in out.splitlines() if l.strip().startswith("Name:") and l.strip().lower().endswith(".dll")})
bad = [d for d in dlls if not (d[:-4] in allowed or d.startswith("api-ms-win-"))]
print("imports:", ", ".join(dlls))
if bad:
    sys.exit(f"unexpected DLL imports: {bad}")
PY
ls -l "$OUT"
echo "$OUT"
