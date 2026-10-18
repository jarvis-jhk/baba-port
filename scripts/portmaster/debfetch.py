#!/usr/bin/env python3
"""Downloads Debian packages (optionally with their dependencies) from a
Debian archive, checks their SHA-256 against the signed-by-Debian Packages
index, and unpacks them into one directory with `dpkg-deb -x`. Used to build
the aarch64 link sysroot and the qemu-user test rootfs for PortMaster
(scripts/portmaster/fetch-toolchain.sh). Stdlib only; nothing is installed.

Usage: debfetch.py --mirror URL --dist buster --arch arm64 --cache DIR \
           --dest DIR [--deps] [--exclude pkg,...] pkg [pkg ...]
"""
import argparse
import hashlib
import lzma
import os
import re
import subprocess
import sys
import urllib.request


def fetch(url, path):
    if os.path.exists(path):
        return
    tmp = path + ".part"
    with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
        while True:
            b = r.read(1 << 16)
            if not b:
                break
            f.write(b)
    os.replace(tmp, path)


def parse_packages(text):
    pkgs, provides = {}, {}
    for block in text.split("\n\n"):
        fields, last = {}, None
        for line in block.splitlines():
            if line.startswith((" ", "\t")) and last:
                fields[last] += "\n" + line
            elif ":" in line:
                k, v = line.split(":", 1)
                fields[k] = v.strip()
                last = k
        if "Package" in fields:
            pkgs.setdefault(fields["Package"], fields)
            for p in fields.get("Provides", "").split(","):
                p = re.sub(r"\(.*?\)", "", p).strip()
                if p:
                    provides.setdefault(p, fields["Package"])
    return pkgs, provides


def deps_of(fields):
    out = []
    for key in ("Pre-Depends", "Depends"):
        for alt in fields.get(key, "").split(","):
            alt = alt.strip()
            if alt:
                out.append([re.sub(r"[\(\[].*?[\)\]]", "", a).split(":")[0].strip()
                            for a in alt.split("|")])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mirror", required=True)
    ap.add_argument("--dist", required=True)
    ap.add_argument("--arch", required=True)
    ap.add_argument("--component", default="main")
    ap.add_argument("--cache", required=True)
    ap.add_argument("--dest", required=True)
    ap.add_argument("--deps", action="store_true")
    ap.add_argument("--exclude", default="")
    ap.add_argument("packages", nargs="+")
    a = ap.parse_args()

    os.makedirs(a.cache, exist_ok=True)
    idx = os.path.join(a.cache, "Packages-%s-%s-%s.xz" % (a.dist, a.component, a.arch))
    fetch("%s/dists/%s/%s/binary-%s/Packages.xz" % (a.mirror, a.dist, a.component, a.arch), idx)
    pkgs, provides = parse_packages(lzma.open(idx).read().decode("utf-8", "replace"))
    exclude = set(filter(None, a.exclude.split(",")))

    want, seen = list(a.packages), []
    while want:
        name = want.pop(0)
        if name in seen or name in exclude:
            continue
        real = name if name in pkgs else provides.get(name)
        if real is None:
            sys.exit("package not found: " + name)
        if real in seen or real in exclude:
            continue
        seen.append(real)
        if a.deps:
            for alt in deps_of(pkgs[real]):
                if any(x in seen or x in exclude for x in alt):
                    continue
                pick = next((x for x in alt if x in pkgs or x in provides), None)
                if pick:
                    want.append(pick)

    os.makedirs(a.dest, exist_ok=True)
    total = 0
    for name in seen:
        f = pkgs[name]
        deb = os.path.join(a.cache, os.path.basename(f["Filename"]))
        fetch(a.mirror + "/" + f["Filename"], deb)
        if hashlib.sha256(open(deb, "rb").read()).hexdigest() != f["SHA256"]:
            os.remove(deb)
            sys.exit("SHA-256 mismatch: " + deb)
        subprocess.run(["dpkg-deb", "-x", deb, a.dest], check=True)
        total += os.path.getsize(deb)
        print("%-40s %s" % (name, f["Version"]))
    print("%d packages, %.1f MB of .deb" % (len(seen), total / 1e6))


if __name__ == "__main__":
    main()
