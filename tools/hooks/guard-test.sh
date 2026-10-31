#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Runs public-guard against a throwaway repo. Fake credentials are built here so no blob holds one.
G=$(cd "$(dirname "$0")" && pwd)/public-guard
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
cd "$T"; git init -q .; git config user.email t@t; git config user.name t
cp "$G" .git/hooks/pre-commit; chmod +x .git/hooks/pre-commit
fails=0
try() { # want path content
  mkdir -p "$(dirname "$2")"; printf '%s\n' "$3" > "$2"; git add "$2"
  if git commit -qm x 2>/dev/null; then got=ok; else got=refused; git reset -q; rm -f "$2"; fi
  [ "$got" = "$1" ] && echo "pass $1 $2" || { echo "FAIL want $1 got $got: $2"; fails=$((fails+1)); }
}
B=-----BEGIN; a36=$(printf 'a%.0s' {1..36}); A16=$(printf 'A%.0s' {1..16})
try ok      kernel/linux/patches/0050-new.patch 'patch'
try ok      kernel/scx_wolos/scx_wolos.c 'code'
try ok      kernel/verify.sh 'code'
try ok      docs/keys.md 'an ssh private key is not named here'
try refused strategy/x.md 'plan'
try refused CLAUDE.local.md 'notes'
try refused test/demo-account.txt 'x'
try refused image/vm/test/demo-account.txt 'x'
try refused a/key1 "AKIA$A16"
try refused a/key2 "$B OPENSSH PRIVATE KEY-----"
try refused a/key3 "$B PRIVATE KEY-----"
try refused a/key4 "ghp_$a36"
try refused a/key5 "github_pat_$a36"
exit $fails
