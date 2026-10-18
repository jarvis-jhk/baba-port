"""Verify candidate solutions (solver output, hand-made ones) in the private Lua oracle and merge
the ones that win into ref/tas/corpus/solved.tsv; print coverage of all corpus files.

    python -I scripts/dev/corpus_solved.py [CANDIDATES.tsv ...] [--solved FILE] [-j N] [--seeds N]

Candidates use the corpus columns (world level name variant source inputs); `source` is e.g.
`solver:bfws` or `manual`. Each is replayed in the oracle from the level start; a candidate is
accepted if it wins (`MF_win`) — the inputs are cut at the winning turn — or, with status
`win-seed`, if it wins only under another oracle RNG seed. solved.tsv keeps the shortest
accepted solution per (world, level, variant), with the columns
    world level name variant source inputs status win_turn
Without candidates it only prints coverage. Needs the private oracle (docs/spec/oracle) of the
main checkout: $BABA_ROOT (set by env.sh) or this script's checkout.
"""
import concurrent.futures as cf
import glob
import os
import re
import sys

ROOT = os.environ.get("BABA_ROOT") or os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.dont_write_bytecode = True  # keep the private oracle dir clean
sys.path.insert(0, os.path.join(ROOT, "docs", "spec", "oracle"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import corpus_verify  # noqa: E402
import oracle  # noqa: E402

HEAD = ["world", "level", "name", "variant", "source", "inputs", "status", "win_turn"]


def read_tsv(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [l.rstrip("\n").split("\t") for l in f][1:]


def levels_of(world):
    """(shown, stale): level files a map shows within its declared `levels=` count, and those
    named only by left-over `[levels]` records beyond it. Maps themselves are excluded."""
    d = os.path.join(oracle.DEFAULT_DATA, "Worlds", world)
    maps, shown, named = set(), set(), set()
    for p in glob.glob(os.path.join(d, "*.ld")):
        txt = open(p, encoding="latin-1").read()
        m = re.search(r"^levels=(\d+)", txt, re.M)
        if not (m and int(m.group(1)) > 0):
            continue
        maps.add(os.path.basename(p)[:-3])
        for i, f in re.findall(r"^(\d+)file=(\S+)", txt, re.M):
            (shown if int(i) < int(m.group(1)) else named).add(f)
    ok = lambda f: f not in maps and os.path.exists(os.path.join(d, f + ".l"))
    shown = {f for f in shown if ok(f)}
    return shown, {f for f in named if ok(f)} - shown


def coverage(solved_rows):
    ver = read_tsv(os.path.join(ROOT, "ref", "tas", "corpus", "verified.tsv"))
    print()
    for world in ("baba", "new_adv", "museum"):
        shown, stale = levels_of(world)
        a = {r[1] for r in ver if r[0] == world and r[5] in ("win", "win-seed")}
        b = {r[1] for r in solved_rows if r[0] == world}
        have = a | b
        print(f"{world}: {len(have & shown)}/{len(shown)} levels shown on a map have a verified solution"
              f" ({len(a & shown)} baba-is-optimized, +{len((b - a) & shown)} solved.tsv);"
              f" stale map entries: {len(have & stale)}/{len(stale)}")


def main(argv):
    solved = os.path.join(ROOT, "ref", "tas", "corpus", "solved.tsv")
    jobs, cands = 6, []
    a = list(argv)
    while a:
        k = a.pop(0)
        if k == "--solved":
            solved = a.pop(0)
        elif k == "-j":
            jobs = int(a.pop(0))
        elif k == "--seeds":
            corpus_verify.SEEDS = int(a.pop(0))
        else:
            cands += [r[:6] for r in read_tsv(k) if len(r) >= 6]
    rows = read_tsv(solved)
    best = {(r[0], r[1], r[3]): r for r in rows}
    accepted = 0
    if cands:
        with cf.ThreadPoolExecutor(jobs) as ex:
            for row, status, wt, ev, err in ex.map(corpus_verify.check, cands):
                print(f"{status:10} {row[0]}/{row[1]:10} {row[2]} [{row[4]}] win@{wt} len={len(row[5])} {err}",
                      flush=True)
                if status not in ("win", "win-seed") or wt <= 0:
                    continue
                inputs = row[5][:wt] if status == "win" else row[5]
                new = [*row[:5], inputs, status, str(wt)]
                k = (row[0], row[1], row[3])
                old = best.get(k)
                if old is None or (old[6] != "win" and status == "win") or \
                        (old[6] == status and len(inputs) < len(old[5])):
                    best[k] = new
                    accepted += 1
        os.makedirs(os.path.dirname(solved), exist_ok=True)
        with open(solved, "w") as f:
            f.write("\t".join(HEAD) + "\n")
            for k in sorted(best, key=lambda k: (k[0], corpus_key(k[1]), k[2])):
                f.write("\t".join(best[k]) + "\n")
        print(f"{accepted} accepted -> {solved} ({len(best)} entries)")
    coverage(list(best.values()))


def corpus_key(stem):
    m = re.match(r"(\d*)(.*)", stem)
    return (int(m.group(1)) if m.group(1) else 1 << 30, m.group(2))


if __name__ == "__main__":
    main(sys.argv[1:])
