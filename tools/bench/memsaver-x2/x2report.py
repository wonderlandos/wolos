#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Tables of docs/bench/memsaver-x2.md from the raw runs of x2drive.py.
  x2report.py RES_DIR   (reads RES_DIR/*.jsonl; prints markdown)"""
import glob
import json
import statistics
import sys

PHASES = [('launch15', 'Start, 15 s'), ('idle', 'Front, idle 60 s'), ('typing', 'Front, typing 60 s'),
          ('bg1', 'Behind 1 min'), ('bg10', 'Behind 10 min'), ('back', 'Back in front, 20 s')]
APP_ORDER = ['discord', 'code', 'signal', 'thunderbird', 'helium', 'minibrowser', 'text-editor', 'todos']
NAMES = {'discord': 'Discord', 'code': 'VS Code (Code - OSS)', 'signal': 'Signal', 'thunderbird': 'Thunderbird',
         'helium': 'Helium', 'minibrowser': 'WebKitGTK MiniBrowser', 'text-editor': 'Text Editor (GTK 4)',
         'todos': 'Wolkit Todos'}


def good(r):
    return 'error' not in r and r.get('bg') and r.get('idle')


def pick(rec, phase, key):
    """The value of KEY at PHASE of one run (the sample the phase is named for)."""
    if phase == 'launch15':
        s = rec.get('launch15') or []
        return s[-1].get(key) if s else None
    if phase in ('idle', 'typing'):
        s = [x for x in rec.get(phase) or [] if key in x]
        return s[-1][key] if s else None
    if phase == 'back':
        s = [x for x in rec.get('back') or [] if key in x]
        return s[-1][key] if s else None
    bg = [x for x in rec.get('bg') or [] if key in x]
    if not bg:
        return None
    t0 = bg[0]['t']
    want = 60 if phase == 'bg1' else 600
    return min(bg, key=lambda x: abs(x['t'] - t0 - want))[key]


def med(vals):
    vals = [v for v in vals if v is not None]
    return statistics.median(vals) if vals else None


def fmt(v, nd=0):
    return '-' if v is None else (f'{v:.{nd}f}' if nd else f'{round(v)}')


def wakeups(samples, lo, hi):
    """Context switches a second over the samples between LO and HI seconds into the series."""
    s = [x for x in samples if 'vol' in x]
    if len(s) < 2:
        return None
    t0 = s[0]['t']
    s = [x for x in s if lo <= x['t'] - t0 <= hi]
    if len(s) < 2:
        return None
    return ((s[-1]['vol'] + s[-1]['nonvol']) - (s[0]['vol'] + s[0]['nonvol'])) / (s[-1]['t'] - s[0]['t'])


def rate(samples, key, lo, hi, scale=1.0):
    """Increase of a counter a second over the samples between LO and HI seconds into the series."""
    s = [x for x in samples if key in x]
    if len(s) < 2:
        return None
    t0 = s[0]['t']
    s = [x for x in s if lo <= x['t'] - t0 <= hi]
    if len(s) < 2:
        return None
    return (s[-1][key] - s[0][key]) / (s[-1]['t'] - s[0]['t']) * scale


def main(d):
    runs = {}
    for f in sorted(glob.glob(f'{d}/*.jsonl')):
        for line in open(f):
            r = json.loads(line)
            if r.get('mode', '').endswith('prime') or r.get('mode') == 'smoke':
                continue
            runs.setdefault((r['app'], r['mode']), []).append(r)
    modes = sorted({m for _, m in runs}, key=lambda m: ['stock', 'shipped'].index(m) if m in ('stock', 'shipped') else 9)
    for key, title in (('pss', 'PSS'), ('rss', 'RSS')):
        print(f'\n### {title} of the whole app, MiB, median of the runs (n in brackets)\n')
        head = '| App | Mode | n | ' + ' | '.join(p[1] for p in PHASES) + ' |'
        print(head)
        print('|---|---|---|' + '---|' * len(PHASES))
        for app in APP_ORDER:
            for m in modes:
                rs = [r for r in runs.get((app, m), []) if good(r)]
                if not rs:
                    continue
                cells = [fmt(med([pick(r, p, key) for r in rs])) for p, _ in PHASES]
                print(f'| {NAMES[app]} | {m} | {len(rs)} | ' + ' | '.join(cells) + ' |')
    print('\n### Where the memory is: PSS split into anonymous, file-backed and shared memory, MiB, median\n')
    print('| App | Mode | 15 s: anon | file | shmem | Behind 10 min: anon | file | shmem |')
    print('|---|---|---|---|---|---|---|---|')
    for app in APP_ORDER:
        for m in modes:
            rs = [r for r in runs.get((app, m), []) if good(r)]
            if not rs:
                continue
            cells = [fmt(med([pick(r, ph, k) for r in rs])) for ph in ('launch15', 'bg10') for k in ('pss_anon', 'pss_file', 'pss_shmem')]
            print(f'| {NAMES[app]} | {m} | ' + ' | '.join(cells) + ' |')
    print('\n### Behind 10 min: PSS, SwapPss (compressed in zram), the scope\'s memory.current, wakeups/s\n')
    print('| App | Mode | PSS | SwapPss | memory.current | Wakeups/s front idle | Wakeups/s behind (5-10 min) | CPU % of a core, front idle | CPU % of a core, behind (5-10 min) | Major faults/s, front idle | Major faults/s, behind | Page-outs counted |')
    print('|---|---|---|---|---|---|---|---|---|---|---|---|')
    for app in APP_ORDER:
        for m in modes:
            rs = [r for r in runs.get((app, m), []) if good(r)]
            if not rs:
                continue
            po = []
            for r in rs:
                try:
                    st = json.loads(r.get('saver_status_behind') or '{}')
                    po.append(sum(a.get('pageouts', 0) for a in st.get('apps', [])))
                except ValueError:
                    pass
            print(f'| {NAMES[app]} | {m} | {fmt(med([pick(r, "bg10", "pss") for r in rs]))} | '
                  f'{fmt(med([pick(r, "bg10", "swap_pss") for r in rs]))} | '
                  f'{fmt(med([pick(r, "bg10", "memory.current") for r in rs]))} | '
                  f'{fmt(med([wakeups(r["idle"], 0, 999) for r in rs]), 0)} | '
                  f'{fmt(med([wakeups(r["bg"], 300, 700) for r in rs]), 0)} | '
                  f'{fmt(med([rate(r["idle"], "ticks", 0, 999, 1.0) for r in rs]), 0)} | '
                  f'{fmt(med([rate(r["bg"], "ticks", 300, 700, 1.0) for r in rs]), 0)} | '
                  f'{fmt(med([rate(r["idle"], "majflt", 0, 999) for r in rs]), 0)} | '
                  f'{fmt(med([rate(r["bg"], "majflt", 300, 700) for r in rs]), 0)} | {fmt(med(po))} |')
    print('\n### What the machine has left: MemAvailable of the whole 8 GiB guest, MiB, median (higher is better)\n')
    print('| App | Mode | Start, 15 s | Front, idle (last sample) | Behind 10 min |')
    print('|---|---|---|---|---|')
    for app in APP_ORDER:
        for m in modes:
            rs = [r for r in runs.get((app, m), []) if good(r)]
            if not rs:
                continue
            print(f'| {NAMES[app]} | {m} | ' + ' | '.join(fmt(med([pick(r, ph, "mem_available") for r in rs])) for ph in ('launch15', 'idle', 'bg10')) + ' |')
    print('\n### Coming back after 10 minutes (what a person feels)\n')
    print('| App | Mode | First visible change after the key, s | Last visible change, s | CPU in the first 4 s, ms | Major faults in 20 s | PSS at the return -> 20 s later |')
    print('|---|---|---|---|---|---|---|')
    for app in APP_ORDER:
        for m in modes:
            rs = [r for r in runs.get((app, m), []) if good(r) and r.get('back') and r.get('pre_return')]
            if not rs:
                continue
            fr = [r.get('return_frames') or {} for r in rs]
            cpu, flt = [], []
            for r in rs:
                pre = (r['pre_return'] or [{}])[0]
                b = r['back']
                if 'ticks' in pre and len(b) >= 3 and 'ticks' in b[2]:
                    cpu.append((b[2]['ticks'] - pre['ticks']) * 10)
                if 'majflt' in pre and 'majflt' in b[-1]:
                    flt.append(b[-1]['majflt'] - pre['majflt'])
            pre_p = med([(r['pre_return'] or [{}])[0].get('pss') for r in rs])
            print(f'| {NAMES[app]} | {m} | {fmt(med([f.get("first_change_s") for f in fr]), 1)} | '
                  f'{fmt(med([f.get("last_change_s") for f in fr]), 1)} | {fmt(med(cpu))} | {fmt(med(flt))} | '
                  f'{fmt(pre_p)} -> {fmt(med([pick(r, "back", "pss") for r in rs]))} |')
    print('\n### Runs\n')
    print('| App | Mode | Runs ok | Runs failed | Window after the launcher, s (median) |')
    print('|---|---|---|---|---|')
    for app in APP_ORDER:
        for m in modes:
            allr = runs.get((app, m), [])
            if not allr:
                continue
            ok = [r for r in allr if good(r)]
            print(f'| {NAMES[app]} | {m} | {len(ok)} | {len(allr) - len(ok)} | {fmt(med([r.get("launch_to_window_s") for r in ok]), 1)} |')


if __name__ == '__main__':
    main(sys.argv[1] if len(sys.argv) > 1 else '.')
