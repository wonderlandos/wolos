#!/bin/sh
# Games end to end on FreeBSD (archicture/modules/games.md). Prints PASS/FAIL/INFO lines; exit = FAILs.
#   doctor       `wol games doctor --json` runs; every check listed
#   steam-setup  WolOS Proton written, second run up to date
#   handoff      a Linux bash (as Steam) runs wolos-proton waitforexitandrun hello.exe: FreeBSD
#                wol-run -> native wine-proton, exit 7, prefix in $STEAM_COMPAT_DATA_PATH/pfx
#   paths        getcompatpath through the same handoff
#   win32        32-bit hello32.exe (WoW64 or the i386 package)
#   exe          wol-run --exe hello.exe (double-click path), prefix per program
#   vulkan       vulkaninfo --summary, then vulkan.exe: Windows Vulkan through winevulkan (DXVK's path)
#   handler      org.wolos.windows-app.desktop (WolOS image only)
# Host (Linux): builds CRT-free Windows fixtures, ships them with this script, runs `games.sh --vm`.
# Usage: games.sh (build VM over ssh; binaries from /work/target/release, linked to /usr/local/bin/wol-run)
#        games.sh --iso out.iso (installed disk: mount_cd9660 /dev/cd0 /mnt; sh /mnt/games.sh --vm /mnt)
# Needs host: clang, lld-link, llvm-dlltool. VM: linux ABI, linux_base-rl9, wine-proton, vulkan-tools.
set -eu
if [ "${1:-}" != --vm ]; then
  REPO=$(cd "$(dirname "$0")/../../.." && pwd); SSH=$REPO/image/vm/ssh.sh; FX=$REPO/image/vm/test/fixtures
  B=$(mktemp -d); trap 'rm -rf "$B"' EXIT
  printf 'LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle\nWriteFile\nExitProcess\n' > "$B/k32.def"
  printf 'LIBRARY kernel32.dll\nEXPORTS\nGetStdHandle@4\nWriteFile@20\nExitProcess@4\n' > "$B/k32_32.def"
  printf 'LIBRARY vulkan-1.dll\nEXPORTS\nvkCreateInstance\nvkEnumeratePhysicalDevices\nvkGetPhysicalDeviceProperties\n' > "$B/vk.def"
  llvm-dlltool -m i386:x86-64 -d "$B/k32.def" -l "$B/kernel32.lib"
  llvm-dlltool -m i386 -k -d "$B/k32_32.def" -l "$B/kernel32_32.lib"
  llvm-dlltool -m i386:x86-64 -d "$B/vk.def" -l "$B/vulkan-1.lib"
  exe() { # exe <target> <source> <out> <libs...>
    t=$1 src=$2 out=$3; shift 3
    clang --target="$t" -O2 -c "$FX/$src" -o "$B/$out.obj"
    lld-link /subsystem:console /entry:mainCRTStartup /nodefaultlib /out:"$B/$out.exe" "$B/$out.obj" "$@" >/dev/null
    rm -f "$B/$out.obj"
  }
  exe x86_64-pc-windows-msvc win-hello.c hello "$B/kernel32.lib"
  exe i686-pc-windows-msvc win-hello.c hello32 "$B/kernel32_32.lib"
  exe x86_64-pc-windows-msvc win-vulkan.c vulkan "$B/kernel32.lib" "$B/vulkan-1.lib"
  rm -f "$B"/*.def "$B"/*.lib
  cp "$0" "$B/games.sh"
  if [ "${1:-}" = --iso ]; then
    xorriso -as mkisofs -quiet -R -o "$2" "$B"; echo "games: $2"; exit 0
  fi
  tar -C "$B" -cf - . | $SSH 'rm -rf /work/gx && mkdir -p /work/gx && tar -C /work/gx -xf -'
  $SSH 'env ASSUME_ALWAYS_YES=yes pkg install -y wine-proton linux_base-rl9 vulkan-tools >/dev/null 2>&1
    kldstat -q -m linux64elf || service linux onestart >/dev/null 2>&1
    BIN=/work/target/release sh /work/gx/games.sh --vm /work/gx'
  exit
fi

# ---- VM side (FreeBSD) ----
set +e
F=$2; BIN=${BIN:-/usr/local/bin}; fails=0
ok() { echo "PASS $*"; }
no() { echo "FAIL $*"; fails=$((fails + 1)); }
info() { echo "INFO $*"; }
H=/tmp/wolgames; rm -rf "$H" /tmp/wolgames-rt; mkdir -p "$H" /tmp/wolgames-rt; chmod 700 /tmp/wolgames-rt
export HOME=$H USER=${USER:-root} XDG_RUNTIME_DIR=/tmp/wolgames-rt
W=$H/work; mkdir -p "$W"; cp "$F"/*.exe "$W/"
info "$(uname -srm); $(/usr/local/wine-proton/bin/wine --version 2>/dev/null || echo 'no wine-proton')"
# The generated launcher execs /usr/local/bin/wol-run: point it at the binaries under test.
if [ "$BIN" != /usr/local/bin ]; then ln -sf "$BIN/wol-run" /usr/local/bin/wol-run; info "linked /usr/local/bin/wol-run -> $BIN/wol-run"; fi

"$BIN/wol" games doctor --json > "$W/doctor.json" 2>&1; rc=$?
if grep -q '"checks"' "$W/doctor.json"; then ok "doctor: JSON report (exit $rc)"; else no "doctor: no report"; cat "$W/doctor.json"; fi
grep -E '^ *"(id|status)":' "$W/doctor.json" | sed 's/[",]//g; s/^ *//' | paste - - | sed 's/^id: /INFO doctor /; s/[[:space:]]*status: / /'
grep -A3 '"futex-waitv"' "$W/doctor.json" | grep detail | sed 's/^ */INFO /'

out=$("$BIN/wol" games steam-setup 2>&1)
case $out in Installed*) ok "steam-setup: installed" ;; *) no "steam-setup: $out" ;; esac
out=$("$BIN/wol" games steam-setup 2>&1)
case $out in *"up to date"*) ok "steam-setup: second run up to date" ;; *) no "steam-setup again: $out" ;; esac
T=$H/.steam/steam/compatibilitytools.d/WolOS-Proton/wolos-proton

# Steam is a Linux process: start from a Linux env + bash with a Linux LD_PRELOAD, as linux-steam-utils does.
# steam_like <seconds> <command...>
D=$H/compatdata/4242; mkdir -p "$D"
steam_like() {
  t=$1; shift
  timeout "$t" /compat/linux/usr/bin/env -i HOME="$H" USER="$USER" XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
    PATH=/compat/linux/usr/bin:/compat/linux/bin \
    LD_PRELOAD=/compat/linux/usr/lib64/libm.so.6 LD_LIBRARY_PATH=/compat/linux/usr/lib64 \
    LSU_FBSD_PATH=/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin \
    STEAM_COMPAT_DATA_PATH="$D" STEAM_COMPAT_CLIENT_INSTALL_PATH="$H/.steam/steam" \
    SteamAppId=4242 SteamGameId=4242 STEAM_RUNTIME=0 PRESSURE_VESSEL_FILESYSTEMS_RO=/usr \
    /compat/linux/bin/bash -c '"$@"' bash "$@"
}
if [ ! -x /compat/linux/bin/bash ] || [ ! -x "$T" ]; then
  no "handoff: needs /compat/linux/bin/bash and $T"
else
  steam_like 900 "$T" waitforexitandrun "$W/hello.exe" > "$W/h.out" 2> "$W/h.err"; rc=$?
  if [ $rc = 7 ] && grep -q '^Hello from a Windows' "$W/h.out"; then ok "handoff: Linux bash -> wolos-proton -> wine-proton, exit 7"
  else no "handoff: exit $rc"; tail -5 "$W/h.err"; fi
  grep -m1 '^wolos-proton:' "$W/h.err" | sed 's/^/INFO /'
  if [ -f "$D/pfx/system.reg" ]; then ok "handoff: prefix in STEAM_COMPAT_DATA_PATH/pfx"; else no "handoff: no $D/pfx/system.reg"; fi
  out=$(steam_like 300 "$T" getcompatpath /tmp 2>/dev/null | tr -d '\r')
  case $out in [A-Z]:\\*) ok "paths: getcompatpath /tmp -> $out" ;; *) no "paths: getcompatpath gave '$out'" ;; esac
  steam_like 600 "$T" waitforexitandrun "$W/hello32.exe" > "$W/h32.out" 2> "$W/h32.err"; rc=$?
  if [ $rc = 7 ]; then ok "win32: 32-bit exe, exit 7"; else no "win32: exit $rc (doctor win32 above)"; tail -3 "$W/h32.err"; fi
fi

timeout 900 "$BIN/wol-run" --exe "$W/hello.exe" > "$W/e.out" 2> "$W/e.err"; rc=$?
if [ $rc = 7 ] && grep -q '^Hello' "$W/e.out"; then ok "exe: wol-run --exe, exit 7"; else no "exe: exit $rc"; tail -5 "$W/e.err"; fi
pfx=$(ls -d "$H"/.local/share/wolos/prefixes/exe-hello-*/pfx 2>/dev/null | head -1)
if [ -n "$pfx" ]; then ok "exe: own prefix ${pfx#"$H"/}"; else no "exe: no prefix under ~/.local/share/wolos/prefixes"; fi

vk=$(vulkaninfo --summary 2>/dev/null | grep -E 'deviceType|deviceName' | sed 's/^[[:space:]]*//; s/  */ /g' | paste -s -d ' ' -)
if [ -n "$vk" ]; then ok "vulkan: native: $vk"; else no "vulkan: vulkaninfo found no device"; fi
if [ -x /compat/linux/bin/bash ] && [ -x "$T" ]; then
  steam_like 600 "$T" waitforexitandrun "$W/vulkan.exe" > "$W/v.out" 2> "$W/v.err"; rc=$?
  tr -d '\r' < "$W/v.out" | sed 's/^/INFO /'
  if [ $rc = 0 ]; then ok "vulkan: Windows Vulkan through wine-proton"; else no "vulkan: vulkan.exe exit $rc"; tail -3 "$W/v.err"; fi
fi

A=/usr/local/share/applications/org.wolos.windows-app.desktop
if [ -f "$A" ]; then
  if grep -qx 'Exec=wol-run --exe %f' "$A"; then ok "handler: .exe opens with wol-run --exe"; else no "handler: $(grep ^Exec "$A")"; fi
else info "handler: $A missing (not the WolOS image)"; fi

echo "games: $fails failed"
exit $fails
