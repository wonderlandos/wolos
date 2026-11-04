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
| Photos | shotwell | `org.gnome.Shotwell` |
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

Weather is the WolOS desktop widget (gnome-weather would add gjs + spidermonkey, ~50 MB, and needs geoclue). Sizes: the image's package list is measured by the ISO build (`crates/controller/tests/module_sizes.rs` for the modules); the FreeBSD 15.1 figures this page carried no longer apply (QA-PWR-24).
