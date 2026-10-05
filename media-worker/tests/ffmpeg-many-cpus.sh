#!/bin/sh
set -eu

reported_cpu_count=192
exec ffmpeg -cpucount "$reported_cpu_count" "$@"
