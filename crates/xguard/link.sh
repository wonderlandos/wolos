#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# libxguard.so.1 from the Rust staticlib (ADR 0022): exports and symbol versions come only from
# abi/xguard.map, the soname is the ABI major. Usage: link.sh <libxguard.a> <out-dir>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
lib=$1
out=$2
mkdir -p "$out"
${CC:-cc} -shared -o "$out/libxguard.so.1" \
	-Wl,--whole-archive "$lib" -Wl,--no-whole-archive \
	-Wl,--version-script="$here/abi/xguard.map" -Wl,-soname,libxguard.so.1 \
	-Wl,--gc-sections -Wl,-z,relro,-z,now -Wl,-z,noexecstack \
	-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
ln -sf libxguard.so.1 "$out/libxguard.so"
