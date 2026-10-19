#!/bin/sh
set -eu

output_dir="${1:?usage: assert-codec-output.sh PROCESSED_DIR}"

gif_frames="$(ffprobe -v error -count_frames -select_streams v:0 \
  -show_entries stream=nb_read_frames -of default=nw=1:nk=1 \
  "$output_dir/fixture.gif/image-preview.gif")"
if [ "${gif_frames:-0}" -le 1 ]; then
  echo "animated GIF preview lost its animation" >&2
  exit 1
fi

for fixture in fixture.mp4 fixture-1080p.mp4 fixture-unmapped-audio.mp4 fixture.mov fixture.webm fixture-rotated.mov fixture-hdr.mp4; do
  playback="$output_dir/$fixture/video-playback.mp4"
  codecs="$(ffprobe -v error -show_entries stream=codec_name -of default=nw=1:nk=1 "$playback")"
  printf '%s\n' "$codecs" | grep -qx h264
  if [ "$fixture" != fixture-hdr.mp4 ]; then
    audio="$(ffprobe -v error -select_streams a \
      -show_entries stream=codec_name,profile,channels -of csv=p=0 "$playback")"
    if [ "$audio" != "aac,LC,2" ]; then
      echo "$fixture playback audio is not one stereo AAC-LC stream: $audio" >&2
      exit 1
    fi
  fi
  colors="$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=color_space,color_transfer,color_primaries \
    -of default=nw=1:nk=1 "$playback")"
  bt709_count="$(printf '%s\n' "$colors" | grep -c '^bt709$')"
  if [ "$bt709_count" -ne 3 ]; then
    echo "$fixture playback is not explicitly BT.709 SDR" >&2
    exit 1
  fi
done

trace="$(mktemp)"
trap 'rm -f "$trace"' EXIT
for fixture in fixture.mp4 fixture.mov; do
  if ! ffmpeg -hide_banner -nostdin -loglevel trace \
    -i "$output_dir/$fixture/video-playback.mp4" -map 0:v:0 -c copy \
    -bsf:v trace_headers -f null - >"$trace" 2>&1; then
    echo "$fixture remuxed playback headers could not be traced" >&2
    exit 1
  fi
  nal_units="$(awk '/trace_headers/ && /nal_unit_type +[01]+ = [0-9]+$/ { count++ } END { print count + 0 }' "$trace")"
  if [ "$nal_units" -eq 0 ]; then
    echo "$fixture remuxed playback trace listed no NAL units" >&2
    exit 1
  fi
  sei_units="$(awk '/trace_headers/ && /nal_unit_type +[01]+ = 6$/ { count++ } END { print count + 0 }' "$trace")"
  if [ "$sei_units" -ne 0 ]; then
    echo "$fixture remuxed playback kept $sei_units SEI units" >&2
    exit 1
  fi
done

rotated_size="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=p=0:s=x \
  "$output_dir/fixture-rotated.mov/video-playback.mp4")"
if [ "$rotated_size" != 180x320 ]; then
  echo "rotated MOV playback has unexpected dimensions: $rotated_size" >&2
  exit 1
fi

full_hd_size="$(ffprobe -v error -select_streams v:0 \
  -show_entries stream=width,height -of csv=p=0:s=x \
  "$output_dir/fixture-1080p.mp4/video-playback.mp4")"
if [ "$full_hd_size" != 1920x1080 ]; then
  echo "1080p playback has unexpected dimensions: $full_hd_size" >&2
  exit 1
fi
