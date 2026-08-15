import re
import sys

p = sys.argv[1] if len(sys.argv) > 1 else r"V:\Projects\sassytalkie\sassy-talks\docs\evidence\emulator-2026-08-14\uidump-auth.xml"
t = open(p, encoding="utf-8").read()
print("=== TEXT ===")
for m in re.finditer(r'text="([^"]+)"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', t):
    txt = m.group(1).encode("ascii", "replace").decode("ascii")
    print(f"{txt:50} [{m.group(2)},{m.group(3)}][{m.group(4)},{m.group(5)}]")
print("=== CLICKABLE NODES ===")
for m in re.finditer(r"<node [^>]*>", t):
    s = m.group(0)
    if 'clickable="true"' not in s:
        continue
    text = re.search(r'text="([^"]*)"', s)
    desc = re.search(r'content-desc="([^"]*)"', s)
    bounds = re.search(r'bounds="([^"]+)"', s)
    print((text.group(1) if text else ""), "|", (desc.group(1) if desc else ""), "|", bounds.group(1) if bounds else "")
