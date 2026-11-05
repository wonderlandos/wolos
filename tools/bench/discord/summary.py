#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Summarise phases.sh samples: per label and phase, PSS (median and max), memory.current and
swap at chosen points. Usage: summary.py OUT/*.jsonl"""
import json
import statistics
import sys

rows = {}
for path in sys.argv[1:]:
    t0 = {}
    for line in open(path):
        try:
            s = json.loads(line)
        except ValueError:
            continue
        if 'pss' not in s:
            continue
        key = (s['label'], s['phase'])
        t0.setdefault(s['phase'], s['t'])
        s['dt'] = s['t'] - t0[s['phase']]
        rows.setdefault(key, []).append(s)

def at(ss, secs):
    """The sample nearest to `secs` into the phase."""
    return min(ss, key=lambda s: abs(s['dt'] - secs))

print('| label | phase | n | PSS median | PSS max | cg current | swap | PSS @60s | PSS @600s |')
print('|---|---|---|---|---|---|---|---|---|')
for (label, phase), ss in rows.items():
    p = [s['pss'] for s in ss]
    last = ss[-1]
    a60 = at(ss, 60)['pss'] if phase == 'bg' else ''
    a600 = at(ss, 600)['pss'] if phase == 'bg' and ss[-1]['dt'] >= 540 else ''
    print(f"| {label} | {phase} | {len(ss)} | {statistics.median(p):.0f} | {max(p):.0f} | "
          f"{last['memory.current']} | {last['memory.swap.current']} | {a60} | {a600} |")
