#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Delivery delay through a freeze duty cycle, the shape of a chat app's websocket.
  push.py serve PORT COUNT GAP   host side: one client; send COUNT stamped lines GAP s apart (with
                                 jitter), print each round trip (send -> the client's echo) in ms
  push.py client HOST PORT       guest side, run inside the app's scope: echo every line at once
"""
import random
import socket
import sys
import time


def serve(port, count, gap):
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(('127.0.0.1', int(port)))
    s.listen(1)
    c, _ = s.accept()
    f = c.makefile('rwb', buffering=0)
    rtts = []
    for i in range(int(count)):
        time.sleep(float(gap) * random.uniform(0.5, 1.5))
        t = time.monotonic()
        f.write(f'{i}\n'.encode())
        f.readline()
        rtts.append((time.monotonic() - t) * 1000)
        print(f'msg {i} rtt_ms {rtts[-1]:.0f}', flush=True)
    rtts.sort()
    print(f'n {len(rtts)} p50_ms {rtts[len(rtts) // 2]:.0f} max_ms {rtts[-1]:.0f}', flush=True)


def client(host, port):
    c = socket.create_connection((host, int(port)))
    f = c.makefile('rwb', buffering=0)
    while True:
        line = f.readline()
        if not line:
            return
        f.write(line)


if __name__ == '__main__':
    {'serve': serve, 'client': client}[sys.argv[1]](*sys.argv[2:])
