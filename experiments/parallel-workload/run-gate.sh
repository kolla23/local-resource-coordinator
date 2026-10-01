#!/usr/bin/env bash
# Decision gate step 2: 3 ripgrep worktrees, cargo build + cargo test, native Windows.
# Arms: P = all 3 at once (uncoordinated); S = one at a time (static cap 1).
# 3 repetitions, alternating order, cold target dirs, dependencies pre-fetched.
# Usage (Git Bash): run-gate.sh <bench-dir with wt1 wt2 wt3> <out-dir>
set -u
BENCH=${1:?bench dir}
OUT=${2:?out dir}
HERE=$(cd "$(dirname "$0")" && pwd)
RUNNER="$HERE/target/release/parallel-workload.exe"
CMD='set PATH=C:\Users\kolla\.cargo\bin;%PATH% && cargo build --locked && cargo test --locked'
mkdir -p "$OUT"

for rep in 1 2 3; do
  if [ $((rep % 2)) -eq 1 ]; then order="P S"; else order="S P"; fi
  for arm in $order; do
    rm -rf "$BENCH"/wt1/target "$BENCH"/wt2/target "$BENCH"/wt3/target
    sleep 20  # let the machine settle
    if [ "$arm" = P ]; then mode=parallel; else mode=serial; fi
    echo "=== rep $rep arm $arm ($mode) $(date -u +%FT%TZ)"
    "$RUNNER" --mode "$mode" --out "$OUT" --label "rep${rep}-${arm}" --cmd "$CMD" \
      "$BENCH/wt1" "$BENCH/wt2" "$BENCH/wt3" | grep -E "makespan|aborted|^w[0-9]|sum_of|system_|responsiveness"
  done
done
echo "=== done $(date -u +%FT%TZ)"
