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
| Photos | wolos-app-gallery (Gallery; Shotwell is in the Store) | `org.wolos.gallery` |
| Calendar | gnome-calendar | `org.gnome.Calendar` |
| Notes | gnote | `org.gnome.Gnote` |
| Terminal | foot | `foot` |

Apps also lists Calculator, Clocks, Papers, Celluloid (+ mpv), File Roller, Disk Usage, Text Editor from their `.desktop` files; a pinned app appears once, under its WolOS name.
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
