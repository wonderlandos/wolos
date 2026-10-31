#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Drive a firmware screen over the VM's serial socket: wait for text, send keys.
usage: serial_expect.py SOCK LOG TIMEOUT step...   each step is WAIT=>KEYS; KEYS: text with \\r \\e[B etc.
Ends with exit 0 when all steps ran, 1 on a timeout (the last screen goes to stderr)."""
import re, socket, sys, time
ANSI = re.compile(rb"\x1b\[[0-9;=?]*[A-Za-z]|\x1b[()][0-9A-B]")
def main():
    sock, log, timeout, steps = sys.argv[1], sys.argv[2], float(sys.argv[3]), sys.argv[4:]
    s = socket.socket(socket.AF_UNIX); s.settimeout(0.2)
    for _ in range(100):
        try:
            s.connect(sock); break
        except OSError:
            time.sleep(0.1)
    raw = b""; seen = b""; out = open(log, "ab")
    def pump():
        nonlocal raw, seen
        try:
            d = s.recv(65536)
            if d:
                out.write(d); out.flush(); raw += d; seen += ANSI.sub(b"", d)
        except socket.timeout:
            pass
    for st in steps:
        wait, _, keys = st.partition("=>")
        keys = keys.encode().decode("unicode_escape").encode("latin-1")
        end = time.time() + timeout
        while wait.encode() not in seen:
            pump()
            if time.time() > end:
                sys.stderr.write(f"timeout waiting for {wait!r}; screen:\n{seen[-1500:].decode('latin-1')}\n")
                return 1
        seen = b""
        time.sleep(0.4)
        for i in range(0, len(keys)):
            k = keys[i:i+1]
            if k == b"\x1b":
                s.sendall(keys[i:i+3])
                continue
            if keys[max(0, i-2):i+1].startswith(b"\x1b"):
                continue
            s.sendall(k); time.sleep(0.05)
    end = time.time() + timeout
    while time.time() < end:
        pump()
    return 0
sys.exit(main())
