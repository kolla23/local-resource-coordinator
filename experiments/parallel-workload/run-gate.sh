#!/usr/bin/env bash
# Decision gate step 2: 3 ripgrep worktrees, cargo build + cargo test, native Windows.
# Arms: P = all 3 at once (uncoordinated); S = one at a time (static cap 1).
# 3 repetitions, alternating order, cold target dirs, dependencies pre-fetched.
# Usage (Git Bash): run-gate.sh <bench-dir with wt1 wt2 wt3> <absolute out-dir> [arms] [temp-root]
#   arms: "P S" (default, alternating order per rep) or just "P" / "S".
#   temp-root: give each workload its own fresh TEMP/TMP under this directory.
# Relative out-dir and temp-root are made absolute: the workloads run with their
# worktree as the current directory.
set -u
BENCH=${1:?bench dir}
OUT=${2:?out dir}
ARMS=${3:-P S}
TEMP_ROOT=${4:-}
HERE=$(cd "$(dirname "$0")" && pwd)
RUNNER="$HERE/target/release/parallel-workload.exe"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd -W)
if [ -n "$TEMP_ROOT" ]; then mkdir -p "$TEMP_ROOT" && TEMP_ROOT=$(cd "$TEMP_ROOT" && pwd -W); fi
# Whichever cargo this shell finds, as a Windows path for cmd.exe.
CARGO=$(command -v cargo) || { echo "cargo not found on PATH" >&2; exit 1; }
CARGO_BIN=$(cygpath -w "$(dirname "$CARGO")")
# Quoted: a ")" in the expanded %PATH% (e.g. "Program Files (x86)") would
# otherwise close the runner's "( ... )" group.
CMD="set \"PATH=$CARGO_BIN;%PATH%\" && cargo build --locked && cargo test --locked"

for rep in 1 2 3; do
  if [ "$ARMS" != "P S" ]; then order=$ARMS
  elif [ $((rep % 2)) -eq 1 ]; then order="P S"; else order="S P"; fi
  for arm in $order; do
    rm -rf "$BENCH"/wt1/target "$BENCH"/wt2/target "$BENCH"/wt3/target
    sleep 20  # let the machine settle
    if [ "$arm" = P ]; then mode=parallel; else mode=serial; fi
    echo "=== rep $rep arm $arm ($mode) $(date -u +%FT%TZ)"
    "$RUNNER" --mode "$mode" --out "$OUT" --label "rep${rep}-${arm}" --cmd "$CMD" \
      ${TEMP_ROOT:+--temp-root "$TEMP_ROOT"} \
      "$BENCH/wt1" "$BENCH/wt2" "$BENCH/wt3" | grep -E "makespan|aborted|^w[0-9]|sum_of|system_|responsiveness"
  done
done
echo "=== done $(date -u +%FT%TZ)"
