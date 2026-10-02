#!/usr/bin/env bash
#
# Tests `check-review-rounds.sh`.
#
# It needs no network, no git and no cargo, so the review job runs it before it
# reads the body of the pull request. A broken check that accepts every body
# would otherwise stay green and look like a pass.
#
# Usage: check-review-rounds-test.sh

set -uo pipefail

check="$(cd "$(dirname "$0")" && pwd)/check-review-rounds.sh"
head=e5e4a3a1fa556b367cbd6f4b7733bb892ea9e341
fail=0

# expect <name> <exit code> <body>
expect() {
    local got
    printf '%s' "$3" | "$check" "$head" >/dev/null 2>&1
    got=$?
    if [ "$got" = "$2" ]; then
        echo "ok   $1"
    else
        echo "FAIL $1: exit $got, want $2"
        fail=1
    fi
}

expect "a clean last round on the head commit passes" 0 "## Review rounds

**Round 1 @ 651295f: 2 issues.** Fixed.

**Round 2 @ e5e4a3a: no real issues.**
"

expect "a round on an earlier commit fails" 1 "## Review rounds

**Round 1 @ 651295f: no real issues.**
"

expect "a last round that found issues fails" 1 "## Review rounds

Round 1 @ e5e4a3a: 1 real issue, fixed in the next push.
"

expect "the highest round number counts, not the last line" 1 "## Review rounds

Round 2 @ 651295f: no real issues.
Round 1 @ e5e4a3a: no real issues.
"

expect "a body with no review section fails" 1 "Round 1 @ e5e4a3a: no real issues."

expect "a section with no rounds fails" 1 "## Review rounds

_(none yet)_
"

expect "a round outside the section does not count" 1 "## Review rounds

Round 1 @ 651295f: no real issues.

## Notes

Round 2 @ e5e4a3a: no real issues.
"

expect "a full commit id passes" 0 "## Review rounds
- Round 1 @ $head: no real issues
"

expect "a short id of fewer than 7 characters fails" 1 "## Review rounds
Round 1 @ e5e4a3: no real issues
"

expect "upper-case verdict and backticks still pass" 0 "## Review rounds
**Round 3 @ \`e5e4a3a\`: No real issues.**
"

expect "a CRLF body still passes" 0 "$(printf '## Review rounds\r\n\r\nRound 1 @ e5e4a3a: no real issues.\r\n')"

printf '%s' "x" | "$check" >/dev/null 2>&1
if [ "$?" = 2 ]; then echo "ok   a missing head commit is a usage error"; else echo "FAIL a missing head commit is a usage error"; fail=1; fi

exit "$fail"
