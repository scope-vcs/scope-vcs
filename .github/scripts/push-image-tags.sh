#!/usr/bin/env bash
set -euo pipefail
# Pushes newline-separated local image tags from stdin. Repeating a push of the
# same local image is idempotent, so registry transport failures are retried.
while IFS= read -r tag; do
  for attempt in 1 2 3; do
    docker push "$tag" && continue 2
    if ((attempt < 3)); then
      echo "Image push failed; retrying ($attempt/3)." >&2
      sleep $((attempt * 5))
    fi
  done
  echo "Image push failed after 3 attempts: $tag" >&2
  exit 1
done
