#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Run a command as the desktop user of uid 1000 with the session's environment (guest, root).
# Usage: as-user.sh hyprctl dispatch exec discord
u=$(id -nu 1000)
rt=/run/user/1000
sig=$(ls -t "$rt/hypr" 2>/dev/null | head -n 1)
way=$(ls "$rt" | grep -m1 '^wayland-[0-9]*$')
exec runuser -u "$u" -- env HOME="$(getent passwd "$u" | cut -d: -f6)" USER="$u" XDG_RUNTIME_DIR="$rt" \
	WAYLAND_DISPLAY="${way:-wayland-1}" HYPRLAND_INSTANCE_SIGNATURE="$sig" \
	DBUS_SESSION_BUS_ADDRESS="unix:path=$rt/bus" XDG_SESSION_TYPE=wayland XDG_CURRENT_DESKTOP=Hyprland "$@"
