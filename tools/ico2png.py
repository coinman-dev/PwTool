#!/usr/bin/env python3
"""Extract all images of Windows .ico files as PNG files.

Usage: tools/ico2png.py OUTDIR FILE.ico...
Writes OUTDIR/<name>_<size>.png for every entry (32-bit DIB or PNG)."""
import os
import struct
import sys
import zlib


def png_bytes(w, h, rows):
    def chunk(t, data):
        c = struct.pack(">I", len(data)) + t + data
        return c + struct.pack(">I", zlib.crc32(t + data) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + r for r in rows)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def dib_to_png(d, w, h):
    hdr = struct.unpack("<IiiHHIIiiII", d[:40])
    bpp = hdr[4]
    if bpp != 32:
        raise ValueError(f"unsupported bit depth {bpp}")
    stride = w * 4
    rows = []
    for y in range(h):
        src = d[40 + (h - 1 - y) * stride: 40 + (h - y) * stride]
        row = bytearray()
        for x in range(w):
            b, g, r, a = src[4 * x: 4 * x + 4]
            row += bytes((r, g, b, a))
        rows.append(bytes(row))
    return png_bytes(w, h, rows)


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for fn in sys.argv[2:]:
        d = open(fn, "rb").read()
        n = struct.unpack("<HHH", d[:6])[2]
        base = os.path.splitext(os.path.basename(fn))[0]
        for i in range(n):
            w, h, _c, _r, _pl, _bpp, sz, off = struct.unpack("<BBBBHHII", d[6 + 16 * i: 22 + 16 * i])
            w, h = w or 256, h or 256
            img = d[off: off + sz]
            data = img if img[:4] == b"\x89PNG" else dib_to_png(img, w, h)
            with open(os.path.join(out, f"{base}_{w}.png"), "wb") as f:
                f.write(data)


if __name__ == "__main__":
    main()
