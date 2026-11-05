#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""The budget governor of `wol_core::memsaver::budget::govern`, as a guest loop for the bench
(root): every PERIOD s, PSS of the app's scope against TARGET MiB moves memory.high, never under
FLOOR MiB nor over twice the target. Usage: govern.py COMM TARGET FLOOR [PERIOD]"""
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import msx  # noqa: E402  (the bench's own sampler, next to this file)

MIB = 1 << 20


def govern(target, floor, pss, high):
    if pss > target:
        nxt = max(0, high - (pss - target))
    else:
        nxt = high + (target - pss) // 2
    return min(max(nxt, floor), 2 * target)


def main(comm, target, floor, period=2.0):
    target, floor = int(target) * MIB, int(floor) * MIB
    high = target
    while True:
        cg = msx.scope_of(comm)
        if cg:
            pss = 0
            for p in msx.procs_under(cg):
                try:
                    pss += sum(int(l.split()[1]) for l in open(f'/proc/{p}/smaps_rollup') if l.startswith('Pss:')) * 1024
                except (OSError, ValueError, IndexError):
                    pass
            high = govern(target, floor, pss, high)
            try:
                with open(f'{msx.CG}{cg}/memory.high', 'w') as f:
                    f.write(str(high))
            except OSError:
                pass
        time.sleep(float(period))


if __name__ == '__main__':
    main(*sys.argv[1:])
