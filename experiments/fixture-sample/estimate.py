#!/usr/bin/env python3
"""Estimate the cost of porting the e2e tests, from the fixture sample.

Fixture sample, docs/decision-gate/05-test-fixture-sample.md.

1. A test is the test function itself: from its `#[test]` to its first line
   that is a lone `}` at column 0. What follows it, up to the next `#[test]`
   (helper functions, structs), is harness code and is counted separately.
   Full-line `//` comments are dropped from both before matching, so a Unix
   word in a comment doesn't count. (experiments/unix-inventory/e2e-unix.py
   took everything up to the next `#[test]` and kept comments.)
2. A test is a Unix test when it matches any category's pattern, and it is
   filed under ONE category, its most expensive matching one, so overlapping
   categories don't add up twice.
3. Each category's cost per test is the mean changed lines of the sampled tests
   chosen for it (PORT_PLAN's table), read from the cost-per-test.py output.
   "other Unix" was chosen for no sampled test, so it takes the highest cost
   measured on one test. The script prints where each sampled test lands under
   rule 2, because that can differ from the category it was chosen for.
4. The "other Unix" tests are broken down into disjoint groups (each test in
   the first group it matches), so the groups add up to the category.
5. The harness lines with a Unix pattern are counted, not priced.

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
# `Command::new("kill")`. And not `SIGNAL`, which is a word, not a signal.
SIGNAL = r"SIG(?!NAL\b)[A-Z]+\b"
# Unix tools a test starts itself or submits as a job (after `"--"`).
TOOL_NAMES = r"(?:ps|mkfifo|bwrap|lsof|chmod|cp|touch|printf)"
TOOLS = r"Command::new\(\"" + TOOL_NAMES + r"\"\)|\"--\",\s*\"" + TOOL_NAMES + r"\""
LIBC_OTHER = r"libc::(?!kill\b|SIG)\w+"

CATS = {
    "libc::kill": re.compile(r"libc::kill\b"),
    "true/false": re.compile(r'"true"|"false"'),
    "echo": re.compile(r'"echo"'),
    "sh -c / bash": re.compile(r'"sh"|"bash"'),
    "sleep": re.compile(r'"sleep"'),
    "/tmp": re.compile(r"/tmp"),
    "other Unix": re.compile(
        LIBC_OTHER + r"|libc::SIG\w+|std::os::unix|/proc|\"/bin/|\.sh\"|\"cat\"|"
        + SIGNAL + "|" + TOOLS),
}

# Disjoint groups of the "other Unix" tests, for the write-up: each test goes
# to the first group whose pattern it matches, so the counts add up to the
# category. A label names the pattern matched, not what the test does with it.
GROUPS = [
    ("matches UnixListener or UnixStream", re.compile(r"Unix(?:Listener|Stream)")),
    ("matches /proc", re.compile(r"/proc")),
    ("matches a Unix tool", re.compile(TOOLS)),
    ("matches a libc item other than kill and the SIG* constants", re.compile(LIBC_OTHER)),
    ("matches another std::os::unix item", re.compile(r"std::os::unix")),
    ("matches a \"/bin/\" path, a .sh script or \"cat\"", re.compile(r"\"/bin/|\.sh\"|\"cat\"")),
    ("matches only a signal name", re.compile(r"libc::SIG\w+|" + SIGNAL)),
]

# The product port, in lines of code that change for Windows
# (docs/decision-gate/03-unix-inventory.md, "Short answer").
PRODUCT = 3635


def code(lines):
    """The lines without full-line `//` comments."""
    return [line for line in lines if not line.lstrip().startswith("//")]


def split(text):
    """({test name: body}, harness lines): each test from its `#[test]` to its
    closing brace, and every line outside a test, comments dropped."""
    parts = re.split(r"\n#\[test\]\n", text)
    tests, harness = {}, code(parts[0].split("\n"))
    for part in parts[1:]:
        lines = part.split("\n")
        end = next((i for i, line in enumerate(lines) if line == "}"), len(lines) - 1)
        body = "\n".join(code(lines[:end + 1]))
        tests[re.search(r"fn\s+(\w+)", body).group(1)] = body
        harness += code(lines[end + 1:])
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
        return next((c for c in order if CATS[c].search(body)), None)

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
    print(f"tests: {len(tests)}; Unix tests: {unix}")

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
    print("'other Unix' tests by their first matching group (disjoint):")
    grouped = {label: 0 for label, _ in GROUPS}
    for name, body in tests.items():
        if filed[name] == "other Unix":
            label = next((lab for lab, rx in GROUPS if rx.search(body)), "(none)")
            grouped[label] = grouped.get(label, 0) + 1
    for label, n in grouped.items():
        print(f"  {label}: {n}")
    print(f"  sum: {sum(grouped.values())}")
    print(f"every Unix test at the highest sampled cost: {unix * cost['other Unix']:.0f}")
    signals = sum(1 for body in tests.values() if re.search(r"libc::kill|" + SIGNAL, body))
    print(f"tests that name a signal (libc::kill or a SIG name): {signals}")
    lines = [line for line in harness
             if any(rx.search(line) for rx in CATS.values())]
    print(f"harness and helper lines with a Unix pattern (comments excluded): {len(lines)}")


if __name__ == "__main__":
    main()
