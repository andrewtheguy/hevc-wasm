#!/usr/bin/env bash
# bench/run.sh [BUILD...]: the routine benchmark, a few minutes. Each BUILD is a directory holding a build of the module
# (hevc.js, hevc.wasm), build/out when none is named, or the word ffmpeg for the host's native FFmpeg, ffmpeg-rows for it with its threads on the rows of one picture. They decode the samples of bench/samples.sh alternately, round by
# round on the same cores, on one thread and on four, and the medians over the rounds are printed per picture: the
# user-space instructions and cycles of the whole process, which do not depend on the host's load, and the mean and
# median milliseconds. The times are of runs without perf, whose counting lengthens a thread's waking by a tenth of a
# millisecond or more; the counts are of one more run of each round under perf stat. The module runs under Node, whose
# V8 is the engine of the Chrome it is for; Bun's JavaScriptCore is Safari's, which decodes the Mac's HEVC itself.
#   ROUNDS=3 THREADS="1 4" PICTURES=all SAMPLES="name ..." CPUS1=2 CPUS4=2-5
# The first build named is first checked against every sample's MD5s, and the host is waited on until it is quiet.
set -euo pipefail
cd "$(dirname "$0")/.."
[ $# -gt 0 ] || set -- build/out
rounds=${ROUNDS:-3}
# PICTURES=all, the default, is no limit: neither decoder is then given a count.
limit=() fflimit=()
[ "${PICTURES:-all}" = all ] || { limit=("$PICTURES") fflimit=(-frames:v "$PICTURES"); }
# A Mac's 1440×900 display in four strips, each picture a strip: a video over the whole of it at 1x and at 2x, Chrome
# scrolling an article at 2x, and typing in a terminal at 1x.
samples=${SAMPLES:-video-1x video-2x scroll-2x typing-1x}
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
  [[ $1 = ffmpeg || $1 = ffmpeg-rows ]] && break
  HEVC_WASM_DIR=$1 CHECK=1 node bench/decode.ts "$dir/$s.h265" 4 "${limit[@]}" | sed "s|^|$s: |"
done

log=$(mktemp tmp/bench-run.XXXXXX); trap 'rm -f "$log"' EXIT
for t in ${THREADS:-1 4}; do
  [ "$t" = 1 ] && cpus=${CPUS1:-2} || cpus=${CPUS4:-2-5}
  for s in $samples; do
    quiet; : >"$log"
    for ((r = 0; r < rounds; r++)); do
      for b in "$@"; do
        if [[ $b = ffmpeg || $b = ffmpeg-rows ]]; then
          # ffmpeg's threads each decode a picture of their own; ffmpeg-rows' share one picture's rows, as the module's do.
          rows=(); [ "$b" = ffmpeg-rows ] && rows=(-thread_type slice)
          cmd=(ffmpeg -hide_banner -nostats -threads "$t" "${rows[@]}" -i "$dir/$s.h265" "${fflimit[@]}" -benchmark -f null -)
          out=$(taskset -c "$cpus" "${cmd[@]}" 2>&1)
          n=$(grep -o 'frame= *[0-9]*' <<<"$out" | tail -1 | tr -dc 0-9)
          # Its wall time over the pictures; ffmpeg gives no time per picture, so the median is the mean.
          mean=$(grep -o 'rtime=[0-9.]*' <<<"$out" | cut -d= -f2 | awk -v n="$n" '{print $1 * 1000 / n}'); median=$mean
        else
          cmd=(env "HEVC_WASM_DIR=$b" node bench/decode.ts "$dir/$s.h265" "$t" "${limit[@]}")
          out=$(taskset -c "$cpus" "${cmd[@]}" 2>&1)
          n=$(grep -o '^[0-9]* pictures' <<<"$out" | cut -d' ' -f1)
          mean=$(grep -o 'mean [0-9.]*' <<<"$out" | cut -d' ' -f2); median=$(grep -o 'median [0-9.]*' <<<"$out" | cut -d' ' -f2)
        fi
        counted=$(taskset -c "$cpus" perf stat -e instructions:u,cycles:u -x, "${cmd[@]}" 2>&1 >/dev/null)
        i=$(grep instructions <<<"$counted" | cut -d, -f1); c=$(grep cycles <<<"$counted" | cut -d, -f1)
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
