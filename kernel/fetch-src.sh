#!/bin/sh
# SPDX-License-Identifier: BSD-2-Clause
# Shallow-clone FreeBSD releng/15.1 (kernel source tree).
# Usage: kernel/fetch-src.sh [DIR]   (default: $WOLOS_SRC or /usr/src)
# Needs ~3.5 GB free (shallow clone ~2.6 GB + headroom). Build objs need ~8 GB more.
set -eu
DIR=${1:-${WOLOS_SRC:-/usr/src}}
BRANCH=${WOLOS_SRC_BRANCH:-releng/15.1}
URL=${WOLOS_SRC_URL:-https://git.freebsd.org/src.git}   # mirror: https://github.com/freebsd/freebsd-src.git
NEED_MB=${WOLOS_SRC_NEED_MB:-3500}
mkdir -p "$DIR"
[ -z "$(ls -A "$DIR" 2>/dev/null)" ] || { [ -d "$DIR/.git" ] && { echo "exists: $DIR (git -C $DIR pull to update)"; exit 0; }; echo "not empty: $DIR" >&2; exit 1; }
free=$(df -Pm "$DIR" | awk 'NR==2{print $4}')
[ "$free" -ge "$NEED_MB" ] || { echo "need ${NEED_MB} MB free in $DIR, have ${free} MB" >&2; exit 2; }
command -v git >/dev/null || { echo "git missing (FreeBSD: pkg install git)" >&2; exit 1; }
exec git clone --depth 1 --single-branch --branch "$BRANCH" "$URL" "$DIR"
