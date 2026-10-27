#!/bin/sh
# VM side of compat.sh. Usage: sh compat-vm.sh <fixture dir>   Env: R=wol-run path (default: on PATH).
# Bundles go in /tmp/compat. gears.wol runs only when sway + linux glxgears exist (build VM).
set -u
F=$1; R=${R:-wol-run}; W=/tmp/compat
rm -rf $W; mkdir -p $W; cp "$F"/futex_waitv "$F"/syscall_regs "$F"/hello.exe "$F"/headless.sh $W/; cd $W
mk() { mkdir -p "$1.wol/bin"; printf '{"id":"org.wolos.test.%s","version":"1","targets":[%s]}' "$1" "$2" > "$1.wol/Info.json"; }
L() { printf '{"abi":"linux","arch":"amd64","entry":"bin/%s"}' "$1"; }
mk uname "$(L uname)"; cp /compat/linux/usr/bin/uname uname.wol/bin/
mk futex "$(L futex_waitv)"; cp futex_waitv futex.wol/bin/
mk regs "$(L syscall_regs)"; cp syscall_regs regs.wol/bin/
mk win "$(L missing),{\"abi\":\"proton\",\"arch\":\"amd64\",\"entry\":\"bin/hello.exe\"}"; cp hello.exe win.wol/bin/
export HOME=/tmp/wolhome XDG_RUNTIME_DIR=/tmp/hlrt; mkdir -p $HOME $XDG_RUNTIME_DIR; chmod 700 $XDG_RUNTIME_DIR
uname -a; kldstat | grep -E 'linux64|wolos_qos'
for b in uname futex regs; do echo "== wol-run $b.wol"; a=; [ $b = uname ] && a=-a; $R $b.wol $a; echo "exit=$?"; done
echo "== wol-run win.wol > file (linux entry missing -> proton)"
timeout 300 $R win.wol >win.out 2>/dev/null; echo "exit=$?"; cat win.out
[ -x /compat/linux/usr/bin/glxgears ] && command -v sway >/dev/null || exit 0
mk gears "$(L glxgears)"; cp /compat/linux/usr/bin/glxgears gears.wol/bin/
sh headless.sh start >/dev/null
echo "== wol-run gears.wol (Linux GUI, Xwayland, llvmpipe)"
(DISPLAY=:0 timeout 12 $R gears.wol -info > gears.out 2>&1; echo "exit=$? (124 = stopped by timeout)" >> gears.out) &
sleep 7; XDG_RUNTIME_DIR=/tmp/hlrt WAYLAND_DISPLAY=wayland-1 grim /tmp/gears.png && echo "screenshot: /tmp/gears.png"
wait; grep -E 'GL_RENDERER|FPS|exit=' gears.out | head -4
