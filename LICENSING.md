# Licensing

WolOS. Copyright (c) 2026 Julia Klee.
Each file's `SPDX-License-Identifier` line, where present, decides; otherwise the table below.

| Path | License | Text |
|---|---|---|
| everything not listed below (system API, daemons, CLI, image, desktop, build scripts, `kernel/*.sh`, `kernel/conf/`) | BSD-2-Clause | [`LICENSE`](LICENSE) |
| `crates/shell/` (menu bar, Dock, Control Center, Settings, Kit) | GPL-3.0-or-later | [`crates/shell/LICENSE`](crates/shell/LICENSE) |
| `kernel/patches/` (with `tests/`), `kernel/drm/patches/`, `kernel/wolos_qos/`, `kernel/verify.sh`, `kernel/drm/verify.sh`, `kernel/tests/` | Wonderland License 1.0 (`LicenseRef-Wonderland-1.0`), source-available | [`LICENSES/Wonderland-1.0.md`](LICENSES/Wonderland-1.0.md), [`kernel/LICENSE`](kernel/LICENSE) |
| `design/` (submodule, wonderlandos/wolos-brand), except `design/tools/` and `design/icons/store/` | Wonderland Brand License 1.0 (`LicenseRef-Wonderland-Brand-1.0`) | [`LICENSES/Wonderland-Brand-1.0.md`](LICENSES/Wonderland-Brand-1.0.md) |
| `design/tools/` | BSD-2-Clause | each file's SPDX line |
| `design/icons/store/` | each icon's own license | `design/icons/store/LICENSES.md` |
| `assets/` (screenshots) | Wonderland Brand License 1.0 | [`assets/LICENSE`](assets/LICENSE) |
| `third_party/blocking/` | Apache-2.0 OR MIT | its own files |
| `crates/core/src/theme/builtin/` imported themes | MIT, as noted | `crates/core/src/theme/builtin/NOTICE` |

The Wonderland License parts are source-available: anyone may read, build, change, ship and sell them under that license's terms.
Patches apply to FreeBSD and drm-kmod sources, which keep their own licenses.

"WolOS", "Wonderland OS", "WolKernel", the WolOS logo and the WolOS characters (the White Rabbit, the Keyhole, the Cheshire, the Pocket Watch, the Teacup) are marks of Julia Klee; no license above grants rights in them beyond what the Wonderland Brand License states.
Permissions: github.com/wonderlandos.
