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

for fixture in fixture.mp4 fixture-1080p.mp4 fixture.mov fixture.webm fixture-rotated.mov fixture-hdr.mp4; do
  playback="$output_dir/$fixture/video-playback.mp4"
  codecs="$(ffprobe -v error -show_entries stream=codec_name -of default=nw=1:nk=1 "$playback")"
  printf '%s\n' "$codecs" | grep -qx h264
  if [ "$fixture" != fixture-hdr.mp4 ]; then
    printf '%s\n' "$codecs" | grep -qx aac
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
