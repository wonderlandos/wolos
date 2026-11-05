#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Host side (the forge) of the MEMSAVER-X2 measurement (docs/bench/memsaver-x2.md): drives one wolvm guest
through the real launcher (Search: wol-search, typed through QEMU, Return) and records, per app and run,
the memory of the whole app at 15 s, in front idle 60 s, typing 60 s (real key events), behind 1 min and
10 min, and back in front 20 s, plus the time from the return key to the first and the last visible change.

  x2drive.py VM MODE APP[,APP..] RUNS OUT.jsonl [--fast] [--shots DIR] [--monitor-run N]

MODE is a label (stock, shipped); the guest is set up by the caller. Needs PIL for the frame analysis."""
import argparse
import json
import os
import subprocess
import sys
import threading
import time

WOLVM = ['/scratch/jsrx/wolvm', '--home', '/scratch/x2/home']
G = 'python3 -I /var/tmp/x2/x2g.py'
APPS = {
    'discord': dict(query='discord', rx='discord'),
    'code': dict(query='code', rx='^code|code-oss|visual studio'),
    'signal': dict(query='signal', rx='signal'),
    'thunderbird': dict(query='thunderbird', rx='thunderbird'),
    'helium': dict(query='helium', rx='helium'),
    'minibrowser': dict(query='minibrowser', rx='minibrowser'),
    'text-editor': dict(query='text editor', rx='texteditor|text-editor'),
    'todos': dict(query='wolkit todos', rx='todos'),
}


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, file=sys.stderr, flush=True)


def wv(vm, *a, timeout=60):
    r = subprocess.run(WOLVM + list(a[:1]) + [vm] + list(a[1:]), capture_output=True, text=True, timeout=timeout)
    return r


def gx(vm, cmd, timeout=120):
    r = subprocess.run(WOLVM + ['ssh', '--timeout', str(timeout), vm, cmd], capture_output=True, text=True,
                       timeout=timeout + 30)
    return r.stdout


def unit_watch(vm, name, pid, secs, every):
    """A watch that runs as a guest unit, so the guest agent stays free for other calls meanwhile."""
    f = f'/var/tmp/x2/{name}.out'
    gx(vm, f'systemctl stop x2w-{name} 2>/dev/null; rm -f {f}; systemd-run --unit=x2w-{name} --collect '
           f'sh -c "python3 -I /var/tmp/x2/x2g.py watch {pid} {secs} {every} > {f}"')
    return f


def unit_collect(vm, name, f, end_ts):
    time.sleep(max(0, end_ts - time.time()))
    for _ in range(20):
        if gx(vm, f'systemctl is-active x2w-{name} || true').strip() not in ('active', 'activating'):
            break
        time.sleep(3)
    return jl(gx(vm, f'cat {f}', timeout=60))


def jl(text):
    out = []
    for line in text.splitlines():
        if line.startswith('{'):
            try:
                out.append(json.loads(line))
            except ValueError:
                pass
    return out


def key(vm, combo):
    wv(vm, 'key', combo)


def shot(vm, path):
    wv(vm, 'screenshot', path)


def search_alive(vm):
    return gx(vm, 'pgrep -x wol-search >/dev/null && echo yes || echo no').strip() == 'yes'


def search_launch(vm, query):
    """The real launcher: Search (the panel `wol-search` toggles; it is open while the process lives), the
    name typed through QEMU, Return."""
    for _ in range(4):
        for _ in range(3):
            if not search_alive(vm):
                break
            key(vm, 'esc')
            time.sleep(1)
        gx(vm, f'{G} hypr dispatch exec wol-search')
        time.sleep(2)
        if search_alive(vm):
            break
    wv(vm, 'type', query)
    time.sleep(1.5)
    key(vm, 'ret')
    time.sleep(1.5)
    if search_alive(vm):  # still open: nothing matched or the Return was lost
        key(vm, 'ret')
        time.sleep(1.5)


def launch(vm, app):
    spec = APPS[app]
    search_launch(vm, spec['query'])
    return spec


def analyse_frames(d, t_key, stamps):
    """Seconds from the key to the first frame that differs from the one before it and to the last that
    does (ratio of changed pixels), from the files' times."""
    from PIL import Image, ImageChops
    fs = sorted(f for f in os.listdir(d) if f.endswith('.png'))
    ims = [(stamps[i], Image.open(os.path.join(d, f)).convert('L').resize((320, 200))) for i, f in enumerate(fs[:len(stamps)])]
    ref = None
    for t, im in ims:
        if t < t_key:
            ref = im
    if ref is None:
        return None

    def frac(a, b):
        diff = ImageChops.difference(a, b).point(lambda v: 255 if v > 24 else 0)
        return sum(1 for v in diff.tobytes() if v) / (320 * 200)

    after = [(t, im) for t, im in ims if t >= t_key]
    first = settle = None
    prev = ref
    last_big = None
    for t, im in after:
        if first is None and frac(ref, im) > 0.02:
            first = t - t_key
        if frac(prev, im) > 0.004:
            last_big = t - t_key
        prev = im
    return {'first_change_s': None if first is None else round(first, 2),
            'last_change_s': None if last_big is None else round(last_big, 2), 'frames': len(after)}


def run_one(vm, app, mode, idx, a):
    P = dict(launch15=15, idle=60, typing=60, bg1=60, bg10=600, back=20)
    if a.fast:
        P = dict(launch15=5, idle=10, typing=12, bg1=15, bg10=int(os.environ.get('X2_BG', '30')), back=10)
    rec = {'vm': vm, 'mode': mode, 'app': app, 'run': idx, 'started': time.time(), 'params': P}
    spec = APPS[app]
    # A clean start: nothing of the app, caches dropped, workspace 1, no idle daemon.
    w0 = jl(gx(vm, f'{G} window "{spec["rx"]}" 0'))
    gx(vm, f'pkill hypridle; {G} hypr dispatch workspace 1; sync; echo 3 > /proc/sys/vm/drop_caches; true')
    for _ in range(3):
        w0 = gx(vm, f'{G} window "{spec["rx"]}" 0').strip()
        if w0 in ('', 'null'):
            break
        w = json.loads(w0)
        gx(vm, f'{G} kill {w["pid"]}')
    time.sleep(3)
    wv(vm, 'key', 'ctrl-alt-1')
    t_launch = time.time()
    launch(vm, app)
    w = None
    for _ in range(2):
        out = gx(vm, f'{G} window "{spec["rx"]}" 180', timeout=220).strip()
        if out and out != 'null':
            w = json.loads(out)
            break
        log(vm, app, 'no window yet; trying the launcher again')
        launch(vm, app)
    if not w:
        rec['error'] = 'window did not appear'
        return rec
    t_win = time.time()
    rec['window'] = w
    rec['launch_to_window_s'] = round(t_win - t_launch, 1)
    pid = w['pid']
    gx(vm, f'{G} hypr dispatch focuswindow address:{w["address"]}')
    # 15 s after the window showed
    time.sleep(max(0, P['launch15'] - (time.time() - t_win)))
    rec['launch15'] = jl(gx(vm, f'{G} sample {pid}'))
    if a.shots:
        shot(vm, f'{a.shots}/{mode}-{app}-{idx}-front.png')
    rec['idle'] = jl(gx(vm, f'{G} watch {pid} {P["idle"]} 10', timeout=P['idle'] + 60))
    # typing: real key events through QEMU while the guest samples (as a unit)
    f = unit_watch(vm, 'typing', pid, P['typing'], 10)
    t0 = time.time()
    end = t0 + P['typing']
    n = 0
    while time.time() < end - 5:
        n += 1
        wv(vm, 'type', f'hello from x2 {n} ')
        time.sleep(8)
    rec['typing'] = unit_collect(vm, 'typing', f, end + 2)
    if a.shots:
        shot(vm, f'{a.shots}/{mode}-{app}-{idx}-typing.png')
    # behind: another workspace (Memory Saver's cue), by the real key combination
    key(vm, 'ctrl-alt-2')
    t_bg = time.time()
    f = unit_watch(vm, 'bg', pid, P['bg10'] + 10, 30)
    mon = None
    if a.monitor_run and idx == a.monitor_run:
        time.sleep(max(0, P['bg10'] - 70 - (time.time() - t_bg)))
        search_launch(vm, 'system mon')
        time.sleep(10)
        # The pointer moves away first: a click where the pointer already is did not reach the sidebar
        # while a thrashing app held the CPUs (docs/bench/memsaver-x2.md).
        wv(vm, 'click', '700', '500')
        time.sleep(2)
        wv(vm, 'click', '262', '204')
        time.sleep(4)
        wv(vm, 'click', '262', '206')
        time.sleep(10)
        if a.shots:
            shot(vm, f'{a.shots}/{mode}-{app}-{idx}-monitor-behind.png')
        mon = True
    rec['bg'] = unit_collect(vm, 'bg', f, t_bg + P['bg10'] + 12)
    gx(vm, f'cat /var/run/wol-memsaver.json > /dev/null; true')
    rec['saver_status_behind'] = gx(vm, 'cat /var/run/wol-memsaver.json').strip()
    rec['wold_log'] = gx(vm, 'journalctl -u wold --no-pager -o cat --since "-14 min" | grep -i "memsaver" | tail -n 12').strip()
    # back in front: sample before, press the keys, watch the frames and the guest
    rec['pre_return'] = jl(gx(vm, f'{G} sample {pid}'))
    fdir = f'/tmp/x2-frames-{vm}'
    subprocess.run(['rm', '-rf', fdir])
    os.makedirs(fdir)
    stamps = []
    stop = threading.Event()

    def grab():
        i = 0
        while not stop.is_set():
            wv(vm, 'screenshot', f'{fdir}/s{i:03d}.png')
            stamps.append(time.time())
            i += 1

    th = threading.Thread(target=grab)
    th.start()
    time.sleep(2.5)
    t_key = time.time()
    key(vm, 'ctrl-alt-1')
    rec['back'] = jl(gx(vm, f'{G} watch {pid} {P["back"]} 2', timeout=P['back'] + 60))
    stop.set()
    th.join()
    fr = None
    try:
        rec['return_frames'] = analyse_frames(fdir, t_key, stamps)
    except Exception as e:  # analysis is a bonus; the memory numbers do not depend on it
        rec['return_frames'] = {'error': str(e)}
    if a.shots:
        shot(vm, f'{a.shots}/{mode}-{app}-{idx}-back.png')
    if mon:
        gx(vm, f'{G} hypr dispatch closewindow class:org.wolos.monitor')
    # leave nothing of the app behind
    gx(vm, f'{G} kill {pid}')
    rec['finished'] = time.time()
    return rec


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('vm')
    ap.add_argument('mode')
    ap.add_argument('apps')
    ap.add_argument('runs', type=int)
    ap.add_argument('out')
    ap.add_argument('--fast', action='store_true')
    ap.add_argument('--shots')
    ap.add_argument('--monitor-run', type=int, default=0)
    ap.add_argument('--prime', action='store_true', help='one throwaway start per app first')
    a = ap.parse_args()
    if a.shots:
        os.makedirs(a.shots, exist_ok=True)
    for app in a.apps.split(','):
        if a.prime:
            log(a.vm, app, 'prime')
            r = run_one(a.vm, app, a.mode + '-prime', 0, argparse.Namespace(fast=True, shots=None, monitor_run=0))
            log(a.vm, app, 'prime done', r.get('error', 'ok'), r.get('window'))
        for i in range(1, a.runs + 1):
            log(a.vm, a.mode, app, 'run', i)
            try:
                rec = run_one(a.vm, app, a.mode, i, a)
            except Exception as e:
                rec = {'vm': a.vm, 'mode': a.mode, 'app': app, 'run': i, 'error': repr(e)}
            with open(a.out, 'a') as f:
                f.write(json.dumps(rec) + '\n')
            log(a.vm, a.mode, app, i, rec.get('error', 'ok'))


if __name__ == '__main__':
    main()
