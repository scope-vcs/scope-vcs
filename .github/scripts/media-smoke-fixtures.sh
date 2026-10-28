#!/usr/bin/env bash
set -euo pipefail

fixture_dir="${1:?usage: media-smoke-fixtures.sh <directory>}"
ffmpeg -hide_banner -loglevel error -f lavfi -i color=c=blue:s=96x64 -frames:v 1 "$fixture_dir/fixture.png"
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc=size=96x64:rate=12 -t 1 \
  -pix_fmt yuv420p "$fixture_dir/fixture.mp4"
