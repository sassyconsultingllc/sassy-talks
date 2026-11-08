#!/usr/bin/env python3
# Copyright (c) 2026 Shane Smith / Sassy Consulting LLC. All rights reserved.
# Proprietary source. This notice is Copyright Management Information (17 U.S.C. 1202); removal or alteration prohibited.
# CodeMark: SCLLC1-sassytalkie-IOSFFICHK7Q2
"""Check that the hand-maintained iOS bridging header matches the Rust FFI.

The Swift app calls the Rust static library through
`ios-native/SassyTalkie-Bridging-Header.h`. Nothing generates that header, so a
Rust signature change (an added parameter, a bool that became a u8) compiles on
both sides and then corrupts the C ABI at runtime. This compares every
`#[no_mangle] extern "C" fn` in ios-native/src against the header prototypes:
same set of names, same arity, same C types. Nullability annotations are
ignored; they are a Swift-import concern, not an ABI one.

Exit 0 when they agree, 1 with a list of mismatches otherwise.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
RUST_DIR = ROOT / "ios-native" / "src"
HEADER = ROOT / "ios-native" / "SassyTalkie-Bridging-Header.h"

RUST_TO_C = {
    "bool": "bool",
    "u8": "uint8_t",
    "u16": "uint16_t",
    "u32": "uint32_t",
    "u64": "uint64_t",
    "i8": "int8_t",
    "i16": "int16_t",
    "i32": "int32_t",
    "i64": "int64_t",
    "usize": "size_t",
    "c_char": "char",
    "()": "void",
}

RUST_FN = re.compile(
    r"#\[no_mangle\]\s*pub\s+(?:unsafe\s+)?extern\s+\"C\"\s+fn\s+(\w+)\s*\((.*?)\)\s*(?:->\s*([^{]+?))?\s*\{",
    re.S,
)
C_FN = re.compile(r"^\s*([A-Za-z_][\w\s\*]*?[\s\*])(sassytalkie_\w+)\s*\(([^)]*)\)\s*;", re.M)


def rust_type_to_c(t: str) -> str:
    """`*const c_char` -> `const char*`, `*mut usize` -> `size_t*`."""
    t = t.strip()
    pointers = []  # outermost first: True = const
    while True:
        m = re.match(r"^\*(const|mut)\s+(.*)$", t)
        if not m:
            break
        pointers.append(m.group(1) == "const")
        t = m.group(2).strip()
    out = RUST_TO_C.get(t.split("::")[-1], t)
    for is_const in reversed(pointers):
        out = f"const {out}*" if is_const else f"{out}*"
    return out


def normalize_c(t: str) -> str:
    t = re.sub(r"_Nullable|_Nonnull|_Null_unspecified", "", t)
    t = re.sub(r"\s+", " ", t).strip()
    t = t.replace(" *", "*").replace("* ", "*")
    return t


def parse_rust():
    fns = {}
    for path in sorted(RUST_DIR.rglob("*.rs")):
        src = path.read_text(encoding="utf-8")
        for name, params, ret in RUST_FN.findall(src):
            args = []
            params = params.strip()
            if params:
                for p in params.split(","):
                    p = p.strip()
                    if not p:
                        continue
                    _, ty = p.split(":", 1)
                    args.append(normalize_c(rust_type_to_c(ty)))
            r = normalize_c(rust_type_to_c(ret)) if ret else "void"
            fns[name] = (r, args, path.name)
    return fns


def parse_header():
    src = HEADER.read_text(encoding="utf-8")
    src = re.sub(r"//[^\n]*", "", src)
    fns = {}
    for ret, name, params in C_FN.findall(src):
        params = params.strip()
        args = []
        if params and params != "void":
            for p in params.split(","):
                p = normalize_c(p)
                # Drop the parameter name (last identifier).
                m = re.match(r"^(.*?[\s\*])(\w+)$", p)
                args.append(normalize_c(m.group(1) if m else p))
        fns[name] = (normalize_c(ret), args)
    return fns


def main() -> int:
    rust = parse_rust()
    header = parse_header()
    problems = []
    for name in sorted(set(rust) - set(header)):
        problems.append(f"missing from header: {name} (defined in {rust[name][2]})")
    for name in sorted(set(header) - set(rust)):
        problems.append(f"declared in header but not exported by Rust: {name}")
    for name in sorted(set(rust) & set(header)):
        r_ret, r_args, _ = rust[name]
        h_ret, h_args = header[name]
        if r_ret != h_ret:
            problems.append(f"{name}: return type Rust `{r_ret}` vs header `{h_ret}`")
        if r_args != h_args:
            problems.append(f"{name}: params Rust {r_args} vs header {h_args}")
    if problems:
        print("iOS FFI header drift:")
        for p in problems:
            print("  - " + p)
        return 1
    print(f"iOS FFI header matches Rust ({len(rust)} functions).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
