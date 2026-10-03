#!/usr/bin/env python3
"""Estimate the cost of porting the e2e tests, from the fixture sample.

Fixture sample, docs/decision-gate/05-test-fixture-sample.md.

1. Each test (a body from its `#[test]` to the next, as in
   experiments/unix-inventory/e2e-unix.py) is matched against the categories.
2. A test that matches several categories is counted ONCE, under the most
   expensive one, so overlapping categories don't add up twice.
3. Each category's cost per test is the mean of the changed lines measured on
   its sampled tests, read from the cost-per-test.py output. The "other Unix"
   category has no sample, so it takes the highest cost measured on one test.
4. Lines with a Unix pattern OUTSIDE the tests (the harness and helpers) are
   counted separately: the sample does not price them.

Helpers are not followed, so a test that uses Unix only through a helper is
not counted: the total is a lower bound.

Usage: estimate.py <commit> <cost-per-test.tsv>   (from the repo root; the
       commit is the one before the sample was converted)
"""
import re
import subprocess
import sys

# The category each sampled test was chosen for (PORT_PLAN's sample table).
SAMPLE = {
    "a_job_that_succeeds_gives_the_exit_code_zero": "true/false",
    "a_job_that_fails_gives_the_exit_code_one": "true/false",
    "a_usage_error_of_a_command_that_speaks_for_a_job_uses_the_band": "true/false",
    "a_signal_to_the_wait_gives_the_code_of_a_broken_wait": "sleep",
    "follow_obeys_the_time_limit_of_the_reader": "sleep",
    "a_wait_gives_the_exit_code_of_the_job": "sh -c / bash",
    "status_with_wait_ends_with_the_record_of_the_job": "sh -c / bash",
    "a_job_that_is_too_large_runs_when_the_queue_is_empty": "echo",
    "a_wait_for_many_jobs_says_why_the_later_job_waits": "libc::kill",
    "the_harness_stops_a_coordinator_on_a_long_socket_path": "/tmp",
}

# Not `"kill"`: in tests/e2e.rs that is qex's own `kill` subcommand
# (`h.ok(&["kill", &id])`), never a Unix program; no test runs
# `Command::new("kill")`. And not `SIGNAL`, which is a word in comments and
# in a completion string, not a signal.
SIGNAL = r"SIG(?!NAL\b)[A-Z]+\b"

CATS = {
    "libc::kill": re.compile(r"libc::kill\b"),
    "true/false": re.compile(r'"true"|"false"'),
    "echo": re.compile(r'"echo"'),
    "sh -c / bash": re.compile(r'"sh"|"bash"'),
    "sleep": re.compile(r'"sleep"'),
    "/tmp": re.compile(r"/tmp"),
    "other Unix": re.compile(
        r"libc::(?!kill\b)\w+|std::os::unix|/proc|\"/bin/|\.sh\"|\"cat\""
        r"|" + SIGNAL +
        r"|Command::new\(\"(?:ps|mkfifo|bwrap|lsof|chmod|cp|touch|printf)\"\)"),
}
# The product port, in lines of code that change for Windows
# (docs/decision-gate/03-unix-inventory.md, "Short answer").
PRODUCT = 3635

# What the "other Unix" tests hold, for the write-up. A test can use several.
KINDS = {
    "std::os::unix": re.compile(r"std::os::unix"),
    "/proc": re.compile(r"/proc"),
    "another Unix tool": re.compile(
        r"Command::new\(\"(?:ps|mkfifo|bwrap|lsof|chmod|cp|touch|printf)\"\)"),
    "another libc call": re.compile(r"libc::(?!kill\b)\w+"),
    "a signal name": re.compile(SIGNAL),
}

UNIX = re.compile(r'"sh"|"bash"|"/bin/|"sleep"|"true"|"false"|"cat"|"echo"|libc::'
                  r'|std::os::unix|' + SIGNAL + r'|/proc|/tmp|\.sh"')


def costs(path):
    """Mean changed lines per category, from cost-per-test.py's output."""
    per_test = {}
    for line in open(path, encoding="utf-8"):
        name, _added, _removed, total = line.rstrip("\n").split("\t")
        per_test[name] = int(total)
    missing = sorted(set(SAMPLE) - set(per_test))
    if missing:
        sys.exit(f"cost-per-test output has no line for: {missing}")
    cost = {}
    for cat in set(SAMPLE.values()):
        values = [per_test[t] for t, c in SAMPLE.items() if c == cat]
        cost[cat] = sum(values) / len(values)
    cost["other Unix"] = float(max(per_test[t] for t in SAMPLE))
    return cost


def main():
    commit, cost_file = sys.argv[1], sys.argv[2]
    cost = costs(cost_file)
    text = subprocess.run(["git", "show", f"{commit}:tests/e2e.rs"], capture_output=True,
                          text=True, check=True, encoding="utf-8").stdout
    parts = re.split(r"\n#\[test\]\n", text)
    tests = parts[1:]
    # Most expensive first; ties keep a fixed order, so the output is stable.
    order = sorted(cost, key=lambda c: (-cost[c], c))
    counted = {c: 0 for c in cost}
    unmatched = 0
    for body in tests:
        if not UNIX.search(body):
            continue
        hit = [c for c in order if CATS[c].search(body)]
        if hit:
            counted[hit[0]] += 1
        else:
            unmatched += 1
    total = 0.0
    print("category\ttests\tlines per test\testimate")
    for c in order:
        lines = counted[c] * cost[c]
        total += lines
        print(f"{c}\t{counted[c]}\t{cost[c]:.1f}\t{lines:.0f}")
    print(f"total\t{sum(counted.values())}\t\t{total:.0f}")
    print(f"every Unix test at the highest sampled cost: "
          f"{sum(counted.values()) * cost['other Unix']:.0f} (not an upper bound: "
          f"'other Unix' has no sample)")

    # The share that rests on the unsampled category, what it holds, and the
    # cost per test at which the test port would reach the product port.
    other = counted["other Unix"] * cost["other Unix"]
    print(f"'other Unix' share of the total: {other:.0f} of {total:.0f} "
          f"({100 * other / total:.0f}%), with no sample")
    if counted["other Unix"]:
        rest = total - other
        print(f"break-even: the test port reaches {PRODUCT} lines if the "
              f"{counted['other Unix']} 'other Unix' tests average "
              f"{(PRODUCT - rest) / counted['other Unix']:.0f} changed lines each")
    for label, rx in KINDS.items():
        n = sum(1 for b in tests if UNIX.search(b) and CATS["other Unix"].search(b)
                and not any(CATS[c].search(b) for c in order if cost[c] > cost["other Unix"])
                and rx.search(b))
        print(f"  'other Unix' tests that use {label}: {n}")
    print(f"tests: {len(tests)}; Unix tests: {sum(counted.values()) + unmatched}; "
          f"Unix tests in no category: {unmatched}")
    signals = sum(1 for b in tests if re.search(r"libc::kill|" + SIGNAL, b))
    print(f"tests that name a signal (libc::kill or a SIG name): {signals}")

    # Harness and helpers: every line outside a test body that has a Unix pattern.
    outside = parts[0].split("\n")
    for body in tests:
        # A test body ends at its first line that is a lone `}` at column 0;
        # what follows, up to the next `#[test]`, is helpers and other items.
        lines = body.split("\n")
        end = next((i for i, line in enumerate(lines) if line == "}"), len(lines) - 1)
        outside += lines[end + 1:]
    harness = [line for line in outside
               if UNIX.search(line) and not line.lstrip().startswith("//")]
    print(f"harness and helper lines with a Unix pattern (comments excluded): {len(harness)}")


if __name__ == "__main__":
    main()
