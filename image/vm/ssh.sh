#!/bin/sh
# Run a command in the VM as root.
exec ssh -i "${WOLOS_VM:-$HOME/WolOS-vm}/vmkey" -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no \
  -o LogLevel=ERROR -o ConnectTimeout=5 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -p 2222 root@127.0.0.1 "$@"
