#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Guest-side Discord/WebKitGTK memory run (root). jsx.py LABEL 'PRE' 'LAUNCH' [BGSECS]
PRE: shell run as root before launch (kernel knobs). LAUNCH: command line run as the user via hyprctl exec.
Samples: t=45,60 front-idle; 'POST' hook (env JSX_POST, root shell) at t=60 then sample t=+30,+60;
background (workspace 2) 60 s and BGSECS."""
import json, os, re, subprocess, sys, time, collections
label = sys.argv[1]
bg = int(sys.argv[2]) if len(sys.argv) > 2 else 120
rd = lambda e: open(f'/var/tmp/jsx/{label}.{e}').read().strip() if os.path.exists(f'/var/tmp/jsx/{label}.{e}') else ''
pre, launch, post = rd('pre'), rd('launch'), rd('post')
COMMS = (rd('comms') or 'Discord').split(',')
WINCLASS = rd('winclass') or 'discord'
out = open(f'/var/tmp/jsx/{label}.jsonl', 'w')
def sh(c, **k): return subprocess.run(c, shell=True, capture_output=True, text=True, **k)
def user(c): return sh(f'sh /var/tmp/msx/as-user.sh {c}')
def pids(comm=None):
    comms = [comm] if comm else COMMS
    r = []
    for p in os.listdir('/proc'):
        if p.isdigit():
            try:
                if open(f'/proc/{p}/comm').read().strip() in comms: r.append(int(p))
            except OSError: pass
    return sorted(r)
def classify(name, perms):
    if name.startswith('[anon:'): return name[6:-1] if name.endswith(']') else name
    if name == '': return 'anon-unnamed'
    if name.startswith('['): return name
    if name.startswith('/memfd:') or name.startswith('/dev/shm'): return 'shm'
    b = os.path.basename(name.replace(' (deleted)', ''))
    b = re.sub(r'\.so.*', '.so', b)
    return 'file:' + ('Discord' if b == 'Discord' else b) + (':x' if 'x' in perms else ':r')
extra = {}
def sample(tag):
    tot = collections.Counter(); cls = collections.Counter(); roles = {}
    ps = pids(); n = 0
    for p in ps:
        try:
            cmd = open(f'/proc/{p}/cmdline','rb').read().replace(b'\0',b' ').decode()
            m = re.search(r'--type=([\w-]+)', cmd); t = m.group(1) if m else open(f'/proc/{p}/comm').read().strip()
            if t == 'utility':
                u = re.search(r'--utility-sub-type=([\w.]+)', cmd); t += ':' + (u.group(1).split('.')[-2] if u and '.' in u.group(1) else '')
            roll = {}
            for l in open(f'/proc/{p}/smaps_rollup'):
                k, _, v = l.partition(':')
                if k in ('Pss','Pss_Anon','Pss_File','Pss_Shmem','Rss','SwapPss','Swap'): roll[k] = int(v.split()[0])
            cur = None; perms = ''
            pc = collections.Counter()
            for line in open(f'/proc/{p}/smaps'):
                h = re.match(r'^[0-9a-f]+-[0-9a-f]+ (\S+) \S+ \S+ \S+\s*(.*)$', line)
                if h: perms = h.group(1); cur = classify(h.group(2).strip(), perms); continue
                if line.startswith('Pss:'): pc[cur] += int(line.split()[1])
            st = open(f'/proc/{p}/stat').read().rsplit(')', 1)[1].split()
            tot['cpu_ticks'] += int(st[11]) + int(st[12]); tot['minflt'] += int(st[7]); tot['majflt'] += int(st[9])
            n += 1
            for k, v in roll.items(): tot[k] += v
            for k, v in pc.items(): cls[k] += v
            c, s = roles.get(t, (0, 0)); roles[t] = (c + 1, s + roll.get('Pss', 0))
        except OSError: pass
    mi = {}
    for l in open('/proc/meminfo'):
        k, _, v = l.partition(':')
        if k in ('MemAvailable','MemFree','AnonPages','Zswapped','Zswap','AnonHugePages','Cached','Mapped','Shmem'): mi[k] = int(v.split()[0]) // 1024
    ksm = {}
    for f in ('pages_sharing','pages_shared','pages_unshared'):
        try: ksm[f] = int(open(f'/sys/kernel/mm/ksm/{f}').read())
        except OSError: pass
    swapused = sh("awk 'NR>1{s+=$4}END{print s/1024}' /proc/swaps").stdout.strip()
    kd = 0.0
    for p in pids('ksmd'):
        st = open(f'/proc/{p}/stat').read().rsplit(')', 1)[1].split(); kd += (int(st[11]) + int(st[12])) / os.sysconf('SC_CLK_TCK')
    extra['ksmd_s'] = round(kd, 2)
    rec = {'label': label, 'tag': tag, 't': round(time.time()), 'procs': n,
           'pss': round(tot['Pss']/1024,1), 'anon': round(tot['Pss_Anon']/1024,1), 'file': round(tot['Pss_File']/1024,1),
           'shmem': round(tot['Pss_Shmem']/1024,1), 'swap_pss': round(tot['SwapPss']/1024,1), 'rss': round(tot['Rss']/1024,1),
           'roles': {k: [c, round(s/1024,1)] for k, (c, s) in sorted(roles.items())},
           'classes': {k: round(v/1024,1) for k, v in cls.most_common(14) if v >= 1024},
           'sys': mi, 'ksm': ksm, 'swap_used_mb': swapused, 'cpu_s': round(tot['cpu_ticks']/os.sysconf('SC_CLK_TCK'),1), 'minflt': tot['minflt'], 'majflt': tot['majflt']}
    rec.update(extra)
    out.write(json.dumps(rec) + '\n'); out.flush()
    return rec
sh('pkill -u 1000 -x Discord; pkill -u 1000 -f MiniBrowser; pkill -u 1000 -x bun; sleep 3; pkill -9 -u 1000 -x Discord; pkill -9 -u 1000 -f MiniBrowser; true')
user('hyprctl dispatch workspace 1')
RESET = ('echo 65536 > /sys/kernel/debug/fault_around_bytes; echo 0 > /sys/kernel/mm/ksm/run; echo none > /sys/kernel/mm/ksm/advisor_mode; echo 100 > /sys/kernel/mm/ksm/pages_to_scan; echo 20 > /sys/kernel/mm/ksm/sleep_millisecs; echo 0 > /sys/kernel/mm/ksm/use_zero_pages; echo madvise > /sys/kernel/mm/transparent_hugepage/enabled; '
         'echo 1000 > /sys/kernel/mm/lru_gen/min_ttl_ms; echo 100 > /proc/sys/vm/swappiness; echo Y > /sys/module/zswap/parameters/enabled; true')
if not os.path.exists(f'/var/tmp/jsx/{label}.noreset'): sh(RESET)
r = sh(pre); sys.stderr.write(f'pre rc={r.returncode} {r.stderr[:200]}\n')
sh('sync; echo 3 > /proc/sys/vm/drop_caches; echo 1 > /proc/sys/vm/compact_memory')
t0 = time.time()
user(f'hyprctl dispatch exec "{launch}"')
tw = None
while time.time() - t0 < 40:
    if f'"class": "{WINCLASS}"' in user('hyprctl clients -j').stdout.replace(f'"class":"{WINCLASS}"', f'"class": "{WINCLASS}"'): tw = round(time.time() - t0, 1); break
    time.sleep(0.25)
extra['t_window_s'] = tw
def cdp(kind, n, gap):
    r = sh(f'timeout 100 python3 -I /var/tmp/msx/cdp.py {kind} 9222 {n} {gap}')
    vals = [l.split()[2] for l in r.stdout.splitlines() if l.startswith(kind + ' ')]
    sm = [l for l in r.stdout.splitlines() if l.startswith('summary')]
    return {'first': vals[0] if vals else None, 'summary': sm[0] if sm else r.stderr[-120:]}
time.sleep(max(0, 45 - (time.time() - t0))); sample('front-45')
time.sleep(10)
if os.path.exists(f'/var/tmp/jsx/{label}.cdp'):
    extra['cdp_frame'] = cdp('frame', 10, 0.5)
sample('front-60')
if post:
    sh(post)
    if os.path.exists(f'/var/tmp/jsx/{label}.cdp'): extra['cdp_after_post'] = cdp('eval', 8, 0.3)
    sample('post-0'); time.sleep(30); sample('post-30'); time.sleep(30); sample('post-60')
user('hyprctl dispatch workspace 2')
loop = os.path.exists(f'/var/tmp/jsx/{label}.loop')
tb = time.time()
while time.time() - tb < bg:
    time.sleep(min(30, max(1, bg - (time.time() - tb))))
    if loop and post: sh(post)
    if abs((time.time() - tb) - 60) < 16: sample('bg-60')
sample(f'bg-{bg}')
if os.path.exists(f'/var/tmp/jsx/{label}.cdp'): extra['cdp_bg_eval'] = cdp('eval', 8, 0.3)
user('hyprctl dispatch workspace 1')
time.sleep(5)
user(f'hyprctl dispatch focuswindow class:{WINCLASS}')
time.sleep(20); sample('back-20')
print('done', label)
