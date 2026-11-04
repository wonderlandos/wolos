#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Memory of one third-party app's whole cgroup, the way the Discord budget is judged
(docs/bench/memsaver-discord.md): PSS summed over every process in the app's scope (the Memory
Saver scope `app-wol-<id>-<pid>.scope`, or the scope its main process sits in), plus what the
cgroup itself reports. Runs as root in the guest; Python standard library only.

  msx.py sample COMM             one JSON line (pss, rss, swap_pss in MiB; cgroup files; roles)
  msx.py watch COMM SECS EVERY   one JSON line every EVERY seconds for SECS seconds
  msx.py scope COMM              the app's cgroup path
"""
import json
import os
import re
import sys
import time

CG = '/sys/fs/cgroup'


def pids_named(comm):
    out = []
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                if open(f'/proc/{p}/comm').read().strip() == comm:
                    out.append(int(p))
            except OSError:
                pass
    return sorted(out)


def cgroup(pid):
    try:
        return open(f'/proc/{pid}/cgroup').read().strip().split('::', 1)[-1]
    except OSError:
        return ''


def scope_of(comm):
    """The scope of the oldest process named COMM: its `app-*.scope` (or the deepest scope)."""
    ps = pids_named(comm)
    if not ps:
        return None
    cg = cgroup(ps[0])
    m = re.match(r'(/user\.slice/.*?/app-[^/]*\.scope)', cg)
    if m:
        return m.group(1)
    m = re.match(r'(.*\.scope)', cg)
    return m.group(1) if m else cg


def procs_under(cg):
    out = []
    for root, _dirs, files in os.walk(CG + cg):
        if 'cgroup.procs' in files:
            try:
                out += [int(x) for x in open(os.path.join(root, 'cgroup.procs')).read().split()]
            except OSError:
                pass
    return sorted(set(out))


def rd(cg, f):
    try:
        return open(f'{CG}{cg}/{f}').read().strip()
    except OSError:
        return None


def kv(cg, f, keys):
    s = rd(cg, f) or ''
    d = dict(l.split(' ', 1) for l in s.splitlines() if ' ' in l)
    return {k: int(d[k]) for k in keys if k in d and d[k].strip().isdigit()}


def role(p):
    try:
        cmd = open(f'/proc/{p}/cmdline', 'rb').read().replace(b'\0', b' ').decode('utf-8', 'replace')
    except OSError:
        return '?'
    m = re.search(r'--type=([\w-]+)', cmd)
    if m:
        t = m.group(1)
        if t == 'utility':
            u = re.search(r'--utility-sub-type=([\w.]+)', cmd)
            t += ':' + (u.group(1).split('.')[-2] if u and '.' in u.group(1) else (u.group(1) if u else ''))
        return t
    try:
        return os.path.basename(os.readlink(f'/proc/{p}/exe'))
    except OSError:
        return '?'


def sample(comm):
    cg = scope_of(comm)
    if cg is None:
        return {'t': time.time(), 'error': f'no process named {comm}'}
    pids = procs_under(cg)
    tot = {'rss': 0, 'pss': 0, 'swap': 0, 'swap_pss': 0}
    keys = {'Rss:': 'rss', 'Pss:': 'pss', 'Swap:': 'swap', 'SwapPss:': 'swap_pss'}
    roles = {}
    n = 0
    for p in pids:
        try:
            got = {}
            for line in open(f'/proc/{p}/smaps_rollup'):
                f = line.split()
                if f and f[0] in keys:
                    got[keys[f[0]]] = int(f[1])
        except OSError:
            continue
        n += 1
        for k, v in got.items():
            tot[k] += v
        r = role(p)
        c, s = roles.get(r, (0, 0))
        roles[r] = (c + 1, s + got.get('pss', 0))
    mib = lambda k: round(k / 1024, 1)
    out = {'t': round(time.time(), 1), 'scope': cg, 'procs': n}
    out.update({k: mib(v) for k, v in tot.items()})
    for f in ('memory.current', 'memory.swap.current', 'memory.high', 'memory.max', 'memory.swap.max',
              'cgroup.freeze'):
        v = rd(cg, f)
        out[f] = mib(int(v) // 1024) if v and v.isdigit() and f != 'cgroup.freeze' else v
    out['stat'] = {k: mib(v // 1024) for k, v in kv(cg, 'memory.stat', ['anon', 'file', 'kernel', 'shmem', 'sock']).items()}
    out['events'] = kv(cg, 'memory.events', ['high', 'max', 'oom_kill'])
    out['cpu_usec'] = kv(cg, 'cpu.stat', ['usage_usec']).get('usage_usec')
    # Memory pressure (PSI) of the scope: the share of time its tasks waited for memory.
    psi = {}
    for line in (rd(cg, 'memory.pressure') or '').splitlines():
        f = line.split()
        if f:
            psi[f[0]] = {k: float(v) for k, v in (x.split('=') for x in f[1:]) if k in ('avg10', 'avg60')}
            psi[f[0]]['total_us'] = int(dict(x.split('=') for x in f[1:]).get('total', 0))
    out['psi'] = psi
    out['roles'] = {k: [c, mib(s)] for k, (c, s) in sorted(roles.items())}
    return out


def main(a):
    if len(a) >= 2 and a[0] == 'sample':
        print(json.dumps(sample(a[1])))
    elif len(a) >= 2 and a[0] == 'scope':
        print(scope_of(a[1]) or '')
    elif len(a) >= 4 and a[0] == 'watch':
        end = time.time() + float(a[2])
        while time.time() < end:
            print(json.dumps(sample(a[1])), flush=True)
            time.sleep(float(a[3]))
    else:
        sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
