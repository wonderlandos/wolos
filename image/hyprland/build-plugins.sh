#!/bin/sh
# Build hyprbars (hyprland-plugins, commit pinned for the installed Hyprland) + plugins/wol-hypr
# against the installed hyprland-wolos headers; install into STAGE/usr/lib/hyprland/plugins/.
# Usage: build-plugins.sh STAGE   (run by the wolos-system PKGBUILD; needs hyprland-wolos, git, c++)
# Env: WOLOS_HP_REV (hyprland-plugins commit; default = hyprpm.toml pin for 0.56.0-0.56.2),
#      WOLOS_RT_LIB (librealtime.a), WOLOS_WORK (clone cache).
set -eu
STAGE=$1
IMG=$(cd "$(dirname "$0")/.." && pwd); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-${TMPDIR:-/tmp}}
REV=${WOLOS_HP_REV:-7644cecdb947060682891a0db2a0cdc5c0b9e704}
RT=${WOLOS_RT_LIB:-${CARGO_TARGET_DIR:-$REPO/target}/release/librealtime.a}
DST=$STAGE/usr/lib/hyprland/plugins
CF=$(pkg-config --cflags pixman-1 libdrm hyprland libinput libudev wayland-server xkbcommon lua)
HP=$WORK/hyprland-plugins
[ -d "$HP/.git" ] || git clone -q https://github.com/hyprwm/hyprland-plugins "$HP"
git -C "$HP" fetch -q --depth 1 origin "$REV"; git -C "$HP" checkout -q "$REV"
make -C "$HP/hyprbars" INCLUDES="$CF"
[ -f "$RT" ] || cargo build --release -p realtime --manifest-path "$REPO/Cargo.toml"
c++ -std=c++2b -fPIC -O2 -fno-access-control $CF -shared "$REPO/plugins/wol-hypr/main.cpp" "$RT" -o "$WORK/libwol-hypr.so"
mkdir -p "$DST"
install -m 755 "$HP/hyprbars/hyprbars.so" "$DST/libhyprbars.so"
install -m 755 "$WORK/libwol-hypr.so" "$DST/libwol-hypr.so"
for so in "$DST"/*.so; do nm -D --defined-only "$so" | grep -q ' T pluginInit$' || { echo "no pluginInit in $so" >&2; exit 1; }; done
echo "plugins: $DST"
