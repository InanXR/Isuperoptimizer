#!/usr/bin/env python3
"""check_style.py — house-style gate on the RENDERED document.

Checks the extracted PDF text (prose, captions, tables, references) for
em dashes and the banned stock constructions. Any match fails the
build, so the checks are enforced by construction rather than by
memory.
"""
import re
import subprocess
import sys

txt = subprocess.run(["pdftotext", "paper.pdf", "-"],
                     capture_output=True, text=True).stdout
pats = {
    "em dash": "\u2014",
    "is not just": r"(?i)\bis not just\b",
    "isn't just": r"(?i)\bisn['\u2019]t just\b",
    "not only": r"(?i)\bnot only\b",
    "but rather": r"(?i)\bbut rather\b",
    "is not merely": r"(?i)\bis not merely\b",
    "not merely": r"(?i)\bnot merely\b",
}
fails = 0
for name, pat in pats.items():
    ms = list(re.finditer(pat, txt))
    print(f"  {name}: {len(ms)}")
    for m in ms[:5]:
        s = txt[max(0, m.start() - 60):m.end() + 60].replace("\n", " ")
        print(f"    ...{s}...")
    fails += len(ms)
if fails:
    print(f"STYLE CHECK FAILED: {fails} occurrence(s) to revise")
    sys.exit(1)
print("style check: clean")
