#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# fuzz-replay.sh: build every fuzz target on the stable toolchain (no sanitizer, no coverage) and run
# each over its fuzz/regressions/<target>/ corpus once (-runs=0: replay, no fuzzing). A crash found by
# the nightly smoke job is saved there and so stays a test (audit B10).
set -euo pipefail
cd "$(dirname "$0")/../.."
target=${CARGO_TARGET_DIR:-fuzz/target}
cargo build --manifest-path fuzz/Cargo.toml --locked --bins --target-dir "$target"
fail=0
for src in fuzz/fuzz_targets/*.rs; do
  t=$(basename "$src" .rs)
  dir=fuzz/regressions/$t
  [ -d "$dir" ] || { echo "fuzz-replay: $t has no $dir" >&2; fail=1; continue; }
  if "$target/debug/$t" -runs=0 "$dir" >"$target/replay-$t.log" 2>&1; then
    echo "fuzz-replay: $t ok ($(find "$dir" -type f | wc -l) inputs)"
  else
    echo "fuzz-replay: $t FAILED" >&2; tail -30 "$target/replay-$t.log" >&2; fail=1
  fi
done
exit $fail
