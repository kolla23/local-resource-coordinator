#!/usr/bin/env python3
"""Changed lines per sampled test, from the git diff of tests/e2e.rs.

Fixture sample, docs/decision-gate/05-test-fixture-sample.md. A changed line
belongs to the test whose body (from its `#[test]` to its first line that is
a lone `}` at column 0, as in estimate.py) holds it in the new file; a removed
line belongs to the test that holds it in the old file. Lines outside every
sampled test (the header, the TESTJOB constant) are shown as shared.

Usage: cost-per-test.py <old commit> <new commit> [test...]   (from the repo
       root). Without test names it counts #41's ten sampled tests.
"""
import re
import subprocess
import sys

SAMPLE = [
    "a_job_that_succeeds_gives_the_exit_code_zero",
    "a_job_that_fails_gives_the_exit_code_one",
    "a_usage_error_of_a_command_that_speaks_for_a_job_uses_the_band",
    "a_signal_to_the_wait_gives_the_code_of_a_broken_wait",
    "follow_obeys_the_time_limit_of_the_reader",
    "a_wait_gives_the_exit_code_of_the_job",
    "status_with_wait_ends_with_the_record_of_the_job",
    "a_job_that_is_too_large_runs_when_the_queue_is_empty",
    "a_wait_for_many_jobs_says_why_the_later_job_waits",
    "the_harness_stops_a_coordinator_on_a_long_socket_path",
]


def spans(text):
    """{test name: (first line, last line)}, 1-based, for each test body."""
    out = {}
    lines = text.split("\n")
    starts = [i for i, line in enumerate(lines) if line == "#[test]"]
    for s in starts:
        end = next((i for i in range(s, len(lines)) if lines[i] == "}"), len(lines) - 1) + 1
        m = re.search(r"fn\s+(\w+)", "\n".join(lines[s:s + 5]))
        out[m.group(1)] = (s + 1, end)
    return out


def owner(table, line):
    for name, (a, b) in table.items():
        if a <= line <= b:
            return name if name in SAMPLE else "(other test)"
    return "(shared)"


def show(rev):
    return subprocess.run(["git", "show", f"{rev}:tests/e2e.rs"], capture_output=True,
                          text=True, check=True, encoding="utf-8").stdout


def main():
    old, new = sys.argv[1], sys.argv[2]
    if sys.argv[3:]:
        SAMPLE[:] = sys.argv[3:]
    old_spans, new_spans = spans(show(old)), spans(show(new))
    diff = subprocess.run(["git", "diff", "-U0", old, new, "--", "tests/e2e.rs"],
                          capture_output=True, text=True, check=True, encoding="utf-8").stdout
    count = {}
    for m in re.finditer(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", diff, re.M):
        o, on, n, nn = int(m[1]), int(m[2] or 1), int(m[3]), int(m[4] or 1)
        for i in range(on):
            key = owner(old_spans, o + i)
            count.setdefault(key, [0, 0])[1] += 1
        for i in range(nn):
            key = owner(new_spans, n + i)
            count.setdefault(key, [0, 0])[0] += 1
    for name in SAMPLE + ["(shared)", "(other test)"]:
        added, removed = count.get(name, [0, 0])
        print(f"{name}\t+{added}\t-{removed}\t{added + removed}")


if __name__ == "__main__":
    main()
