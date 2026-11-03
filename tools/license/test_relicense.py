# SPDX-License-Identifier: GPL-2.0-or-later
"""python3 -m unittest discover -s tools/license   (needs git)"""
import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

import relicense as R

POLICY = json.loads((Path(R.HERE) / "policy.json").read_text())
G = "GPL-2.0-or-later"
EXC = "GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception"
BSD = "BSD-2-Clause"
BRAND = "LicenseRef-Wonderland-Brand-1.0"
README_OLD = "**License:** BSD-2-Clause; the shell is GPL-3.0-or-later; brand art is CC BY-NC-ND 4.0 (share it to talk about WolOS; ask for anything else)."

FILES = {
    "crates/api/src/lib.rs": "//! Core.\npub fn f() {}\n",
    "crates/api/src/old.rs": "// SPDX-License-Identifier: BSD-2-Clause\npub fn g() {}\n",
    "crates/shell/src/main.rs": "// SPDX-License-Identifier: GPL-3.0-or-later\nfn main() {}\n",
    "crates/wol-abi/include/wol.h": "/* SPDX-License-Identifier: BSD-2-Clause */\n#pragma once\n",
    "crates/wol-abi/src/lib.rs": "//! libwol\n",
    "crates/wol-abi/tests/new.rs": "// SPDX-License-Identifier: GPL-2.0-or-later\nfn t() {}\n",
    "crates/api/ok.c": f"/* SPDX-License-Identifier: {G} */\nint x;\n",
    "wolos-iso/build.sh": "#!/bin/sh\nset -e\n",
    "image/old.sh": "#!/bin/sh\n# SPDX-License-Identifier: BSD-2-Clause\nset -e\n",
    "tools/win.py": "import os\r\nprint(1)\r\n",
    "crates/api/data.json": "{}\n",
    "crates/api/notes.md": "# notes\n",
    "crates/api/etc.conf": "# SPDX-License-Identifier: BSD-2-Clause\nkey=1\n",
    "third_party/blocking/src/lib.rs": "// SPDX-License-Identifier: Apache-2.0 OR MIT\n",
    "third_party/blocking/src/other.rs": "fn x() {}\n",
    "wolos-pkgbuilds/hyprland-wolos/patches/0001.patch": "+// SPDX-License-Identifier: BSD-3-Clause\n",
    "crates/api/src/vendored.rs": "// SPDX-License-Identifier: BSD-3-Clause\n// Copyright (c) someone else\n",
    "design/wallpapers/x.svg": "<svg/>\n",
    "design/tools/gen.py": "# SPDX-License-Identifier: BSD-2-Clause\nprint(1)\n",
    "Cargo.toml": '[workspace.package]\nlicense = "BSD-2-Clause"\n',
    "crates/shell/Cargo.toml": '[package]\nlicense = "GPL-3.0-or-later"\n',
    "third_party/blocking/Cargo.toml": 'license = "Apache-2.0 OR MIT"\n',
    "wolos-pkgbuilds/wolos-system/PKGBUILD": "# x\nlicense=(BSD-2-Clause GPL-3.0-or-later CC-BY-NC-ND-4.0)\n",
    "wolos-pkgbuilds/hyprland-wolos/PKGBUILD": "# x\nlicense=(BSD-3-Clause)\n",
    "wopkg/shard.yml": "name: wopkg\nlicense: BSD-2-Clause\n",
    "wopkg/recipes/bend/recipe.toml": 'license = "Apache-2.0"\n',
    "README.md": README_OLD + " See COPYRIGHT.\n",
    "crates/wol-abi/src/old.rs": f"// SPDX-License-Identifier: {EXC}\n",
    "crates/shared/src/lib.rs": "//! Shared.\n",
    "crates/core/src/lib.rs": f"// SPDX-License-Identifier: {G}\npub fn c() {{}}\n",
    "crates/core/Cargo.toml": '[package]\nlicense.workspace = true\n',
    "crates/shared/Cargo.toml": '[package]\nlicense.workspace = true\n',
    "crates/wol-abi/Cargo.toml": '[package]\nlicense.workspace = true\n',
    "docs/adr/0020-libwol-stable-abi.md": "libwol and `wol.h` are BSD-2-Clause, usable by apps of any license. (`shared::view`, BSD)\n",
    "design/LICENSE": "Licensor: Julia Klee and her company (with affiliates and successors)\n",
    "assets/LICENSE": "CC\n",
    "LICENSES/MIT.txt": "mit\n",
    "tools/license/staged/LICENSE.bsd": "BSD text\n",
    "tools/license/staged/LICENSE.assets": "brand pointer\n",
    "LICENSE": "BSD 2-Clause License\n",
    "COPYRIGHT": "old\n",
    "crates/shell/LICENSE": "GPLv3\n",
    "tools/license/staged/LICENSE": "GPL text\n",
    "tools/license/staged/COPYRIGHT": "new table\n",
    "tools/license/staged/LICENSE.shell": "shell GPL text\n",
}


def make(tmp, extra=None):
    root = Path(tmp)
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    for rel, body in {**FILES, **(extra or {})}.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(body.encode())
    subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
    return root


def snapshot(root):
    return {str(p.relative_to(root)): p.read_bytes() for p in Path(root).rglob("*")
            if p.is_file() and ".git" not in p.parts}


class Relicense(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = make(self.tmp.name)
        self.pol = R.Policy(POLICY)

    def tearDown(self):
        self.tmp.cleanup()

    def read(self, rel):
        return (self.root / rel).read_bytes().decode()

    def kinds(self, issues):
        return {(i[0], i[1]) for i in issues}

    def test_check_reports_missing_wrong_foreign_and_stale(self):
        issues, changes = R.run(self.root, self.pol, apply=False)
        k = self.kinds(issues)
        self.assertIn(("crates/api/src/lib.rs", "missing"), k)
        self.assertIn(("crates/api/src/old.rs", "wrong"), k)
        self.assertIn(("crates/shell/src/main.rs", "wrong"), k)
        self.assertIn(("crates/api/src/vendored.rs", "foreign"), k)
        self.assertIn(("crates/api/etc.conf", "wrong"), k)
        self.assertIn(("LICENSE", "stale"), k)
        self.assertIn(("README.md", "text"), k)

    def test_check_does_not_write(self):
        before = snapshot(self.root)
        R.run(self.root, self.pol, apply=False)
        self.assertEqual(before, snapshot(self.root))

    def test_apply_then_check_is_clean_except_foreign(self):
        R.run(self.root, self.pol, apply=True)
        issues, _ = R.run(self.root, self.pol, apply=False)
        self.assertEqual(self.kinds(issues), {("crates/api/src/vendored.rs", "foreign")})

    def test_apply_is_idempotent(self):
        R.run(self.root, self.pol, apply=True)
        once = snapshot(self.root)
        _, changes = R.run(self.root, self.pol, apply=True)
        self.assertEqual(changes, [])
        self.assertEqual(once, snapshot(self.root))

    def test_headers_by_style_and_shebang(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("crates/api/src/lib.rs"), f"// SPDX-License-Identifier: {G}\n//! Core.\npub fn f() {{}}\n")
        self.assertEqual(self.read("wolos-iso/build.sh"), f"#!/bin/sh\n# SPDX-License-Identifier: {G}\nset -e\n")
        self.assertEqual(self.read("image/old.sh"), f"#!/bin/sh\n# SPDX-License-Identifier: {G}\nset -e\n")
        self.assertEqual(self.read("crates/api/etc.conf"), f"# SPDX-License-Identifier: {G}\nkey=1\n")

    def test_extensionless_ruby_scripts_get_a_hash_header(self):
        pol = R.Policy(json.loads((Path(R.HERE) / "policy.json").read_text()))
        self.assertEqual(pol.style("desktop/overlay/usr/bin/wol-x", "#!/usr/bin/ruby\nputs 1\n"), "#")
        self.assertEqual(pol.style("tools/x", "#!/usr/bin/env ruby\nputs 1\n"), "#")
        self.assertIsNone(pol.style("tools/x", "#!/usr/bin/node\n"))

    def test_crlf_is_kept(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("tools/win.py"), f"# SPDX-License-Identifier: {G}\r\nimport os\r\nprint(1)\r\n")

    def test_libwol_and_what_it_links_are_bsd(self):
        R.run(self.root, self.pol, apply=True)
        for rel in ("crates/wol-abi/src/lib.rs", "crates/wol-abi/src/old.rs", "crates/wol-abi/tests/new.rs",
                    "crates/shared/src/lib.rs", "crates/core/src/lib.rs"):
            self.assertIn(f"SPDX-License-Identifier: {BSD}", self.read(rel), rel)
        self.assertIn(G, self.read("crates/api/src/lib.rs"))
        self.assertEqual(self.read("crates/wol-abi/include/wol.h"), f"/* SPDX-License-Identifier: {BSD} */\n#pragma once\n")
        for c in ("core", "shared", "wol-abi"):
            self.assertIn(f'license = "{BSD}"', self.read(f"crates/{c}/Cargo.toml"), c)

    def test_adr_keeps_saying_bsd(self):
        before = self.read("docs/adr/0020-libwol-stable-abi.md")
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("docs/adr/0020-libwol-stable-abi.md"), before)

    def test_brand_files_stay_as_they_are(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn("her company", self.read("design/LICENSE"))
        self.assertEqual(self.read("assets/LICENSE"), "CC\n")

    def test_staged_copyright_matches_the_decisions(self):
        text = (Path(R.HERE) / "staged" / "COPYRIGHT").read_text()
        self.assertNotIn("exception", text)
        self.assertIn("CC BY-NC-ND", text)
        self.assertIn("`crates/core/`", text)
        self.assertIn("`kernel/scx_wolos/`", text)
        self.assertIn("GPL-2.0-only", text)

    def test_mit_portions_are_expected_and_named(self):
        # docs/THIRD-PARTY.md lists the files that carry MIT portions; the policy expects MIT
        # there (and nowhere else), and COPYRIGHT names each of them.
        repo = Path(R.HERE).parent.parent
        listed = set()
        for line in (repo / "docs" / "THIRD-PARTY.md").read_text(encoding="utf-8").splitlines():
            if line.startswith("Our files with portions:"):
                listed.update(re.findall(r"`([^`]+)`", line))
        mit = {e["glob"] for e in POLICY["expected"] if e["spdx"] == "MIT"}
        self.assertEqual(listed, mit)
        pol = R.Policy(POLICY)
        for f in listed:
            self.assertEqual(pol.expected(f), "MIT", f)
        named = set()
        for item in re.findall(r"`([^`]+)`", (Path(R.HERE) / "staged" / "COPYRIGHT").read_text()):
            m = re.fullmatch(r"(.*)\{([^}]*)\}(.*)", item)
            named.update(m.group(1) + x + m.group(3) for x in m.group(2).split(",")) if m else named.add(item)
        self.assertEqual(listed - named, set())

    def test_a_copy_without_git_is_walked_like_the_index(self):
        # The build box gets the tree without .git: the walk finds the same files as
        # `git ls-files`, minus build output.
        tracked = R.files(self.root)
        with tempfile.TemporaryDirectory() as t:
            copy = Path(t)
            for rel in tracked:
                (copy / rel).parent.mkdir(parents=True, exist_ok=True)
                (copy / rel).write_bytes((self.root / rel).read_bytes())
            for junk in ["target/debug/x.rs", "crates/api/target/y.rs", "node_modules/m/i.js",
                         "tools/__pycache__/r.pyc"]:
                (copy / junk).parent.mkdir(parents=True, exist_ok=True)
                (copy / junk).write_text("fn x() {}\n")
            self.assertEqual(R.files(copy), tracked)
            self.assertEqual(R.run(copy, self.pol, apply=False)[0],
                             R.run(self.root, self.pol, apply=False)[0])

    def test_generated_theme_files_are_skipped(self):
        # `wol theme` writes these (crates/core/src/theme); a header would be overwritten.
        pol = R.Policy(POLICY)
        self.assertTrue(pol.skipped("desktop/overlay/usr/share/wolos/toolkits/bash/colors.sh"))
        self.assertFalse(pol.skipped("desktop/overlay/etc/profile.d/wolos-console.sh"))

    def test_third_party_patches_art_and_data_are_untouched(self):
        before = snapshot(self.root)
        R.run(self.root, self.pol, apply=True)
        after = snapshot(self.root)
        for rel in ["third_party/blocking/src/lib.rs", "third_party/blocking/src/other.rs",
                    "third_party/blocking/Cargo.toml", "wolos-pkgbuilds/hyprland-wolos/patches/0001.patch",
                    "design/wallpapers/x.svg", "crates/api/data.json", "crates/api/notes.md",
                    "wopkg/recipes/bend/recipe.toml",
                    "crates/api/src/vendored.rs"]:
            self.assertEqual(before[rel], after[rel], rel)

    def test_packaged_software_license_field_is_kept(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn("license=(BSD-3-Clause)", self.read("wolos-pkgbuilds/hyprland-wolos/PKGBUILD"))

    def test_design_tools_move_but_art_does_not(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn(G, self.read("design/tools/gen.py"))

    def test_fields(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn(f'license = "{G}"', self.read("Cargo.toml"))
        self.assertIn(f'license = "{G}"', self.read("crates/shell/Cargo.toml"))
        self.assertNotIn(BRAND, self.read("wolos-pkgbuilds/wolos-system/PKGBUILD"))
        self.assertIn(f"license: {G}", self.read("wopkg/shard.yml"))

    def test_docs_and_staged_copies(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("README.md"), f"**License:** {G}; libwol (`crates/wol-abi`, `crates/shared`, `crates/core`) is {BSD}; kernel patches are GPL-2.0-only; brand art is CC BY-NC-ND 4.0 (share it to talk about WolOS; ask for anything else). See COPYRIGHT.\n")
        self.assertEqual(self.read("LICENSE"), "GPL text\n")
        self.assertEqual(self.read("COPYRIGHT"), "new table\n")
        self.assertEqual(self.read("crates/shell/LICENSE"), "shell GPL text\n")

    def test_dry_run_changes_nothing(self):
        before = snapshot(self.root)
        _, changes = R.run(self.root, self.pol, apply=True, dry=True)
        self.assertTrue(changes)
        self.assertEqual(before, snapshot(self.root))

    def test_accepted_value_is_kept(self):
        pol = R.Policy({**POLICY, "accept": [{"glob": "crates/api/src/old.rs", "spdx": ["BSD-2-Clause"]}]})
        R.run(self.root, pol, apply=True)
        self.assertIn("BSD-2-Clause", self.read("crates/api/src/old.rs"))


if __name__ == "__main__":
    unittest.main()
