#!/bin/sh
# SPDX-License-Identifier: GPL-2.0-or-later
# One Discord budget run, driven from the VM host with wolvm: fresh start, focused idle, focused
# typing (real key events through QEMU), background (another workspace) for BG seconds, back in
# front. msx.py samples land in OUT/LABEL.jsonl with the phase added.
# Usage: phases.sh VM LABEL [BG_SECS]
# Env: WOLVM (wolvm command, default `python3 wolvm`), OUT (default ./out), G (guest directory
# with msx.py, as-user.sh and duty.sh, default /var/tmp/msx), LAUNCH (what the compositor starts,
# default `discord`), and the layers under test, all optional:
#   FRONT_HIGH / BACK_HIGH   memory.high of the app's scope in front / in the background (e.g. 100M)
#   RAMP=MiB                 lower memory.high to its value step by step (MiB a second), not at once
#   SWAP_MAX                 memory.swap.max of the scope
#   BG_RECLAIM=1             memory.reclaim of the whole scope plus a page-out on entering the background
#   GOVERN=TARGET:FLOOR      run govern.py (MiB) from the end of the start to the end of the run
#   CDP=PORT                 the app listens for DevTools on PORT: cdp.py probes front (frame) and
#                            behind (eval: how long a message waits) into OUT/LABEL-*.cdp
#   DUTY=STOP:RUN            in the background, frozen STOP s, thawed RUN s, repeated (duty.sh)
set -eu
vm=${1:?vm}
label=${2:?label}
bg=${3:-600}
V=${WOLVM:-python3 wolvm}
G=${G:-/var/tmp/msx}
out=${OUT:-./out}
mkdir -p "$out"
f=$out/$label.jsonl
: >"$f"
g() { $V ssh "$vm" "$1"; }
u() { g "sh $G/as-user.sh $1"; }
mark() { g "python3 -I $G/msx.py sample Discord" | grep '^{' | sed "s/^{/{\"phase\": \"$1\", \"label\": \"$label\", /" >>"$f" || true; }
watch() { # phase secs every
	end=$(($(date +%s) + $2))
	while [ "$(date +%s)" -lt "$end" ]; do mark "$1"; sleep "$3"; done
}
scope() { echo "/sys/fs/cgroup\$(python3 -I $G/msx.py scope Discord)"; }
high() { # memory.high, at once or (RAMP=MiB) stepping down from memory.current by RAMP MiB a second
	[ -n "$1" ] || return 0
	if [ -n "${RAMP:-}" ]; then
		g "s=$(scope); want=\$(numfmt --from=iec $1); h=\$(cat \$s/memory.current); while [ \$h -gt \$want ]; do h=\$((h - ${RAMP} * 1048576)); [ \$h -lt \$want ] && h=\$want; echo \$h > \$s/memory.high; sleep 1; done; echo \$want > \$s/memory.high" || true
	else
		g "echo $1 > $(scope)/memory.high" || true
	fi
}
g "systemctl stop msx-govern; systemctl stop msx-duty; pkill -f govern.p[y]; pkill -f duty.s[h]; pkill -u 1000 -x Discord; pkill -u 1000 -f /opt/helium/heliu[m]; sleep 5; pgrep -u 1000 -x Discord && pkill -9 -u 1000 -x Discord; true" >/dev/null 2>&1 || true
u "hyprctl dispatch workspace 1" >/dev/null
u "hyprctl dispatch exec ${LAUNCH:-discord}" >/dev/null
sleep 45
g "pkill -u 1000 -f /opt/helium/heliu[m]; true" >/dev/null 2>&1 || true
[ -n "${SWAP_MAX:-}" ] && g "echo $SWAP_MAX > $(scope)/memory.swap.max"
high "${FRONT_HIGH:-}"
[ -n "${GOVERN:-}" ] && g "systemd-run --unit=msx-govern --collect python3 -I $G/govern.py Discord ${GOVERN%%:*} ${GOVERN##*:}" >/dev/null
u "hyprctl dispatch focuswindow class:discord" >/dev/null
watch launch 15 5
watch front-idle 60 10
[ -n "${CDP:-}" ] && g "timeout 120 python3 -I $G/cdp.py frame $CDP 20 1" >"$out/$label-front.cdp" 2>&1
# Focused and in use: real key presses into the focused field for about 30 s.
for i in 1 2 3 4 5 6; do
	$V type "$vm" "hello from msx $i " >/dev/null 2>&1 || true
	mark front-typing
	sleep 3
done
$V screenshot "$vm" "$out/$label-typing.png" >/dev/null 2>&1 || true
u "hyprctl dispatch workspace 2" >/dev/null
high "${BACK_HIGH:-}"
[ -n "${BG_RECLAIM:-}" ] && g "s=$(scope); echo \$(cat \$s/memory.current) > \$s/memory.reclaim 2>/dev/null; true"
[ -n "${DUTY:-}" ] && g "systemd-run --unit=msx-duty --collect sh $G/duty.sh $(scope) ${DUTY%%:*} ${DUTY##*:}" >/dev/null
# The probe runs as a guest unit and is read back after: the guest agent takes one caller at a time,
# so a probe held open from here would turn the samples below into "no way into the guest" errors.
[ -n "${CDP:-}" ] && g "systemctl stop msx-cdp 2>/dev/null; rm -f $G/cdp-bg.out; systemd-run --unit=msx-cdp --collect sh -c 'timeout $((bg + 60)) python3 -I $G/cdp.py eval $CDP $((bg / 4)) 3 > $G/cdp-bg.out 2>&1'" >/dev/null
watch bg "$bg" 15
[ -n "${CDP:-}" ] && { sleep 60; g "cat $G/cdp-bg.out" >"$out/$label-bg.cdp" 2>&1 || true; }
g "systemctl stop msx-duty 2>/dev/null; echo 0 > $(scope)/cgroup.freeze" >/dev/null 2>&1 || true
u "hyprctl dispatch workspace 1" >/dev/null
u "hyprctl dispatch focuswindow class:discord" >/dev/null
high "${FRONT_HIGH:-}"
[ -n "${CDP:-}" ] && g "timeout 120 python3 -I $G/cdp.py frame $CDP 10 1" >"$out/$label-back.cdp" 2>&1
watch back 60 10
g "systemctl stop msx-govern 2>/dev/null; true" >/dev/null 2>&1 || true
$V screenshot "$vm" "$out/$label-back.png" >/dev/null 2>&1 || true
echo "done $f"
