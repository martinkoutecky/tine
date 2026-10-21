#!/bin/bash
# Baseline matrix for the PDF perf harness (evidence branch; never integrate).
#   scripts/perf-pdf/matrix.sh <outDir> [reps=3] [cpus=6-11] [phase=all|docs]
# phase docs reruns only the S2/S3 documents (docs-* and mem3-* sessions).
# Sessions (each a fresh Tine launch, pinned with taskset):
#   main-*  : all scenarios, ours then stock, light theme, DPR 1 and 2
#   dark-*  : S1a only, dark theme, DPR 1 and 2
#   mem-*   : stock only (fresh web process) for S1a + S3, so stock RSS is not
#             inflated by what Tine's own viewer left in the shared web process
# Before each session waits (max 15 min) for 1-min load < 4; each trial row
# records loadavg before/after anyway. Writes <log>.exit for wait-for.sh.
OUT=${1:?outDir}; REPS=${2:-3}; CPUS=${3:-6-11}; PHASE=${4:-all}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
LOG="$OUT/matrix.log"
mkdir -p "$OUT"
trap 'echo $? > "$LOG.exit"' EXIT
rm -f "$LOG.exit"
waitload() {
  for _ in $(seq 90); do
    l=$(cut -d' ' -f1 /proc/loadavg)
    awk -v l="$l" 'BEGIN{exit !(l<4)}' && return 0
    sleep 10
  done
  echo "$(date +%T) load still $l after 15 min; running anyway" >> "$LOG"
}
run() { # tag dpr args...
  local tag=$1 dpr=$2; shift 2
  local screen=1920x1080x24; [ "$dpr" = 2 ] && screen=3840x2160x24
  waitload
  echo "$(date +%T) start $tag load $(cut -d' ' -f1-3 /proc/loadavg)" >> "$LOG"
  (cd "$ROOT" && LANG=C.UTF-8 LC_ALL=C.UTF-8 taskset -c "$CPUS" timeout 3600 \
    xvfb-run -a -s "-screen 0 $screen" node scripts/perf-pdf/run.mjs --dpr "$dpr" --out "$OUT" --tag "$tag" "$@" \
    > "$OUT/$tag.log" 2>&1)
  local rc=$?
  echo "$(date +%T) end $tag exit $rc load $(cut -d' ' -f1-3 /proc/loadavg)" >> "$LOG"
}
if [ "$PHASE" = docs ]; then
  for r in $(seq "$REPS"); do
    run "docs-dpr1-r$r" 1 --scenarios S2,S3
    run "docs-dpr2-r$r" 2 --scenarios S2,S3
  done
  for r in $(seq "$REPS"); do
    run "mem3-dpr1-r$r" 1 --viewers stock --scenarios S3
    run "mem3-dpr2-r$r" 2 --viewers stock --scenarios S3
  done
  exit 0
fi
for r in $(seq "$REPS"); do
  run "main-dpr1-r$r" 1
  run "main-dpr2-r$r" 2
done
for r in $(seq "$REPS"); do
  run "dark-dpr1-r$r" 1 --theme dark --scenarios S1a
  run "dark-dpr2-r$r" 2 --theme dark --scenarios S1a
  run "mem-dpr1-r$r" 1 --viewers stock --scenarios S1a,S3
  run "mem-dpr2-r$r" 2 --viewers stock --scenarios S1a,S3
done
