#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Screenshots of the System Monitor Apps page for a few apps, on the forge, after the measured runs:
  x2showcase.py VM MODE APP[,APP..]
Each app: started through the launcher, 2 minutes in front, 2.5 minutes behind, then the System Monitor
(opened through the same launcher on the behind workspace) on its Apps page. Shots in
/scratch/x2/shots/showcase/. The numbers of the doc come from x2drive.py, not from here."""
import os
import sys
import time

sys.path.insert(0, '/scratch/x2/tools')
import x2drive as d  # noqa: E402

vm, mode, apps = sys.argv[1], sys.argv[2], sys.argv[3].split(',')
shots = '/scratch/x2/shots/showcase'
os.makedirs(shots, exist_ok=True)
G = d.G
for app in apps:
    spec = d.APPS[app]
    d.gx(vm, f'pkill hypridle; {G} hypr dispatch workspace 1; true')
    d.wv(vm, 'key', 'ctrl-alt-1')
    d.launch(vm, app)
    w = d.jl(d.gx(vm, f'{G} window "{spec["rx"]}" 180', timeout=220))[0]
    d.gx(vm, f'{G} hypr dispatch focuswindow address:{w["address"]}')
    time.sleep(120)
    d.shot(vm, f'{shots}/{mode}-{app}-front.png')
    d.key(vm, 'ctrl-alt-2')
    time.sleep(150)
    d.search_launch(vm, 'system mon')
    time.sleep(20)
    d.wv(vm, 'click', '700', '500')
    time.sleep(2)
    d.wv(vm, 'click', '262', '204')
    time.sleep(4)
    d.wv(vm, 'click', '262', '206')
    time.sleep(20)
    d.shot(vm, f'{shots}/{mode}-{app}-monitor-behind.png')
    d.gx(vm, f'{G} hypr dispatch closewindow class:org.wolos.monitor')
    d.gx(vm, f'{G} kill {w["pid"]}')
    time.sleep(3)
    print(vm, mode, app, 'done', flush=True)
