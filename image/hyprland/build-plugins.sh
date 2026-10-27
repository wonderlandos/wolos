#!/bin/sh
# Build hyprbars (hyprland-plugins, commit pinned for the installed Hyprland) + plugins/wol-hypr
# against the build host's pkg hyprland headers; install into STAGE/usr/local/lib/hyprland/plugins/.
# Usage: build-plugins.sh STAGE   (FreeBSD; pkg hyprland gmake evdev-proto glslang git, librealtime.a built)
# Env: WOLOS_HP_REV (hyprland-plugins commit; default = hyprpm.toml pin for 0.56.0-0.56.2),
#      WOLOS_RT_LIB (librealtime.a), WOLOS_WORK.
set -eu
STAGE=$1
IMG=$(cd "$(dirname "$0")/.." && pwd); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-/work}
REV=${WOLOS_HP_REV:-7644cecdb947060682891a0db2a0cdc5c0b9e704}
RT=${WOLOS_RT_LIB:-${CARGO_TARGET_DIR:-$REPO/target}/release/librealtime.a}
DST=$STAGE/usr/local/lib/hyprland/plugins
env ASSUME_ALWAYS_YES=yes pkg install -y hyprland gmake evdev-proto glslang git >/dev/null
# hyprland.pc omits lua and evdev include paths; add them.
CF=$(pkg-config --cflags pixman-1 libdrm hyprland libinput libudev wayland-server xkbcommon lua-5.5)
HP=$WORK/hyprland-plugins
[ -d "$HP/.git" ] || git clone -q https://github.com/hyprwm/hyprland-plugins "$HP"
git -C "$HP" fetch -q --depth 1 origin "$REV"; git -C "$HP" checkout -q "$REV"
gmake -C "$HP/hyprbars" INCLUDES="$CF"
[ -f "$RT" ] || cargo build --release -p realtime --manifest-path "$REPO/Cargo.toml"
c++ -std=c++2b -fPIC -O2 -fno-access-control $CF -shared "$REPO/plugins/wol-hypr/main.cpp" "$RT" -o "$WORK/libwol-hypr.so"
mkdir -p "$DST"
install -m 555 "$HP/hyprbars/hyprbars.so" "$DST/libhyprbars.so"
install -m 555 "$WORK/libwol-hypr.so" "$DST/libwol-hypr.so"
for so in "$DST"/*.so; do nm -D --defined-only "$so" | grep -q ' T pluginInit$' || { echo "no pluginInit in $so" >&2; exit 1; }; done
echo "plugins: $DST"
