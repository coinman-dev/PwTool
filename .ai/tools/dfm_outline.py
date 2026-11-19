#!/usr/bin/env python3
"""Print a compact outline (object tree, captions, shortcuts, geometry) of the
form outlines produced by dfm2txt.py.

Usage:  python3 -I dfm_outline.py <forms-dir> [--menus-only] [Form ...]
"""
import re
import sys
from pathlib import Path

VK = {8: "BkSp", 9: "Tab", 13: "Enter", 27: "Esc", 32: "Space", 33: "PgUp",
      34: "PgDn", 35: "End", 36: "Home", 37: "Left", 38: "Up", 39: "Right",
      40: "Down", 45: "Ins", 46: "Del"}


def shortcut(code: int) -> str:
    mods = []
    if code & 0x4000:
        mods.append("Ctrl")
    if code & 0x8000:
        mods.append("Alt")
    if code & 0x2000:
        mods.append("Shift")
    key = code & 0xFF
    if 0x70 <= key <= 0x87:
        name = f"F{key - 0x6F}"
    elif key in VK:
        name = VK[key]
    elif 0x30 <= key <= 0x5A:
        name = chr(key)
    else:
        name = f"VK{key}"
    return "+".join(mods + [name])


def unquote(v: str) -> str:
    """Decode a DFM string literal such as 'a'#13#10'b' (or a \\-quoted one)."""
    out, i = [], 0
    while i < len(v):
        c = v[i]
        if c == "'":
            j = i + 1
            buf = []
            while j < len(v):
                if v[j] == "\\" and j + 1 < len(v):
                    buf.append({"n": "\n", "r": "\r"}.get(v[j + 1], v[j + 1]))
                    j += 2
                elif v[j] == "'":
                    if j + 1 < len(v) and v[j + 1] == "'":
                        buf.append("'")
                        j += 2
                    else:
                        break
                else:
                    buf.append(v[j])
                    j += 1
            out.append("".join(buf))
            i = j + 1
        elif c == "#":
            m = re.match(r"#(\d+)", v[i:])
            out.append(chr(int(m.group(1))))
            i += len(m.group(0))
        else:
            i += 1
    return "".join(out).replace("\r", "").replace("\n", "\\n")


def outline(path: Path, menus_only: bool):
    stack = []   # (indent, dict)
    printed = []

    def flush(obj):
        if obj.get("_done"):
            return
        obj["_done"] = True
        cls = obj["cls"]
        is_menu = cls in ("TMenuItem", "TMainMenu", "TPopupMenu")
        if menus_only and not is_menu:
            return
        parts = [f"{'  ' * obj['depth']}{obj['name']}: {cls}"]
        if "Caption" in obj:
            parts.append(f"\"{obj['Caption']}\"")
        if "ShortCut" in obj:
            parts.append(f"[{shortcut(int(obj['ShortCut']))}]")
        if not menus_only and "Left" in obj and "Width" in obj:
            parts.append(f"@{obj.get('Left')},{obj.get('Top')} "
                         f"{obj.get('Width')}x{obj.get('Height')}")
        if "Hint" in obj:
            parts.append(f"hint=\"{obj['Hint']}\"")
        for k in ("Checked", "RadioItem", "AutoCheck", "Visible", "Enabled"):
            if k in obj:
                parts.append(f"{k}={obj[k]}")
        printed.append(" ".join(parts))

    lines = path.read_text(encoding="utf-8").splitlines()
    i = 0
    while i < len(lines):
        ln = lines[i]
        s = ln.strip()
        m = re.match(r"(object|inherited|inline) (\w+): (\w+)", s)
        if m:
            if stack:
                flush(stack[-1])
            stack.append({"cls": m.group(3), "name": m.group(2),
                          "depth": len(stack)})
        elif s == "end" and stack:
            flush(stack.pop())
        elif stack and " = " in s:
            k, v = s.split(" = ", 1)
            # multi-line string continuation in text DFM
            while v.rstrip().endswith("+") and i + 1 < len(lines):
                i += 1
                v = v.rstrip()[:-1] + lines[i].strip()
            if k in ("Caption", "Hint"):
                stack[-1][k] = unquote(v)
            elif k in ("ShortCut", "Left", "Top", "Width", "Height", "Checked",
                       "RadioItem", "AutoCheck", "Visible", "Enabled"):
                stack[-1].setdefault(k, v)
        i += 1
    print(f"===== {path.stem} =====")
    print("\n".join(printed))


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2
    menus_only = "--menus-only" in sys.argv
    forms_dir = Path(args[0])
    names = args[1:] or sorted(p.stem for p in forms_dir.glob("*.txt"))
    for n in names:
        outline(forms_dir / (n + ".txt"), menus_only)
    return 0


if __name__ == "__main__":
    sys.exit(main())
