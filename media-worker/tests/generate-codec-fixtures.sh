#!/bin/sh
set -eu

output_dir="${1:?usage: generate-codec-fixtures.sh OUTPUT_DIR}"
mkdir -p "$output_dir"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "color=c=0x336699:size=320x180:rate=1" \
  -frames:v 1 "$output_dir/fixture.png"
ffmpeg -hide_banner -loglevel error -y \
  -i "$output_dir/fixture.png" -frames:v 1 "$output_dir/fixture.jpg"
ffmpeg -hide_banner -loglevel error -y \
  -i "$output_dir/fixture.png" -frames:v 1 -c:v libwebp "$output_dir/fixture.webp"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=320x180:rate=6:duration=1" \
  -c:v gif -loop 0 "$output_dir/fixture.gif"
heif-enc -q 80 -o "$output_dir/fixture.heic" "$output_dir/fixture.png" >/dev/null

# Insert a standards-compliant little-endian EXIF orientation=6 APP1 segment.
{
  printf '\377\330'
  printf '\377\341\000\042Exif\000\000II\052\000\010\000\000\000\001\000\022\001\003\000\001\000\000\000\006\000\000\000\000\000\000\000'
  tail -c +3 "$output_dir/fixture.jpg"
} > "$output_dir/fixture-oriented.jpg"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=320x180:rate=12:duration=1" \
  -f lavfi -i "sine=frequency=880:sample_rate=48000:duration=1" \
  -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest -movflags +faststart \
  "$output_dir/fixture.mp4"
# Exercise decoder and encoder allocations at the maximum playback resolution.
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=1920x1080:rate=30:duration=8" \
  -f lavfi -i "sine=frequency=1000:sample_rate=48000:duration=8" \
  -c:v libx264 -threads 2 -preset ultrafast \
  -b:v 17M -minrate 17M -maxrate 17M -bufsize 34M \
  -x264-params 'nal-hrd=cbr:force-cfr=1' \
  -c:a aac -b:a 128k -shortest -movflags +faststart \
  "$output_dir/fixture-1080p.mp4"
ffmpeg -hide_banner -loglevel error -y \
  -i "$output_dir/fixture.mp4" -c copy "$output_dir/fixture.mov"
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=320x180:rate=12:duration=1" \
  -f lavfi -i "sine=frequency=660:sample_rate=48000:duration=1" \
  -c:v libvpx-vp9 -deadline realtime -cpu-used 8 -row-mt 1 \
  -c:a libopus -shortest "$output_dir/fixture.webm"
if ! ffmpeg -hide_banner -loglevel error -y \
  -display_rotation:v:0 90 -i "$output_dir/fixture.mp4" -c copy \
  "$output_dir/fixture-rotated.mov"; then
  ffmpeg -hide_banner -loglevel error -y \
    -i "$output_dir/fixture.mp4" -c copy -metadata:s:v:0 rotate=90 \
    "$output_dir/fixture-rotated.mov"
fi

# Mark a 10-bit HEVC source as BT.2020/PQ so the worker exercises its HDR tone map.
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=320x180:rate=12:duration=1" \
  -vf "format=yuv420p10le" \
  -c:v libx265 -preset ultrafast -x265-params log-level=error \
  -color_primaries bt2020 -color_trc smpte2084 -colorspace bt2020nc -an \
  "$output_dir/fixture-hdr.mp4"

ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "color=c=black:size=8000x7000:rate=1" \
  -frames:v 1 -compression_level 9 "$output_dir/fixture-oversize.png"
printf '\377\330\377corrupt-payload' > "$output_dir/corrupt.jpg"
printf 'extension is not a signature' > "$output_dir/spoof.jpg"
