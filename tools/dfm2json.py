#!/usr/bin/env python3
"""Convert the original Delphi/C++Builder forms (.dfm, binary or text) into
JSON form descriptions for the GTK user interface, and extract the PNG images
of image collections.

Output:
  <dest>/forms/<Form>.json        object tree with all properties
  <dest>/icons/<Form>/<Collection>/<ImageName>.png

Value encoding in JSON:
  integer/float -> number, string -> string, True/False -> bool,
  identifier -> {"id": name}, set -> {"set": [...]}, list -> {"list": [...]},
  collection -> {"items": [{"index": n|null, "props": {...}}]},
  binary -> {"bin": length} (PNG images are written to files).

Usage: python3 -I tools/dfm2json.py <dir-with-dfm> <dest-dir>
"""
import json
import re
import struct
import sys
from pathlib import Path

PNG_SIG = b"\x89PNG\r\n\x1a\n"


# ---------------------------------------------------------------------------
# binary DFM

class BinReader:
    def __init__(self, data):
        self.d = data
        self.p = 0

    def u8(self):
        v = self.d[self.p]
        self.p += 1
        return v

    def peek(self):
        return self.d[self.p]

    def take(self, n):
        v = self.d[self.p:self.p + n]
        if len(v) != n:
            raise ValueError("unexpected end")
        self.p += n
        return v

    def sstr(self):
        return self.take(self.u8()).decode("utf-8", "replace")

    def i32(self):
        return struct.unpack("<i", self.take(4))[0]


def bin_value(r):
    t = r.u8()
    if t == 0:
        return None
    if t == 1:
        items = []
        while r.peek() != 0:
            items.append(bin_value(r))
        r.u8()
        return {"list": items}
    if t == 2:
        return struct.unpack("<b", r.take(1))[0]
    if t == 3:
        return struct.unpack("<h", r.take(2))[0]
    if t == 4:
        return r.i32()
    if t == 5:
        mant, exp = struct.unpack("<QH", r.take(10))
        sign = -1.0 if exp & 0x8000 else 1.0
        exp &= 0x7FFF
        return 0.0 if (exp == 0 and mant == 0) else sign * mant * 2.0 ** (exp - 16383 - 63)
    if t == 6:
        return r.take(r.u8()).decode("cp1252", "replace")
    if t == 7:
        s = r.sstr()
        return {"id": s}
    if t == 8:
        return False
    if t == 9:
        return True
    if t == 10:
        n = r.i32()
        return {"bin": r.take(n)}
    if t == 11:
        items = []
        while True:
            s = r.sstr()
            if not s:
                break
            items.append(s)
        return {"set": items}
    if t == 12:
        return r.take(r.i32()).decode("cp1252", "replace")
    if t == 13:
        return {"id": "nil"}
    if t == 14:
        items = []
        while r.peek() != 0:
            idx = None
            if r.peek() in (2, 3, 4):
                idx = bin_value(r)
            if r.u8() != 1:
                raise ValueError("bad collection item")
            items.append({"index": idx, "props": bin_props(r)})
        r.u8()
        return {"items": items}
    if t == 15:
        return struct.unpack("<f", r.take(4))[0]
    if t == 16:
        return struct.unpack("<q", r.take(8))[0] / 10000.0
    if t == 17:
        return struct.unpack("<d", r.take(8))[0]
    if t == 18:
        return r.take(r.i32() * 2).decode("utf-16-le", "replace")
    if t == 19:
        return struct.unpack("<q", r.take(8))[0]
    if t == 20:
        return r.take(r.i32()).decode("utf-8", "replace")
    if t == 21:
        return struct.unpack("<d", r.take(8))[0]
    raise ValueError(f"unknown value type {t}")


def bin_props(r):
    props = {}
    while True:
        name = r.sstr()
        if not name:
            return props
        props[name] = bin_value(r)


def bin_object(r):
    kind = "object"
    if r.peek() & 0xF0 == 0xF0:
        flags = r.u8() & 0x0F
        if flags & 1:
            kind = "inherited"
        if flags & 4:
            kind = "inline"
        if flags & 2:
            bin_value(r)
    cls = r.sstr()
    name = r.sstr()
    obj = {"kind": kind, "name": name, "class": cls, "props": bin_props(r), "children": []}
    while r.peek() != 0:
        obj["children"].append(bin_object(r))
    r.u8()
    return obj


# ---------------------------------------------------------------------------
# text DFM

TOKEN_RE = re.compile(r"""
    (?P<ws>\s+)
  | (?P<str>(?:'(?:[^']|'')*'|\#\d+)+)
  | (?P<num>[-+]?(?:\d+\.\d*(?:[eE][-+]?\d+)?|\d+(?:[eE][-+]?\d+)?|\$[0-9A-Fa-f]+))
  | (?P<ident>[^\W\d][\w.]*)
  | (?P<punct>[=<>\[\](){},:+\-])
""", re.VERBOSE)


def decode_str(tok):
    out = []
    i = 0
    while i < len(tok):
        if tok[i] == "'":
            j = i + 1
            buf = []
            while True:
                if tok[j] == "'":
                    if j + 1 < len(tok) and tok[j + 1] == "'":
                        buf.append("'")
                        j += 2
                        continue
                    break
                buf.append(tok[j])
                j += 1
            out.append("".join(buf))
            i = j + 1
        else:
            m = re.match(r"#(\d+)", tok[i:])
            out.append(chr(int(m.group(1))))
            i += len(m.group(0))
    return "".join(out)


class TextParser:
    def __init__(self, text):
        self.toks = []
        pos = 0
        while pos < len(text):
            m = TOKEN_RE.match(text, pos)
            if not m:
                raise ValueError(f"bad char {text[pos]!r} at {pos}: {text[pos:pos+40]!r}")
            pos = m.end()
            kind = m.lastgroup
            if kind == "ws":
                continue
            self.toks.append((kind, m.group(kind)))
        self.i = 0

    def peek(self, k=0):
        return self.toks[self.i + k] if self.i + k < len(self.toks) else (None, None)

    def next(self):
        t = self.toks[self.i]
        self.i += 1
        return t

    def expect(self, v):
        k, t = self.next()
        if t != v:
            raise ValueError(f"expected {v!r}, got {t!r} near token {self.i}")

    def parse_object(self):
        _, kind = self.next()  # object / inherited / inline
        _, name = self.next()
        self.expect(":")
        _, cls = self.next()
        if self.peek()[1] == "[":  # [index]
            while self.next()[1] != "]":
                pass
        obj = {"kind": kind, "name": name, "class": cls, "props": {}, "children": []}
        while True:
            k, t = self.peek()
            if t == "end":
                self.next()
                return obj
            if t in ("object", "inherited", "inline") and self.peek(2)[1] == ":":
                obj["children"].append(self.parse_object())
                continue
            _, pname = self.next()
            self.expect("=")
            obj["props"][pname] = self.parse_value()

    def parse_value(self):
        k, t = self.next()
        if k == "str":
            s = decode_str(t)
            while self.peek()[1] == "+" and self.peek(1)[0] == "str":
                self.next()
                s += decode_str(self.next()[1])
            return s
        if k == "num":
            if t.startswith("$"):
                return int(t[1:], 16)
            if "." in t or "e" in t.lower():
                return float(t)
            return int(t)
        if t == "-" and self.peek()[0] == "num":
            v = self.parse_value()
            return -v
        if k == "ident":
            if t == "True":
                return True
            if t == "False":
                return False
            return {"id": t}
        if t == "[":
            items = []
            while True:
                k2, t2 = self.next()
                if t2 == "]":
                    return {"set": items}
                if t2 != ",":
                    items.append(t2)
        if t == "(":
            items = []
            while self.peek()[1] != ")":
                items.append(self.parse_value())
            self.next()
            return {"list": items}
        if t == "<":
            items = []
            while self.peek()[1] != ">":
                idx = None
                if self.peek()[1] == "item":
                    self.next()
                    if self.peek()[1] == "[":
                        self.next()
                        idx = self.parse_value()
                        self.expect("]")
                props = {}
                while self.peek()[1] != "end":
                    _, pname = self.next()
                    self.expect("=")
                    props[pname] = self.parse_value()
                self.next()
                items.append({"index": idx, "props": props})
            self.next()
            return {"items": items}
        if t == "{":
            hexs = []
            while True:
                k2, t2 = self.next()
                if t2 == "}":
                    break
                hexs.append(t2)
            return {"bin": bytes.fromhex("".join(hexs))}
        raise ValueError(f"unexpected token {t!r}")


def tokenize_hex_safe(text):
    # Hex blocks may contain tokens like "0A1B" that the tokenizer splits into
    # numbers and identifiers; joining the raw token texts restores them.
    return text


# ---------------------------------------------------------------------------

def extract_images(obj, form, dest):
    count = 0
    if obj["class"] == "TImageCollection":
        coll = obj["props"].get("Images", {}).get("items", [])
        for item in coll:
            name = item["props"].get("Name", f"img{len(coll)}")
            src = item["props"].get("SourceImages", {}).get("items", [])
            for k, s in enumerate(src):
                data = s["props"].get("Image.Data", {}).get("bin")
                if not data:
                    continue
                pos = data.find(PNG_SIG)
                if pos < 0:
                    continue
                d = dest / "icons" / form / obj["name"]
                d.mkdir(parents=True, exist_ok=True)
                suffix = "" if k == 0 else f"_{k}"
                (d / f"{name}{suffix}.png").write_bytes(data[pos:])
                count += 1
    for c in obj["children"]:
        count += extract_images(c, form, dest)
    return count


def strip_binary(v):
    if isinstance(v, dict):
        if "bin" in v:
            return {"bin": len(v["bin"])}
        return {k: strip_binary(x) for k, x in v.items()}
    if isinstance(v, list):
        return [strip_binary(x) for x in v]
    return v


def convert(path, dest):
    data = path.read_bytes()
    pos = data.find(b"TPF0")
    if 0 <= pos < 64:
        obj = bin_object(BinReader(data[pos + 4:]))
    else:
        if data.startswith(b"\xef\xbb\xbf"):
            data = data[3:]
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            text = data.decode("cp1252")
        obj = TextParser(text).parse_object()
    n = extract_images(obj, path.stem, dest)
    (dest / "forms").mkdir(parents=True, exist_ok=True)
    (dest / "forms" / f"{path.stem}.json").write_text(
        json.dumps(strip_binary(obj), ensure_ascii=False, indent=1), encoding="utf-8")
    return n


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    src, dest = Path(sys.argv[1]), Path(sys.argv[2])
    rc = 0
    for f in sorted(src.glob("*.dfm")):
        try:
            n = convert(f, dest)
            print(f"{f.name:28s} ok, {n} images")
        except Exception as e:
            print(f"{f.name:28s} FAILED: {e}")
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
