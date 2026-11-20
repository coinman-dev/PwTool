#!/usr/bin/env python3
"""Download GTK 4 for Windows (MSYS2 CLANG64 packages) into a sysroot.

Password Tool for Windows is cross-compiled on Linux with llvm-mingw for the
Rust target x86_64-pc-windows-gnullvm. The MSYS2 CLANG64 repository is built
with the same kind of toolchain (clang, UCRT, libc++), so its import
libraries, headers and DLLs can be linked and shipped as they are.

The script reads the repository database, resolves the dependency closure of
the requested packages, checks every archive against the SHA-256 sum from the
database, and unpacks it into SYSROOT (the result is SYSROOT/clang64/...).
Downloads are cached in SYSROOT/.cache. Needs Python 3.14+ or the zstd tool.

Usage: fetch-gtk.py SYSROOT [PACKAGE...]
"""

import hashlib
import io
import os
import re
import sys
import tarfile
import subprocess
import urllib.request

try:
    from compression import zstd

    def unzstd(data: bytes) -> bytes:
        return zstd.decompress(data)
except ImportError:  # Python < 3.14: the zstd command-line tool

    def unzstd(data: bytes) -> bytes:
        return subprocess.run(["zstd", "-dc"], input=data, capture_output=True, check=True).stdout

REPO = os.environ.get("MSYS2_REPO", "https://repo.msys2.org/mingw/clang64")
PREFIX = "mingw-w64-clang-x86_64-"
DEFAULT_PACKAGES = [
    "gtk4",
    "adwaita-icon-theme",
    "hicolor-icon-theme",
    "librsvg",
    "glib-networking",
]
# Packages that are dependencies only for building or for tools we do not ship.
SKIP = {
    PREFIX + "python",
    PREFIX + "gettext-tools",
    PREFIX + "gobject-introspection",
    PREFIX + "gobject-introspection-runtime",
}


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "pwtool-build"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def strip_version(dep: str) -> str:
    return re.split(r"[<>=]", dep, maxsplit=1)[0]


def read_db(cache: str) -> dict:
    data = fetch(f"{REPO}/clang64.db")
    with open(os.path.join(cache, "clang64.db"), "wb") as f:
        f.write(data)
    if data[:4] == b"\x28\xb5\x2f\xfd":
        data = unzstd(data)
    pkgs, provides = {}, {}
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:
        for m in tar:
            if not m.name.endswith("/desc"):
                continue
            fields, key = {}, None
            for line in tar.extractfile(m).read().decode().splitlines():
                if line.startswith("%") and line.endswith("%"):
                    key = line.strip("%")
                    fields[key] = []
                elif line and key:
                    fields[key].append(line)
            name = fields["NAME"][0]
            pkgs[name] = fields
            for p in fields.get("PROVIDES", []):
                provides.setdefault(strip_version(p), name)
    return pkgs, provides


def resolve(pkgs: dict, provides: dict, wanted: list[str]) -> list[str]:
    seen, order, stack = set(), [], [PREFIX + w if not w.startswith(PREFIX) else w for w in wanted]
    while stack:
        name = stack.pop()
        name = name if name in pkgs else provides.get(name, name)
        if name in seen or name in SKIP:
            continue
        if name not in pkgs:
            sys.exit(f"package not found in repository: {name}")
        seen.add(name)
        order.append(name)
        stack.extend(strip_version(d) for d in pkgs[name].get("DEPENDS", []))
    return sorted(order)


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    root = os.path.abspath(sys.argv[1])
    wanted = sys.argv[2:] or DEFAULT_PACKAGES
    cache = os.path.join(root, ".cache")
    os.makedirs(cache, exist_ok=True)

    pkgs, provides = read_db(cache)
    names = resolve(pkgs, provides, wanted)
    total = 0
    for name in names:
        desc = pkgs[name]
        filename, sha = desc["FILENAME"][0], desc["SHA256SUM"][0]
        path = os.path.join(cache, filename)
        data = open(path, "rb").read() if os.path.exists(path) else None
        if data is None or hashlib.sha256(data).hexdigest() != sha:
            print(f"download {filename}", flush=True)
            data = fetch(f"{REPO}/{filename}")
            if hashlib.sha256(data).hexdigest() != sha:
                sys.exit(f"SHA-256 mismatch: {filename}")
            with open(path, "wb") as f:
                f.write(data)
        total += len(data)
        raw = unzstd(data) if filename.endswith(".zst") else data
        with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
            members = [m for m in tar if not m.name.startswith(".")]
            tar.extractall(root, members=members, filter="tar")
    with open(os.path.join(root, "packages.txt"), "w") as f:
        for name in names:
            f.write(f"{name} {pkgs[name]['VERSION'][0]}\n")
    print(f"{len(names)} packages, {total / 1e6:.1f} MB, sysroot {root}/clang64")


if __name__ == "__main__":
    main()
