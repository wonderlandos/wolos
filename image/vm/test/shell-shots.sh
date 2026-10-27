#!/bin/sh
# WolOS shell on headless sway (pixman, no GPU), build VM, as root. Shots land in /tmp/shots.
# Needs: wol binaries in $B (built --features shell/layer-shell), sway grim wtype zenity, user wolt / wolpass.
# Input: wol-shell.sock (`wol shell ...`), wtype (first key of a wtype run can be dropped: lead with x BackSpace).
set -u
B=${B:-/work/target/release}; O=/tmp/shots; rm -rf $O; mkdir -p $O; chmod 777 $O
id wolt >/dev/null 2>&1 || pw useradd wolt -m -c "Wol Tester" -s /bin/sh
echo wolpass | pw usermod wolt -h 0
install -m 4755 -o root $B/wol-authcheck /usr/local/libexec/wol-authcheck
service wold onestop >/dev/null 2>&1; pkill -x wold; sleep 1; ($B/wold >/tmp/wold.log 2>&1 &)
pkill -u wolt; sleep 1
cat > /tmp/shell-user.sh <<EOS
set -u; B=$B; O=$O
export XDG_RUNTIME_DIR=/tmp/hlu GDK_BACKEND=wayland XDG_CONFIG_DIRS=/usr/local/etc/xdg
rm -rf \$XDG_RUNTIME_DIR; mkdir -p -m 700 \$XDG_RUNTIME_DIR
printf 'output HEADLESS-1 mode 1440x900\ndefault_border none\nfor_window [app_id=".*"] floating enable\n' > /tmp/hlu.conf
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 sway -c /tmp/hlu.conf > /tmp/hlu-sway.log 2>&1 &
sleep 3; export WAYLAND_DISPLAY=wayland-1 SWAYSOCK=\$(ls \$XDG_RUNTIME_DIR/sway-ipc.*.sock)
eval \$(dbus-launch --sh-syntax)
\$B/wol-shell > /tmp/hlu-shell.log 2>&1 &
sleep 6; grim \$O/01-desktop.png
(\$B/wol-spotlight >/dev/null 2>&1 &); sleep 3; wtype -s 40 x -k BackSpace term; sleep 2; grim \$O/02-spotlight.png; pkill -f wol-spotlight; sleep 1
\$B/wol shell show control-center; sleep 2; grim \$O/03-control-center.png; \$B/wol shell hide control-center; sleep 1
(\$B/wol-settings >/dev/null 2>&1 &); (zenity --info --text "WolOS" >/dev/null 2>&1 &); sleep 4
\$B/wol shell show switcher; sleep 2; grim \$O/04-switcher.png; \$B/wol shell hide switcher; pkill zenity; pkill -f wol-settings; sleep 1
notify-send -a Mail "New message" "Lunch at 1?"; sleep 2; grim \$O/05-notification.png; sleep 6
\$B/wol-lock > /tmp/hlu-lock.log 2>&1 &
sleep 5; grim \$O/06-lock.png
wtype -s 60 x -k BackSpace wrongpw -k Return; sleep 4; grim \$O/07-lock-wrong.png
echo "after wrong password: wol-lock running=\$(pgrep -f \$B/wol-lock | wc -l | tr -d ' ')"
wtype -s 60 x -k BackSpace wolpass -k Return; sleep 4; grim \$O/08-unlocked.png
echo "after right password: wol-lock running=\$(pgrep -f \$B/wol-lock | wc -l | tr -d ' ')"
echo "lock surfaces: \$(swaymsg -t get_tree | grep -c '\"app_id\": \"wol-lock\"')"
EOS
su -l wolt -c "sh /tmp/shell-user.sh" 2>&1 | grep -E '^after|^lock'
ls $O
