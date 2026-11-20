#!/bin/bash
# Cross-compile Password Tool for Windows x64 on Linux and pack a portable zip.
#
#   packaging/windows/build.sh [--no-build] [--no-zip] [--tests]
#
# --tests also builds the workspace test executables into target/windows/tests
# (run them on Windows next to the DLLs in PwTool/bin).
#
# Needs: rustup target x86_64-pc-windows-gnullvm, llvm-mingw (UCRT) and
# Python 3 (with zstd). GTK 4 comes from the MSYS2 CLANG64 repository
# (packaging/windows/fetch-gtk.py).
#
# Environment:
#   LLVM_MINGW        llvm-mingw directory (default: ~/.local/opt/llvm-mingw-*)
#   PWTOOL_WIN_SYSROOT GTK sysroot (default: ~/.cache/pwtool-win/sysroot)
#   OUTPUT            zip file (default: target/pwtool_<version>_windows_x64.zip)
#
# Result layout (GTK finds its modules relative to bin/):
#   PwTool/bin/pwtool.exe + DLLs, PwTool/lib/{gdk-pixbuf-2.0,gio},
#   PwTool/share/{glib-2.0/schemas,icons,pwtool}
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TARGET=x86_64-pc-windows-gnullvm
BUILD=1 ZIP=1 TESTS=0
for a in "$@"; do
  case "$a" in
    --no-build) BUILD=0 ;;
    --no-zip) ZIP=0 ;;
    --tests) TESTS=1 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done

if [ -z "${LLVM_MINGW:-}" ]; then
  LLVM_MINGW=$(ls -d "$HOME"/.local/opt/llvm-mingw-*-ucrt-* 2>/dev/null | sort | tail -1 || true)
fi
[ -x "${LLVM_MINGW:-}/bin/x86_64-w64-mingw32-clang" ] || { echo "llvm-mingw not found (set LLVM_MINGW)" >&2; exit 1; }
SYSROOT=${PWTOOL_WIN_SYSROOT:-$HOME/.cache/pwtool-win/sysroot}
if [ ! -f "$SYSROOT/clang64/lib/pkgconfig/gtk4.pc" ]; then
  python3 -I "$HERE/fetch-gtk.py" "$SYSROOT"
fi
PREFIX="$SYSROOT/clang64"

export PATH="$LLVM_MINGW/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang
export CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang
export CXX_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang++
export AR_x86_64_pc_windows_gnullvm=llvm-ar
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR="$SYSROOT"
export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig:$PREFIX/share/pkgconfig"
unset PKG_CONFIG_PATH

if [ $BUILD = 1 ]; then
  (cd "$ROOT" && cargo build --release --locked --target $TARGET -p pwtool)
fi
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
[ -f "$EXE" ] || { echo "missing $EXE (build first)" >&2; exit 1; }

VERSION=$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 | python3 -I -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="pwtool"))')
STAGE="$ROOT/target/windows/PwTool"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/lib" "$STAGE/share/pwtool"
cp "$EXE" "$STAGE/bin/pwtool.exe"
llvm-strip --strip-unneeded "$STAGE/bin/pwtool.exe" 2>/dev/null || true

# GDK-Pixbuf loaders (PNG is built in; SVG via librsvg) and GIO modules
# (TLS from glib-networking for the update check).
# GTK decodes PNG/JPEG itself; gdk-pixbuf is needed only for SVG icons of
# the icon theme. Only the TLS module of GIO: the proxy modules need GNOME
# settings schemas.
mkdir -p "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders" "$STAGE/lib/gio/modules"
cp "$PREFIX"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*svg*.dll "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders/"
cp "$PREFIX/lib/gio/modules/libgiognutls.dll" "$STAGE/lib/gio/modules/"

# Copy the DLLs the executable and the modules need (transitively).
python3 -I - "$PREFIX/bin" "$STAGE" <<'PY'
import os, subprocess, sys
bindir, stage = sys.argv[1], sys.argv[2]
available = {f.lower(): f for f in os.listdir(bindir) if f.lower().endswith(".dll")}
todo = [os.path.join(stage, "bin", "pwtool.exe")]
for d, _, files in os.walk(os.path.join(stage, "lib")):
    todo += [os.path.join(d, f) for f in files if f.lower().endswith(".dll")]
done = set()
while todo:
    f = todo.pop()
    out = subprocess.run(["llvm-readobj", "--coff-imports", f], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("Name:") and line.lower().endswith(".dll"):
            dll = line.split(":", 1)[1].strip().lower()
            if dll in available and dll not in done:
                done.add(dll)
                dst = os.path.join(stage, "bin", available[dll])
                subprocess.run(["cp", os.path.join(bindir, available[dll]), dst], check=True)
                todo.append(dst)
print(f"{len(done)} DLLs")
PY

# Pixbuf loader cache (gdk-pixbuf-query-loaders cannot run here); the path
# is relative to the installation directory.
LOADERS_DIR="$STAGE/lib/gdk-pixbuf-2.0/2.10.0"
SVG_LOADER=$(basename "$(ls "$LOADERS_DIR"/loaders/*svg*.dll | head -1)")
printf '"lib/gdk-pixbuf-2.0/2.10.0/loaders/%s"\n"svg" 6 "gdk-pixbuf" "Scalable Vector Graphics" "LGPL"\n"image/svg+xml" "image/svg" "image/svg-xml" "image/vnd.adobe.svg+xml" "text/xml-svg" "image/svg+xml-compressed" ""\n"svg" "svgz" "svg.gz" ""\n" <svg" "*    " 100\n" <!DOCTYPE svg" "*             " 100\n\n' \
  "$SVG_LOADER" > "$LOADERS_DIR/loaders.cache"

# GSettings schemas (GTK needs its own) compiled for the bundle.
mkdir -p "$STAGE/share/glib-2.0/schemas"
cp "$PREFIX"/share/glib-2.0/schemas/org.gtk.gtk4.*.gschema.xml "$STAGE/share/glib-2.0/schemas/"
glib-compile-schemas "$STAGE/share/glib-2.0/schemas"

# Icon themes: Adwaita (symbolic icons used by GTK widgets) and hicolor.
mkdir -p "$STAGE/share/icons"
cp -r "$PREFIX/share/icons/Adwaita" "$PREFIX/share/icons/hicolor" "$STAGE/share/icons/"
for s in 16 32 48 64 128 256; do
  mkdir -p "$STAGE/share/icons/hicolor/${s}x${s}/apps"
  cp "$ROOT/assets/appicons/pwtool_default_$s.png" "$STAGE/share/icons/hicolor/${s}x${s}/apps/io.github.coinman_dev.PwTool.png"
done

# Program data (same set as packaging/install.sh).
SHARE="$STAGE/share/pwtool"
mkdir -p "$SHARE/languages/port" "$SHARE/script_examples"
cp "$ROOT/assets/common_passwords.txt" "$ROOT/assets/license.txt" "$SHARE/"
cp "$ROOT"/assets/languages/*.po "$ROOT"/assets/languages/*.lng "$ROOT"/assets/languages/*.pot "$SHARE/languages/"
cp "$ROOT"/assets/languages/port/*.po "$SHARE/languages/port/"
cp "$ROOT/assets/script_examples/"* "$SHARE/script_examples/"
MANUALS="$ROOT/target/manuals"
if [ ! -f "$MANUALS/manual.pdf" ] && command -v soffice >/dev/null; then
  mkdir -p "$MANUALS"
  PROFILE=$(mktemp -d)
  timeout 300 soffice -env:UserInstallation=file://"$PROFILE" --headless --convert-to pdf \
    --outdir "$MANUALS" "$ROOT/doc/manual/manual.odt" "$ROOT/doc/manual/scripting.odt" >/dev/null 2>&1 || true
  rm -rf "$PROFILE"
fi
for f in manual.pdf scripting.pdf; do
  [ -f "$MANUALS/$f" ] && cp "$MANUALS/$f" "$SHARE/$f"
done
cp "$ROOT/LICENSE" "$STAGE/LICENSE.txt"
cp "$PREFIX/../packages.txt" "$STAGE/GTK-PACKAGES.txt" 2>/dev/null || true

if [ $ZIP = 1 ]; then
  OUT="${OUTPUT:-$ROOT/target/pwtool_${VERSION//-/.}_windows_x64.zip}"
  mkdir -p "$(dirname "$OUT")"
  rm -f "$OUT"
  (cd "$STAGE/.." && python3 -I -m zipfile -c "$OUT" PwTool)
  echo "$OUT"
else
  echo "$STAGE"
fi
