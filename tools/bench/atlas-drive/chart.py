#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Chart and tables for docs/bench/atlas-drive.md from bench.py's results.json.
  chart.py results.json out.png tables.md
One chart: for each metric, Playwright is 100 % (grey) and Atlas Drive is its share of that (accent);
below 100 % Drive is ahead, above 100 % it is behind. The value labels are the medians."""
import json, sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

res = json.load(open(sys.argv[1]))
S = res["summary"]
D, P = S["drive"], S["pw"]

def med(side, key):
    v = side.get(key)
    return None if v is None else v["median"]

# (label, drive key, playwright key, unit)
ROWS = [
    ("cold start to first command", "cold_ms", "cold_ms", "ms"),
    ("goto + load (local app page)", "app.goto_ms", "app.goto_ms", "ms"),
    ("click round trip (by ref / selector)", "click_ms", "click_ms", "ms"),
    ("100 sequential clicks", "clicks100_ms", "clicks100_ms", "ms"),
    ("full-page screenshot", "shot_ms", "shot_ms", "ms"),
    ("snapshot time, small page", "hub.snap_ms", "hub.aria_ms", "ms"),
    ("snapshot time, heavy page", "heavy.snap_ms", "heavy.aria_ms", "ms"),
    ("snapshot size, hub page", "hub.snap_tokens", "hub.aria_tokens", "tokens"),
    ("snapshot size, heavy page", "heavy.snap_tokens", "heavy.aria_tokens", "tokens"),
    ("agent task: tokens", "task_tokens", "task_tokens", "tokens"),
    ("agent task: time", "task_ms", "task_ms", "ms"),
    ("driver process memory", "driver_rss_mb", "driver_rss_mb", "MB"),
    ("browser memory with driver attached", "browser_rss_mb", "browser_rss_mb", "MB"),
    ("driver idle wakeups", "idle_wakeups_per_s", "idle_wakeups_per_s", "/s"),
]

rows = []
for label, dk, pk, unit in ROWS:
    d, p = med(D, dk), med(P, pk)
    if d is None or p is None:
        continue
    rows.append((label, d, p, unit))

ACCENT, GREY, INK = "#2f6fed", "#b9bec6", "#1b1f24"


def fmt(v):
    return f"{v:,.0f}" if v >= 100 else f"{v:.1f}".rstrip("0").rstrip(".")

fig, ax = plt.subplots(figsize=(10, 0.62 * len(rows) + 1.6), dpi=150)
y = list(range(len(rows)))[::-1]
H = 0.34
for yi, (label, d, p, unit) in zip(y, rows):
    share = 100.0 * d / p if p else 0.0
    ax.barh(yi + H / 2, 100, height=H, color=GREY)
    ax.barh(yi - H / 2, min(share, 260), height=H, color=ACCENT)
    ax.text(101, yi + H / 2, f"{fmt(p)} {unit}", va="center", fontsize=8, color=INK)
    ax.text(min(share, 260) + 1, yi - H / 2, f"{fmt(d)} {unit}  ({share:.0f} %)", va="center", fontsize=8, color=INK, fontweight="bold")
ax.set_yticks(y)
ax.set_yticklabels([r[0] for r in rows], fontsize=9, color=INK)
ax.set_xlim(0, 280)
ax.set_xlabel("share of Playwright's median (Playwright = 100 %, lower is better for every row)", fontsize=9, color=INK)
ax.axvline(100, color=INK, lw=0.6)
for s in ("top", "right"):
    ax.spines[s].set_visible(False)
ax.tick_params(axis="x", labelsize=8)
ax.set_title(f"Atlas Drive vs Playwright {res.get('pw_version', '')}, same Atlas binary, median of {res['runs']} runs", fontsize=11, color=INK, loc="left")
from matplotlib.patches import Patch
ax.legend(handles=[Patch(color=GREY, label="Playwright"), Patch(color=ACCENT, label="Atlas Drive")], loc="lower right", fontsize=8, frameon=False)
fig.tight_layout()
fig.savefig(sys.argv[2])

def cell(side, key, nd=1):
    v = side.get(key)
    if v is None:
        return "-"
    f = lambda x: f"{x:.{nd}f}".rstrip("0").rstrip(".") if nd else f"{x:.0f}"
    return f"{f(v['median'])} [{f(v['min'])}-{f(v['max'])}]"

out = []
out.append("| Metric | Playwright median [min-max] | Atlas Drive median [min-max] | Drive / Playwright |")
out.append("|---|---|---|---|")
ALL = ROWS + [
    ("click round trip by locator", "click_locator_ms", "click_locator_ms", "ms"),
    ("agent task with known locators: time", "task_known_ms", "task_known_ms", "ms"),
    ("agent task with known locators: tokens", "task_known_tokens", "task_known_tokens", "tokens"),
]
for page in ("app", "wolos", "hub", "heavy", "fed", "chrome"):
    ALL.append((f"goto + load: {page}", f"{page}.goto_ms", f"{page}.goto_ms", "ms"))
    ALL.append((f"snapshot time: {page}", f"{page}.snap_ms", f"{page}.aria_ms", "ms"))
    ALL.append((f"snapshot tokens: {page}", f"{page}.snap_tokens", f"{page}.aria_tokens", "tokens"))
    ALL.append((f"snapshot time without the trap scan: {page}", f"{page}.snap_unsafe_ms", f"{page}.aria_ms", "ms"))
    ALL.append((f"snapshot --interactive-only tokens: {page} (Drive) vs content() tokens (Playwright)", f"{page}.snap_io_bytes", f"{page}.content_tokens", "mixed"))
seen = set()
for label, dk, pk, unit in ALL:
    if label in seen:
        continue
    seen.add(label)
    d, p = D.get(dk), P.get(pk)
    if d is None or p is None or unit == "mixed":
        continue
    ratio = d["median"] / p["median"] if p["median"] else float("nan")
    out.append(f"| {label} ({unit}) | {cell(P, pk)} | {cell(D, dk)} | {ratio:.2f} |")
open(sys.argv[3], "w").write("\n".join(out) + "\n")
print("\n".join(out))
