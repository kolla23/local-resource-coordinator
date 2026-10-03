#!/usr/bin/env python3
"""Estimate the cost of porting the e2e tests, from the fixture sample.

Fixture sample, docs/decision-gate/05-test-fixture-sample.md.

1. A test is the test function itself: from its `#[test]` to its first line
   that is a lone `}` at column 0. What follows it, up to the next `#[test]`
   (helper functions, structs), is harness code and is counted separately.
   (experiments/unix-inventory/e2e-unix.py took everything up to the next
   `#[test]`, so it counted helpers as part of the test above them.)
2. A test that names a Unix pattern is filed under ONE category, its most
   expensive matching one, so overlapping categories don't add up twice.
3. Each category's cost per test is the mean changed lines of the sampled tests
   chosen for it (PORT_PLAN's table), read from the cost-per-test.py output.
   "other Unix" was chosen for no sampled test, so it takes the highest cost
   measured on one test. The script prints where each sampled test lands under
   rule 2, because that can differ from the category it was chosen for.
4. The harness lines with a Unix pattern are counted, not priced.

Helpers are not followed, so a test that uses Unix only through a helper is
not counted; and "other Unix" is not capped at the highest sampled cost. So the
total is neither an upper nor a lower bound.

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

# What the "other Unix" tests hold, for the write-up. A test can use several.
# `libc::SIG*` constants are signal names (arguments to `libc::kill`), so the
# libc item excludes them.
TOOLS = r"Command::new\(\"(?:ps|mkfifo|bwrap|lsof|chmod|cp|touch|printf)\"\)"
KINDS = {
    "a Unix socket (UnixListener or UnixStream)": re.compile(r"Unix(?:Listener|Stream)"),
    "std::os::unix": re.compile(r"std::os::unix"),
    "/proc": re.compile(r"/proc"),
    "another Unix tool": re.compile(TOOLS),
    "a libc item other than kill and the SIG* constants": re.compile(
        r"libc::(?!kill\b|SIG)\w+"),
    "a signal name": re.compile(SIGNAL),
}
# The "other Unix" patterns other than signal names: a test that matches none
# of these is in the category only because it names a signal.
NOT_SIGNAL = re.compile(r"libc::(?!kill\b|SIG)\w+|std::os::unix|/proc|\"/bin/|\.sh\"|\"cat\"|"
                        + TOOLS)

UNIX = re.compile(r'"sh"|"bash"|"/bin/|"sleep"|"true"|"false"|"cat"|"echo"|libc::'
                  r'|std::os::unix|' + SIGNAL + r'|/proc|/tmp|\.sh"')

# The product port, in lines of code that change for Windows
# (docs/decision-gate/03-unix-inventory.md, "Short answer").
PRODUCT = 3635


def split(text):
    """({test name: body}, harness lines): each test from its `#[test]` to its
    closing brace, and every line outside a test."""
    parts = re.split(r"\n#\[test\]\n", text)
    tests, harness = {}, parts[0].split("\n")
    for part in parts[1:]:
        lines = part.split("\n")
        end = next((i for i, line in enumerate(lines) if line == "}"), len(lines) - 1)
        body = "\n".join(lines[:end + 1])
        tests[re.search(r"fn\s+(\w+)", body).group(1)] = body
        harness += lines[end + 1:]
    return tests, harness


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
    return cost, per_test


def main():
    commit, cost_file = sys.argv[1], sys.argv[2]
    cost, per_test = costs(cost_file)
    text = subprocess.run(["git", "show", f"{commit}:tests/e2e.rs"], capture_output=True,
                          text=True, check=True, encoding="utf-8").stdout
    tests, harness = split(text)
    # Most expensive first; ties keep a fixed order, so the output is stable.
    order = sorted(cost, key=lambda c: (-cost[c], c))

    def category(body):
        if not UNIX.search(body):
            return None
        return next((c for c in order if CATS[c].search(body)), "(none)")

    filed = {name: category(body) for name, body in tests.items()}
    counted = {c: sum(1 for f in filed.values() if f == c) for c in order}
    total = 0.0
    print("category\ttests\tlines per test\testimate")
    for c in order:
        lines = counted[c] * cost[c]
        total += lines
        print(f"{c}\t{counted[c]}\t{cost[c]:.1f}\t{lines:.0f}")
    unix = sum(counted.values())
    print(f"total\t{unix}\t\t{total:.0f}")
    print(f"tests: {len(tests)}; Unix tests: {unix}; "
          f"Unix tests in no category: {sum(1 for f in filed.values() if f == '(none)')}")

    print("\nwhere each sampled test is filed (chosen for -> filed under, lines):")
    for name, chosen in SAMPLE.items():
        print(f"  {name}: {chosen} -> {filed[name] or 'no Unix pattern'}, {per_test[name]}")

    other = counted["other Unix"] * cost["other Unix"]
    measured = [n for n in SAMPLE if filed[n] == "other Unix"]
    print(f"\n'other Unix': {counted['other Unix']} tests, {other:.0f} of {total:.0f} lines "
          f"({100 * other / total:.0f}%); {len(measured)} of them sampled, for other "
          f"categories: {', '.join(f'{per_test[n]} lines' for n in measured) or 'none'}")
    print(f"break-even: the test port reaches {PRODUCT} lines if the "
          f"{counted['other Unix']} 'other Unix' tests average "
          f"{(PRODUCT - (total - other)) / counted['other Unix']:.0f} changed lines each")
    for label, rx in KINDS.items():
        n = sum(1 for name, body in tests.items()
                if filed[name] == "other Unix" and rx.search(body))
        print(f"  'other Unix' tests that use {label}: {n}")
    only = sum(1 for name, body in tests.items()
               if filed[name] == "other Unix" and not NOT_SIGNAL.search(body))
    print(f"  'other Unix' tests there only because they name a signal: {only}")
    print(f"every Unix test at the highest sampled cost: {unix * cost['other Unix']:.0f}")
    signals = sum(1 for body in tests.values() if re.search(r"libc::kill|" + SIGNAL, body))
    print(f"tests that name a signal (libc::kill or a SIG name): {signals}")
    lines = [line for line in harness
             if UNIX.search(line) and not line.lstrip().startswith("//")]
    print(f"harness and helper lines with a Unix pattern (comments excluded): {len(lines)}")


if __name__ == "__main__":
    main()
