#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Build hyprbars (hyprland-plugins, commit pinned for the installed compositor, + desktop/hyprland/hyprbars patches)
# + plugins/wol-hypr
# against the installed hyprland-wolos headers; install into STAGE/usr/lib/hyprland/plugins/.
# Usage: build-plugins.sh STAGE   (run by the wolos-system PKGBUILD; needs hyprland-wolos, git, c++)
# Env: WOLOS_HP_REV (hyprland-plugins commit; default = hyprpm.toml pin for 0.56.0-0.56.2),
#      WOLOS_RT_LIB (librealtime.a), WOLOS_WORK (clone cache).
set -eu
STAGE=$1
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../.." && pwd)
WORK=${WOLOS_WORK:-${TMPDIR:-/tmp}}
REV=${WOLOS_HP_REV:-7644cecdb947060682891a0db2a0cdc5c0b9e704}
RT=${WOLOS_RT_LIB:-${CARGO_TARGET_DIR:-$REPO/target}/release/librealtime.a}
DST=$STAGE/usr/lib/hyprland/plugins
CF=$(pkg-config --cflags pixman-1 libdrm hyprland libinput libudev wayland-server xkbcommon lua)
HP=$WORK/hyprland-plugins
[ -d "$HP/.git" ] || git clone -q https://github.com/hyprwm/hyprland-plugins "$HP"
git -C "$HP" fetch -q --depth 1 origin "$REV"; git -C "$HP" checkout -q -f "$REV"
# WolOS window buttons (desktop/hyprland/hyprbars), applied in name order on a clean checkout.
for p in "$HERE"/hyprbars/*.patch; do patch -d "$HP" -Np1 -s < "$p"; done
[ -f "$RT" ] || cargo build --release -p realtime --manifest-path "$REPO/Cargo.toml"
# Patched hyprbars takes its button geometry and hover state from realtime.
make -C "$HP/hyprbars" INCLUDES="$CF -I$REPO/crates/realtime/include" LIBS="$RT -Wl,--gc-sections"
# wol-hypr: STANDARDS 3 warnings as errors for our code; the compositor's headers are -isystem (they warn).
# shellcheck disable=SC2086 # CF: one word per flag
CFS=$(printf ' %s' $CF | sed 's/ -I/ -isystem /g')
# shellcheck disable=SC2086 # CFS: one word per flag
c++ -std=c++2b -fPIC -O2 -Wall -Wextra -Werror -fno-access-control $CFS -shared "$REPO/plugins/wol-hypr/main.cpp" "$RT" -o "$WORK/libwol-hypr.so"
mkdir -p "$DST"
install -m 755 "$HP/hyprbars/hyprbars.so" "$DST/libhyprbars.so"
install -m 755 "$WORK/libwol-hypr.so" "$DST/libwol-hypr.so"
for so in "$DST"/*.so; do nm -D --defined-only "$so" | grep -q ' T pluginInit$' || { echo "no pluginInit in $so" >&2; exit 1; }; done
echo "plugins: $DST"
