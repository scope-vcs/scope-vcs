#!/bin/sh
set -eu

# Reproduce hosts whose reported CPU count exceeds the container allocation.
# Automatic decoder threads used to exhaust the worker's address-space limit.
exec ffmpeg -cpucount 192 "$@"
