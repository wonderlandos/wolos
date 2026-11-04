# Everyday userland (ADR 0016)

Decision record: ADR 0016 (Arch base; mature ports under WolOS names).

Mature ports under WolOS names. Tests: `crates/controller/tests/image_userland.rs`, `crates/shell/tests/pins_installed.rs`.

## Apps

| Dock / role | Package | App-id (compositor window class, `apps::PINS` id) |
|---|---|---|
| Files, Trash (`nautilus trash:///`) | nautilus | `org.gnome.Nautilus` |
| Browser | firefox | `firefox` |
| Mail | geary | `org.gnome.Geary` |
| Music | amberol | `io.bassi.Amberol` |
| Photos | shotwell | `org.gnome.Shotwell` |
| Calendar | gnome-calendar | `org.gnome.Calendar` |
| Notes | gnote | `org.gnome.Gnote` |
| Terminal | foot | `foot` |

Apps also lists Calculator, Clocks, Papers, Celluloid (+ mpv), File Roller, Disk Usage, Text Editor, Image Viewer (eog) from their `.desktop` files; a pinned app appears once, under its WolOS name.
Default apps: `/usr/share/applications/mimeapps.list` (`~/.config/mimeapps.list` wins).

## System

| What | Where |
|---|---|
| zsh for accounts WolOS creates (`users::LOGIN_SHELL`); root keeps `/bin/sh` | `/etc/zsh/zshenv` (no wizard; loads `/usr/share/wolos/zsh/zshrc` in interactive shells), skel `.zshrc` (comments only) |
| sudo for `%wheel` | `/etc/sudoers.d/wolos` (0440) |
| Groups | admins `wheel,seat,video,input,audio`, standard accounts `seat,video,input,audio` (`users::ACCOUNT_GROUPS*`). `seat` opens the display: wol-login starts no session for an account outside it, answers the login window `no-display` (`shared::login::REPLY_NO_DISPLAY`, the window says to ask an administrator), and a session that ends within 2 s counts as failed and waits 1 s .. 5 min before the next try |
| User folders | `xdg-user-dirs-update` in `wol-session` |
| Login keyring | wol-login: `gnome-keyring-daemon --daemonize --login` (password on stdin) after PAM succeeds; the session's `--start --components=secrets` finishes it; stopped at logout |
| Printing | `cupsd_enable`, `avahi_daemon_enable` (driverless IPP) |
| USB sticks | `autofs_enable`, `/media -wolmedia`: stock `special_media` + group operator write access on FAT, exFAT (fusefs-exfat), NTFS (fusefs-ntfs) |

Size (15.1 quarterly, 2026-10-27): the apps and services above add 71 packages, 689 MiB installed, 147 MiB download. Largest: webkit2-gtk_41 (Geary) 189 MB, webkit2-gtk_60 + db5 (evolution-data-server, Calendar) 221 MB. Weather is the WolOS desktop widget (gnome-weather would add gjs + spidermonkey, ~50 MB, and needs geoclue).

## GNU and uutils (ADR 0017, 2026-11-04)

`wolos-base` links every GNU coreutils 9.12 program name (102, no more) to uutils-coreutils 0.12.0 (`/usr/bin/ls` -> `uu-ls`) and `sudo`, `sudoedit`, `visudo` to sudo-rs; `provides=('coreutils=9.12' 'sudo')` and `conflicts=('coreutils' 'sudo')` keep `base`, `base-devel` and everything that depends on those names resolving. Everything else stays GNU: findutils, diffutils, procps-ng, util-linux, tar. Ubuntu made the same two defaults (rust-coreutils and sudo-rs, 25.10 and later) and keeps its Rust findutils and diffutils in `universe` as opt-in.

Why the rest stays (checked 2026-11-04, details and sources in wolos-iso README "Userland and GNU"):

- findutils: not in Arch `extra`; uutils-findutils 0.11.0 passes every call WolOS and Arch's tools make (find in `crates/core/src/update.rs` for the slot swap, `find -printf '%P\0'` in the base manifest, makepkg, mkinitcpio, dkms), but ships only as AUR source and its binary is larger than GNU findutils.
- diffutils: uutils-diffutils 0.6.0 has no `-r`, `-q`, `-w`, `-p`, `-N`, `--label`, `--color`, `--version`; devtools and dkms use them.
- procps-ng: uutils procps has no release; its `ps -ax -o pid= -o ppid=` prints nothing, which `wol-login` needs to stop a session's process tree (`jobs::PS_ARGS`).
- util-linux: uutils/util-linux is 0.0.1.

The gate for the next swap is wolos-pkgbuilds `wolos-base/userland-matrix.sh` (170 cases, each tool with every flag form WolOS and the Arch tools use, answers recorded from GNU); a tool joins `wolos-base` when no `ours-`/`third-` case differs. Code that calls these tools must keep to the forms in that script, or add the form there first (so `find -printf`, `ps -o pid=` and `pkill --signal --uid --exact` stay tested). Known differences in the shipped uutils-coreutils: `timeout -s INT -k N CMD` exits 137 where GNU exits 124; `stdbuf` needs `/usr/lib/uutils-coreutils/libstdbuf.so` (the package ships it).

