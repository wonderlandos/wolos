#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""One guest, both modes (docs/bench/memsaver-x2.md), on the forge:

  x2plan.py VM APPS ORDER RUNS

APPS is a comma list, ORDER is `stock,shipped` or `shipped,stock`. `stock` is the 0.12.1 image's own
wold/shell with the kernel knobs at the kernel defaults (fault_around_bytes 65536, KSM off); `shipped`
adds the sysext `zz-x2-shipped` (this tree's wold, wol-shell, wol-search, wol-realtime, wolos-mm, ...) and
the image's tuning file (fault_around_bytes 4096, KSM for opted-in apps). A change of mode reboots the
guest and signs in (boot-running.yaml). Results: /scratch/x2/res/VM-MODE.jsonl, shots under /scratch/x2/shots/."""
import subprocess
import sys
import time

sys.path.insert(0, '/scratch/x2/tools')
import x2drive as d  # noqa: E402

W = d.WOLVM
HERE = '/scratch/x2'


def log(*a):
    print(time.strftime('%H:%M:%S'), *a, file=sys.stderr, flush=True)


def has_ext(vm):
    return d.gx(vm, 'test -x /usr/bin/wolos-mm && echo yes || echo no').strip() == 'yes'


def reboot(vm):
    d.gx(vm, 'systemctl reboot')
    time.sleep(20)
    r = subprocess.run(W + ['journey', vm, f'{HERE}/tools/boot-running.yaml', '--out', f'{HERE}/boot-{vm}-{int(time.time())}'],
                       capture_output=True, text=True, timeout=900)
    log(vm, 'reboot', r.stdout.strip().splitlines()[-1] if r.stdout.strip() else r.stderr[-200:])


def prep(vm, mode):
    if mode == 'shipped' and not has_ext(vm):
        d.gx(vm, 'sh -c "cd /var/tmp/x2 && curl -fsS -o ship.tar http://10.0.2.2:8800/ship.tar && '
                 'rm -rf /var/lib/extensions/zz-x2-shipped && mkdir -p /var/lib/extensions/zz-x2-shipped && '
                 'tar -C /var/lib/extensions/zz-x2-shipped -xf ship.tar && '
                 'ln -sf /usr/lib/systemd/system/wolos-mm.service /etc/systemd/system/multi-user.target.wants/wolos-mm.service && '
                 'systemd-sysext refresh"', timeout=180)
        reboot(vm)
    if mode == 'stock' and has_ext(vm):
        d.gx(vm, 'sh -c "rm -rf /var/lib/extensions/zz-x2-shipped /etc/systemd/system/multi-user.target.wants/wolos-mm.service; systemd-sysext refresh"', timeout=120)
        reboot(vm)
    d.gx(vm, 'pkill hypridle; true')
    if mode == 'stock':
        # the image carries the tuning file; the stock mode runs on the kernel defaults
        d.gx(vm, 'echo 65536 > /sys/kernel/debug/fault_around_bytes; echo 0 > /sys/kernel/mm/ksm/run; true')
    ident = d.gx(vm, 'sh -c "ls -la /usr/bin/wold /usr/bin/wol-shell /usr/bin/wol-search; cat /sys/kernel/debug/fault_around_bytes '
                     '/sys/kernel/mm/ksm/run; pgrep -a wold; pgrep -a wol-realtime; pacman -Q wolos-system"').strip()
    log(vm, mode, 'identity:', ident.replace('\n', ' | '))
    open(f'{HERE}/res/{vm}-{mode}.identity', 'w').write(ident + '\n')


def main():
    vm, apps, order, runs = sys.argv[1], sys.argv[2], sys.argv[3].split(','), int(sys.argv[4])
    for mode in order:
        prep(vm, mode)
        out = f'{HERE}/res/{vm}-{mode}.jsonl'
        sys.argv = ['x2drive', vm, mode, apps, str(runs), out, '--shots', f'{HERE}/shots/{vm}-{mode}',
                    '--monitor-run', str(runs), '--prime']
        d.main()
    log(vm, 'all done')


if __name__ == '__main__':
    main()
