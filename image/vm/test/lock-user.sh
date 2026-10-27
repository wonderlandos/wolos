#!/bin/sh
# Installed disk: lock/unlock as a Setup-created user on headless sway (no GPU). Run after flows.sh prep + Setup.
#   root: mkdir -m 777 /tmp/lk; daemon -f su <user> -c "cd; sh lock-user.sh > /tmp/lk/result.txt 2>&1"  (PW=<password>)
# su -l hangs on the serial console (resizewin); use plain su.
set -u; O=/tmp/lk; mkdir -p $O
export XDG_RUNTIME_DIR=/tmp/lku GDK_BACKEND=wayland XDG_CONFIG_DIRS=/usr/local/etc/xdg
rm -rf $XDG_RUNTIME_DIR; mkdir -p -m 700 $XDG_RUNTIME_DIR
printf 'output HEADLESS-1 mode 1440x900\ndefault_border none\nfor_window [app_id=".*"] floating enable\n' > /tmp/lku.conf
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /tmp/lku.conf > /tmp/lku-sway.log 2>&1 &
sleep 3; export WAYLAND_DISPLAY=wayland-1 SWAYSOCK=$(ls $XDG_RUNTIME_DIR/sway-ipc.*.sock)
eval $(dbus-launch --sh-syntax)
ktrace -i -t cn -f /tmp/lk/lock.kt /usr/local/bin/wol-lock > /tmp/lk/wol-lock.log 2>&1 &
sleep 6; grim $O/10-lock.png
wtype -s 60 x -k BackSpace wrongpw -k Return; sleep 5; grim $O/11-lock-wrong.png
echo "after wrong password: wol-lock running=$(pgrep -x wol-lock | wc -l | tr -d ' ')"
wtype -s 60 x -k BackSpace ${PW:-anapass1} -k Return; sleep 5; grim $O/12-unlocked.png
echo "after right password: wol-lock running=$(pgrep -x wol-lock | wc -l | tr -d ' ')"
echo "lock surfaces in sway tree: $(swaymsg -t get_tree | grep -c wol-lock)"
echo "layer surfaces: $(swaymsg -t get_outputs | grep -c layer)"
