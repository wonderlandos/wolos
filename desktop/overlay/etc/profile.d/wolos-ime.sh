# SPDX-License-Identifier: GPL-2.0-or-later
# WolOS "Input methods" module (wolos-module-ime, docs/modules/modules.md): with fcitx5 installed the
# session tells Qt and XWayland apps to type through it. Read by every login shell, so the whole
# desktop session inherits it (wol-login starts the session through `su -l`, like wolos-console.sh).
# GTK needs nothing: it talks to fcitx5 over Wayland's text-input protocol, and GTK_IM_MODULE would
# switch that off. Without the module (no /usr/bin/fcitx5) nothing is exported. `fcitx5 -d` itself is
# started by the compositor session (hyprland.lua), not here: a login shell is not the session.
# shellcheck shell=sh
if [ -x /usr/bin/fcitx5 ]; then
  export XMODIFIERS="${XMODIFIERS:-@im=fcitx}"
  export QT_IM_MODULE="${QT_IM_MODULE:-fcitx}"
fi
