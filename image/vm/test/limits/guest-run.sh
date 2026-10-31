#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Runs inside the throwaway guest as root (kbomb.service). One pressure kind per boot, from the
# kernel command line: kb=KIND kbmode=before|after. Everything goes to the second serial port.
K=/run/kbomb/t
kind=$(sed -n 's/.*\bkb=\([a-z0-9]*\).*/\1/p' /proc/cmdline)
mode=$(sed -n 's/.*\bkbmode=\([a-z]*\).*/\1/p' /proc/cmdline)
stty -F /dev/ttyS1 raw -echo 115200 2>/dev/null
chmod 666 /dev/ttyS1
exec >/dev/ttyS1 2>&1
ev() { echo "E $(cut -d' ' -f1 /proc/uptime) $*"; }
ev start kind=$kind mode=$mode mem=$(free -m | awk 'NR==2{print $2}')

# Properties memsaver::procs::adopt_args adds after this stream (test: image_limits).
extra=()
if [ "$mode" = after ]; then
	extra=(TasksMax t 8192 MemoryMaxScale u 3435973836 MemorySwapMaxScale u 1073741823)
fi
adopt() { # NAME PID: the same StartTransientUnit wold's Memory Saver sends for a new window.
	local session=$(cut -d: -f3 /proc/$(pgrep -ox comp)/cgroup | xargs basename)
	busctl --system call org.freedesktop.systemd1 /org/freedesktop/systemd1 \
		org.freedesktop.systemd1.Manager StartTransientUnit 'ssa(sv)a(sa(sv))' \
		"$1" fail $((5 + ${#extra[@]} / 3)) PIDs au 1 "$2" Slice s user-1000.slice \
		Description s "$1" CollectMode s inactive-or-failed "${extra[@]}" \
		BindsTo as 1 "$session" 0 >/dev/null
	# The job runs after the call returns.
	for i in $(seq 100); do grep -q "$1" /proc/$2/cgroup && break; sleep 0.05; done
}
as_user() { exec setpriv --reuid=1000 --regid=1000 --init-groups env HOME=/home/wol "$@"; }

systemctl start kbomb-session.service
for i in $(seq 50); do pgrep -x comp >/dev/null && break; sleep 0.1; done
# The second window: a Terminal the user has open while another one misbehaves.
as_user bash -c 'exec '$K'/comp T' &
term=$!
adopt app-wol-term-$term.scope $term
# Automatic recovery, the same code wold runs (after only).
[ "$mode" = after ] && [ -x $K/wol-limits-sweep ] && $K/wol-limits-sweep &

( while :; do
	t0=$(date +%s%N)
	timeout 15 runuser -l root -c 'id -u' >/dev/null 2>&1; rc=$?
	echo "R $(cut -d' ' -f1 /proc/uptime) ms=$((($(date +%s%N) - t0) / 1000000)) rc=$rc"
	sleep 2
done ) &
( while :; do
	s=$(cat /sys/fs/cgroup/user.slice/user-1000.slice/pids.current 2>/dev/null)
	echo "S $(cut -d' ' -f1 /proc/uptime) pids=$s avail=$(awk '/MemAvailable/{print int($2/1024)}' /proc/meminfo) fnr=$(cut -f1 /proc/sys/fs/file-nr) mem=$(sed -n 2p /proc/pressure/memory | cut -d' ' -f2) commit=$(awk '/Committed_AS/{print int($2/1024)}' /proc/meminfo) refused=$(cat /sys/fs/cgroup/user.slice/user-1000.slice/app-wol-bomb-*/pids.events 2>/dev/null | head -1 | cut -d' ' -f2)"
	sleep 1
done ) &
sleep 10
ev nproc $(grep 'Max processes' /proc/$(pgrep -ox comp)/limits | tr -s ' ') bomb-side $(grep 'Max processes' /proc/self/limits | tr -s ' ') threads-max=$(cat /proc/sys/kernel/threads-max)

case $kind in
load) # False kills: a parallel build next to a browser-like tree, both in window scopes.
	mkfifo $K/b; as_user bash -c 'read go < '$K'/b; exec '$K'/load.sh' &
	lp=$!; adopt app-wol-load-$lp.scope $lp; echo go > $K/b
	ev load-start; wait $lp; ev load-end rc=$?
	sleep 2; ev done; poweroff -f;;
esac

mkfifo $K/go
as_user bash -c 'cd; exec '$K'/kb '$kind' 90 < '$K'/go' &
bp=$!
adopt app-wol-bomb-$bp.scope $bp
sc=/sys/fs/cgroup/user.slice/user-1000.slice/app-wol-bomb-$bp.scope
( mkfifo $K/z; exec 8<>$K/z; U=/sys/fs/cgroup/user.slice/user-1000.slice
  for i in 1 2 3 4 5 6 7 8; do read -t 2 <&8
	echo "D1 slice cur=$(<$U/pids.current) max=$(<$U/pids.max) ev=$(<$U/pids.events)"
	echo "D2 scope cur=$(<$sc/pids.current) max=$(<$sc/pids.max) ev=$(<$sc/pids.events)"
	echo "D3 userslice cur=$(</sys/fs/cgroup/user.slice/pids.current) nr=$(</proc/loadavg)"
  done ) &ev go
echo go > $K/go &
for i in $(seq 20); do
	sleep 1
	[ -d $sc ] || { ev scope-gone-auto after=${i}s; break; }
done
if [ -d $sc ]; then
	echo 1 > $sc/cgroup.kill; ev kill
	for i in $(seq 100); do
		[ "$(cat $sc/pids.current 2>/dev/null || echo 0)" -le 0 ] && break
		sleep 0.1
	done
	ev dead pids=$(cat $sc/pids.current 2>/dev/null)
fi
sleep 8
ev cleanup
rm -f /tmp/kb-fill /dev/shm/kb-fill /home/wol/kb-fill
sleep 8
free -m
ev done
poweroff -f
