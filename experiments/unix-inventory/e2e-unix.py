#!/usr/bin/env python3
"""Count the e2e tests that use Unix commands or Unix APIs (decision gate, step 3).

A test counts when its body (the text from its `#[test]` to the next one)
names a Unix command as a program (`"sh"`, `"bash"`, `"sleep"`, `"true"`,
`"false"`, `"cat"`, `"echo"`, `"kill"`, a `"/bin/..."` path or a `.sh` script),
a Unix API (`libc::`, `std::os::unix`, a `SIG...` name) or a Unix path (`/proc`,
`/tmp`). The second count is the tests that run a shell (`"sh"`, `"bash"` or
`"-c"`). Helpers that a test calls are not followed, so a test that only uses
Unix through a helper is not counted: both numbers are lower bounds.

Usage: python experiments/unix-inventory/e2e-unix.py (from the repo root)
"""
import re
from pathlib import Path

UNIX = re.compile(r'"sh"|"bash"|"/bin/|"sleep"|"true"|"false"|"cat"|"echo"|libc::'
                  r'|std::os::unix|"kill"|SIG[A-Z]+|/proc|/tmp|\.sh"')
SHELL = re.compile(r'"sh"|"bash"|"-c"')


def main():
    text = Path("tests/e2e.rs").read_text(encoding="utf-8")
    bodies = re.split(r"\n#\[test\]\n", text)[1:]
    unix = sum(1 for b in bodies if UNIX.search(b))
    shell = sum(1 for b in bodies if SHELL.search(b))
    print(f"e2e tests: {len(bodies)}")
    print(f"using a Unix command, API or path: {unix}")
    print(f"running a shell (sh/bash/-c): {shell}")


if __name__ == "__main__":
    main()
