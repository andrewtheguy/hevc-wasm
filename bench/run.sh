#!/usr/bin/env bash
# bench/run.sh [BUILD...]: the routine benchmark, a few minutes. Each BUILD is a directory holding a build of the module
# (hevc.js, hevc.wasm), build/out when none is named, or the word ffmpeg for the host's native FFmpeg. They decode the samples of bench/samples.sh alternately, round by
# round on the same cores, on one thread and on four, and the medians over the rounds are printed per picture: the
# user-space instructions and cycles of the whole process, which do not depend on the host's load, and the mean and
# median milliseconds.
#   ROUNDS=3 THREADS="1 4" PICTURES=all SAMPLES="name ..." CPUS1=2 CPUS4=2-5
# The first build named is first checked against every sample's MD5s, and the host is waited on until it is quiet.
set -euo pipefail
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || set -- build/out
rounds=${ROUNDS:-3}
# PICTURES=all, the default, is no limit: neither decoder is then given a count.
limit=() fflimit=()
[ "${PICTURES:-all}" = all ] || { limit=("$PICTURES") fflimit=(-frames:v "$PICTURES"); }
# The 1600×1000 Mac capture, dense screen content, and the 1080p camera video x265 coded in the Mac's shape.
samples=${SAMPLES:-cap5 sample}
dir=$(bench/samples.sh $samples)

# Waits, up to ten minutes, for the 1-minute load to fall under 1.5, so no run is timed against a build or the last run.
quiet() {
  local waited=0
  while awk '{exit !($1 >= 1.5)}' /proc/loadavg; do
    [ $waited -lt 600 ] || { echo "!! load still $(cut -d' ' -f1-3 /proc/loadavg): measuring anyway"; return; }
    sleep 15; waited=$((waited + 15))
  done
}

for s in $samples; do
  [ "$1" = ffmpeg ] && break
  HEVC_WASM_DIR=$1 CHECK=1 bun bench/decode.ts "$dir/$s.h265" 4 "${limit[@]}" | sed "s|^|$s: |"
done

log=$(mktemp); trap 'rm -f "$log"' EXIT
for t in ${THREADS:-1 4}; do
  [ "$t" = 1 ] && cpus=${CPUS1:-2} || cpus=${CPUS4:-2-5}
  for s in $samples; do
    quiet; : >"$log"
    for ((r = 0; r < rounds; r++)); do
      for b in "$@"; do
        if [ "$b" = ffmpeg ]; then
          out=$(taskset -c "$cpus" perf stat -e instructions:u,cycles:u -x, ffmpeg -hide_banner -nostats -threads "$t" -i "$dir/$s.h265" "${fflimit[@]}" -benchmark -f null - 2>&1)
          n=$(grep -o 'frame= *[0-9]*' <<<"$out" | tail -1 | tr -dc 0-9)
          # Its wall time over the pictures; ffmpeg gives no time per picture, so the median is the mean.
          mean=$(grep -o 'rtime=[0-9.]*' <<<"$out" | cut -d= -f2 | awk -v n="$n" '{print $1 * 1000 / n}'); median=$mean
        else
          out=$(HEVC_WASM_DIR=$b taskset -c "$cpus" perf stat -e instructions:u,cycles:u -x, bun bench/decode.ts "$dir/$s.h265" "$t" "${limit[@]}" 2>&1)
          n=$(grep -o '^[0-9]* pictures' <<<"$out" | cut -d' ' -f1)
          mean=$(grep -o 'mean [0-9.]*' <<<"$out" | cut -d' ' -f2); median=$(grep -o 'median [0-9.]*' <<<"$out" | cut -d' ' -f2)
        fi
        i=$(grep instructions <<<"$out" | cut -d, -f1); c=$(grep cycles <<<"$out" | cut -d, -f1)
        awk -v b="$b" -v n="$n" -v i="$i" -v c="$c" -v a="$mean" -v m="$median" 'BEGIN {print b, i / n / 1e6, c / n / 1e6, a, m}' >>"$log"
      done
    done
    echo "-- $s, $n pictures, $t thread(s) on cpus $cpus, medians of $rounds rounds, load $(cut -d' ' -f1-3 /proc/loadavg)"
    for b in "$@"; do
      med() { awk -v b="$b" '$1 == b' "$log" | cut -d' ' -f"$1" | sort -n | awk '{v[NR] = $1} END {print v[int((NR + 1) / 2)]}'; }
      printf '%-24s %8.1f M instr %8.1f M cycles   mean %6.2f ms  median %6.2f ms\n' "$b" "$(med 2)" "$(med 3)" "$(med 4)" "$(med 5)"
    done
  done
done
