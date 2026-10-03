#!/usr/bin/env bash
#
# Tests `check-reviewed.sh`. It needs no network, so the workflow runs it
# before it reads the pull request: a broken check that passes every body
# would otherwise look like a pass.
#
# Usage: check-reviewed-test.sh

set -uo pipefail

check="$(cd "$(dirname "$0")" && pwd)/check-reviewed.sh"
head=e5e4a3a1fa556b367cbd6f4b7733bb892ea9e341
old=651295f0aa556b367cbd6f4b7733bb892ea9e341
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

expect "a clean review of the head commit passes" 0 "Some text.

Reviewed at $head: no real issues
"
expect "a review of an earlier commit fails" 1 "Reviewed at $old: no real issues"
expect "a review that found issues fails" 1 "Reviewed at $head: 1 real issue"
expect "a verdict that only mentions the words later fails" 1 \
    "Reviewed at $head: 2 real issues; no real issues after the fix"
expect "a body with no review line fails" 1 "Some text, no review."
expect "an empty body fails" 1 ""
expect "a short commit id fails" 1 "Reviewed at e5e4a3a: no real issues"
expect "the last review line counts" 1 "Reviewed at $head: no real issues
Reviewed at $old: no real issues"
expect "a later line for the head commit replaces an earlier one" 0 "Reviewed at $old: no real issues
Reviewed at $head: no real issues"
expect "bold, backticks and a list dash pass" 0 "- **Reviewed at \`$head\`: No real issues.**"
expect "a line wrapped in backticks, as AGENTS.md shows it, passes" 0 "\`Reviewed at $head: no real issues\`"
expect "a line in bold and backticks passes" 0 "**\`Reviewed at $head: no real issues\`**"
expect "an upper-case commit id passes" 0 "Reviewed at ${head^^}: no real issues"
expect "a CRLF body passes" 0 "$(printf 'Text\r\nReviewed at %s: no real issues\r\n' "$head")"
expect "a line inside other text does not count" 1 "We were Reviewed at $head: no real issues"

printf 'x' | "$check" >/dev/null 2>&1
if [ "$?" = 2 ]; then echo "ok   a missing head commit is a usage error"; else echo "FAIL a missing head commit is a usage error"; fail=1; fi

exit "$fail"
