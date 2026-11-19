#!/usr/bin/env python3
"""Convert Delphi/C++Builder form files (.dfm) to a uniform text outline.

Handles both binary DFM (TPF0 stream, optionally wrapped in a resource
header) and text DFM. Output is one indented text file per form with every
object, its class and all properties, so the forms can be used as the layout
reference for the Linux port.

Usage:  python3 -I dfm2txt.py <src-dir-with-dfm> <dest-dir>
"""
import struct
import sys
from pathlib import Path

VA_NULL, VA_LIST, VA_INT8, VA_INT16, VA_INT32, VA_EXTENDED, VA_STRING, \
    VA_IDENT, VA_FALSE, VA_TRUE, VA_BINARY, VA_SET, VA_LSTRING, VA_NIL, \
    VA_COLLECTION, VA_SINGLE, VA_CURRENCY, VA_DATE, VA_WSTRING, VA_INT64, \
    VA_UTF8STRING, VA_DOUBLE = range(22)


class Reader:
    def __init__(self, data: bytes):
        self.d = data
        self.p = 0

    def u8(self) -> int:
        v = self.d[self.p]
        self.p += 1
        return v

    def peek(self) -> int:
        return self.d[self.p]

    def take(self, n: int) -> bytes:
        if self.p + n > len(self.d):
            raise ValueError("unexpected end of DFM stream")
        v = self.d[self.p:self.p + n]
        self.p += n
        return v

    def sstr(self) -> str:
        return self.take(self.u8()).decode("utf-8", "replace")

    def i32(self) -> int:
        return struct.unpack("<i", self.take(4))[0]


def quote(s: str) -> str:
    return "'" + s.replace("\\", "\\\\").replace("\n", "\\n").replace(
        "\r", "\\r").replace("'", "\\'") + "'"


def read_value(r: Reader, out: list, indent: int):
    """Return a printable one-line value; nested collections are appended to
    `out` as extra indented lines."""
    t = r.u8()
    if t == VA_NULL:
        return "null"
    if t == VA_LIST:
        items = []
        while r.peek() != VA_NULL:
            items.append(read_value(r, out, indent))
        r.u8()
        return "(" + ", ".join(items) + ")"
    if t == VA_INT8:
        return str(struct.unpack("<b", r.take(1))[0])
    if t == VA_INT16:
        return str(struct.unpack("<h", r.take(2))[0])
    if t == VA_INT32:
        return str(r.i32())
    if t == VA_EXTENDED:
        raw = r.take(10)
        mant, exp = struct.unpack("<QH", raw)
        sign = -1.0 if exp & 0x8000 else 1.0
        exp &= 0x7FFF
        if exp == 0 and mant == 0:
            return "0.0"
        return repr(sign * mant * 2.0 ** (exp - 16383 - 63))
    if t == VA_STRING:
        return quote(r.take(r.u8()).decode("cp1252", "replace"))
    if t == VA_IDENT:
        return r.sstr()
    if t == VA_FALSE:
        return "False"
    if t == VA_TRUE:
        return "True"
    if t == VA_BINARY:
        n = r.i32()
        r.take(n)
        return f"{{binary, {n} bytes}}"
    if t == VA_SET:
        items = []
        while True:
            s = r.sstr()
            if not s:
                break
            items.append(s)
        return "[" + ", ".join(items) + "]"
    if t == VA_LSTRING:
        return quote(r.take(r.i32()).decode("cp1252", "replace"))
    if t == VA_NIL:
        return "nil"
    if t == VA_COLLECTION:
        pad = "  " * (indent + 1)
        n = 0
        while r.peek() != VA_NULL:
            order = ""
            if r.peek() in (VA_INT8, VA_INT16, VA_INT32):
                order = " [" + read_value(r, out, indent) + "]"
            if r.u8() != VA_LIST:
                raise ValueError("bad collection item")
            out.append(f"{pad}item{order}")
            read_props(r, out, indent + 2)
            n += 1
        r.u8()
        return f"<collection, {n} items>"
    if t == VA_SINGLE:
        return repr(struct.unpack("<f", r.take(4))[0])
    if t == VA_CURRENCY:
        return str(struct.unpack("<q", r.take(8))[0] / 10000.0)
    if t == VA_DATE:
        return repr(struct.unpack("<d", r.take(8))[0])
    if t == VA_WSTRING:
        return quote(r.take(r.i32() * 2).decode("utf-16-le", "replace"))
    if t == VA_INT64:
        return str(struct.unpack("<q", r.take(8))[0])
    if t == VA_UTF8STRING:
        return quote(r.take(r.i32()).decode("utf-8", "replace"))
    if t == VA_DOUBLE:
        return repr(struct.unpack("<d", r.take(8))[0])
    raise ValueError(f"unknown value type {t} at offset {r.p - 1}")


def read_props(r: Reader, out: list, indent: int):
    pad = "  " * indent
    while True:
        name = r.sstr()
        if not name:
            return
        nested: list = []
        val = read_value(r, nested, indent)
        out.append(f"{pad}{name} = {val}")
        out.extend(nested)


def read_object(r: Reader, out: list, indent: int):
    pad = "  " * indent
    kind = "object"
    if r.peek() & 0xF0 == 0xF0:
        flags = r.u8() & 0x0F
        if flags & 1:
            kind = "inherited"
        if flags & 4:
            kind = "inline"
        if flags & 2:  # child position follows
            read_value(r, [], indent)
    cls = r.sstr()
    name = r.sstr()
    out.append(f"{pad}{kind} {name}: {cls}")
    read_props(r, out, indent + 1)
    while r.peek() != VA_NULL:
        read_object(r, out, indent + 1)
    r.u8()
    out.append(f"{pad}end")


def convert_binary(data: bytes) -> str:
    pos = data.find(b"TPF0")
    if pos < 0:
        raise ValueError("TPF0 signature not found")
    r = Reader(data[pos + 4:])
    out: list = []
    read_object(r, out, 0)
    return "\n".join(out) + "\n"


def convert_text(data: bytes) -> str:
    """Text DFM: normalise encoding and decode #NNN / 'xx' string literals
    are left as is (they are already readable)."""
    if data.startswith(b"\xef\xbb\xbf"):
        data = data[3:]
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = data.decode("cp1252", "replace")
    return text.replace("\r\n", "\n")


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    src, dest = Path(sys.argv[1]), Path(sys.argv[2])
    dest.mkdir(parents=True, exist_ok=True)
    rc = 0
    for f in sorted(src.glob("*.dfm")):
        data = f.read_bytes()
        try:
            if b"TPF0" in data[:64]:
                text, kind = convert_binary(data), "binary"
            else:
                text, kind = convert_text(data), "text"
        except Exception as e:  # report and continue with the other forms
            print(f"{f.name}: FAILED ({e})")
            rc = 1
            continue
        (dest / (f.stem + ".txt")).write_text(text, encoding="utf-8")
        n = sum(1 for ln in text.splitlines()
                if ln.lstrip().startswith(("object ", "inherited ", "inline ")))
        print(f"{f.name:28s} {kind:6s} {n:4d} objects")
    return rc


if __name__ == "__main__":
    sys.exit(main())
