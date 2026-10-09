#!/usr/bin/env bash
# bench/samples.sh NAME...: the benchmark's sample streams named, each an .h265 with FFmpeg's .h265.framemd5, copied once
# from the bench folder of the artifacts drive (or HEVC_BENCH_SOURCE) into tmp/bench, so that no decode reads the drive:
# a copy is made again only when it differs from the drive's. Prints the local directory. What the samples are is in
# that folder's README.
set -euo pipefail
cd "$(dirname "$0")/.."
local=tmp/bench source=${HEVC_BENCH_SOURCE:-}
[ -z "$source" ] || [ -d "$source" ] || { echo "HEVC_BENCH_SOURCE is no directory: $source" >&2; exit 1; }
# The drive is /mnt/dasdata on Linux and /Volumes/dasdata on a Mac.
for d in /mnt/dasdata /Volumes/dasdata; do
  [ -n "$source" ] || { [ -d $d/backup/hevc-artifacts/bench ] && source=$d/backup/hevc-artifacts/bench; } || true
done
mkdir -p $local
if [ -n "$source" ]; then
  for name in "${@/%/.h265}" "${@/%/.h265.framemd5}"; do
    f=$source/$name
    [ -e "$f" ] || { echo "no $name in $source" >&2; exit 1; }
    if ! cmp -s "$f" "$local/$name"; then
      cp "$f" "$local/$name.part" && mv "$local/$name.part" "$local/$name"
    fi
  done
fi
for name in "$@"; do
  [ -e "$local/$name.h265" ] || { echo "no $name in $local and no bench folder on the artifacts drive" >&2; exit 1; }
done
echo "$PWD/$local"
