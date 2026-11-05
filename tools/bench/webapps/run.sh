# shellcheck shell=sh
# SPDX-License-Identifier: GPL-2.0-or-later
# Run the web-app benchmark (webapps.py) in an installed WolOS VM driven by wolvm, from the build
# host: builds the input probe (`vk`), ships them with the runner into the guest, runs it as a transient
# unit (it takes about an hour per mode), and writes $OUT/result.json and $OUT/result.md.
# Usage: sh run.sh VM [runner args...], for example `sh run.sh ms --only atlas,code --idle 30`.
# Env: WOLVM (the wolvm script), WOLVM_HOME (its VM home), OUT (default ./webapps-out),
# WOLBENCH_PASSWORD (unlocks a locked test session), WOLKIT_TODOS + WOLKIT_LIB (optional: a
# release build of the Wolkit todos example and the directory with libwolkit.so.1 and
# libwol.so.1, measured as the native Wolkit row), SERVO_DIR (optional: the directory with the
# Rust browser engine's `servoshell` and its `resources`, measured as the candidate row).
set -eu
vm=${1:?usage: sh run.sh VM [runner args...]}
shift
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
wolvm=${WOLVM:-$root/../wolos-iso/tests/wolvm/wolvm}
out=${OUT:-$PWD/webapps-out}
mkdir -p "$out"
work=$(mktemp -d)
srv=
cleanup() {
	[ -n "$srv" ] && kill "$srv" 2>/dev/null
	rm -rf "$work"
}
trap cleanup EXIT INT TERM

guest() {
	"$wolvm" ssh --timeout 600 "$vm" "$1"
}

sh "$root/design/tools/perf/build.sh"
mkdir -p "$work/p/bin" "$work/p/lib"
cp "$here/webapps.py" "$here/apps.json" "$here/features.js" "$work/p/"
cp "$root/design/tools/perf/bin/vk" "$work/p/bin/"
if [ -n "${WOLKIT_TODOS:-}" ]; then
	cp "$WOLKIT_TODOS" "$work/p/bin/todos"
	cp "${WOLKIT_LIB:?WOLKIT_LIB: directory with libwolkit.so.1}"/libwolkit.so.1 \
		"$WOLKIT_LIB"/libwol.so.1 "$work/p/lib/"
fi
if [ -n "${SERVO_DIR:-}" ]; then
	cp -r "$SERVO_DIR" "$work/p/servo"
fi
tar -C "$work/p" -cf "$work/payload.tar" .

# The guest fetches the payload from the host (QEMU user networking: the host is 10.0.2.2).
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$work" >/dev/null 2>&1 &
srv=$!
sleep 1
guest "rm -rf /var/tmp/wolbench && mkdir -p /var/tmp/wolbench && curl -fsS http://10.0.2.2:$port/payload.tar | tar -x -C /var/tmp/wolbench && echo payload ok"
kill "$srv" 2>/dev/null || true
srv=

# A locked session is unlocked with WOLBENCH_PASSWORD (a test account's password).
args=
[ -n "${WOLBENCH_PASSWORD:-}" ] && args="-E 'WOLBENCH_PASSWORD=$WOLBENCH_PASSWORD'"
run=
for a in "$@"; do
	run="$run '$a'"
done
guest "systemctl reset-failed wolbench-webapps 2>/dev/null; systemd-run --unit=wolbench-webapps --collect $args python3 -I /var/tmp/wolbench/webapps.py run --payload /var/tmp/wolbench --apps /var/tmp/wolbench/apps.json --out /var/tmp/wolbench/result.json $run"
while :; do
	sleep 30
	state=$(guest "systemctl is-active wolbench-webapps" 2>/dev/null | tail -n 1)
	guest "journalctl -u wolbench-webapps --no-pager -n 1 -o cat" 2>/dev/null | tail -n 1
	case $state in
		active | activating) ;;
		*) break ;;
	esac
done

# The result travels base64-encoded between markers, so nothing else on the line can break it.
guest "echo WOLBENCH-BEGIN; base64 -w0 /var/tmp/wolbench/result.json; echo; echo WOLBENCH-END" |
	sed -n '/^WOLBENCH-BEGIN$/,/^WOLBENCH-END$/p' | sed '1d;$d' | base64 -d >"$out/result.json"
guest "journalctl -u wolbench-webapps --no-pager -o cat" >"$out/runner.log" 2>&1 || true
vmdir=${WOLVM_HOME:-$HOME/.local/share/wolvm}/vms/$vm
python3 -I - "$out/result.json" "$vmdir/vm.json" <<'PY'
import hashlib, json, os, sys
res, vmj = sys.argv[1], sys.argv[2]
r = json.load(open(res))
v = json.load(open(vmj)) if os.path.exists(vmj) else {}
iso = v.get('iso', '')
sha = ''
if iso and os.path.exists(iso + '.sha256'):
    sha = open(iso + '.sha256').read().split()[0]
elif iso and os.path.exists(iso):
    h = hashlib.sha256()
    with open(iso, 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    sha = h.hexdigest()
r['vm'] = {k: v.get(k) for k in ('name', 'mem', 'smp', 'gpu', 'res', 'kvm', 'tpm')}
r['vm']['iso'] = os.path.basename(iso)
r['vm']['iso_sha256'] = sha
r['vm']['host_cpu'] = next((l.split(':', 1)[1].strip() for l in open('/proc/cpuinfo') if l.startswith('model name')), '')
json.dump(r, open(res, 'w'), indent=1)
PY
python3 -I "$here/webapps.py" report "$out/result.json" >"$out/result.md"
echo "wrote $out/result.json $out/result.md"
