#!/bin/sh
# Headless sway (wlroots headless + pixman, no GPU) for WolOS screenshots on FreeBSD. VM-side.
# usage: hl.sh start | hl.sh shot <app> <png> [wait]
export XDG_RUNTIME_DIR=/tmp/hlrt XDG_CONFIG_DIRS=/usr/local/etc/xdg
case $1 in
start)
  pkill -x sway; sleep 1; rm -rf $XDG_RUNTIME_DIR; mkdir -p $XDG_RUNTIME_DIR; chmod 700 $XDG_RUNTIME_DIR
  printf 'output HEADLESS-1 mode 1440x900 bg #1e1e1e solid_color\ndefault_border none\nfor_window [app_id=".*"] floating enable\n' > /tmp/hl.conf
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /tmp/hl.conf > /tmp/sway.log 2>&1 &
  sleep 3; ls $XDG_RUNTIME_DIR ;;
shot)
  export WAYLAND_DISPLAY=$(ls $XDG_RUNTIME_DIR | grep -m1 '^wayland-[0-9]*$') GDK_BACKEND=wayland
  pkill -f "release/wol-"; sleep 1
  /work/target/release/$2 > /tmp/hl-$2.log 2>&1 & sleep ${4:-4}
  grim "$3"; echo "pid=$(pgrep -f release/$2)"; tail -3 /tmp/hl-$2.log ;;
esac
