#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# libwol.so.1 from the Rust staticlib (ADR 0020): exports and symbol versions come only from
# abi/wol.map, the soname is the ABI major. Usage: link.sh <libwol.a> <out-dir>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
lib=$1
out=$2
mkdir -p "$out"
${CC:-cc} -shared -o "$out/libwol.so.1" \
	-Wl,--whole-archive "$lib" -Wl,--no-whole-archive \
	-Wl,--version-script="$here/abi/wol.map" -Wl,-soname,libwol.so.1 \
	-Wl,--gc-sections -Wl,-z,relro,-z,now -Wl,-z,noexecstack \
	-lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
ln -sf libwol.so.1 "$out/libwol.so"
