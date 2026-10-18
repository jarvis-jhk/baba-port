"""Verify every corpus entry with the private Lua oracle (docs/spec/oracle/) and report coverage.

    python -I scripts/dev/corpus_verify.py [--corpus FILE] [--out FILE] [-j N] [--filter SUBSTR] [--seeds N]

For each entry the oracle replays the inputs from the level start and reports whether (and on
which turn) the level was won (MF_win), plus other end events (end, done, level conversions).
Entries that do not win are retried with oracle RNG seeds 1..N (default 64): the game's TELE and
other random choices are not reproducible, so such a solution is "win-seed" (valid under some
random outcome).
Writes a TSV (default ref/tas/corpus/verified.tsv) and prints a coverage summary against the
playable levels of each world.
"""
import concurrent.futures as cf
import glob
import json
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
sys.dont_write_bytecode = True  # keep the private oracle dir clean
sys.path.insert(0, os.path.join(ROOT, "docs", "spec", "oracle"))
import oracle  # noqa: E402  (private, not in the public repo)


SEEDS = 64  # nowin entries are retried with other oracle RNG seeds (TELE etc. are random)


def check(row):
    res = check_seed(row, 0)
    if res[1] == "nowin":
        for seed in range(1, SEEDS + 1):
            r = check_seed(row, seed)
            if r[1] == "win":
                return r[0], "win-seed", r[2], r[3] + [f"seed={seed}"], ""
    return res


def check_seed(row, seed):
    world, level, name, variant, source, inputs = row
    if level == "?":
        return row, "unresolved", -1, [], ""
    try:
        rc, out, err = oracle.run(world, level, inputs, opts=["--every", f"--seed={seed}"], timeout=900)
    except Exception as e:  # timeout etc.
        return row, "error", -1, [], str(e)[:200]
    lines = [json.loads(l) for l in out.splitlines() if l.startswith("{")]
    if rc != 0 or not lines:
        return row, "error", -1, [], err.strip().splitlines()[-1][:300] if err.strip() else "no output"
    events, win_turn = [], -1
    for st in lines:
        for e in st["events"]:
            events.append(f"{e}@{st['turn']}")
        if st["win"] and win_turn < 0:
            win_turn = st["turn"]
    status = "win" if win_turn >= 0 else ("event" if events else "nowin")
    return row, status, win_turn, events, ""


def playable_levels(world):
    """Level files of a world that are not maps (no [levels] entries) and are linked from a map."""
    d = os.path.join(oracle.DEFAULT_DATA, "Worlds", world)
    maps, linked = set(), set()
    for p in glob.glob(os.path.join(d, "*.ld")):
        f = os.path.basename(p)[:-3]
        txt = open(p, encoding="latin-1").read()
        m = re.search(r"^levels=(\d+)", txt, re.M)
        if m and int(m.group(1)) > 0:
            maps.add(f)
            linked.update(re.findall(r"^\d+file=(\S+)", txt, re.M))
    return {f for f in linked if f not in maps and os.path.exists(os.path.join(d, f + ".l"))}, maps


def main(argv):
    corpus = os.path.join(ROOT, "ref", "tas", "corpus", "corpus.tsv")
    out = os.path.join(ROOT, "ref", "tas", "corpus", "verified.tsv")
    global SEEDS
    jobs, filt = os.cpu_count() or 4, None
    a = list(argv)
    while a:
        k = a.pop(0)
        if k == "--corpus":
            corpus = a.pop(0)
        elif k == "--out":
            out = a.pop(0)
        elif k == "-j":
            jobs = int(a.pop(0))
        elif k == "--filter":
            filt = a.pop(0)
        elif k == "--seeds":
            SEEDS = int(a.pop(0))
    rows = [l.rstrip("\n").split("\t") for l in open(corpus)][1:]
    if filt:
        rows = [r for r in rows if any(filt in x for x in r[:5])]
    results = []
    with cf.ThreadPoolExecutor(jobs) as ex:
        for res in ex.map(check, rows):
            results.append(res)
            row, status, wt, ev, err = res
            print(f"{status:10} {row[0]}/{row[1]:9} {row[2]} [{row[3]}] win@{wt} len={len(row[5])} {' '.join(ev[:4])} {err}", flush=True)
    with open(out, "w") as f:
        f.write("world\tlevel\tname\tvariant\tsource\tstatus\twin_turn\tlength\tevents\terror\n")
        for row, status, wt, ev, err in results:
            f.write("\t".join([*row[:5], status, str(wt), str(len(row[5])), " ".join(ev), err]) + "\n")
    # coverage
    print()
    for world in sorted({r[0] for r in rows}):
        levels, maps = playable_levels(world)
        won = {r[1] for r, s, *_ in results if r[0] == world and s == "win"}
        wons = {r[1] for r, s, *_ in results if r[0] == world and s == "win-seed"} - won
        conv = {r[1] for r, s, *_ in results if r[0] == world and s == "event"} - won - wons
        print(f"{world}: {len(won & levels)}/{len(levels)} map-linked playable levels have a verified solution"
              f" (+{len(wons & levels)} winning only with another RNG seed, +{len(conv & levels)} with only an"
              f" alternative ending; {len((won | wons) - levels)} more outside that set); {len(maps)} maps")
    tot = {}
    for _, s, *_ in results:
        tot[s] = tot.get(s, 0) + 1
    print("entries:", tot)


if __name__ == "__main__":
    main(sys.argv[1:])
