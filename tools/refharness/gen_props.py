#!/usr/bin/env python3
"""Rewrite C++Builder `__property` declarations into clang `__declspec(property)`.

Usage: gen_props.py <input header> <output header>

clang has no `__property` support, so headers that declare properties are
copied into the build directory with every

    __property TYPE NAME = { read=R [, write=W] };

replaced by

    typedef TYPE __pt_type_NAME;
    __pt_type_NAME __pt_get_NAME() const { return R; }             // R is a field
    void __pt_put_NAME(const __pt_type_NAME& v) { W = v; }         // W is a field
    __declspec(property(get=..., put=...)) TYPE NAME;

If R/W is a member function (does not start with "m_"), the wrapper calls it
(the wrapper is public, so private accessor methods work as in C++Builder).
Nothing else in the header is changed; line numbers are preserved (#line).
The original file in upstream/ is never touched.
"""
import re
import sys

PROP_RE = re.compile(
    r"__property\s+(?P<type>[^=;{}]+?)\s+(?P<name>\w+)\s*=\s*\{\s*"
    r"read\s*=\s*(?P<read>\w+)\s*"
    r"(?:,\s*write\s*=\s*(?P<write>\w+)\s*)?\}\s*;",
    re.S,
)


def is_field(ident: str) -> bool:
    return ident.startswith("m_")


def repl(m: re.Match) -> str:
    typ = " ".join(m.group("type").split())
    name = m.group("name")
    rd = m.group("read")
    wr = m.group("write")
    tname = f"__pt_type_{name}"
    out = [f"typedef {typ} {tname};"]
    # Accessor wrappers are always generated (and are public, like the
    # property): in C++Builder a public property may use private accessor
    # methods, whereas clang checks the accessor's access at the use site.
    if is_field(rd):
        # by value: the field type may differ from the property type
        # (e.g. TSimpleEvent* field behind a TEvent* property)
        out.append(f"{tname} __pt_get_{name}() const {{ return {rd}; }}")
    else:
        # const_cast: the getter method may be non-const (e.g. EntropyManager)
        out.append(f"{tname} __pt_get_{name}() const {{ return const_cast<"
                   f"std::remove_const_t<std::remove_pointer_t<decltype(this)>>*>"
                   f"(this)->{rd}(); }}")
    spec = f"get=__pt_get_{name}"
    if wr:
        if is_field(wr):
            out.append(f"void __pt_put_{name}(const {tname}& v) {{ {wr} = v; }}")
        else:
            out.append(f"void __pt_put_{name}(const {tname}& v) {{ {wr}(v); }}")
        spec += f", put=__pt_put_{name}"
    out.append(f"__declspec(property({spec})) {tname} {name};")
    # keep the line count identical so that compiler diagnostics still match
    # the upstream line numbers
    nl = m.group(0).count("\n")
    return " ".join(out) + "\n" * nl


def main() -> int:
    src, dst = sys.argv[1], sys.argv[2]
    with open(src, "r", encoding="utf-8", errors="surrogateescape", newline="") as f:
        text = f.read()
    new, n = PROP_RE.subn(repl, text)
    if re.search(r"\b__property\b", new):
        sys.stderr.write(f"{src}: unconverted __property left\n")
        return 1
    with open(dst, "w", encoding="utf-8", errors="surrogateescape", newline="") as f:
        f.write(f"// GENERATED from {src} by gen_props.py ({n} properties) - do not edit\n")
        f.write(f'#line 1 "{src}"\n')
        f.write(new)
    return 0


if __name__ == "__main__":
    sys.exit(main())
