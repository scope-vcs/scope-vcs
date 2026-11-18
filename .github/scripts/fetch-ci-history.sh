#!/usr/bin/env bash
set -euo pipefail

checkout_sha="$(git rev-parse HEAD)"
head_sha="${HEAD_SHA:?}"
base_ref='+refs/heads/main:refs/remotes/origin/main'
if [[ "${EVENT_NAME:?}" == pull_request ]]; then
  base_ref="${PULL_REQUEST_BASE_SHA:?}"
fi

for attempt in 1 2; do
  depth=()
  if [[ "$(git rev-parse --is-shallow-repository)" == true ]]; then
    depth=(--unshallow)
  fi
  if timeout --kill-after=5s 45s git fetch --no-tags --no-recurse-submodules --no-auto-maintenance \
    "${depth[@]}" origin "$checkout_sha" "$head_sha" "$base_ref"; then
    exit 0
  fi
  echo "Comparison history fetch attempt $attempt failed" >&2
done
exit 1
