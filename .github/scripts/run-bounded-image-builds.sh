#!/usr/bin/env bash
set -euo pipefail

builder="${1:?usage: run-bounded-image-builds.sh <builder-script> <components...>}"
shift
(($# > 0)) || { echo 'At least one image component is required.' >&2; exit 2; }
# Two BuildKit solves fit the four-vCPU runner without unbounded Git/apt work.
readonly max_parallel_builds=2
declare -A active=()
failed=0

record_completion() {
  local pid="$1" status="$2"
  if ((status == 0)); then
    echo "Prepared ${active[$pid]}"
  else
    echo "Image preparation failed for ${active[$pid]}" >&2
    failed=1
  fi
  unset "active[$pid]"
}

wait_for_one() {
  local count=${#active[@]} pid status
  # Explicit PID waits retain exited children's statuses. Poll this tiny pool
  # because wait -n may overlook an already-exited child and block on its sibling.
  while ((${#active[@]} == count)); do
    for pid in "${!active[@]}"; do
      kill -0 "$pid" 2>/dev/null && continue
      status=0
      wait "$pid" || status=$?
      record_completion "$pid" "$status"
    done
    if ((${#active[@]} == count)); then sleep 0.2; fi
  done
}

for component in "$@"; do
  while ((${#active[@]} >= max_parallel_builds)); do
    wait_for_one
  done
  ((failed == 0)) || break
  bash "$builder" "$component" ".railway-prepared/$component" ".railway-prepared/$component.json" &
  active[$!]="$component"
done
while ((${#active[@]} > 0)); do
  wait_for_one
done
((failed == 0))
