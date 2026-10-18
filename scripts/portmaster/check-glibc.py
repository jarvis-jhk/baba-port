#!/usr/bin/env python3
"""Prints the glibc/libgcc symbol versions an aarch64 ELF really requires
(weak version needs - optional symbols like statx/getrandom that Rust's std
looks up at run time - are listed separately) and fails if a hard
requirement is newer than --max (default 2.28, the sysroot's glibc).
Uses readelf. Usage: check-glibc.py [--max 2.28] binary..."""
import argparse
import re
import subprocess
import sys

ap = argparse.ArgumentParser()
ap.add_argument("--max", default="2.28")
ap.add_argument("files", nargs="+")
a = ap.parse_args()
vt = lambda s: tuple(int(x) for x in s.split("."))  # noqa: E731
bad = False
for f in a.files:
    out = subprocess.run(["readelf", "-VW", f], capture_output=True, text=True, check=True).stdout
    hard, weak = set(), set()
    for m in re.finditer(r"Name: GLIBC_([0-9.]+)\s+Flags: (\S+)", out):
        (weak if m.group(2) == "WEAK" else hard).add(m.group(1))
    top = max(hard, key=vt) if hard else "none"
    print("%s: needs GLIBC %s (hard: %s; weak: %s)" % (f, top, " ".join(sorted(hard, key=vt)),
                                                       " ".join(sorted(weak, key=vt)) or "-"))
    if hard and vt(top) > vt(a.max):
        print("  ERROR: newer than GLIBC_%s" % a.max)
        bad = True
sys.exit(1 if bad else 0)
