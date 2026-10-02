#!/usr/bin/env bash
#
# Tests that a pull request was reviewed at its head commit.
#
# Review rounds run before the pull request is ready: a fresh reviewer reads
# the whole diff, every real issue is fixed, and a new round starts after each
# fix. The reports stay local. When a round finds no real issues, the author
# adds one line to the body and marks the pull request ready:
#
#     Reviewed at <full 40-character head commit id>: no real issues
#
# This test passes only when the last such line names the head commit and its
# verdict starts with "no real issues". A push after the review gives a new
# head commit, so the test fails until the line names it. Bold and backticks
# are allowed around the line and the id.
#
# Usage: check-reviewed.sh <head commit> < body
#
# The body comes on standard input, never inside the text of a command: a
# person outside this project can write it.

set -euo pipefail

head="${1-}"
if [ -z "$head" ]; then
    echo "usage: check-reviewed.sh <head commit> < body" >&2
    exit 2
fi
head_lc="${head,,}"

# The browser saves a body with CRLF line ends.
body="$(tr -d '\r')"

sha=""
verdict=""
pattern='^[-[:space:]]*Reviewed at[[:space:]]+`?([0-9a-fA-F]{40})`?[[:space:]]*:[[:space:]]*(.*)$'
while IFS= read -r line; do
    line="${line//\*/}"
    if [[ "$line" =~ $pattern ]]; then
        sha="${BASH_REMATCH[1],,}"
        verdict="${BASH_REMATCH[2]}"
    fi
done <<<"$body"

if [ -z "$sha" ]; then
    cat <<'EOF'
the body has no line "Reviewed at <full head commit id>: no real issues".

Run the review rounds before marking the pull request ready. When a round on
the head commit finds no real issues, add that line to the body.
EOF
    exit 1
fi

bad=0
if [ "$sha" != "$head_lc" ]; then
    echo "the review read commit ${sha:0:7}, and the head is now ${head_lc:0:7}."
    echo "A push came after the review: review the new head commit."
    bad=1
fi
# The verdict must START with the words: "2 real issues; earlier rounds found
# no real issues" holds them too, and it is not a clean review.
if [[ "${verdict,,}" != "no real issues"* ]]; then
    echo "the last review did not end clean: $verdict"
    bad=1
fi
if [ "$bad" -eq 0 ]; then
    echo "reviewed at the head commit ${head_lc:0:7}: no real issues."
fi
exit "$bad"
