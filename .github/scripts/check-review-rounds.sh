#!/usr/bin/env bash
#
# Tests that the review loop of a pull request ended on its head commit.
#
# The rule of this project: a fresh reviewer reads the whole diff, every real
# issue is fixed, and a NEW round starts after each push. The loop stops only
# when a round finds no real issues. A written rule alone was not enough: a
# round ran, a later push followed, and no round read that push.
#
# The body of the pull request records each round on a line of its own, in the
# section `## Review rounds`:
#
#     **Round 2 @ e5e4a3a: no real issues.**
#
# This test passes only when the round with the highest number names the head
# commit (7 to 40 characters of its id) and says "no real issues". Any push
# gives a new head commit, so the check turns red until a new round names it.
# Bold and backticks are allowed around the line and the id.
#
# Usage: check-review-rounds.sh <head commit> < body
#
# The body comes on standard input, never inside the text of a command: a
# person outside this project can write it.

set -euo pipefail

head="${1-}"
if [ -z "$head" ]; then
    echo "give the head commit of the pull request" >&2
    exit 2
fi

# The browser saves a body with CRLF line ends.
body="$(tr -d '\r')"

# Only the lines of the section `## Review rounds` count, up to the next
# heading of the same level.
section="$(printf '%s\n' "$body" | awk '
    /^## / { inside = ($0 ~ /^## Review rounds[[:space:]]*$/); next }
    inside')"

if ! printf '%s\n' "$body" | grep -q '^## Review rounds[[:space:]]*$'; then
    cat <<'EOF'
the body of the pull request has no section `## Review rounds`.

Add the section, and record each review round on a line of its own:

    **Round 1 @ <head commit>: no real issues.**
EOF
    exit 1
fi

# Find the round with the highest number. A later line with the same number
# replaces an earlier one.
best_n=-1
best_sha=""
best_verdict=""
pattern='^[-[:space:]]*Round[[:space:]]+([0-9]+)[[:space:]]*@[[:space:]]*([0-9a-fA-F]{7,40})[[:space:]]*:[[:space:]]*(.*)$'
while IFS= read -r line; do
    line="${line//\*/}"
    line="${line//\`/}"
    if [[ "$line" =~ $pattern ]]; then
        n=$((10#${BASH_REMATCH[1]}))
        if [ "$n" -ge "$best_n" ]; then
            best_n=$n
            best_sha="${BASH_REMATCH[2],,}"
            best_verdict="${BASH_REMATCH[3]}"
        fi
    fi
done <<<"$section"

if [ "$best_n" -lt 0 ]; then
    cat <<'EOF'
the section `## Review rounds` records no round.

Run a review round on the head commit, then record it on a line of its own:

    **Round 1 @ <head commit>: no real issues.**
EOF
    exit 1
fi

bad=0
head_lc="${head,,}"
if [ "${head_lc#"$best_sha"}" = "$head_lc" ]; then
    echo "the last review round (round $best_n) read commit $best_sha, and the"
    echo "head of the pull request is now ${head_lc:0:7}. A push came after the review."
    bad=1
fi
verdict_lc="${best_verdict,,}"
if [[ "$verdict_lc" != *"no real issues"* ]]; then
    echo "the last review round (round $best_n) did not end clean:"
    echo "    $best_verdict"
    bad=1
fi

if [ "$bad" -eq 0 ]; then
    echo "round $best_n read the head commit ${head_lc:0:7} and found no real issues."
    exit 0
fi

cat <<'EOF'

Start a new round: a fresh reviewer reads the whole diff at the head commit.
Fix every real issue it finds, push, and repeat. Record each round in the
section `## Review rounds`; the loop ends with a line like this one:

    **Round 3 @ <head commit>: no real issues.**

Editing the body re-runs this check. You need no new commit.
EOF
exit 1
