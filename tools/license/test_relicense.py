# SPDX-License-Identifier: GPL-2.0-or-later
"""python3 -m unittest discover -s tools/license   (needs git)"""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import relicense as R

POLICY = json.loads((Path(R.HERE) / "policy.json").read_text())
G = "GPL-2.0-or-later"
EXC = "GPL-2.0-or-later WITH AdditionRef-WolOS-libwol-exception"

FILES = {
    "crates/core/src/lib.rs": "//! Core.\npub fn f() {}\n",
    "crates/core/src/old.rs": "// SPDX-License-Identifier: BSD-2-Clause\npub fn g() {}\n",
    "crates/shell/src/main.rs": "// SPDX-License-Identifier: GPL-3.0-or-later\nfn main() {}\n",
    "crates/wol-abi/include/wol.h": "/* SPDX-License-Identifier: BSD-2-Clause */\n#pragma once\n",
    "crates/wol-abi/src/lib.rs": "//! libwol\n",
    "crates/wol-abi/tests/new.rs": "// SPDX-License-Identifier: GPL-2.0-or-later\nfn t() {}\n",
    "crates/core/ok.c": f"/* SPDX-License-Identifier: {G} */\nint x;\n",
    "image/build.sh": "#!/bin/sh\nset -e\n",
    "image/old.sh": "#!/bin/sh\n# SPDX-License-Identifier: BSD-2-Clause\nset -e\n",
    "tools/win.py": "import os\r\nprint(1)\r\n",
    "crates/core/data.json": "{}\n",
    "crates/core/notes.md": "# notes\n",
    "crates/core/etc.conf": "# SPDX-License-Identifier: BSD-2-Clause\nkey=1\n",
    "third_party/blocking/src/lib.rs": "// SPDX-License-Identifier: Apache-2.0 OR MIT\n",
    "third_party/blocking/src/other.rs": "fn x() {}\n",
    "image/hyprland/patches/0001.patch": "+// SPDX-License-Identifier: BSD-3-Clause\n",
    "crates/core/src/vendored.rs": "// SPDX-License-Identifier: BSD-3-Clause\n// Copyright (c) someone else\n",
    "design/wallpapers/x.svg": "<svg/>\n",
    "design/tools/gen.py": "# SPDX-License-Identifier: BSD-2-Clause\nprint(1)\n",
    "Cargo.toml": '[workspace.package]\nlicense = "BSD-2-Clause"\n',
    "crates/shell/Cargo.toml": '[package]\nlicense = "GPL-3.0-or-later"\n',
    "third_party/blocking/Cargo.toml": 'license = "Apache-2.0 OR MIT"\n',
    "image/repo/wolos-system/PKGBUILD": "# x\nlicense=(BSD-2-Clause GPL-3.0-or-later CC-BY-NC-ND-4.0)\n",
    "image/repo/hyprland-wolos/PKGBUILD": "# x\nlicense=(BSD-3-Clause)\n",
    "wopkg/shard.yml": "name: wopkg\nlicense: BSD-2-Clause\n",
    "wopkg/recipes/bend/recipe.toml": 'license = "Apache-2.0"\n',
    "README.md": "**License:** BSD-2-Clause; the shell is GPL-3.0-or-later; brand art is CC BY-NC-ND 4.0 (x).\n",
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
        self.assertIn(("crates/core/src/lib.rs", "missing"), k)
        self.assertIn(("crates/core/src/old.rs", "wrong"), k)
        self.assertIn(("crates/shell/src/main.rs", "wrong"), k)
        self.assertIn(("crates/core/src/vendored.rs", "foreign"), k)
        self.assertIn(("crates/core/etc.conf", "wrong"), k)
        self.assertIn(("LICENSE", "stale"), k)
        self.assertIn(("README.md", "text"), k)

    def test_check_does_not_write(self):
        before = snapshot(self.root)
        R.run(self.root, self.pol, apply=False)
        self.assertEqual(before, snapshot(self.root))

    def test_apply_then_check_is_clean_except_foreign(self):
        R.run(self.root, self.pol, apply=True)
        issues, _ = R.run(self.root, self.pol, apply=False)
        self.assertEqual(self.kinds(issues), {("crates/core/src/vendored.rs", "foreign")})

    def test_apply_is_idempotent(self):
        R.run(self.root, self.pol, apply=True)
        once = snapshot(self.root)
        _, changes = R.run(self.root, self.pol, apply=True)
        self.assertEqual(changes, [])
        self.assertEqual(once, snapshot(self.root))

    def test_headers_by_style_and_shebang(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("crates/core/src/lib.rs"), f"// SPDX-License-Identifier: {G}\n//! Core.\npub fn f() {{}}\n")
        self.assertEqual(self.read("image/build.sh"), f"#!/bin/sh\n# SPDX-License-Identifier: {G}\nset -e\n")
        self.assertEqual(self.read("image/old.sh"), f"#!/bin/sh\n# SPDX-License-Identifier: {G}\nset -e\n")
        self.assertEqual(self.read("crates/wol-abi/include/wol.h"), f"/* SPDX-License-Identifier: {EXC} */\n#pragma once\n")
        self.assertEqual(self.read("crates/core/etc.conf"), f"# SPDX-License-Identifier: {G}\nkey=1\n")

    def test_crlf_is_kept(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("tools/win.py"), f"# SPDX-License-Identifier: {G}\r\nimport os\r\nprint(1)\r\n")

    def test_interface_exception_applies_to_libwol_and_shared(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn(EXC, self.read("crates/wol-abi/src/lib.rs"))
        self.assertNotIn("WITH", self.read("crates/core/src/lib.rs"))
        self.assertIn(EXC, self.read("crates/wol-abi/tests/new.rs"))

    def test_third_party_patches_art_and_data_are_untouched(self):
        before = snapshot(self.root)
        R.run(self.root, self.pol, apply=True)
        after = snapshot(self.root)
        for rel in ["third_party/blocking/src/lib.rs", "third_party/blocking/src/other.rs",
                    "third_party/blocking/Cargo.toml", "image/hyprland/patches/0001.patch",
                    "design/wallpapers/x.svg", "crates/core/data.json", "crates/core/notes.md",
                    "wopkg/recipes/bend/recipe.toml",
                    "crates/core/src/vendored.rs"]:
            self.assertEqual(before[rel], after[rel], rel)

    def test_packaged_software_license_field_is_kept(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn("license=(BSD-3-Clause)", self.read("image/repo/hyprland-wolos/PKGBUILD"))

    def test_design_tools_move_but_art_does_not(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn(G, self.read("design/tools/gen.py"))

    def test_fields(self):
        R.run(self.root, self.pol, apply=True)
        self.assertIn(f'license = "{G}"', self.read("Cargo.toml"))
        self.assertIn(f'license = "{G}"', self.read("crates/shell/Cargo.toml"))
        self.assertIn(f"license=({G} CC-BY-NC-ND-4.0)", self.read("image/repo/wolos-system/PKGBUILD"))
        self.assertIn(f"license: {G}", self.read("wopkg/shard.yml"))

    def test_docs_and_staged_copies(self):
        R.run(self.root, self.pol, apply=True)
        self.assertEqual(self.read("README.md"), f"**License:** {G}; brand art is CC BY-NC-ND 4.0 (x).\n")
        self.assertEqual(self.read("LICENSE"), "GPL text\n")
        self.assertEqual(self.read("COPYRIGHT"), "new table\n")
        self.assertEqual(self.read("crates/shell/LICENSE"), "shell GPL text\n")

    def test_dry_run_changes_nothing(self):
        before = snapshot(self.root)
        _, changes = R.run(self.root, self.pol, apply=True, dry=True)
        self.assertTrue(changes)
        self.assertEqual(before, snapshot(self.root))

    def test_accepted_value_is_kept(self):
        pol = R.Policy({**POLICY, "accept": [{"glob": "crates/core/src/old.rs", "spdx": ["BSD-2-Clause"]}]})
        R.run(self.root, pol, apply=True)
        self.assertIn("BSD-2-Clause", self.read("crates/core/src/old.rs"))


if __name__ == "__main__":
    unittest.main()
