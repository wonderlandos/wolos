#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Crystal lint (docs/RULES-crystal.md RCR-002, RCR-003): `crystal tool format --check` and ameba
# (rules and baselines in .ameba.yml) over every tracked .cr file, minus the scanner fixtures that
# are wrong on purpose. Run from anywhere: `tools/crystal/lint.sh`. CI runs it on Arch, whose
# crystal is the formatter reference (the output differs between crystal versions). Exit 1 on a
# finding, 2 when a tool is missing and WOLOS_REQUIRE_CRYSTAL_LINT=1 (CI sets it); otherwise a
# missing tool is reported and skipped.
set -eu

# The ameba release whose rule set .ameba.yml lists; --up-to-version keeps newer rules off.
AMEBA_VERSION=1.7.0

cd "$(dirname "$0")/../.."
fail=0
require=${WOLOS_REQUIRE_CRYSTAL_LINT:-0}

# tracked plus new files git does not ignore (a file not yet added is linted too)
files=$(git ls-files -co --exclude-standard -z '*.cr' 2>/dev/null | tr '\0' '\n' | grep -v '^crates/controller/tests/fixtures/' || true)
if [ -z "$files" ]; then
  files=$(find . -name '*.cr' -not -path '*/target/*' -not -path '*/lib/*' -not -path './crates/controller/tests/fixtures/*' | sed 's|^\./||' | sort)
fi

skip() {
  echo "lint.sh: $1" >&2
  [ "$require" = 1 ] && exit 2
  return 0
}

if command -v crystal >/dev/null 2>&1; then
  # shellcheck disable=SC2086 # the file list has no spaces (RCR-001 names are plain)
  crystal tool format --check $files || fail=1
else
  skip "crystal not installed, format check skipped"
fi

if command -v ameba >/dev/null 2>&1 && ameba --version >/dev/null 2>&1; then
  # shellcheck disable=SC2086
  ameba --up-to-version "$AMEBA_VERSION" --no-color $files || fail=1
else
  skip "ameba not installed or not runnable, lint skipped"
fi

exit "$fail"
