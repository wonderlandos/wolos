# SPDX-License-Identifier: GPL-2.0-or-later
"""policies.py: every browser policy WolOS writes is one the engine defines, with the right type and
value (plan AT-7; brief item 5). A policy key that is misspelt, removed by an engine update or given
the wrong type is silently ignored by the browser, so a privacy policy would quietly stop working.

SOURCE is what WolOS writes: `crates/core/src/browser.rs` (the keys of the `json!` blocks of
`policies()` and `recommended_policies()`), or a policy file as installed
(`/etc/atlas/policies/managed/wolos.json`, `.../recommended/wolos.json`; its section is taken from
the folder name or --section). DEFS is what the engine defines: a directory of the engine's policy
definitions (`components/policy/resources/templates/policy_definitions`, YAML, read here with a small
reader because the tools use only the standard library), or a table with the columns
`name type ints recommendable` (the fixture `crates/core/tests/fixtures/atlas-policies.tsv`).

Checks per key: defined; value type matches (bool, int, string, list, dict); an enum value is one of
the allowed ones; a key in the recommended section is one that may be recommended; a policy marked
deprecated is reported (not a failure); a policy not supported on Linux is a failure.

  python3 -I policies.py check --source FILE... --defs DIR|TSV [--json OUT]
  python3 -I policies.py dump --defs DIR > atlas-policies.tsv      (the table AT-1 needs)
  python3 -I policies.py --selftest
Exit: 0 pass, 1 a key failed, 2 the inputs could not be read.
"""
import argparse
import json
import os
import re
import sys
import tempfile

TYPES = {"main": "bool", "int": "int", "int-enum": "int", "string": "string", "string-enum": "string",
         "list": "list", "dict": "dict", "external": "dict", "string-enum-list": "list"}


# ---------------------------------------------------------------------------------------------
# The engine's definitions

def parse_yaml_policy(text):
    """The few fields we need of one policy definition (flat reader for the engine's YAML layout:
    top-level `type:`, `deprecated:`, a `features:` block, `items:` entries with `value:`,
    `supported_on:` entries)."""
    out = {"type": None, "ints": [], "recommendable": False, "deprecated": False, "supported_on": []}
    block, item_open = None, False
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith((" ", "-")):
            m = re.match(r"([A-Za-z_]+):\s*(.*)$", line)
            block = m.group(1) if m else None
            if m and block == "type":
                out["type"] = m.group(2).strip().strip("'\"")
            if m and block == "deprecated":
                out["deprecated"] = m.group(2).strip() == "true"
            if m and block == "items":
                pass
            continue
        if block == "features":
            m = re.match(r"\s+can_be_recommended:\s*(\w+)", line)
            if m:
                out["recommendable"] = m.group(1) == "true"
        elif block == "items":
            m = re.match(r"(?:- |\s+)value:\s*(-?\d+)\s*$", line)
            if m and not line.startswith("    "):
                out["ints"].append(int(m.group(1)))
        elif block == "supported_on":
            m = re.match(r"-\s*([^\s#]+)", line)
            if m:
                out["supported_on"].append(m.group(1))
    return out


def supported_on_linux(entries):
    """True when an entry names Linux desktop (`chrome.*` or `chrome.linux`) and is still open ended
    (`chrome.*:38-` supports it, `chrome.*:38-52` removed it). No entries: not judged."""
    if not entries:
        return True
    for e in entries:
        m = re.fullmatch(r"(chrome\.\*|chrome\.linux)(?::(\d+)-(\d*))?", e)
        if m and not m.group(3):
            return True
    return False


def load_defs_dir(path):
    defs = {}
    for root, _dirs, files in os.walk(path):
        for f in files:
            if not f.endswith(".yaml") or f.startswith("."):
                continue
            with open(os.path.join(root, f), encoding="utf-8") as fh:
                d = parse_yaml_policy(fh.read())
            if d["type"] is None:
                continue  # a group description or a template, not a policy
            d["kind"] = TYPES.get(d["type"], d["type"])
            d["linux"] = supported_on_linux(d["supported_on"])
            defs[f[:-5]] = d
    return defs


def load_defs_tsv(path):
    defs = {}
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            if line.startswith("#") or not line.strip():
                continue
            cols = line.rstrip("\n").split("\t") + ["", "", "", ""]
            name, kind, ints, rec = cols[:4]
            defs[name] = {"type": kind, "kind": kind, "ints": [int(i) for i in ints.split(",") if i],
                          "recommendable": rec == "recommendable", "deprecated": False, "supported_on": [],
                          "linux": True}
    return defs


def load_defs(path):
    if os.path.isdir(path):
        return load_defs_dir(path)
    return load_defs_tsv(path)


def dump_tsv(defs):
    lines = ["# name\ttype\tallowed ints\trecommendable (generated by policies.py dump)"]
    for n in sorted(defs):
        d = defs[n]
        lines.append(f"{n}\t{d['kind']}\t{','.join(map(str, d['ints']))}\t{'recommendable' if d['recommendable'] else ''}")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------------------------
# What WolOS writes

def rust_fn_body(src, name):
    m = re.search(r"\bfn\s+" + re.escape(name) + r"\s*\([^)]*\)[^{]*\{", src)
    if not m:
        return None
    depth, i = 1, m.end()
    while i < len(src) and depth:
        c = src[i]
        if c == '"':
            i += 1
            while i < len(src) and src[i] != '"':
                i += 2 if src[i] == "\\" else 1
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        i += 1
    return src[m.end():i - 1]


def rust_json_pairs(body):
    """(key, value) of the top-level `"Key": value` pairs of the first json!({ ... }) in BODY."""
    m = re.search(r"json!\s*\(\s*\{", body)
    if not m:
        return []
    depth, i, start = 1, m.end(), m.end()
    while i < len(body) and depth:
        c = body[i]
        if c == '"':
            i += 1
            while i < len(body) and body[i] != '"':
                i += 2 if body[i] == "\\" else 1
        elif c in "{[":
            depth += 1
        elif c in "}]":
            depth -= 1
        i += 1
    inner = body[start:i - 1]
    pairs, depth, cur = [], 0, ""
    for ch in inner:
        if ch in "{[":
            depth += 1
        elif ch in "}]":
            depth -= 1
        if ch == "," and depth == 0:
            pairs.append(cur)
            cur = ""
        else:
            cur += ch
    pairs.append(cur)
    out = []
    for p in pairs:
        m = re.match(r'\s*"([A-Za-z0-9_]+)"\s*:\s*(.+?)\s*$', p, re.S)
        if m:
            out.append((m.group(1), parse_rust_value(m.group(2))))
    return out


def parse_rust_value(v):
    v = v.strip()
    if v in ("true", "false"):
        return v == "true"
    if re.fullmatch(r"-?\d+", v):
        return int(v)
    if re.fullmatch(r'"[^"]*"', v):
        return v[1:-1]
    if v.startswith("["):
        return []
    if v.startswith("{"):
        return {}
    return None  # an expression: type unknown, only the name is checked


def source_keys(path, section=None):
    """List of (section, key, value) a source writes."""
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    if path.endswith(".rs"):
        out = []
        for fn, sec in (("policies", "managed"), ("recommended_policies", "recommended")):
            body = rust_fn_body(text, fn)
            if body is None:
                continue
            out += [(sec, k, v) for k, v in rust_json_pairs(body)]
        if not out:
            raise ValueError(f"{path}: no policies()/recommended_policies() with a json!({{...}}) block found")
        return out
    data = json.loads(text)
    if not isinstance(data, dict):
        raise ValueError(f"{path}: not a JSON object")
    sec = section or ("recommended" if "recommended" in os.path.dirname(path).split(os.sep)[-1:] else "managed")
    return [(sec, k, v) for k, v in data.items()]


# ---------------------------------------------------------------------------------------------

def kind_of(v):
    if v is None:
        return None
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, int):
        return "int"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "list"
    return "dict"


def judge(entries, defs):
    rows, fails, notes = [], [], []
    for sec, key, val in entries:
        d = defs.get(key)
        row = {"section": sec, "key": key, "value": val, "state": "ok", "why": ""}
        if d is None:
            row["state"], row["why"] = "UNKNOWN", "the engine defines no policy with this name (ignored by the browser)"
        else:
            want, got = d["kind"], kind_of(val)
            if got is not None and want != got:
                row["state"], row["why"] = "WRONG TYPE", f"engine type {d['type']} ({want}), WolOS writes {got}"
            elif d["ints"] and got == "int" and val not in d["ints"]:
                row["state"], row["why"] = "WRONG VALUE", f"allowed {d['ints']}"
            elif sec == "recommended" and not d["recommendable"]:
                row["state"], row["why"] = "NOT RECOMMENDABLE", "the engine does not allow this policy as a recommendation"
            elif not d["linux"]:
                row["state"], row["why"] = "NOT ON LINUX", f"supported_on {d['supported_on']}"
            elif d["deprecated"]:
                notes.append(f"{key} is deprecated in the engine")
        if row["state"] != "ok":
            fails.append(f"{sec}/{key}: {row['state']}: {row['why']}")
        rows.append(row)
    return rows, fails, notes


def check(a):
    try:
        defs = load_defs(a.defs)
    except OSError as e:
        print(f"policies: cannot read definitions: {e}", file=sys.stderr)
        return 2
    if not defs:
        print(f"policies: no policy definitions found in {a.defs}", file=sys.stderr)
        return 2
    entries = []
    for s in a.source:
        try:
            entries += source_keys(s, a.section)
        except (OSError, ValueError) as e:
            print(f"policies: cannot read {s}: {e}", file=sys.stderr)
            return 2
    if not entries:
        print("policies: the sources list no policy at all (an empty check proves nothing)", file=sys.stderr)
        return 2
    rows, fails, notes = judge(entries, defs)
    verdict = "FAIL" if fails else "PASS"
    print(f"POLICIES {verdict}: {len(rows) - len(fails)} of {len(rows)} keys ok against {len(defs)} engine definitions")
    for r in rows:
        print(f"  {r['state']:17} {r['section']}/{r['key']} = {r['value']!r} {r['why']}")
    for n in notes:
        print(f"  note: {n}")
    if a.json:
        with open(a.json, "w", encoding="utf-8") as fh:
            json.dump({"tool": "policies", "definitions": len(defs), "rows": rows, "notes": notes,
                       "verdict": verdict}, fh, indent=2)
            fh.write("\n")
    return 1 if fails else 0


# ---------------------------------------------------------------------------------------------

YAML_RECOMMENDABLE_INT = """caption: Enable network prediction
default: null
features:
  can_be_recommended: true
  dynamic_refresh: true
items:
- caption: any
  name: NetworkPredictionAlways
  value: 0
- caption: |-
    never
              (Deprecated in 50.)
  name: NetworkPredictionNever
  value: 2
owners:
- file://components/policy/OWNERS
schema:
  enum:
  - 0
  - 2
  type: integer
supported_on:
- chrome.*:38-
tags: []
type: int-enum
"""
YAML_BOOL = """caption: Metrics
features:
  can_be_recommended: false
schema:
  type: boolean
supported_on:
- chrome.*:1-
type: main
"""
YAML_WINDOWS_ONLY = """caption: Win
supported_on:
- chrome.win:1-
type: main
"""
RUST_SAMPLE = '''
pub fn policies() -> String {
    let mut v = json!({
        "MetricsReportingEnabled": false,
        "BrowserSignin": 3,
        "Misspelt": true,
        "WinOnly": true
    });
    v
}
pub fn recommended_policies() -> String {
    let v = json!({
        "NetworkPredictionOptions": 2,
        "MetricsReportingEnabled": false
    });
    v
}
'''


def selftest():
    fails = []

    def check_(ok, what):
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        if not ok:
            fails.append(what)

    print("policies selftest")
    d = parse_yaml_policy(YAML_RECOMMENDABLE_INT)
    check_(d["type"] == "int-enum" and d["ints"] == [0, 2], f"enum values read from items only (got {d['ints']})")
    check_(d["recommendable"] is True, "can_be_recommended true")
    check_(parse_yaml_policy(YAML_BOOL)["recommendable"] is False, "can_be_recommended false")
    with tempfile.TemporaryDirectory(prefix="policies-selftest-") as t:
        pol = os.path.join(t, "defs", "Misc")
        os.makedirs(pol)
        for n, y in (("NetworkPredictionOptions", YAML_RECOMMENDABLE_INT), ("MetricsReportingEnabled", YAML_BOOL),
                     ("BrowserSignin", YAML_RECOMMENDABLE_INT.replace("type: int-enum", "type: int-enum")),
                     ("WinOnly", YAML_WINDOWS_ONLY)):
            with open(os.path.join(pol, n + ".yaml"), "w") as fh:
                fh.write(y)
        with open(os.path.join(pol, ".group.details.yaml"), "w") as fh:
            fh.write("name: Misc\n")
        defs = load_defs(os.path.join(t, "defs"))
        check_(sorted(defs) == ["BrowserSignin", "MetricsReportingEnabled", "NetworkPredictionOptions", "WinOnly"],
               f"definitions found, group file skipped ({sorted(defs)})")
        src = os.path.join(t, "browser.rs")
        with open(src, "w") as fh:
            fh.write(RUST_SAMPLE)
        entries = source_keys(src)
        check_(len(entries) == 6 and entries[0] == ("managed", "MetricsReportingEnabled", False),
               f"Rust json! keys of both functions read ({len(entries)})")
        rows, bad, _ = judge(entries, defs)
        states = {(r["section"], r["key"]): r["state"] for r in rows}
        check_(states[("managed", "Misspelt")] == "UNKNOWN", "a misspelt key is named")
        check_(states[("managed", "WinOnly")] == "NOT ON LINUX", "a policy not on Linux fails")
        check_(states[("managed", "BrowserSignin")] == "WRONG VALUE", "an int outside the enum fails")
        check_(states[("recommended", "MetricsReportingEnabled")] == "NOT RECOMMENDABLE", "recommending a mandatory-only key fails")
        check_(states[("managed", "MetricsReportingEnabled")] == "ok" and states[("recommended", "NetworkPredictionOptions")] == "ok",
               "valid keys pass")
        pj = os.path.join(t, "managed")
        os.makedirs(pj)
        with open(os.path.join(pj, "wolos.json"), "w") as fh:
            json.dump({"MetricsReportingEnabled": "no"}, fh)
        rows, bad, _ = judge(source_keys(os.path.join(pj, "wolos.json")), defs)
        check_(rows[0]["state"] == "WRONG TYPE", "an installed JSON file is checked, wrong type named")
        tsv = os.path.join(t, "t.tsv")
        with open(tsv, "w") as fh:
            fh.write(dump_tsv(defs))
        again = load_defs_tsv(tsv)
        check_({k: (v["kind"], v["ints"], v["recommendable"]) for k, v in again.items()} ==
               {k: (v["kind"], v["ints"], v["recommendable"]) for k, v in defs.items()}, "dump and reload keep the columns")
    print(f"policies selftest: {'PASS' if not fails else 'FAIL'} ({len(fails)} failed)")
    return 0 if not fails else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--selftest", action="store_true")
    sub = ap.add_subparsers(dest="cmd")
    c = sub.add_parser("check")
    c.add_argument("--source", nargs="+", required=True, help="browser.rs or installed policy JSON files")
    c.add_argument("--defs", required=True, help="policy_definitions directory or the name/type/ints/recommendable table")
    c.add_argument("--section", choices=("managed", "recommended"), help="section of JSON sources (default: from the folder name)")
    c.add_argument("--json", help="write the report here")
    d = sub.add_parser("dump")
    d.add_argument("--defs", required=True)
    a = ap.parse_args()
    if a.selftest:
        return selftest()
    if a.cmd == "check":
        return check(a)
    if a.cmd == "dump":
        sys.stdout.write(dump_tsv(load_defs(a.defs)))
        return 0
    ap.print_help()
    return 2


if __name__ == "__main__":
    sys.exit(main())
