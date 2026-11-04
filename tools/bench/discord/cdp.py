#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Responsiveness of a Chromium/Electron page through its DevTools port (the app started with
--remote-debugging-port=PORT), Python standard library only (a minimal websocket client).
  cdp.py eval PORT N GAP    N evaluations of a constant at GAP s (jittered): how long the page's
                            main thread takes to answer, frozen or not (the delay a message sees)
  cdp.py frame PORT N GAP   N waits for two animation frames: in front, how fast it can draw
Prints one line per probe and a summary line `n p50_ms p95_ms max_ms`."""
import base64
import json
import os
import random
import socket
import struct
import sys
import time
import urllib.request


class WS:
    def __init__(self, url):
        host, path = url[len('ws://'):].split('/', 1)
        h, p = host.split(':')
        self.s = socket.create_connection((h, int(p)), timeout=120)
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall((f'GET /{path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\n'
                        f'Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n'
                        'Sec-WebSocket-Version: 13\r\n\r\n').encode())
        buf = b''
        while b'\r\n\r\n' not in buf:
            buf += self.s.recv(4096)
        if b' 101 ' not in buf.split(b'\r\n', 1)[0]:
            raise SystemExit('cdp: no websocket upgrade: ' + buf[:200].decode(errors='replace'))
        self.rest = buf.split(b'\r\n\r\n', 1)[1]

    def send(self, text):
        data = text.encode()
        mask = os.urandom(4)
        n = len(data)
        head = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack('>H', n))
        self.s.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _read(self, n):
        while len(self.rest) < n:
            got = self.s.recv(65536)
            if not got:
                raise EOFError
            self.rest += got
        out, self.rest = self.rest[:n], self.rest[n:]
        return out

    def recv(self):
        b0, b1 = self._read(2)
        n = b1 & 0x7f
        if n == 126:
            n = struct.unpack('>H', self._read(2))[0]
        elif n == 127:
            n = struct.unpack('>Q', self._read(8))[0]
        return self._read(n).decode(errors='replace')


def page(port):
    for t in json.load(urllib.request.urlopen(f'http://127.0.0.1:{port}/json', timeout=60)):
        if t.get('type') == 'page':
            return t['webSocketDebuggerUrl']
    raise SystemExit('cdp: no page target')


def run(kind, port, n, gap):
    ws = WS(page(port))
    expr = {'eval': '1',
            'frame': 'new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(1))))'}[kind]
    out = []
    for i in range(int(n)):
        time.sleep(float(gap) * random.uniform(0.5, 1.5))
        t = time.monotonic()
        ws.send(json.dumps({'id': i + 1, 'method': 'Runtime.evaluate',
                            'params': {'expression': expr, 'awaitPromise': True}}))
        while json.loads(ws.recv()).get('id') != i + 1:
            pass
        out.append((time.monotonic() - t) * 1000)
        print(f'{kind} {i} {out[-1]:.0f}', flush=True)
    out.sort()
    print(f'summary {kind} n {len(out)} p50_ms {out[len(out) // 2]:.0f} '
          f'p95_ms {out[int(len(out) * 0.95) - 1 if len(out) > 1 else 0]:.0f} max_ms {out[-1]:.0f}', flush=True)


if __name__ == '__main__':
    run(*sys.argv[1:5])
