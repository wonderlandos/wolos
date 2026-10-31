#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
# Table from the guest logs: report.py DIR (files KIND-MODE.data). Times are seconds after `go`.
# responsive: longest run of seconds with a compositor stall (max input latency over 100 ms, or a
# second without a report). recover: seconds from the end of the bomb to the first 3 clean seconds.
import re
import sys
from pathlib import Path

STALL_US = 100_000


def parse(path):
    d = dict(L=[], P=[], R=[], E={}, TP=[], S=[], B=[], end=False)
    for line in path.read_text(errors="replace").splitlines():
        f = line.split()
        if len(f) < 3:
            if f and f[0] == "E":
                continue
            continue
        kv = dict(x.split("=", 1) for x in f[2:] if "=" in x)
        try:
            t = float(f[2]) if f[1] in "LP" else float(f[1])
        except ValueError:
            continue
        if f[0] in "CT" and f[1] in "LP" and not {"p99", "max", "spawn"} & kv.keys():
            continue
        if f[0] in "CT" and ("max" if f[1] == "L" else "spawn") not in kv:
            continue
        if f[0] == "C" and f[1] == "L":
            if "p99" not in kv:
                continue
            d["L"].append((t, int(kv["p99"]), int(kv["max"])))
        elif f[0] == "C" and f[1] == "P":
            d["P"].append((t, int(kv["spawn"])))
        elif f[0] == "T" and f[1] == "P":
            d["TP"].append((t, int(kv["spawn"])))
        elif f[0] == "R":
            if "rc" in kv:
                d["R"].append((t, int(kv["ms"]), int(kv["rc"])))
        elif f[0] == "S":
            d["S"].append((t, kv))
        elif f[0] == "E":
            d["E"][f[2]] = t
            d["end"] |= f[2] == "done"
    return d


def row(path):
    d = parse(path)
    go = d["E"].get("go")
    if go is None:
        return None
    end = d["E"].get("kill") or d["E"].get("scope-gone-auto") or go + 20
    run = [x for x in d["L"] if go <= x[0] <= end]
    expected = int(end - go)
    gap, worst = 0.0, 0
    last = go
    for t, _, mx in run:
        worst = max(worst, mx)
        gap = max(gap, t - last)
        last = t
    gap = max(gap, end - last)
    stalled = sum(1 for x in run if x[2] > STALL_US)
    p99 = sorted(x[1] for x in run)
    spawns = [x[1] for x in d["P"] if go <= x[0] <= end]
    fail = sum(1 for s in spawns if s < 0)
    ok = sorted(s for s in spawns if s >= 0)
    rec = None
    after = [x for x in d["L"] if x[0] > end]
    for i in range(len(after) - 2):
        if all(x[2] < STALL_US for x in after[i : i + 3]):
            rec = after[i][0] - end
            break
    roots = [x for x in d["R"] if x[0] >= go]
    rok = [x[1] for x in roots if x[2] == 0]
    tsp = [x[1] for x in d["TP"] if go <= x[0] <= end]
    return dict(
        name=path.stem,
        ok=d["end"],
        stalled=f"{stalled}/{len(run)}",
        gap=round(gap, 1),
        p99=round(p99[len(p99) // 2] / 1000, 1) if p99 else None,
        worst=round(worst / 1000),
        spawn=round(ok[len(ok) // 2] / 1000, 1) if ok else None,
        sfail=f"{fail}/{len(spawns)}",
        tfail=f"{sum(1 for s in tsp if s < 0)}/{len(tsp)}",
        rec=round(rec, 1) if rec is not None else None,
        root=f"{len(rok)}/{len(roots)} max {max(rok) if rok else '-'}ms",
        auto=round(d["E"]["scope-gone-auto"] - go, 1) if "scope-gone-auto" in d["E"] else None,
    )


rows = [r for r in (row(p) for p in sorted(Path(sys.argv[1]).glob("*.data"))) if r]
cols = "name ok stalled gap p99 worst spawn sfail tfail rec root auto".split()
print("| " + " | ".join(cols) + " |")
print("|" + "---|" * len(cols))
for r in rows:
    print("| " + " | ".join(str(r[c]) for c in cols) + " |")
for p in sorted(Path(sys.argv[1]).glob("load-*.data")):
    for line in p.read_text(errors="replace").splitlines():
        if re.match(r"B |E [0-9.]+ load", line):
            print(p.stem, line)
