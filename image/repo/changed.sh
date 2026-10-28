#!/usr/bin/env bash
# changed.sh < PATHS: the [wolos] packages to rebuild for these changed repo paths (one per line,
# e.g. `git diff --name-only A B`), each package plus every package that depends on it, on one line
# in build order (image/repo/packages). Prints nothing when no package is touched.
set -euo pipefail
T=$(dirname "$0")/packages
mapfile -t paths
declare -A want=()
while read -r name _dir deps watched; do
  case $name in '' | '#'*) continue ;; esac
  for d in ${deps//,/ }; do [ -n "${want[$d]:-}" ] && want[$name]=1; done
  for w in $watched; do
    for p in "${paths[@]}"; do
      case $p in "$w"*) want[$name]=1 ;; esac
    done
  done
done < "$T"
out=
while read -r name _; do
  case $name in '' | '#'*) continue ;; esac
  [ -n "${want[$name]:-}" ] && out="$out $name"
done < "$T"
echo "${out# }"
