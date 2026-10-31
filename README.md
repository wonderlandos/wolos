<p align="center"><img src="design/brand/wolos-mark.svg" width="96" alt="WolOS"></p>

<h1 align="center">WolOS</h1>
<p align="center"><b>A calm, simple desktop. On the PC you already own.</b></p>

<p align="center"><img src="assets/desktop.png" alt="The WolOS desktop"></p>

WolOS is an open source desktop operating system for ordinary PCs. It is built on Arch Linux packages, with a Rust system API, a GTK 4 shell (menu bar, Dock, Search, Control Center, Settings, Store, Setup Assistant), a Hyprland-based window server, and WolKernel, a Linux kernel tuned for foreground responsiveness and Steam/Proton games. Updates land in a new ZFS boot environment, so the old system is one reboot away.

> Active development. The image installs offline and boots; the full desktop is still being proven on real hardware. Expect rough edges.

<p align="center"><img src="assets/setup.png" width="49%" alt="Setup Assistant"> <img src="assets/spotlight.png" width="49%" alt="Search"></p>

## What is in this repo

| Path | What |
|---|---|
| `crates/` | Rust workspace: system API and daemons (`wold`, `wol-admind`, `wol-login`, `wol`), the GTK 4 shell (`crates/shell`), shared libraries |
| `kernel/` | WolKernel packaging for `linux-wolos`: config fragment, version, removed-ABI list, UAPI headers |
| `image/` | Live and installer ISO, the `[wolos]` package repo recipes, QEMU test scripts |
| `desktop/`, `plugins/` | Desktop overlay and window server plugins |
| `archicture/` | Architecture notes and decision records |
| `design/`, `assets/` | Brand and design art (see License) |

Related repositories:

- Kernel fork: https://github.com/wonderlandos/linux (the `wolos` branch holds the WolKernel patches, all public)
- Website: https://github.com/wonderlandos/site

## Build

Develop: `cargo test --workspace`

Image (host with archiso, about 20 GB free), as documented in [`image/README.md`](image/README.md):

```sh
WOLOS_UNSIGNED=1 image/vm/build.sh     # local test repo and ISO
WOLOS_KEEP_DISK=1 image/vm/test-install.sh
image/vm/run-installed.sh
```

See `kernel/README.md` for the kernel and `archicture/` for how the pieces fit.

## Contributing

Issues and pull requests are welcome. Run `cargo test --workspace` before sending a change, keep changes focused, and say what you tested.

## License

Under [`COPYRIGHT`](COPYRIGHT), as of today: BSD-2-Clause for everything not listed below (`LICENSE`); the shell (`crates/shell/`) is GPL-3.0-or-later; `design/` and `assets/` are CC BY-NC-ND 4.0; `third_party/blocking/` is Apache-2.0 OR MIT. The WolOS name and logo are trademarks of Julia Klee; forks should use their own name and art.

Copyright (c) 2026 Julia Klee.