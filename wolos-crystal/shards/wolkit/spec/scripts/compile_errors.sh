# shellcheck shell=sh
# SPDX-License-Identifier: BSD-2-Clause
# Compile-fail specs: every spec/compile_errors/*.crfail is a small program that must not compile. Its
# `# expect:` line is the message the compiler must print and the line that ends in `# <= error` is
# where it must point, so the messages the plan promises (plan-wolkit section 3.7) are pinned, with
# the user's own file and line. A script and not a Crystal spec because specs may not start the
# compiler (RCR-021). The files end in .crfail so the formatter and the linter leave them alone.
set -eu
cd "$(dirname "$0")/../.."
failed=0
count=0
for file in spec/compile_errors/*.crfail; do
  count=$((count + 1))
  expect=$(sed -n 's/^# expect: //p' "$file")
  line=$(grep -n '# <= error$' "$file" | head -1 | cut -d: -f1)
  if [ -z "$expect" ] || [ -z "$line" ]; then
    echo "FAIL $file: needs an '# expect:' line and a line ending in '# <= error'"
    failed=1
    continue
  fi
  if out=$(crystal build --no-codegen --no-color "$file" 2>&1); then
    echo "FAIL $file: it compiled"
    failed=1
    continue
  fi
  case $out in
    *"Error: $expect"*) ;;
    *)
      echo "FAIL $file: the compiler did not say: $expect"
      echo "$out"
      failed=1
      continue
      ;;
  esac
  if ! printf '%s\n' "$out" | grep -Eq "(In|Code in) $file:$line:"; then
    echo "FAIL $file: the compiler did not point at line $line"
    echo "$out"
    failed=1
    continue
  fi
  echo "ok   $file"
done
echo "$count compile-fail specs"
# And the shard's own programs must compile with no engine library (bin/wolkit-doctor.cr runs where
# the app cannot start).
for file in bin/*.cr; do
  if crystal build --no-codegen --error-on-warnings --no-color "$file" >/dev/null 2>&1; then
    echo "ok   $file builds"
  else
    echo "FAIL $file does not build"
    crystal build --no-codegen --error-on-warnings --no-color "$file" || true
    failed=1
  fi
done
exit "$failed"
