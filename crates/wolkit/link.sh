#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# libwolkit.so.1 from the Rust staticlib (ADR 0039): exports and symbol versions come only from
# abi/wolkit.map, the soname is the ABI major. The native backend calls libwayland-client (the
# wayland-client crate's `system` feature, so Vulkan WSI gets real display and surface pointers)
# and libxkbcommon (keymaps, compose).
# Usage: link.sh <libwolkit.a> <out-dir>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
lib=$1
out=$2
mkdir -p "$out"
${CC:-cc} -shared -o "$out/libwolkit.so.1" \
	-Wl,--whole-archive "$lib" -Wl,--no-whole-archive \
	-Wl,--version-script="$here/abi/wolkit.map" -Wl,-soname,libwolkit.so.1 \
	-Wl,--gc-sections -Wl,-z,relro,-z,now -Wl,-z,noexecstack \
	-lwayland-client -lxkbcommon -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
ln -sf libwolkit.so.1 "$out/libwolkit.so"
