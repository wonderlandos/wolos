#!/bin/sh
# fetch.sh REMOTE LOCAL: copy a VM file back over ssh, resuming after a cut (tail -c +N), and verify it
# against REMOTE.sha256 (written in the VM). A full-length local file with a wrong sum is refetched once.
set -eu
R=$1 L=$2
SSH=${WOLOS_SSH:-$(dirname "$0")/ssh.sh}
die() { echo "fetch: $*" >&2; exit 2; }
want=$("$SSH" "cat '$R.sha256'" | cut -d' ' -f1)
[ ${#want} -eq 64 ] || die "no sha256 for $R"
size=$("$SSH" "wc -c < '$R'" | tr -d ' ')
for pass in 1 2; do
  tries=0
  while :; do
    have=0; [ -f "$L" ] && have=$(wc -c < "$L" | tr -d ' ')
    [ "$have" -lt "$size" ] || break
    tries=$((tries + 1)); [ $tries -le 20 ] || die "gave up at $have/$size bytes"
    "$SSH" "tail -c +$((have + 1)) '$R'" >> "$L" || echo "fetch: cut at $(wc -c < "$L") bytes, resuming" >&2
  done
  if [ "$(sha256sum "$L" | cut -d' ' -f1)" = "$want" ]; then
    echo "$want  $(basename "$L")" > "$L.sha256"; exit 0
  fi
  echo "fetch: sha256 mismatch, refetching" >&2; rm -f "$L"
done
die "sha256 mismatch for $L"
