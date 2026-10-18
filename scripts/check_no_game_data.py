#!/usr/bin/env python3
"""The checks behind scripts/check-no-game-data.sh (read its header).

Standard library only; run with `python3 -I`. Exit status 1 on any finding.
"""
import argparse
import fnmatch
import hashlib
import io
import os
import re
import sys
import tarfile
import zipfile

LIMIT = 256 * 1024
SAMPLE = 32
SAMPLES_PER_FILE = 4
DENSE_STEP = 256

# Large files that are ours or third-party under a permissive license
# (path globs relative to the repository / the archive's top folder).
ALLOW_LARGE = {
    "Cargo.lock": "dependency lock file",
    "*/Cargo.lock": "dependency lock file",
    "docs/*.md": "documentation (text)",
    "licenses/RUST_DEPENDENCIES.txt": "generated license texts of the Rust crates",
    "*/licenses/RUST_DEPENDENCIES.txt": "the same, in the PortMaster port / the macOS bundle",
}
# Compiled code that belongs in release archives: name below the archive's
# top-level folder (or the archive root) -> why.
ARCHIVE_BINARIES = {
    "baba-port": "our Linux binary",
    "baba-port.exe": "our Windows binary",
    "SDL2.dll": "SDL2 (zlib), official mingw release",
    "lib/libSDL2-2.0.so.0": "SDL2 (zlib), built by scripts/ci/build-sdl2-linux.sh",
    "lib/arm64-v8a/libbabaport.so": "our Android library",
    "babaport/baba-port.aarch64": "our PortMaster binary",
    "Contents/MacOS/baba-port": "our macOS binary (universal)",
    "Contents/Frameworks/SDL2.framework/Versions/A/SDL2": "SDL2 (zlib), official macOS release",
}
# Files inside macOS frameworks that are code but not ours (code signature).
ARCHIVE_BINARY_GLOBS = ["Contents/Frameworks/SDL2.framework/Versions/A/_CodeSignature/*",
                        "Contents/_CodeSignature/*"]
# The Android binary manifest is a binary file by design.
ARCHIVE_OPAQUE = {"AndroidManifest.xml": "binary XML written by scripts/android/apk.py"}

BAD_EXT = re.compile(
    r"\.(png|jpe?g|gif|bmp|webp|tga|ico|icns|psd|"
    r"ogg|oga|wav|mp3|flac|xm|mod|it|s3m|"
    r"ttf|otf|ttc|woff2?|fnt|"
    r"lua|luac|l|ld|ba|dat|"
    r"dll|exe|so|dylib|pdb|"
    r"mfa|ccn|chowdren)$", re.I)
BAD_NAME = re.compile(
    r"(^|/)(values\.lua|Assets\.dat|Chowdren|libsteam_api\.so|SettingsC\.txt|"
    r"[0-9]+ba\.ba|ba\.ba|world_data\.txt|lang_[a-z_]+\.txt)$", re.I)
# Names that are fine despite matching BAD_EXT (path globs): text formats
# of ours that happen to share an extension.
OK_PATHS = ["crates/sim/scenarios/*.txt"]

MAGIC = [
    (b"\x89PNG\r\n\x1a\n", "PNG image"),
    (b"\xff\xd8\xff", "JPEG image"),
    (b"GIF8", "GIF image"),
    (b"OggS", "Ogg audio"),
    (b"fLaC", "FLAC audio"),
    (b"ID3", "MP3 audio"),
    (b"\x00\x01\x00\x00\x00", "TrueType font"),
    (b"OTTO", "OpenType font"),
    (b"ttcf", "TrueType collection"),
    (b"wOFF", "WOFF font"),
    (b"wOF2", "WOFF2 font"),
    (b"ACHTUNG!", "Baba level (.l)"),
    (b"\x1bLua", "Lua bytecode"),
    (b"8BPS", "Photoshop image"),
]
# Lua source: lines typical of Lua and rare elsewhere (character classes so
# that this file does not match itself).
LUA_LINE = re.compile(
    rb"^\s*(local\s+function\s+\w|function\s+[\w.:]+\s*\(.*\)\s*$|end\s*$|"
    rb"elseif\s.*\st[h]en\s*$|if\s.*\st[h]en\s*$|for\s.*\sd[o]\s*$|.*~=.*\st[h]en\s*$)",
    re.M)
LUA_GAME_WORDS = re.compile(rb"\b(MF_\w+|editor_objlist|tileslist|unitlists?|featureindex|"
                            rb"objectlist|generaldata\d?|mapdata|levelconversions)\b")


def sniff(head):
    for m, what in MAGIC:
        if head.startswith(m):
            return what
    if head[:4] == b"RIFF" and head[8:12] in (b"WAVE", b"WEBP", b"AVI "):
        return "RIFF " + head[8:12].decode()
    if head[:2] == b"BM" and len(head) > 14 and head[6:10] == b"\0\0\0\0":
        return "BMP image"
    if head[:2] == b"MZ" or head[:4] == b"\x7fELF" or head[:4] in (
            b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe", b"\xce\xfa\xed\xfe"):
        return "executable"
    return None


# Lua table entries: `key = value,` / `[1] = {...},` (Rust, TOML and INI
# lines do not end like that).
LUA_TABLE_LINE = re.compile(rb'^\s*(\w+|\[[^\]\n]+\])\s*=\s*[^=>\n].*,\s*$', re.M)


def looks_like_lua(data):
    if b"\0" in data[:4096]:
        return None
    n = len(LUA_LINE.findall(data))
    if n >= 12 or (n >= 4 and len(LUA_GAME_WORDS.findall(data)) >= 3):
        return "Lua source"
    if len(LUA_TABLE_LINE.findall(data)) >= 20:
        return "Lua table"
    return None


# ---------------------------------------------------------------- install index

def install_index(game_dir):
    """SHA-256 of every file of the install, and 32-byte samples of its data
    files (not of the engine executables or SDL's controller database,
    which share code/tables/lines with other software)."""
    hashes, samples = {}, {}
    if not game_dir or not os.path.isfile(os.path.join(game_dir, "Data", "values.lua")):
        return hashes, samples, 0
    nfiles = 0
    for dp, dn, fn in os.walk(game_dir):
        dn.sort()
        for f in sorted(fn):
            p = os.path.join(dp, f)
            rel = os.path.relpath(p, game_dir)
            try:
                data = open(p, "rb").read()
            except OSError:
                continue
            nfiles += 1
            hashes.setdefault(hashlib.sha256(data).hexdigest(), rel)
            if rel.startswith("bin64") or f in ("gamecontrollerdb.txt", "run.sh") \
                    or sniff(data[:16]) == "executable" or len(data) < 2 * SAMPLE:
                continue
            # Code and text tables densely (an extract of a table is caught
            # too), everything else at a few points spread over the file.
            if f.endswith(".lua") or (f.endswith(".txt") and "Languages" in rel):
                offs = range(0, len(data) - SAMPLE, DENSE_STEP)
            else:
                offs = [(len(data) * (2 * k + 1)) // (2 * SAMPLES_PER_FILE)
                        for k in range(SAMPLES_PER_FILE)]
            for off in offs:
                # Skip low-entropy windows (runs, indentation, zero padding).
                off = min(off, len(data) - SAMPLE)
                for shift in range(0, min(DENSE_STEP, len(data) - SAMPLE - off + 1), 7):
                    w = data[off + shift: off + shift + SAMPLE]
                    # Text windows must hold prose or code (several spaces),
                    # not just one identifier: the port looks up the game's
                    # keys and names (lang keys, object names) by design.
                    text = all(32 <= b < 127 or b in (9, 10, 13) for b in w)
                    if len(set(w)) >= 14 and (not text or w.count(b" ") >= 3):
                        samples.setdefault(w[:16], []).append((w, rel))
                        break
    return hashes, samples, nfiles


def find_samples(data, samples):
    if not samples or len(data) < SAMPLE:
        return None
    keys = samples
    get = keys.get
    for i in range(len(data) - SAMPLE + 1):
        hit = get(data[i:i + 16])
        if hit:
            for w, rel in hit:
                if data[i:i + SAMPLE] == w:
                    return rel
    return None


# ---------------------------------------------------------------- checks

class Checker:
    def __init__(self, hashes, samples):
        self.hashes, self.samples = hashes, samples
        self.bad = 0
        self.checked = 0

    def report(self, name, why):
        self.bad += 1
        print(f"BAD {name}: {', '.join(why)}")

    def check(self, name, rel, data, in_archive):
        """name: shown; rel: path for the rules (below the archive's top folder)."""
        self.checked += 1
        why = []
        head = data[:4096]
        binary_ok = in_archive and (rel in ARCHIVE_BINARIES
                                    or any(fnmatch.fnmatch(rel, g) for g in ARCHIVE_BINARY_GLOBS))
        opaque_ok = in_archive and rel in ARCHIVE_OPAQUE
        ok_path = any(fnmatch.fnmatch(rel, g) for g in OK_PATHS)
        if BAD_NAME.search(rel) and not ok_path:
            why.append("game file name")
        if BAD_EXT.search(rel) and not binary_ok and not ok_path:
            why.append("file type")
        s = sniff(head)
        if s and not (s == "executable" and binary_ok):
            why.append(s)
        lua = None if binary_ok or opaque_ok else looks_like_lua(data)
        if lua:
            why.append(lua)
        if len(data) > LIMIT and not binary_ok and not any(fnmatch.fnmatch(rel, g) for g in ALLOW_LARGE):
            why.append(f"large file ({len(data) // 1024} KB, not allowlisted)")
        if self.hashes:
            h = hashlib.sha256(data).hexdigest()
            if h in self.hashes and len(data) > 0:
                why.append(f"identical to the install's {self.hashes[h]}")
            else:
                src = find_samples(data, self.samples)
                if src:
                    why.append(f"contains bytes of the install's {src}")
        if why:
            self.report(name, why)

    def archive(self, label, blob, kind):
        """Every member of a zip/apk or tar.gz (bytes), recursively for zips."""
        if kind == "zip":
            z = zipfile.ZipFile(io.BytesIO(blob))
            members = [(i.filename, lambda i=i: z.read(i)) for i in z.infolist() if not i.is_dir()]
        else:
            t = tarfile.open(fileobj=io.BytesIO(blob))
            members = [(m.name, lambda m=m: t.extractfile(m).read()) for m in t.getmembers() if m.isfile()]
            for m in t.getmembers():
                if m.issym() or m.islnk():
                    target = os.path.normpath(os.path.join(os.path.dirname(m.name), m.linkname))
                    if m.linkname.startswith("/") or target.startswith(".."):
                        self.report(f"{label}:{m.name}", [f"link out of the archive ({m.linkname})"])
        for name, read in members:
            data = read()
            rel = name
            # Below the top-level folder (baba-port-<v>/..., X.app/...).
            parts = name.split("/", 1)
            if len(parts) == 2 and (parts[0].startswith("baba-port") or parts[0].endswith(".app")):
                rel = parts[1]
            if ".app/" in name:
                rel = name.split(".app/", 1)[1]
            if name.endswith((".zip", ".apk")):
                self.check(f"{label}:{name}", rel, b"", True)
                self.archive(f"{label}:{name}", data, "zip")
                continue
            self.check(f"{label}:{name}", rel, data, True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", choices=["tracked", "worktree", "archives"], required=True)
    ap.add_argument("--game-dir", default="")
    ap.add_argument("files", nargs="*")
    a = ap.parse_args()

    hashes, samples, ninstall = install_index(a.game_dir)
    if ninstall:
        print(f"install: {ninstall} files of {a.game_dir} hashed, {sum(map(len, samples.values()))} samples")
    else:
        print("install: none ($BABA_GAME_DIR unset or no Data/values.lua): type/content/size rules only")
    c = Checker(hashes, samples)

    if a.mode in ("tracked", "worktree"):
        if a.mode == "tracked":
            paths = [p for p in sys.stdin.buffer.read().decode().split("\0") if p]
        else:
            ignored = {".git"}
            if os.path.exists(".gitignore"):
                for line in open(".gitignore"):
                    line = line.strip()
                    if line.startswith("/") and not any(ch in line for ch in "*?["):
                        ignored.add(line.strip("/"))
            paths = []
            for dp, dn, fn in os.walk("."):
                rel = os.path.relpath(dp, ".")
                rel = "" if rel == "." else rel + "/"
                dn[:] = sorted(d for d in dn if (rel + d) not in ignored
                               and not os.path.islink(os.path.join(dp, d)))
                paths += [rel + f for f in sorted(fn)
                          if (rel + f) not in ignored and not os.path.islink(os.path.join(dp, f))]
        private = re.compile(r"^(ref|cache|scratch|tools|target|dist|ext|docs/spec)(/|$)")
        for p in paths:
            if private.match(p):
                c.report(p, ["path below a private, git-ignored directory"])
                continue
            if not os.path.isfile(p):
                continue
            c.check(p, p, open(p, "rb").read(), False)
        where = "tracked files" if a.mode == "tracked" else "files in the tree (minus git-ignored dirs)"
    else:
        if not a.files:
            sys.exit("--archives needs at least one file")
        for f in a.files:
            n0 = c.checked
            blob = open(f, "rb").read()
            if f.endswith((".zip", ".apk")):
                c.archive(os.path.basename(f), blob, "zip")
            elif f.endswith((".tar.gz", ".tgz", ".tar")):
                c.archive(os.path.basename(f), blob, "tar")
            else:
                c.check(os.path.basename(f), os.path.basename(f), blob, False)
            print(f"{os.path.basename(f)}: {c.checked - n0} files")
        where = "archive members"
    if c.bad:
        sys.stdout.flush()
        print(f"check-no-game-data: {c.bad} problem(s) in {c.checked} {where}", file=sys.stderr)
        sys.exit(1)
    extra = ", compared with the install" if hashes else ""
    print(f"check-no-game-data: ok ({c.checked} {where}{extra}; nothing from the game)")


if __name__ == "__main__":
    main()
