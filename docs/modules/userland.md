# Everyday userland (ADR 0016)

Decision record: ADR 0016 (Arch base; mature ports under WolOS names).

Mature ports under WolOS names. Tests: `crates/controller/tests/image_userland.rs`, `crates/shell/tests/pins_installed.rs`.

## Apps

| Dock / role | Package | App-id (compositor window class, `apps::PINS` id) |
|---|---|---|
| Files, Trash (`nautilus trash:///`) | nautilus | `org.gnome.Nautilus` |
| Browser | helium-browser (`wol_core::theme::helium::HELIUM.bin`) | `helium` |
| Mail | geary | `org.gnome.Geary` |
| Music | amberol | `io.bassi.Amberol` |
| Photos | wolos-app-gallery (Gallery; Shotwell is in the Store) | `org.wolos.gallery` |
| Calendar | gnome-calendar | `org.gnome.Calendar` |
| Notes | gnote | `org.gnome.Gnote` |
| Terminal | foot | `foot` |
| App Store, System Settings | wolos-shell | `org.wolos.store`, `org.wolos.settings` |

Apps also lists Calculator, Clocks, Papers, Celluloid (+ mpv), File Roller, Disk Usage, Text Editor, Image Viewer (eog) from their `.desktop` files; a pinned app appears once, under its WolOS name.
Default apps: `/usr/share/applications/mimeapps.list` (`~/.config/mimeapps.list` wins); the web opens in Helium (`helium.desktop`).

## System

| What | Where |
|---|---|
| zsh for accounts WolOS creates (`users::LOGIN_SHELL`); root keeps `/bin/sh` | `/etc/zsh/zshenv` (no wizard; loads `/usr/share/wolos/zsh/zshrc` in interactive shells), skel `.zshrc` (comments only) |
| sudo for `%wheel` | `/etc/sudoers.d/wolos` (0440) |
| Groups | admins `wheel,seat,video`, standard accounts `seat,video` (`users::ACCOUNT_GROUPS*`; not `input` or `audio`: every keyboard and the sound devices go through the session, QA-FILES-2; `wol doctor` names accounts still in the groups older releases gave). `seat` opens the display: wol-login starts no session for an account outside it, answers the login window `no-display` (`shared::login::REPLY_NO_DISPLAY`, the window says to ask an administrator), and a session that ends within 2 s counts as failed and waits 1 s .. 5 min before the next try |
| User folders | `xdg-user-dirs-update` in `wol-session` |
| Login keyring | wol-login: `gnome-keyring-daemon --daemonize --login` (password on stdin) after PAM succeeds; the session's `--start --components=secrets` finishes it; stopped at logout |
| Printing | `cups.socket` and `avahi-daemon.service` enabled by the preset `files/usr/lib/systemd/system-preset/50-wolos.preset` (driverless IPP Everywhere); Settings > Printers adds, removes and sets the default through wol-admind and prints a test page or cancels jobs as the person (`wol_core::printers`) |
| USB sticks | `udisks2.service` (same preset): every session mounts removable volumes through udisks (Files' sidebar); no group needed (`settings.rs` `ACCOUNT_GROUPS_ADMIN`) |

Weather is the WolOS desktop widget (gnome-weather would add gjs + spidermonkey, ~50 MB, and needs geoclue). Sizes: the image's package list is measured by the ISO build (`crates/controller/tests/module_sizes.rs` for the modules); the figures this page carried for the old base no longer apply (QA-PWR-24).

## GNU and uutils (ADR 0017, 2026-11-04)

`wolos-base` links every GNU coreutils 9.12 program name (102, no more) to uutils-coreutils 0.12.0 (`/usr/bin/ls` -> `uu-ls`) and `sudo`, `sudoedit`, `visudo` to sudo-rs; `provides=('coreutils=9.12' 'sudo')` and `conflicts=('coreutils' 'sudo')` keep `base`, `base-devel` and everything that depends on those names resolving. Everything else stays GNU: findutils, diffutils, procps-ng, util-linux, tar. Ubuntu made the same two defaults (rust-coreutils and sudo-rs, 25.10 and later) and keeps its Rust findutils and diffutils in `universe` as opt-in.

Why the rest stays (checked 2026-11-04, details and sources in wolos-iso README "Userland and GNU"):

- findutils: not in Arch `extra`; uutils-findutils 0.11.0 passes every call WolOS and Arch's tools make (find in `crates/core/src/update.rs` for the slot swap, `find -printf '%P\0'` in the base manifest, makepkg, mkinitcpio, dkms), but ships only as AUR source and its binary is larger than GNU findutils.
- diffutils: uutils-diffutils 0.6.0 has no `-r`, `-q`, `-w`, `-p`, `-N`, `--label`, `--color`, `--version`; devtools and dkms use them.
- procps-ng: uutils procps has no release; its `ps -ax -o pid= -o ppid=` prints nothing, which `wol-login` needs to stop a session's process tree (`jobs::PS_ARGS`).
- util-linux: uutils/util-linux is 0.0.1.

The gate for the next swap is wolos-pkgbuilds `wolos-base/userland-matrix.sh` (170 cases, each tool with every flag form WolOS and the Arch tools use, answers recorded from GNU); a tool joins `wolos-base` when no `ours-`/`third-` case differs. Code that calls these tools must keep to the forms in that script, or add the form there first (so `find -printf`, `ps -o pid=` and `pkill --signal --uid --exact` stay tested). Known differences in the shipped uutils-coreutils: `timeout -s INT -k N CMD` exits 137 where GNU exits 124; `stdbuf` needs `/usr/lib/uutils-coreutils/libstdbuf.so` (the package ships it).

