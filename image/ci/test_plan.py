# SPDX-License-Identifier: BSD-2-Clause
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import plan  # noqa: E402

LIST = """# comment
linux-wolos    kernel/linux-wolos          -                 kernel/
hyprland-wolos image/repo/hyprland-wolos   -                 image/
wolos-system   image/repo/wolos-system     hyprland-wolos    crates/
wolos-base     image/repo/wolos-base       linux-wolos,hyprland-wolos,wolos-system  image/
"""


class Plan(unittest.TestCase):
    def test_independent_packages_build_together(self):
        p = plan.parse(LIST)
        self.assertEqual(
            plan.waves(p, list(p)),
            [["hyprland-wolos", "linux-wolos"], ["wolos-system"], ["wolos-base"]],
        )

    def test_published_deps_take_no_wave(self):
        p = plan.parse(LIST)
        self.assertEqual(plan.waves(p, ["wolos-base", "wolos-system"]), [["wolos-system"], ["wolos-base"]])
        self.assertEqual(plan.waves(p, ["wolos-base"]), [["wolos-base"]])

    def test_unknown_names_are_ignored(self):
        self.assertEqual(plan.waves(plan.parse(LIST), ["nope"]), [])

    def test_cycle_fails(self):
        p = plan.parse("a d - x\nb d a x\n")
        p["a"]["deps"] = ["b"]
        with self.assertRaises(ValueError):
            plan.waves(p, ["a", "b"])

    def test_kernel_gets_a_whole_host(self):
        self.assertEqual(max(plan.SIZES.values()), plan.size("linux-wolos"))


if __name__ == "__main__":
    unittest.main()
