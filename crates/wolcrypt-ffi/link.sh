#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# libwolcrypt.so.1 from the Rust staticlib: exports and symbol versions come only from
# abi/wolcrypt.map, the soname is the ABI major.
# Usage: link.sh <libwolcrypt_ffi.a> <out-dir>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
lib=$1
out=$2
mkdir -p "$out"
${CC:-cc} -shared -o "$out/libwolcrypt.so.1" \
	-Wl,--whole-archive "$lib" -Wl,--no-whole-archive \
	-Wl,--version-script="$here/abi/wolcrypt.map" -Wl,-soname,libwolcrypt.so.1 \
	-Wl,--gc-sections -Wl,-z,relro,-z,now -Wl,-z,noexecstack \
	-lgcc_s -lpthread -lm -ldl -lc
ln -sf libwolcrypt.so.1 "$out/libwolcrypt.so"
