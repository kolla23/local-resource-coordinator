#!/usr/bin/env bash
#
# Runs one e2e test alone N times and prints its wall times, marking a failed
# run with F (fixture sample, docs/decision-gate/05-test-fixture-sample.md).
# Used to tell a real runtime change from noise when a mean of three moves.
#
# Usage: repeat-one.sh <test name> <runs>   (on Linux, from the root of a
#        checkout of the commit to measure)

set -uo pipefail

test="${1:?give a test name}"
runs="${2:-10}"
features=()
grep -q '^test-fixtures *=' Cargo.toml && features=(--features test-fixtures)
cargo build -q "${features[@]}" --tests --bins || exit 1

line="$(git log --oneline -1 | cut -c1-7):"
for _ in $(seq "$runs"); do
    start=$(date +%s.%N)
    mark=""
    cargo test -q "${features[@]}" --test e2e -- --exact "$test" --test-threads=1 \
        >/dev/null 2>&1 || mark=F
    end=$(date +%s.%N)
    line="$line $(printf '%.1f' "$(echo "$end - $start" | bc)")$mark"
done
echo "$line"
