#!/bin/sh
# /usr/bin/helium: the official launcher with WolOS defaults. Wayland when the session is one
# (Hyprland), the app-id `helium` (Dock running dot, menu bar name), hardware video decode when
# the machine has a GPU render node. Flags, whitespace separated, from /etc/helium-flags.conf then
# ~/.config/helium-flags.conf come after the defaults and before the command line.
export CHROME_DESKTOP=helium.desktop

n=$#
for conf in /etc/helium-flags.conf "${XDG_CONFIG_HOME:-$HOME/.config}/helium-flags.conf"; do
  [ -r "$conf" ] || continue
  while IFS= read -r line; do
    case $line in '' | '#'*) continue ;; esac
    # shellcheck disable=SC2086
    set -- "$@" $line
  done < "$conf"
done
# The command line goes last: move its $n words behind the flags just added.
i=0
while [ "$i" -lt "$n" ]; do
  set -- "$@" "$1"
  shift
  i=$((i + 1))
done

defaults="--ozone-platform-hint=auto"
# VA-API decode and encode on the GPU, only with a render node: a VM without one stays in software.
for node in /dev/dri/renderD*; do
  if [ -e "$node" ]; then
    defaults="$defaults --enable-features=AcceleratedVideoDecodeLinuxGL,AcceleratedVideoEncoder"
    break
  fi
done
# shellcheck disable=SC2086
set -- $defaults "$@"

exec /opt/helium/helium-wrapper "$@"
