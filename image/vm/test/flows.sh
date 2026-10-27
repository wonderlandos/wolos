#!/bin/sh
# Installed-disk flows with no GPU (run as root on a throwaway copy of test/disk.raw; fixtures ISO at $F).
#   sh flows.sh <iso dir> prep     install sway/wtype pkgs, swap in greeter-sway.sh, restart wol-login
#   sh flows.sh <iso dir> shot N   grim the greeter into /tmp/setup-N.png
#   sh flows.sh <iso dir> click X Y | type TEXT | key NAME   input into the greeter's sway (wlrctl, wtype)
F=$1; G=/var/run/wol-login
as() { chroot -u _wollogin -g _wollogin / env XDG_RUNTIME_DIR=$G WAYLAND_DISPLAY=wayland-1 SWAYSOCK=$(ls $G/sway-ipc.*.sock 2>/dev/null | head -1) "$@"; }
case $2 in
prep)
  pkg add -q "$F"/pkgs/*.pkg 2>&1 | grep -v already
  if [ -d "$F/bin" ]; then   # binaries under test (built from the current tree on the build VM)
    for b in "$F"/bin/*; do install -m 755 "$b" /usr/local/bin/; done
    install -m 4555 -o root "$F/bin/wol-authcheck" /usr/local/libexec/wol-authcheck
    install -m 555 "$F/wold.rc" /usr/local/etc/rc.d/wold
    sh "$F/icons/install.sh" >/dev/null; gtk-update-icon-cache -f /usr/local/share/icons/WolOS >/dev/null 2>&1
  fi
  cp /usr/local/bin/wol-greeter-session /var/tmp/wol-greeter-session.orig
  install -m 755 "$F/greeter-sway.sh" /usr/local/bin/wol-greeter-session
  service wold stop >/dev/null 2>&1; sleep 3; service wold start >/dev/null 2>&1; sleep 10
  pgrep -lf 'wol-login|sway|wol-setup' ;;
shot) as grim /tmp/setup-$3.png && nc -N 10.0.2.2 47123 < /tmp/setup-$3.png ;;   # host: listener on 127.0.0.1:47123
click) as "$F/vptr" 1440 900 m "$3" "$4" c ;;   # one virtual-pointer session (fixtures/vptr.c): headless has no pointer
type) as wtype -s 40 x -k BackSpace "$3" ;;
key) as wtype -s 40 x -k BackSpace -k "$3" ;;
esac
