#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# The signature data a system ships with. Usage: build.sh <wol-xguard> <out-dir> [VERSION]
# Downloads replace it (ADR 0024). Extra sources: rule folders in $XGUARD_RULES, signature lists in
# $XGUARD_LISTS (each keeps its own license, listed in NOTICES). The result is unsigned: a shipped
# copy is covered by the image's own seal; `rules bundle` + image/sign-release.sh make an update.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin=$1
out=$2
version=${3:-1}
mkdir -p "$out"
"$bin" rules build "$out/base.xgdb" "$version" "$here" ${XGUARD_LISTS:-}
"$bin" rules yara "$out/rules.yxc" "$here" ${XGUARD_RULES:-}
"$bin" rules bundle "$out" "$version"
