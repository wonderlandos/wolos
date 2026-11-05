#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Background duty cycle (guest, root): freeze the cgroup STOP seconds, thaw it RUN seconds, until
# stopped; always leaves it thawed. Usage: duty.sh /sys/fs/cgroup/<scope> STOP RUN
cg=${1:?cgroup dir}
stop=${2:?stop secs}
run=${3:?run secs}
trap 'echo 0 > "$cg/cgroup.freeze"; exit 0' INT TERM EXIT
while :; do
	echo 1 >"$cg/cgroup.freeze"
	sleep "$stop"
	echo 0 >"$cg/cgroup.freeze"
	sleep "$run"
done
