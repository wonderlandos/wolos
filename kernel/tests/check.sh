#!/bin/sh
# Kernel series contract (Arch/Linux layout): the WolKernel extra series, scx_wolos and the KMI tools,
# with the markers the public build relies on. KSRC=<fork checkout> also applies the series on its HEAD.
set -eu
O=$(cd "$(dirname "$0")/../.." && pwd); fail=0
has() { grep -qF -- "$2" "$O/$1" || { echo "FAIL $1 lacks: $2"; fail=1; }; }
L=kernel/linux/patches
set -- "$O"/$L/*.patch
[ -f "$1" ] || { echo "FAIL no patches in $L"; exit 1; }
for p in "$@"; do
  n=${p##*/}
  case "$n" in [0-9][0-9][0-9][0-9]-*.patch) ;; *) echo "FAIL bad patch name: $n"; fail=1;; esac
  grep -q '^Subject: ' "$p" || { echo "FAIL $n is not a git format-patch"; fail=1; }
  grep -q '^diff --git' "$p" || { echo "FAIL $n has no diff"; fail=1; }
done
# The fork (github.com/wonderlandos/linux, branch wolos) carries the hint interface, the bounded
# futex spin and the i915 deadline boost; the series must not carry them again.
for gone in 0001-futex-bounded-spin 0002-wolos-hint-interface 0003-i915-deadline-boost-foreground-waitboost; do
  [ ! -f "$O/$L/$gone.patch" ] || { echo "FAIL $gone is in the fork, drop it here"; fail=1; }
done
has $L/0004-futex-adaptive-spin.patch 'futex_spin_next('
has $L/0005-wolos-policy-kunit.patch 'kunit_test_suite(wolos_policy_suite)'
has $L/0006-tsc-trust-single-package.patch 'wolos_tsc_trusted('
has $L/0007-vt-kbd-graphics-bypass.patch 'wolos_kbd_bypass('
has $L/0008-oom-foreground-bias.patch 'wolos_oom_bias('
has $L/0009-input-boost.patch 'input_register_handler(&wib_handler)'
has $L/0009-input-boost.patch 'wolos_input_boost_util(max)'
has $L/0010-slub-sheaf-scale.patch 'slub_sheaf_scale'
has $L/0011-futex-spin-rcu.patch 'guard(rcu)()'
has $L/0012-futex-waitv-inline.patch 'FUTEX_WAITV_INLINE'
has $L/0013-futex-precheck.patch 'futex_value_differs('
has kernel/scx_wolos/scx_wolos.bpf.c 'SCX_OPS_DEFINE(wolos_ops'
has kernel/scx_wolos/scx_wolos.service 'Restart=on-failure'
has kernel/scx_wolos/scx_wolos_class.h 'wolos_class_preempts('
has kernel/scx_wolos/tests/class_test.c 'class_test: ok'
has kernel/kmi/kmi.sh 'Module.symvers'
# With a fork checkout (KSRC), the series applies in name order on its HEAD, like prepare.sh.
if [ -n "${KSRC:-}" ] && [ -d "$KSRC/.git" ]; then
  t=$(mktemp -d "${TMPDIR:-$HOME}/wolos-check.XXXXXX"); rmdir "$t"
  git -C "$KSRC" worktree add -q --detach "$t" HEAD
  for p in "$@"; do (cd "$t" && patch -Np1 -s --fuzz=0 < "$p" >/dev/null) || { echo "FAIL does not apply: ${p##*/}"; fail=1; }; done
  git -C "$KSRC" worktree remove --force "$t"
fi
# The Arch package applies $WOLOS_TREE/kernel/linux/patches in order and stops on a failure.
PUB=$O
P=$PUB/kernel/linux-wolos/prepare.sh
for want in '"${WOLOS_TREE:-/nonexistent}"/kernel/linux/patches/*.patch' 'exit 3'; do
  grep -qF -- "$want" "$P" || { echo "FAIL $P lacks: $want"; fail=1; }
done
grep -qF 'WOLOS_TREE=' "$PUB/kernel/linux-wolos/PKGBUILD" || { echo "FAIL PKGBUILD does not pass WOLOS_TREE"; fail=1; }
[ $fail = 0 ] && echo "kernel series: ok"; exit $fail
