#!/bin/sh
# Test-only stand-in for /usr/local/bin/wol-greeter-session on a GPU-less VM: same user, same apps, but
# headless sway (pixman) instead of Hyprland. Installed by flows.sh on a throwaway copy of the disk.
export PATH=/usr/local/bin:/usr/local/sbin:/usr/bin:/bin:/usr/sbin:/sbin
export XDG_RUNTIME_DIR=/var/run/wol-login HOME=/var/run/wol-login XDG_SESSION_TYPE=wayland
install -d -o _wollogin -g _wollogin -m 0700 "$XDG_RUNTIME_DIR" "$HOME/.cache"
app=wol-loginwindow; [ "${1:-}" = setup ] && app=wol-setup
printf 'output HEADLESS-1 mode 1440x900\ndefault_border none\nexec sh -c "/usr/local/bin/%s; swaymsg exit"\n' $app > "$HOME/sway.conf"
chown _wollogin "$HOME/sway.conf"
exec chroot -u _wollogin -g _wollogin -G _wollogin,video / env WLR_BACKENDS=headless WLR_RENDERER=pixman \
  WLR_LIBINPUT_NO_DEVICES=1 sway -c "$HOME/sway.conf"
