#!/usr/bin/env bash
#
# Runs each test of the fixture sample alone, three times, and prints its
# result and wall time (fixture sample, docs/decision-gate/05-test-fixture-sample.md).
#
# Usage: time-sample.sh <label> [test...]   (on Linux, from the root of a
#        checkout of the commit to measure; the script may live outside that
#        checkout). Without test names it runs #41's ten sampled tests.
#
# Run it once on the commit before the tests were converted and once on the
# commit after, on the same machine. A commit without the `test-fixtures`
# feature (main before #40) is built and tested without it.

set -uo pipefail

label="${1:?give a label, e.g. before or after}"
tests=(
    a_job_that_succeeds_gives_the_exit_code_zero
    a_job_that_fails_gives_the_exit_code_one
    a_usage_error_of_a_command_that_speaks_for_a_job_uses_the_band
    a_signal_to_the_wait_gives_the_code_of_a_broken_wait
    follow_obeys_the_time_limit_of_the_reader
    a_wait_gives_the_exit_code_of_the_job
    status_with_wait_ends_with_the_record_of_the_job
    a_job_that_is_too_large_runs_when_the_queue_is_empty
    a_wait_for_many_jobs_says_why_the_later_job_waits
    the_harness_stops_a_coordinator_on_a_long_socket_path
)
shift
[ $# -gt 0 ] && tests=("$@")

features=()
grep -q '^test-fixtures *=' Cargo.toml && features=(--features test-fixtures)

cargo build -q "${features[@]}" --tests --bins || exit 1
echo "# $label: $(git log --oneline -1) on $(uname -sr), features: ${features[*]:-none}"
for t in "${tests[@]}"; do
    for run in 1 2 3; do
        start=$(date +%s.%N)
        if cargo test -q "${features[@]}" --test e2e -- --exact "$t" \
            --test-threads=1 >/dev/null 2>&1; then result=pass; else result=FAIL; fi
        end=$(date +%s.%N)
        secs=$(echo "$end - $start" | bc)
        printf '%s\t%s\trun %s\t%s\t%.1f s\n' "$label" "$t" "$run" "$result" "$secs"
    done
done
