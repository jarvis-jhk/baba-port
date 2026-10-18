#!/usr/bin/env python3
"""Writes the licences of every Rust crate linked into a baba-port build.

Usage: python3 -I packaging/gen-licenses.py OUT.txt [TARGET_TRIPLE] [PACKAGE]
(run from the workspace root after `source env.sh`; PACKAGE defaults to
baba-port, the Android build uses baba-android).

Follows the normal (non-dev, non-build) dependencies of PACKAGE for
TARGET_TRIPLE through `cargo metadata` (Cargo.lock, so the vendored ureq and
the Warpzone checkout are what is actually linked), then writes, for each
distinct licence text found in the crates' source directories (LICENSE*,
COPYING*, NOTICE*, ...; for path dependencies also up to two folders above),
the crates it belongs to followed by the text. Crates of this workspace are
skipped (their licence is the project's own). Prints a summary of licence
expressions and exits 1 if one looks like copyleft without an alternative.
Adapted from bopl-port's script (same author).
"""
import hashlib
import json
import os
import re
import subprocess
import sys

out_path = sys.argv[1]
triple = sys.argv[2] if len(sys.argv) > 2 else "x86_64-unknown-linux-gnu"
package = sys.argv[3] if len(sys.argv) > 3 else "baba-port"
meta = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", triple],
        check=True, capture_output=True, text=True,
    ).stdout
)
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = next(p["id"] for p in meta["packages"] if p["name"] == package)
workspace = set(meta["workspace_members"])

seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k.get("kind") is None for k in d["dep_kinds"]):
            stack.append(d["pkg"])
# `cargo metadata` resolves features for the whole workspace (e.g. sdl2 shows
# up under baba-android); `cargo tree -p` has this package's own feature set,
# so keep only what it lists.
tree = subprocess.run(
    ["cargo", "tree", "--locked", "-p", package, "--target", triple, "-e", "normal",
     "--prefix", "none", "-f", "{p}"],
    check=True, capture_output=True, text=True,
).stdout
linked = {tuple(line.split()[:2]) for line in tree.splitlines() if line.strip()}
seen = {i for i in seen if (pkgs[i]["name"], "v" + pkgs[i]["version"]) in linked}

LICENSE_RE = re.compile(r"^(licen[sc]e|copying|notice|unlicense|copyright)", re.I)


def license_files(d):
    return [os.path.join(d, f) for f in sorted(os.listdir(d))
            if os.path.isfile(os.path.join(d, f)) and LICENSE_RE.match(f)
            and not f.endswith((".rs", ".toml"))]


texts = {}  # sha -> (text, [crates])
exprs = {}  # expression -> [crates]
missing = []
for i in sorted(seen, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
    if i in workspace:
        continue
    p = pkgs[i]
    label = f'{p["name"]} {p["version"]}'
    exprs.setdefault(p.get("license") or "(none)", []).append(label)
    d = os.path.dirname(p["manifest_path"])
    files = license_files(d)
    if not files and p.get("source") is None:
        # Path dependency inside another workspace (ext/warpzone/crates/core).
        for up in (os.path.dirname(d), os.path.dirname(os.path.dirname(d))):
            files = license_files(up)
            if files:
                break
    found = False
    for fp in files:
        raw = open(fp, "rb").read().decode("utf-8", "replace").strip()
        if not raw:
            continue
        h = hashlib.sha256(re.sub(r"\s+", " ", raw).encode()).hexdigest()
        texts.setdefault(h, (raw, []))[1].append(label)
        found = True
    if not found:
        missing.append(label)

n = len(seen - workspace)
with open(out_path, "w") as o:
    o.write(f"Licences of the Rust crates linked into {package}\n")
    o.write(f"({n} crates, target {triple})\n\n")
    o.write("Licence expressions as declared by the crates:\n\n")
    for e, cs in sorted(exprs.items()):
        o.write(f"  {e}  ({len(cs)} crates)\n")
    o.write("\nCrates without a licence file in their source (licence per the expression above):\n")
    o.write("  " + (", ".join(missing) if missing else "none") + "\n")
    for text, crates in sorted(texts.values(), key=lambda t: t[1][0]):
        o.write("\n" + "=" * 78 + "\n")
        o.write("Used by: " + ", ".join(crates) + "\n")
        o.write("=" * 78 + "\n\n" + text + "\n")

print(f"{out_path}: {n} crates, {len(texts)} distinct licence texts")
bad = [e for e in exprs if re.search(r"GPL", e) and "OR" not in e]
for e in sorted(exprs):
    print(f"  {len(exprs[e]):4}  {e}")
if bad:
    print("copyleft needs attention:", bad, file=sys.stderr)
    sys.exit(1)
