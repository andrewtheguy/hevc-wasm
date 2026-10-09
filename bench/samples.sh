#!/usr/bin/env bash
# bench/samples.sh NAME...: the benchmark's sample streams named, each an .h265 with FFmpeg's .h265.framemd5, copied once
# from the captures' folder of the artifacts drive (or HEVC_BENCH_SOURCE) into tmp/bench, so that no run reads the drive.
# A stream that came without its digests has them written here by the host's ffmpeg. Prints the local directory.
set -euo pipefail
cd "$(dirname "$0")/.."
local=tmp/bench source=${HEVC_BENCH_SOURCE:-}
# The drive is /mnt/dasdata on Linux and /Volumes/dasdata on a Mac.
for d in /mnt/dasdata /Volumes/dasdata; do
  [ -n "$source" ] || { [ -d $d/tmp/mac-hevc ] && source=$d/tmp/mac-hevc; } || true
done
mkdir -p $local
for name in "$@"; do
  for f in "$name.h265" "$name.h265.framemd5"; do
    [ -n "$source" ] && [ -e "$source/$f" ] || continue
    if [ ! -e "$local/$f" ] || [ "$(wc -c < "$local/$f")" != "$(wc -c < "$source/$f")" ]; then
      cp "$source/$f" "$local/$f.part" && mv "$local/$f.part" "$local/$f"
    fi
  done
  [ -e "$local/$name.h265" ] || { echo "no $name.h265 in $local and none on the artifacts drive" >&2; exit 1; }
  if [ ! -e "$local/$name.h265.framemd5" ]; then
    ffmpeg -hide_banner -loglevel error -i "$local/$name.h265" -f framemd5 "$local/$name.h265.framemd5.part" </dev/null
    mv "$local/$name.h265.framemd5.part" "$local/$name.h265.framemd5"
  fi
done
echo "$PWD/$local"
