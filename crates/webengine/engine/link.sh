#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# libwolwebengine.so.1 from the shim's staticlib (ADR 0042), the way libwolkit is linked: exports and
# symbol versions come only from wolwebengine.map (a cdylib would also export every C symbol of the
# engine's dependencies), the soname is the ABI major. The native libraries are the ones
# `cargo rustc -- --print native-static-libs` names at the pin; EGL and the GPU drivers are loaded
# at run time by the engine.
# Not --whole-archive (as libwolkit): the engine's archive carries some C objects twice (the JS
# engine bundles a compressor a Rust dependency builds too); each exported symbol is pulled as
# undefined instead, and the archive gives what it needs, as a cdylib link would.
# Pulling by reference misses what registers itself from a static constructor and is never named:
# the engine's resource reader is such an `inventory` entry ("No resource reader registered" at
# the first page). So every Rust object of the archive with an .init_array is linked explicitly.
# Usage: link.sh <libwolwebengine.a> <out-dir>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
lib=$(realpath "$1")
out=$2
mkdir -p "$out"
ctors=$(mktemp -d)
trap 'rm -rf "$ctors"' EXIT
(cd "$ctors" && ar x "$lib" && for o in *.rcgu.o; do
	readelf -SW "$o" | grep -q '\.init_array' || rm -f "$o"
done && find . -maxdepth 1 ! -name '*.rcgu.o' -type f -delete)
need=$(sed -n 's/^ *\(wwe_[a-z_]*\);$/-Wl,--undefined=\1/p' "$here/wolwebengine.map")
# shellcheck disable=SC2086 # one word per symbol
${CC:-cc} -shared -o "$out/libwolwebengine.so.1" $need "$ctors"/*.rcgu.o "$lib" \
	-Wl,--version-script="$here/wolwebengine.map" -Wl,-soname,libwolwebengine.so.1 \
	-Wl,--gc-sections -Wl,-z,relro,-z,now -Wl,-z,noexecstack \
	-lstdc++ -lfontconfig -lz -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
ln -sf libwolwebengine.so.1 "$out/libwolwebengine.so"
