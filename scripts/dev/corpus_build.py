"""Build the normalized solution corpus from downloaded third-party sources (untrusted data).

    python -I scripts/dev/corpus_build.py [--game-data DIR] [--ref DIR] [--out FILE]

Sources (each downloaded into its own directory under ref/tas/, see docs/reference-capture.md):
  ref/tas/baba-is-optimized/src/*/BABA IS YOU/solutions/*.txt   (main game, "Discord Collective"
      shortest-known solutions; file name "Level <code>, <name>[, <variant>...], <steps>.txt",
      move string U/D/L/R/W + repeat counts, '-' separators, X = exit to map, Z = reset)

Output: a TSV (default ref/tas/corpus/corpus.tsv) with columns
  world  level  name  variant  source  inputs
where `level` is the level file (e.g. 177level) and `inputs` uses the corpus alphabet:
  U D L R = moves, W = wait, Z = undo, r = restart.
Entries whose level cannot be identified go to the same file with level "?" (reported).
Only data is read; nothing from the sources is executed.
"""
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def read_ini(path):
    t, g = {}, None
    with open(path, encoding="latin-1") as f:
        for line in f:
            line = line.rstrip("\r\n")
            m = re.match(r"^\[(.*)\]$", line)
            if m:
                g = m.group(1)
                t.setdefault(g, {})
            elif "=" in line and g is not None:
                k, v = line.split("=", 1)
                t[g][k] = v
    return t


def world_index(game_data, world):
    """name (lower case) -> [level files], preferring levels that some map links to."""
    d = os.path.join(game_data, "Worlds", world)
    names, linked = {}, set()
    for p in glob.glob(os.path.join(d, "*.ld")):
        f = os.path.basename(p)[:-3]
        t = read_ini(p)
        names[f] = t.get("general", {}).get("name", "")
        lv = t.get("levels", {})
        for i in range(int(t.get("general", {}).get("levels", "0") or 0)):
            c = lv.get(f"{i}file")
            if c:
                linked.add(c)
    idx = {}
    for f, n in names.items():
        idx.setdefault(norm(n), []).append(f)
    for k, fs in idx.items():
        fs.sort(key=lambda f: (f not in linked, int(re.sub(r"\D", "", f) or 0)))
    return idx


def norm(name):
    return re.sub(r"\s+", " ", name.replace("(q)", "?").strip().lower())


def expand(moves):
    """baba-is-optimized move string -> corpus inputs (None if it exits to the map)."""
    out = []
    for tok, rep in re.findall(r"([UDLRWXZ])(\d*)", moves.replace("-", "").replace(" ", "")):
        n = int(rep) if rep else 1
        if tok == "X":
            return None
        out.append(("r" if tok == "Z" else tok) * n)
    return "".join(out)


def from_baba_is_optimized(ref, idx, rows, problems):
    srcs = sorted(glob.glob(os.path.join(ref, "baba-is-optimized", "src", "*", "BABA IS YOU", "solutions", "*.txt")))
    for p in srcs:
        base = os.path.basename(p)[:-4]
        parts = base.split(", ")
        if parts[0].startswith("Level "):
            code, name, variant = parts[0][6:], parts[1], ", ".join(parts[2:-1])
        else:  # "Gallery, 38", "The End, 48"
            code, name, variant = "", parts[0], ", ".join(parts[1:-1])
        with open(p, encoding="utf-8", errors="replace") as f:
            text = f.read().strip()
        inputs = expand(text)
        files = idx.get(norm(name), [])
        level = files[0] if files else "?"
        if not files:
            problems.append(f"no level named {name!r} ({base})")
        if inputs is None:
            problems.append(f"exits to map, skipped: {base}")
            continue
        rows.append(("baba", level, name.replace("(q)", "?"), variant or "-",
                     "baba-is-optimized:" + (code or name), inputs))


def main(argv):
    game_data = os.path.join(ROOT, "tools", "baba-original", "Data")
    ref = os.path.join(ROOT, "ref", "tas")
    out = os.path.join(ref, "corpus", "corpus.tsv")
    a = list(argv)
    for flag in ("--game-data", "--ref", "--out"):
        if flag in a:
            i = a.index(flag)
            v = a[i + 1]
            del a[i:i + 2]
            if flag == "--game-data":
                game_data = v
            elif flag == "--ref":
                ref = v
            else:
                out = v
    rows, problems = [], []
    from_baba_is_optimized(ref, world_index(game_data, "baba"), rows, problems)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        f.write("world\tlevel\tname\tvariant\tsource\tinputs\n")
        for r in rows:
            f.write("\t".join(r) + "\n")
    for p in problems:
        print("note:", p, file=sys.stderr)
    print(f"{len(rows)} entries -> {out}")


if __name__ == "__main__":
    main(sys.argv[1:])
