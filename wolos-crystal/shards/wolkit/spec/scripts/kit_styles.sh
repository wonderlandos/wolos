# shellcheck shell=sh
# SPDX-License-Identifier: BSD-2-Clause
# The default look is committed as generated Crystal (src/wolkit/style/kit_classes.cr and
# kit_table.cr). This check compiles styles/kit.wolcss again with wolkit-stylec and compares, so the
# sheet and the files cannot drift apart. Usage: kit_styles.sh STYLEC (the wolkit-stylec binary built
# by `cargo build -p wolkit`); without one it says so and passes.
set -eu
cd "$(dirname "$0")/../.."
stylec=${1:-}
if [ -z "$stylec" ] || [ ! -x "$stylec" ]; then
  echo "skip: wolkit-stylec not found (cargo build -p wolkit builds it)"
  exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
sh bin/kit-styles.sh "$stylec" "$tmp" > /dev/null
failed=0
for file in kit_classes.cr kit_table.cr; do
  if cmp -s "$tmp/$file" "src/wolkit/style/$file"; then
    echo "ok   $file matches styles/kit.wolcss"
  else
    echo "FAIL $file differs from styles/kit.wolcss: run make styles"
    failed=1
  fi
done
exit "$failed"
