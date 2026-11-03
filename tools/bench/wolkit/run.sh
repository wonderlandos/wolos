# shellcheck shell=sh
# SPDX-License-Identifier: BSD-2-Clause
# Run the shard-side benchmark (bench.cr) in release mode against the libraries in $WOLKIT_BUILD
# (default: wolos-crystal/shards/wolkit/build, built by `make -C wolos-crystal/shards/wolkit all`).
# Prints one JSON line: median and p95 over $WOLKIT_BENCH_RUNS runs (30 by default). Same machine,
# same scripted scenario; the report states the commit, the machine and the refresh rate
# (docs/modules/wolkit-bench.md).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
build=${WOLKIT_BUILD:-$root/wolos-crystal/shards/wolkit/build}
out=${TMPDIR:-/tmp}/wolkit-bench.$$
trap 'rm -rf "$out"' EXIT
mkdir -p "$out"
crystal build --release "$here/bench.cr" -o "$out/bench" --link-flags "-L$build -lwol -Wl,-rpath,$build"
"$out/bench"
