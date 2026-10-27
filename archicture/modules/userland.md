# Everyday userland

Mature ports under WolOS names. Tests: `crates/controller/tests/image_userland.rs`, `crates/shell/tests/pins_installed.rs`.

## Apps

| Dock / role | Package | App-id (Hyprland class, `apps::PINS` id) |
|---|---|---|
| Files, Trash (`nautilus trash:///`) | nautilus | `org.gnome.Nautilus` |
| Browser | firefox | `firefox` |
| Mail | geary | `org.gnome.Geary` |
| Music | amberol | `io.bassi.Amberol` |
| Photos | shotwell | `org.gnome.Shotwell` |
| Calendar | gnome-calendar | `org.gnome.Calendar` |
| Notes | gnote | `org.gnome.Gnote` |
| Terminal | foot | `foot` |

Launchpad also lists Calculator, Clocks, Papers, Celluloid (+ mpv), File Roller, Disk Usage, Text Editor, Image Viewer (eog) from their `.desktop` files; a pinned app appears once, under its WolOS name.
Default apps: `/usr/local/share/applications/mimeapps.list` (`~/.config/mimeapps.list` wins).

## System

| What | Where |
|---|---|
| zsh for accounts WolOS creates (`users::LOGIN_SHELL`); root keeps `/bin/sh` | `/usr/local/etc/{zshrc,zprofile}`, skel `.zshrc` (comments only), `/usr/local/share/zsh/scripts/newuser` (no wizard) |
| sudo for `%wheel` | `/usr/local/etc/sudoers.d/wolos` (0440) |
| Groups | admins `wheel,video,operator`, standard accounts `video` (operator can read raw disks) |
| User folders | `xdg-user-dirs-update` in `wol-session` |
| Login keyring | wol-login: `gnome-keyring-daemon --daemonize --login` (password on stdin) after PAM succeeds; the session's `--start --components=secrets` finishes it; stopped at logout |
| Printing | `cupsd_enable`, `avahi_daemon_enable` (driverless IPP) |
| USB sticks | `autofs_enable`, `/media -wolmedia`: stock `special_media` + group operator write access on FAT, exFAT (fusefs-exfat), NTFS (fusefs-ntfs) |

Size (15.1 quarterly, 2026-10-27): the apps and services above add 71 packages, 689 MiB installed, 147 MiB download. Largest: webkit2-gtk_41 (Geary) 189 MB, webkit2-gtk_60 + db5 (evolution-data-server, Calendar) 221 MB. Weather is the WolOS desktop widget (gnome-weather would add gjs + spidermonkey, ~50 MB, and needs geoclue).
