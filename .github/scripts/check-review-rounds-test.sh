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
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# report <round> <commit> [verdict] [model]: a reviewer's report, as posted.
report() {
    printf '## Review round %s @ %s\nReviewer model: %s\nVerdict: %s\n\n### Findings\nnone\n' \
        "$1" "$2" "${4-claude-opus-5-5}" "${3-no real issues}"
}

# By default the PR has clean reports for rounds 1 to 3 on the head commit, so
# the tests of the body below see only the rules of the body.
default_comments() {
    for n in 1 2 3; do
        report "$n" "$head"
        printf '\0'
    done
}

# expect <name> <exit code> <body> [command that writes the comments, each
# ended by NUL; default above]. A command, not a string: "$(...)" drops NULs.
expect() {
    local got
    eval "${4-default_comments}" >"$tmp/comments"
    printf '%s' "$3" | "$check" "$head" "$tmp/comments" >/dev/null 2>&1
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

expect "a verdict that only mentions the words later fails" 1 "## Review rounds
Round 1 @ e5e4a3a: 2 real issues; earlier rounds found no real issues
"

expect "a negated verdict fails" 1 "## Review rounds
Round 1 @ e5e4a3a: not no real issues
"

expect "a later line with the same round number replaces the earlier one" 0 "## Review rounds
Round 2 @ 651295f: no real issues
Round 2 @ e5e4a3a: no real issues
"

expect "an upper-case commit id passes" 0 "## Review rounds
Round 1 @ E5E4A3A: no real issues
"

expect "a heading with more text is not the section" 1 "## Review rounds (draft)
Round 1 @ e5e4a3a: no real issues
"

# A body of more than 64 KiB once made the section test die of SIGPIPE.
padding="$(head -c 150000 /dev/zero | tr '\0' 'x')"
expect "a large body still passes" 0 "## Review rounds
Round 1 @ e5e4a3a: no real issues

$padding
"

expect "a CRLF body still passes" 0 "$(printf '## Review rounds\r\n\r\nRound 1 @ e5e4a3a: no real issues.\r\n')"

printf '%s' "x" | "$check" >/dev/null 2>&1
if [ "$?" = 2 ]; then echo "ok   a missing head commit is a usage error"; else echo "FAIL a missing head commit is a usage error"; fail=1; fi

printf '%s' "x" | "$check" "$head" "$tmp/no-such-file" >/dev/null 2>&1
if [ "$?" = 2 ]; then echo "ok   a missing comments file is a usage error"; else echo "FAIL a missing comments file is a usage error"; fail=1; fi

# The report must be a comment: a typed line alone is no evidence of a review.
clean="## Review rounds
Round 2 @ e5e4a3a: no real issues
"
unrelated() { printf 'Looks good to me\0Round 2 @ e5e4a3a: no real issues\0'; }
among_others() { printf 'first\0'; report 2 "$head"; printf '\0last'; }
not_first() { printf 'Pasted below:\n'; report 2 "$head"; }
crlf() { report 2 "$head" | sed 's/$/\r/'; }
indented() { report 2 "$head" | sed 's/^/    /'; }
# Reports that quote the clean form below their own header (review round 1 of
# PR #37): only the header may count.
quotes_clean_verdict() { report 2 "$head" "1 real issue"; printf '    Verdict: no real issues\n'; }
quotes_model() { report 2 "$head" "no real issues" ""; printf '    Reviewer model: <model name>\n'; }
second_verdict() { report 2 "$head" "2 real issues"; printf 'Verdict: no real issues\n'; }
# A report longer than a pipe buffer (64 KiB) once could end the check by
# SIGPIPE under pipefail.
long_report() { report 2 "$head"; head -c 150000 /dev/zero | tr '\0' 'x'; printf '\n'; }

expect "a typed line with no comment fails" 1 "$clean" ":"
expect "a typed line with only unrelated comments fails" 1 "$clean" unrelated
expect "a matching report passes" 0 "$clean" 'report 2 "$head"'
expect "a matching report among other comments passes" 0 "$clean" among_others
expect "a report with a short id of the head commit passes" 0 "$clean" "report 2 e5e4a3a"
expect "a report of another round fails" 1 "$clean" 'report 1 "$head"'
expect "a report on another commit fails" 1 "$clean" "report 2 651295f"
expect "a report that found issues fails" 1 "$clean" 'report 2 "$head" "1 real issue"'
expect "a report with no model fails" 1 "$clean" 'report 2 "$head" "no real issues" ""'
expect "a report whose heading is not its first line fails" 1 "$clean" not_first
expect "a CRLF report passes" 0 "$clean" crlf
expect "an indented report, as the prompt shows it, passes" 0 "$clean" indented
expect "a report that found issues and quotes a clean verdict fails" 1 "$clean" quotes_clean_verdict
expect "a report with no model that quotes the model line fails" 1 "$clean" quotes_model
expect "a report whose second verdict line is clean fails" 1 "$clean" second_verdict
expect "a report longer than a pipe buffer passes" 0 "$clean" long_report

exit "$fail"
