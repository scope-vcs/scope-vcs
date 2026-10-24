#!/usr/bin/env bash
set -euo pipefail

shopt -s nullglob
for source in /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
  if ! grep -Eq '(archive|ports|security)\.ubuntu\.com|apt-mirrors\.txt' "$source"; then
    rm -- "$source"
    echo "Disabled apt source $source"
  fi
done
