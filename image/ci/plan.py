# SPDX-License-Identifier: BSD-2-Clause
"""Build waves from image/repo/packages: each wave only needs packages from earlier waves.

Line format: NAME DIR LOCAL-DEPS(comma, - for none) PATHS...
"""
import sys


def parse(text):
    pkgs = {}
    for line in text.splitlines():
        f = line.split()
        if not f or f[0].startswith("#"):
            continue
        deps = [] if f[2] == "-" else f[2].split(",")
        pkgs[f[0]] = {"dir": f[1], "deps": deps}
    return pkgs


def waves(pkgs, only):
    """Waves for `only`; deps outside `only` are already published and need no slot."""
    want = set(only) & set(pkgs)
    done, out = set(), []
    while want - done:
        ready = sorted(
            p for p in want - done if all(d in done or d not in want for d in pkgs[p]["deps"])
        )
        if not ready:
            raise ValueError(f"dependency cycle in {sorted(want - done)}")
        out.append(ready)
        done |= set(ready)
    return out


# CPU units per task (1024 = 1 vCPU): the kernel takes a whole host, the rest share one.
SIZES = {"linux-wolos": (63488, 110000), "wolos-system": (16384, 28000)}  # 64-vCPU hosts
DEFAULT_SIZE = (8192, 16000)


def size(name):
    return SIZES.get(name, DEFAULT_SIZE)


if __name__ == "__main__":
    pkgs = parse(open(sys.argv[1]).read())
    for w in waves(pkgs, sys.argv[2:] or list(pkgs)):
        print(" ".join(w))
