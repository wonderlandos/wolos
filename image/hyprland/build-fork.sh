#!/bin/sh
# Optional: build a Hyprland fork (default pkg `hyprland` is used otherwise) on FreeBSD 15.1.
# Usage: image/hyprland/build-fork.sh [DESTDIR]
# Env: WOLOS_HYPR_URL (default https://github.com/hyprwm/Hyprland.git), WOLOS_HYPR_REF (v0.56.2 = x11-wm/hyprland port),
#      WOLOS_HYPR_DIR (/work/hyprland). Applies image/hyprland/patches/*.patch in order (patch -p1),
#      Perf defaults ship in the desktop overlay (hypr/wolos-perf.conf). Map: archicture/modules/hyprland-fork.md
# Disk: ~3 GB in WOLOS_HYPR_DIR (checked).
set -eu
H=$(cd "$(dirname "$0")" && pwd)
DEST=${1:-/}
URL=${WOLOS_HYPR_URL:-https://github.com/hyprwm/Hyprland.git}
REF=${WOLOS_HYPR_REF:-v0.56.2}
DIR=${WOLOS_HYPR_DIR:-/work/hyprland}
[ "$(uname -s)" = FreeBSD ] || { echo "build-fork.sh: run on FreeBSD" >&2; exit 1; }
mkdir -p "$(dirname "$DIR")"
free=$(df -Pm "$(dirname "$DIR")" | awk 'NR==2{print $4}')
[ "$free" -ge 3000 ] || { echo "build-fork.sh: need 3000 MB free, have $free MB" >&2; exit 2; }
# Build deps = the port's deps; installing pkg hyprland pulls the runtime libs (aquamarine, hyprutils, ...).
env ASSUME_ALWAYS_YES=yes pkg install -y git cmake ninja pkgconf hyprland hyprwayland-scanner \
  evdev-proto glaze glslang hyprland-protocols wayland-protocols python3 >/dev/null   # port BUILD_DEPENDS
[ -d "$DIR/.git" ] || git clone --recursive --depth 1 --branch "$REF" "$URL" "$DIR"
cd "$DIR"
# Patch set = FreeBSD port x11-wm/hyprland files/ (ports 5a00402de54d, FreeBSD fixes) then ours. The tree is
# reset whenever the set changes, so a half-applied tree never builds.
stamp=$(cat "$H"/port-files/patch-* "$H"/patches/*.patch | sha256 -q)
if [ "$(cat .wolos-patched 2>/dev/null)" != "$stamp" ]; then
  git checkout -q -f . && git submodule foreach -q git checkout -q -f . && git clean -q -fdx -e build
  for p in "$H"/port-files/patch-*; do patch -p0 -s -f -E < "$p"; done
  # Port post-patch: consolekit2 runtime dir, PREFIX wallpapers, linprocfs under LINUXBASE.
  sed -i '' 's|/run/user|/var&|' hyprctl/src/main.cpp hyprpm/src/core/HyprlandSocket.cpp src/Compositor.cpp
  sed -i '' 's|/usr/share|/usr/local/share|' src/render/OpenGL.cpp
  sed -i '' -e 's|/proc|/compat/linux&|g' src/debug/HyprCtl.cpp src/debug/crash/CrashReporter.cpp \
    src/desktop/view/Window.cpp src/helpers/MiscFunctions.cpp src/plugins/HookSystem.cpp src/plugins/PluginAPI.cpp
  for p in "$H"/patches/*.patch; do patch -p1 -s -f < "$p"; done
  echo "$stamp" > .wolos-patched
fi
# Port recipe: CMAKE_ON NO_SYSTEMD, CMAKE_OFF CMAKE_CXX_SCAN_FOR_MODULES (else clang-scan-deps fails on
# -include-pch), X11 on, LDFLAGS --as-needed -pthread, base clang.
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local \
  -DNO_SYSTEMD=ON -DCMAKE_CXX_SCAN_FOR_MODULES=OFF -DNO_XWAYLAND=OFF \
  -DCMAKE_EXE_LINKER_FLAGS="-Wl,--as-needed -pthread" -DCMAKE_SHARED_LINKER_FLAGS="-Wl,--as-needed -pthread"
cmake --build build
rm -f "$DEST/usr/local/bin/hyprland"
DESTDIR=$DEST cmake --install build
ln -sf Hyprland "$DEST/usr/local/bin/hyprland"   # port post-install
