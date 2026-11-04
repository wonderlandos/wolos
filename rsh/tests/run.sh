#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# rsh golden and example runner.
#   golden/NAME.rsh   run in an empty directory; stdout plus a last line "status: N" must equal
#                     NAME.out; if NAME.explain exists, `rsh --explain NAME.rsh` must equal it
#   examples/NAME.rsh the same, with fakebin/ first on PATH (stand-ins for ssh, rsync, git ...),
#                     arguments from NAME.args when that file exists (written as for sh: quotes allowed)
# Usage: RSH=path/to/rsh sh run.sh [--update] [name ...]
# An (even empty) NAME.explain file opts a script in to the classification check, a NAME.err
# file to the standard-error check.
# --update writes the expected files from the current output (look at the diff before you keep it).
set -u
here=$(cd "$(dirname "$0")" && pwd)
rsh=${RSH:-$here/../build/rsh}
case $rsh in /*) ;; *) rsh=$PWD/$rsh ;; esac
update=0
if [ "${1:-}" = "--update" ]; then
	update=1
	shift
fi
names=" $* "
have_names=0
[ $# -gt 0 ] && have_names=1
pass=0
fail=0
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# Same output on every machine: C locale, a fixed home, no colour, a private working directory.
run_one() { # dir name -> writes $tmp/got.out
	dir=$1
	name=$2
	work=$tmp/work
	rm -rf "$work"
	mkdir -p "$work/home"
	args=
	[ -f "$here/$dir/$name.args" ] && args=$(cat "$here/$dir/$name.args")
	eval "set -- $args"
	path=$PATH
	[ "$dir" = examples ] && path=$here/fakebin:$PATH
	(
		cd "$work" || exit 99
		env -i PATH="$path" HOME="$work/home" LC_ALL=C TERM=dumb USER=tester RSH_TESTS="$here" \
			"$rsh" "$here/$dir/$name.rsh" "$@" 2>"$tmp/got.err" >"$tmp/got.body"
		echo "status: $?" >>"$tmp/got.body"
	)
	sed "s#$work#WORK#g" "$tmp/got.body" >"$tmp/got.out"
}

check() { # expected-file got-file label
	if [ "$update" = 1 ]; then
		cp "$2" "$1"
		echo "updated $3"
	elif cmp -s "$1" "$2"; then
		pass=$((pass + 1))
	else
		fail=$((fail + 1))
		echo "FAIL $3"
		diff "$1" "$2" | head -20
	fi
}

for dir in golden examples; do
	for f in "$here/$dir"/*.rsh; do
		[ -f "$f" ] || continue
		name=$(basename "$f" .rsh)
		if [ "$have_names" = 1 ]; then
			case "$names" in *" $name "*) ;; *) continue ;; esac
		fi
		run_one "$dir" "$name"
		check "$here/$dir/$name.out" "$tmp/got.out" "$dir/$name"
		if [ -f "$here/$dir/$name.err" ]; then
			sed "s#$tmp/work#WORK#g; s#$here/##g" "$tmp/got.err" >"$tmp/got.err2"
			check "$here/$dir/$name.err" "$tmp/got.err2" "$dir/$name (stderr)"
		fi
		if [ -f "$here/$dir/$name.explain" ]; then
			"$rsh" --explain "$f" 2>&1 | sed "s#$here/##" >"$tmp/got.explain"
			check "$here/$dir/$name.explain" "$tmp/got.explain" "$dir/$name (explain)"
		fi
	done
done
echo "rsh golden: $pass passed, $fail failed"
[ "$fail" = 0 ]
