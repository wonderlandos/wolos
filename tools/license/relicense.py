#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""SPDX headers and license fields: check, or rewrite in place (idempotent).

  relicense.py [--root DIR] [--policy FILE] check            list every problem, exit 1 if any
  relicense.py [--root DIR] [--policy FILE] apply [--dry-run] fix them, print what changed

Files come from `git ls-files --recurse-submodules`, so submodules are covered and untracked
files are not. Rules live in policy.json: skip globs, per-path expected license expressions,
which old identifiers may be rewritten, license fields (Cargo.toml, PKGBUILD, shard.yml),
exact-text doc edits and files copied from tools/license/staged/.
A foreign identifier (not in `rewrite_from`, not expected) is reported and never rewritten.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPDX = re.compile(r"SPDX-License-Identifier:\s*(.+?)\s*(?:\*/|-->)?\s*$")


def glob_re(g):
    out, i = "", 0
    while i < len(g):
        if g.startswith("**/", i):
            out += "(?:.*/)?"
            i += 3
        elif g.startswith("**", i):
            out += ".*"
            i += 2
        elif g[i] == "*":
            out += "[^/]*"
            i += 1
        elif g[i] == "?":
            out += "[^/]"
            i += 1
        else:
            out += re.escape(g[i])
            i += 1
    return re.compile("^" + out + "$")


def any_match(globs, path):
    return any(glob_re(g).match(path) for g in globs)


class Policy:
    def __init__(self, d):
        self.d = d
        self.default = d["default"]

    def skipped(self, path):
        return any_match(self.d["skip"], path) and not any_match(self.d.get("unskip", []), path)

    def expected(self, path):
        for e in self.d.get("expected", []):
            if glob_re(e["glob"]).match(path):
                return e["spdx"]
        return self.default

    def accepted(self, path):
        ok = {self.expected(path)}
        for a in self.d.get("accept", []):
            if glob_re(a["glob"]).match(path):
                ok.update(a["spdx"])
        return ok

    def style(self, path, head):
        name = path.rsplit("/", 1)[-1]
        if name in self.d["code_names"]:
            return self.d["code_names"][name]
        if "." in name:
            return self.d["code"].get(name.rsplit(".", 1)[1])
        if head.startswith("#!") and re.match(r"#!.*\b(sh|bash|zsh|python3?|env)\b", head.split("\n", 1)[0]):
            return "#"
        return None


def norm(s):
    return " ".join(s.split())


def files(root):
    out = subprocess.run(["git", "-C", str(root), "ls-files", "--recurse-submodules", "-z"],
                         check=True, capture_output=True).stdout
    return sorted(p for p in out.decode().split("\0") if p)


def read_text(path, limit):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        return None
    b = path.read_bytes()
    if b"\0" in b[:4096]:
        return None
    try:
        return b.decode("utf-8")
    except UnicodeDecodeError:
        return None


def find_spdx(text, n):
    for i, line in enumerate(text.split("\n")[:n]):
        m = SPDX.search(line)
        if m and "SPDX-License-Identifier" in line:
            return i, norm(m.group(1))
    return None, None


def insert_header(text, style, spdx):
    nl = "\r\n" if "\r\n" in text[:4096] else "\n"
    line = {"/*": f"/* SPDX-License-Identifier: {spdx} */", "//": f"// SPDX-License-Identifier: {spdx}",
            "#": f"# SPDX-License-Identifier: {spdx}", "--": f"-- SPDX-License-Identifier: {spdx}"}[style]
    if text.startswith("#!"):
        first, _, rest = text.partition("\n")
        return first + "\n" + line + nl + rest
    return line + nl + text


def replace_spdx(text, idx, spdx):
    lines = text.split("\n")
    lines[idx] = re.sub(r"(SPDX-License-Identifier:\s*)(.+?)(\s*(?:\*/|-->))?(\r?)$",
                        lambda m: m.group(1) + spdx + (m.group(3) or "") + m.group(4), lines[idx])
    return "\n".join(lines)


def field_new(kind, text, to, frm):
    """Return the text with the license field rewritten, or None if there is no such field."""
    if kind == "toml":
        pat = re.compile(r'(?m)^(license\s*=\s*")([^"]*)(")')
        m = pat.search(text)
        if not m:
            return None
        return pat.sub(lambda m: m.group(1) + (to if m.group(2) in frm else m.group(2)) + m.group(3), text, 1)
    if kind == "yaml":
        pat = re.compile(r"(?m)^(license:\s*)(\S.*?)(\s*)$")
        m = pat.search(text)
        if not m:
            return None
        return pat.sub(lambda m: m.group(1) + (to if m.group(2) in frm else m.group(2)) + m.group(3), text, 1)
    if kind == "pkgbuild":
        pat = re.compile(r"(?m)^(license=\()([^)]*)(\))")
        m = pat.search(text)
        if not m:
            return None

        def fix(m):
            seen, toks = set(), []
            for t in m.group(2).split():
                t = to if t.strip("'\"") in frm else t
                if t not in seen:
                    seen.add(t)
                    toks.append(t)
            return m.group(1) + " ".join(toks) + m.group(3)
        return pat.sub(fix, text, 1)
    raise ValueError(kind)


def run(root, pol, apply, dry=False):
    """Returns (issues, changes). issues are what `check` reports; changes what `apply` wrote."""
    root = Path(root)
    issues, changes = [], []
    n = pol.d.get("head_lines", 12)
    for rel in files(root):
        p = root / rel
        if p.is_dir() or pol.skipped(rel):
            continue
        text = read_text(p, pol.d["max_bytes"])
        if text is None:
            continue
        new = text
        idx, found = find_spdx(text, n)
        want = pol.expected(rel)
        if found is None:
            style = pol.style(rel, text)
            if style:
                issues.append((rel, "missing", None, want))
                new = insert_header(text, style, want)
        elif found not in pol.accepted(rel):
            if found in pol.d["rewrite_from"] or found in (want, pol.default):
                issues.append((rel, "wrong", found, want))
                new = replace_spdx(text, idx, want)
            else:
                issues.append((rel, "foreign", found, want))
        for f in pol.d.get("fields", []):
            if glob_re(f["glob"]).match(rel):
                fixed = field_new(f["kind"], new, pol.default, pol.d["field_from"])
                if fixed is not None and fixed != new:
                    issues.append((rel, "field", None, pol.default))
                    new = fixed
        if new != text:
            changes.append(rel)
            if apply and not dry:
                p.write_bytes(new.encode("utf-8"))
    for d in pol.d.get("docs", []):
        p = root / d["file"]
        text = read_text(p, pol.d["max_bytes"]) if p.exists() else None
        if text is not None and d["old"] in text:
            issues.append((d["file"], "text", d["old"][:50], d["new"][:50]))
            changes.append(d["file"])
            if apply and not dry:
                p.write_text(text.replace(d["old"], d["new"]), encoding="utf-8", newline="")
    for c in pol.d.get("copy", []):
        src, dst = root / c["from"], root / c["to"]
        if not src.exists():
            issues.append((c["from"], "staged-missing", None, None))
        elif not dst.exists() or dst.read_bytes() != src.read_bytes():
            issues.append((c["to"], "stale", None, c["from"]))
            changes.append(c["to"])
            if apply and not dry:
                dst.write_bytes(src.read_bytes())
    return issues, changes


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=str(HERE.parent.parent))
    ap.add_argument("--policy", default=str(HERE / "policy.json"))
    ap.add_argument("cmd", choices=["check", "apply"])
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)
    pol = Policy(json.loads(Path(a.policy).read_text()))
    issues, changes = run(a.root, pol, a.cmd == "apply", a.dry_run)
    if a.cmd == "check":
        for rel, kind, found, want in issues:
            print(f"{kind}: {rel}" + (f" has {found}" if found else "") + (f", want {want}" if want else ""))
        print(f"{len(issues)} problem(s)")
        return 1 if issues else 0
    for rel in changes:
        print(("would change " if a.dry_run else "changed ") + rel)
    for rel, kind, found, want in issues:
        if kind == "foreign":
            print(f"left alone (foreign): {rel} has {found}")
    print(f"{len(changes)} file(s) {'would change' if a.dry_run else 'changed'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
