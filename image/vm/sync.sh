#!/bin/sh
# Copy the repo and vendored crates into the running build VM (/work/repo, /work/vendor). Shared by build-in-vm.sh and update-disk.sh. Run from the repo root after up.sh.
set -eu
cd "$(dirname "$0")/../.."
VM=${WOLOS_VM:-$HOME/WolOS-vm}
SSH=image/vm/ssh.sh
tar --exclude target --exclude .git -cf - . | $SSH 'rm -rf /work/repo && mkdir -p /work/repo && tar -C /work/repo -xf -'
# Crates are vendored on the host (cargo verifies Cargo.lock checksums) and built offline in the VM: the VM's
# user-mode network truncates crate downloads. Refreshed whenever Cargo.lock changes.
LOCK=$(sha256sum Cargo.lock | cut -d' ' -f1)
if [ "$(cat "$VM/vendor.lock" 2>/dev/null)" != "$LOCK" ]; then
  rm -rf "$VM/vendor"; cargo vendor --locked --quiet "$VM/vendor" >/dev/null; echo "$LOCK" > "$VM/vendor.lock"
fi
if [ "$($SSH 'cat /work/vendor.lock 2>/dev/null')" != "$LOCK" ]; then
  tar -C "$VM" -cf - vendor | $SSH 'rm -rf /work/vendor /work/vendor.lock && tar -C /work -xf -'
  echo "$LOCK" | $SSH 'cat > /work/vendor.lock'
fi
