#!/usr/bin/env bash
# Builds the reference harness into .ref/refharness/refharness.exe.
# Incremental (make). Usage: build.sh [clean] [make args...]
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
TC="${TOOLCHAIN_BIN:-$ROOT/.ref/toolchain/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64/bin}"
OUT="${REFHARNESS_OUT:-$ROOT/.ref/refharness}"

if [ ! -x "$TC/x86_64-w64-mingw32-clang++" ]; then
  echo "llvm-mingw toolchain not found in $TC (set TOOLCHAIN_BIN)" >&2
  exit 2
fi
if [ ! -f "$ROOT/upstream/src/passw/PasswGen.cpp" ]; then
  echo "upstream sources missing (git submodule update --init)" >&2
  exit 2
fi

if [ "${1:-}" = "clean" ]; then
  shift
  make -C "$HERE" -f Makefile ROOT="$ROOT" OUT="$OUT" TC="$TC" clean
fi

exec make -C "$HERE" -f Makefile ROOT="$ROOT" OUT="$OUT" TC="$TC" -j"$(nproc)" "$@"
