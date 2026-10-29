#!/usr/bin/env bash
# SPDX-License-Identifier: BSD-2-Clause
# Build [wolos] packages or the image on the ECS build cluster (wonderlandos/wolos-build): one task
# per package, independent packages at the same time (image/ci/plan.py waves). Hosts start for the
# run and stop after it.
#   aws-build.sh pkg <out dir> <package...>   built packages (unsigned) into <out dir>
#   aws-build.sh iso <out dir>                the ISO and its release manifest into <out dir>
# Inputs: this tree (with submodules; extra patches in ./wolos-overlay when present), the kept
# packages in $WOLOS_REPO_DIR/x86_64 (local deps; for iso: the signed repo), [wolos-cachy] from
# $WOLOS_CACHY_DIR when set (iso).
# Env: CLUSTER, TASK_DEF, TASK_DEF_ISO, CAPACITY, BUCKET (repository variables), AWS credentials.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../.." && pwd)
mode=$1 out=$2; shift 2
run="$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo local)-$(date +%s)-$mode"
s3=s3://$BUCKET/runs/$run
mkdir -p "$out"

echo "== source -> $s3"
# Extra patches travel as ./wolos-overlay (where CI checks them out); the builder finds them there.
tar -C "$REPO" --exclude=./out --exclude=./target --exclude=.git -I 'zstd -T0' -cf - . |
  aws s3 cp --quiet - "$s3/src.tar.zst"
rd=${WOLOS_REPO_DIR:-$REPO/out/repo}/x86_64
if compgen -G "$rd/*" >/dev/null; then
  aws s3 sync --quiet "$rd/" "$s3/repo/"
fi
if [ "$mode" = iso ] && [ -n "${WOLOS_CACHY_DIR:-}" ]; then
  aws s3 sync --quiet "$WOLOS_CACHY_DIR/x86_64/" "$s3/cachy/"
fi

# start <task def> <cpu> <memory> <env json...>: the task ARN.
start() {
  local def=$1 cpu=$2 mem=$3; shift 3
  local env; env=$(printf '%s,' "$@"); env=${env%,}
  aws ecs run-task --cluster "$CLUSTER" --task-definition "$def" \
    --capacity-provider-strategy "capacityProvider=$CAPACITY,weight=1" \
    --overrides "{\"cpu\":\"$cpu\",\"memory\":\"$mem\",\"containerOverrides\":[{\"name\":\"builder\",\"cpu\":$cpu,\"memory\":$mem,\"environment\":[$env]}]}" \
    --query 'tasks[0].taskArn' --output text
}
kv() { printf '{"name":"%s","value":"%s"}' "$1" "$2"; }

# wait_all <arn...>: until every task stopped (hosts may take minutes to start); fail on any non-zero.
wait_all() {
  until aws ecs wait tasks-stopped --cluster "$CLUSTER" --tasks "$@" 2>/dev/null; do :; done
  local bad
  bad=$(aws ecs describe-tasks --cluster "$CLUSTER" --tasks "$@" \
    --query 'tasks[?containers[0].exitCode!=`0`].[overrides.containerOverrides[0].environment[?name==`PKG`].value|[0],containers[0].exitCode,stoppedReason]' \
    --output text)
  if [ -n "$bad" ]; then
    echo "failed: $bad (logs: aws logs tail /wolos-build)" >&2
    return 1
  fi
}

# lost <arn...>: packages whose host went away under them (Spot reclaim): no exit code at all.
lost() {
  aws ecs describe-tasks --cluster "$CLUSTER" --tasks "$@" \
    --query 'tasks[?containers[0].exitCode==null].overrides.containerOverrides[0].environment[?name==`PKG`].value|[]' \
    --output text
}

# wave <package...>: build them at once; a package whose host was reclaimed runs once more.
wave() {
  local try p cpu mem tasks
  for try in 1 2; do
    tasks=()
    for p in "$@"; do
      read -r cpu mem < <(python3 -c "import sys; sys.path.insert(0, '$HERE'); import plan; print(*plan.size('$p'))")
      tasks+=("$(start "$TASK_DEF" "$cpu" "$mem" "$(kv MODE pkg)" "$(kv PKG "$p")" "$(kv RUN "$run")" "$(kv BUCKET "$BUCKET")")")
      echo "  $p: ${tasks[-1]##*/}"
    done
    until aws ecs wait tasks-stopped --cluster "$CLUSTER" --tasks "${tasks[@]}" 2>/dev/null; do :; done
    local again; again=$(lost "${tasks[@]}")
    if [ "$try" = 1 ] && [ -n "$again" ] && [ "$again" != None ]; then
      # A real failure next to a reclaimed host still fails the wave.
      local bad
      bad=$(aws ecs describe-tasks --cluster "$CLUSTER" --tasks "${tasks[@]}" \
        --query 'tasks[?containers[0].exitCode!=null && containers[0].exitCode!=`0`].[overrides.containerOverrides[0].environment[?name==`PKG`].value|[0],containers[0].exitCode]' \
        --output text)
      if [ -n "$bad" ]; then
        echo "failed: $bad (logs: aws logs tail /wolos-build)" >&2
        return 1
      fi
      echo "  host reclaimed under: $again; building them again"
      # shellcheck disable=SC2086 # one package name per word
      set -- $again
      continue
    fi
    wait_all "${tasks[@]}"
    return
  done
}

t0=$(date +%s)
case $mode in
pkg)
  python3 "$HERE/plan.py" "$REPO/image/repo/packages" "$@" | while read -r wave; do
    echo "== wave: $wave"
    # shellcheck disable=SC2086 # one package name per word
    wave $wave
  done
  aws s3 sync --quiet "$s3/out/" "$out/"
  ;;
iso)
  arn=$(start "$TASK_DEF_ISO" 31744 57600 "$(kv MODE iso)" "$(kv RUN "$run")" "$(kv BUCKET "$BUCKET")" \
    "$(kv WOLOS_IMAGE_URL "${WOLOS_IMAGE_URL:-}")")
  echo "== iso: ${arn##*/}"
  wait_all "$arn"
  aws s3 sync --quiet "$s3/iso/" "$out/"
  ;;
*)
  echo "usage: aws-build.sh pkg|iso <out dir> [package...]" >&2
  exit 2
  ;;
esac
echo "== $mode done in $(( $(date +%s) - t0 ))s"
ls -l "$out"
