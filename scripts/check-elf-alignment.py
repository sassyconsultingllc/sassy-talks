#!/usr/bin/env python3
"""Fail when an Android ELF shared library is not 16 KiB page compatible."""

from __future__ import annotations

import argparse
import pathlib
import re
import shutil
import subprocess
import sys

MIN_ALIGNMENT = 0x4000
LOAD_RE = re.compile(
    r"^\s*LOAD\s+(0x[0-9a-f]+)\s+(0x[0-9a-f]+)\s+"
    r"(0x[0-9a-f]+).*?\s(0x[0-9a-f]+)\s*$",
    re.IGNORECASE,
)


def elf_files(paths: list[str]) -> list[pathlib.Path]:
    found: set[pathlib.Path] = set()
    for raw in paths:
        path = pathlib.Path(raw)
        if path.is_dir():
            found.update(p for p in path.rglob("*.so") if p.is_file())
        elif path.is_file():
            found.add(path)
    return sorted(found)


def inspect(readelf: str, path: pathlib.Path) -> list[str]:
    proc = subprocess.run(
        [readelf, "-lW", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode:
        return [f"could not read ELF headers: {proc.stderr.strip()}"]

    segments = []
    errors = []
    for line in proc.stdout.splitlines():
        match = LOAD_RE.match(line)
        if not match:
            continue
        offset, vaddr, _paddr, alignment = (int(value, 16) for value in match.groups())
        segments.append((offset, vaddr, alignment))
        if alignment < MIN_ALIGNMENT:
            errors.append(
                f"LOAD p_align=0x{alignment:x}, expected >=0x{MIN_ALIGNMENT:x}"
            )
        elif (vaddr - offset) % alignment:
            errors.append(
                f"LOAD offset 0x{offset:x} and vaddr 0x{vaddr:x} are not congruent"
            )
    if not segments:
        errors.append("no LOAD program headers found")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("paths", nargs="+", help="ELF .so files or directories")
    args = parser.parse_args()

    readelf = shutil.which("readelf") or shutil.which("llvm-readelf")
    if not readelf:
        print("error: readelf or llvm-readelf is required", file=sys.stderr)
        return 2

    files = elf_files(args.paths)
    if not files:
        print("error: no .so files found", file=sys.stderr)
        return 2

    failed = False
    for path in files:
        errors = inspect(readelf, path)
        if errors:
            failed = True
            for error in errors:
                print(f"FAIL {path}: {error}", file=sys.stderr)
        else:
            print(f"PASS {path}: 16 KiB compatible")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
