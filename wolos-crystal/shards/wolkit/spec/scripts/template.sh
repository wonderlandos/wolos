# shellcheck shell=sh
# SPDX-License-Identifier: BSD-2-Clause
# The app template: `wolkit-new` writes a project that type-checks and is formatted. With
# WOLKIT_BUILD=<dir holding libwolkit.so.1 and libwol.so.1> it also builds the program and runs the
# project's own headless spec (what `make spec-engine` does). A script because specs may not start the
# compiler (RCR-021).
set -eu
cd "$(dirname "$0")/../.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
shards=$(cd ../ && pwd)
system_path=$(crystal env CRYSTAL_PATH)
CRYSTAL_PATH="$shards:$system_path"
export CRYSTAL_PATH
crystal run bin/wolkit-new.cr -- demo "$tmp/demo" > "$tmp/files"
test "$(wc -l < "$tmp/files")" -ge 8
cd "$tmp/demo"
crystal build --no-codegen --error-on-warnings src/main.cr
crystal tool format --check src spec
echo "ok   generated project type-checks and is formatted"
if [ -n "${WOLKIT_BUILD:-}" ]; then
  link="-L$WOLKIT_BUILD -lwol -Wl,-rpath,$WOLKIT_BUILD"
  mkdir -p bin
  crystal build --error-on-warnings --link-flags "$link" src/main.cr -o bin/demo
  crystal spec --error-on-warnings --link-flags "$link" > "$tmp/spec.out" 2>&1 || { cat "$tmp/spec.out"; exit 1; }
  grep -q "1 examples, 0 failures" "$tmp/spec.out"
  echo "ok   generated project builds and passes its own spec"
fi
