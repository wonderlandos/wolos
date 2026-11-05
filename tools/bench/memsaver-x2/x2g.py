#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Guest side of the MEMSAVER-X2 measurement (docs/bench/memsaver-x2.md). Runs as root in a WolOS guest
whose desktop session is uid 1000; Python standard library only.

  x2g.py window REGEX [SECS]       wait for a window whose class matches, print {pid, class, address, ws}
  x2g.py sample PID                one JSON line: PSS, RSS, SwapPss of the app (the process tree of PID plus
                                   its app scope), CPU ticks, faults, context switches, kernel knobs
  x2g.py watch PID SECS EVERY      sample repeatedly, one line each
  x2g.py kill PID                  close the app and wait until nothing of it is left
  x2g.py hypr ARGS...              hyprctl as the desktop user
"""
import json
import os
import pwd
import re
import subprocess
import sys
import time

CG = '/sys/fs/cgroup'


def user_env():
    pw = pwd.getpwuid(1000)
    rt = '/run/user/1000'
    sigs = sorted(os.listdir(rt + '/hypr'), key=lambda s: os.stat(f'{rt}/hypr/{s}').st_mtime) if os.path.isdir(rt + '/hypr') else []
    way = sorted(f for f in os.listdir(rt) if re.fullmatch(r'wayland-\d+', f))
    return pw, {'HOME': pw.pw_dir, 'USER': pw.pw_name, 'LOGNAME': pw.pw_name, 'XDG_RUNTIME_DIR': rt,
                'WAYLAND_DISPLAY': way[0] if way else 'wayland-1',
                'HYPRLAND_INSTANCE_SIGNATURE': sigs[-1] if sigs else '',
                'DBUS_SESSION_BUS_ADDRESS': f'unix:path={rt}/bus', 'PATH': '/usr/local/bin:/usr/bin'}


def as_user(argv, timeout=20):
    pw, env = user_env()
    return subprocess.run(['runuser', '-u', pw.pw_name, '--', 'env'] + [f'{k}={v}' for k, v in env.items()] + argv,
                          capture_output=True, text=True, timeout=timeout)


def clients():
    r = as_user(['hyprctl', '-j', 'clients'])
    try:
        return json.loads(r.stdout)
    except ValueError:
        return []


def window(rx, secs=0):
    end = time.time() + secs
    while True:
        for c in clients():
            if (re.search(rx, c.get('class', '') + ' ' + c.get('initialClass', ''), re.I) and c.get('mapped', True)
                    and 'updater' not in c.get('title', '').lower()):
                return {'pid': c['pid'], 'class': c['class'], 'address': c['address'], 'ws': c['workspace']['id'],
                        'title': c.get('title', '')}
        if time.time() >= end:
            return None
        time.sleep(1)


def ppids():
    out = {}
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                s = open(f'/proc/{p}/stat').read()
                out[int(p)] = int(s[s.rindex(')') + 2:].split()[1])
            except (OSError, ValueError, IndexError):
                pass
    return out


def tree(root):
    kids = {}
    for p, pp in ppids().items():
        kids.setdefault(pp, []).append(p)
    seen, todo = set(), [root]
    while todo:
        p = todo.pop()
        if p in seen or not os.path.exists(f'/proc/{p}'):
            continue
        seen.add(p)
        todo += kids.get(p, [])
    return seen


def cgroup(pid):
    try:
        return open(f'/proc/{pid}/cgroup').read().strip().split('::', 1)[-1]
    except OSError:
        return ''


def scope_of(pid):
    m = re.match(r'(.*/app-[^/]*\.scope)', cgroup(pid))
    return m.group(1) if m else None


def procs_under(cg):
    out = set()
    for root, _d, files in os.walk(CG + cg):
        if 'cgroup.procs' in files:
            try:
                out |= {int(x) for x in open(os.path.join(root, 'cgroup.procs')).read().split()}
            except OSError:
                pass
    return out


def rd(path):
    try:
        return open(path).read().strip()
    except OSError:
        return None


def sample(pid):
    if not os.path.exists(f'/proc/{pid}'):
        return {'t': round(time.time(), 2), 'error': 'gone'}
    sc = scope_of(pid)
    pids = tree(pid)
    if sc:
        pids |= procs_under(sc)
    tot = dict(rss=0, pss=0, swap_pss=0, pss_anon=0, pss_file=0, pss_shmem=0, ticks=0, majflt=0, minflt=0, vol=0, nonvol=0)
    keys = {'Rss:': 'rss', 'Pss:': 'pss', 'SwapPss:': 'swap_pss', 'Pss_Anon:': 'pss_anon', 'Pss_File:': 'pss_file',
            'Pss_Shmem:': 'pss_shmem'}
    n = 0
    for p in pids:
        try:
            for line in open(f'/proc/{p}/smaps_rollup'):
                f = line.split()
                if f and f[0] in keys:
                    tot[keys[f[0]]] += int(f[1])
            s = open(f'/proc/{p}/stat').read()
            f = s[s.rindex(')') + 2:].split()
            tot['minflt'] += int(f[7])
            tot['majflt'] += int(f[9])
            tot['ticks'] += int(f[11]) + int(f[12])
            for t in os.listdir(f'/proc/{p}/task'):
                try:
                    for l in open(f'/proc/{p}/task/{t}/status'):
                        if l.startswith('voluntary_ctxt_switches'):
                            tot['vol'] += int(l.split()[1])
                        elif l.startswith('nonvoluntary_ctxt_switches'):
                            tot['nonvol'] += int(l.split()[1])
                except OSError:
                    pass
            n += 1
        except (OSError, ValueError, IndexError):
            continue
    mib = lambda k: round(k / 1024, 1)
    out = {'t': round(time.time(), 2), 'procs': n, 'scope': sc or cgroup(pid)}
    for k in ('rss', 'pss', 'swap_pss', 'pss_anon', 'pss_file', 'pss_shmem'):
        out[k] = mib(tot[k])
    for k in ('ticks', 'majflt', 'minflt', 'vol', 'nonvol'):
        out[k] = tot[k]
    if sc:
        for f in ('memory.current', 'memory.swap.current', 'memory.high'):
            v = rd(f'{CG}{sc}/{f}')
            out[f] = round(int(v) / 1048576, 1) if v and v.isdigit() else v
    mi = {l.split(':')[0]: int(l.split()[1]) for l in open('/proc/meminfo')}
    out['mem_available'] = mib(mi.get('MemAvailable', 0))
    out['ksm_sharing'] = int(rd('/sys/kernel/mm/ksm/pages_sharing') or 0)
    out['ksm_run'] = rd('/sys/kernel/mm/ksm/run')
    out['fault_around'] = rd('/sys/kernel/debug/fault_around_bytes')
    st = rd('/var/run/wol-memsaver.json')
    out['saver'] = st
    return out


def kill(pid):
    for c in clients():
        if c['pid'] == pid:
            as_user(['hyprctl', 'dispatch', 'closewindow', 'address:' + c['address']])
    for _ in range(15):
        if not os.path.exists(f'/proc/{pid}'):
            break
        time.sleep(1)
    left = tree(pid) if os.path.exists(f'/proc/{pid}') else set()
    for p in left:
        try:
            os.kill(p, 9)
        except OSError:
            pass
    time.sleep(2)


def main(a):
    if not a:
        sys.exit(__doc__)
    if a[0] == 'window':
        w = window(a[1], float(a[2]) if len(a) > 2 else 0)
        print(json.dumps(w))
        sys.exit(0 if w else 1)
    elif a[0] == 'sample':
        print(json.dumps(sample(int(a[1]))))
    elif a[0] == 'watch':
        end = time.time() + float(a[2])
        while time.time() < end:
            print(json.dumps(sample(int(a[1]))), flush=True)
            time.sleep(float(a[3]))
    elif a[0] == 'kill':
        kill(int(a[1]))
    elif a[0] == 'hypr':
        r = as_user(['hyprctl'] + a[1:])
        print(r.stdout, end='')
    else:
        sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv[1:])
