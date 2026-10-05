#!/usr/bin/env bash
set -euo pipefail

marker="${1:?usage: write-release-marker.sh PATH [EPOCH_MILLISECONDS]}"
timestamp="${2-$(date +%s%3N)}"
if [[ ! "$timestamp" =~ ^[1-9][0-9]*$ ]]; then
  echo "release marker must contain positive epoch milliseconds" >&2
  exit 1
fi

temporary="$(mktemp "${marker}.XXXXXX")"
trap 'rm -f -- "$temporary"' EXIT
printf '%s\n' "$timestamp" > "$temporary"
mv -f -- "$temporary" "$marker"
