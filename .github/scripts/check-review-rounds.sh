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
# A typed line is not evidence that a review ran, so that last round must also
# match a PR comment holding the reviewer's report, which starts like this:
#
#     ## Review round 2 @ e5e4a3a1fa556b367cbd6f4b7733bb892ea9e341
#     Reviewer model: <model name>
#     Verdict: no real issues
#
# with the same round number, an id of the head commit, a named model and a
# clean verdict. The workflow passes only comments written by the repository's
# owner, members and collaborators.
#
# Usage: check-review-rounds.sh <head commit> <comments file> < body
#
# The comments file holds the comment bodies, each ended by a NUL byte. The
# body comes on standard input, never inside the text of a command: a person
# outside this project can write it.

set -euo pipefail

head="${1-}"
comments="${2-}"
if [ -z "$head" ] || [ -z "$comments" ] || [ ! -f "$comments" ]; then
    echo "usage: check-review-rounds.sh <head commit> <comments file> < body" >&2
    exit 2
fi

# The browser saves a body with CRLF line ends.
body="$(tr -d '\r')"

# Only the lines of the section `## Review rounds` count, up to the next
# heading of the same level.
section="$(printf '%s\n' "$body" | awk '
    /^## / { inside = ($0 ~ /^## Review rounds[[:space:]]*$/); next }
    inside')"

# A here-string, not a pipe: `grep -q` stops at the first match, and under
# pipefail the writer's SIGPIPE on a large body would read as "no section".
if ! grep -q '^## Review rounds[[:space:]]*$' <<<"$body"; then
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
# The verdict must START with the words. "2 real issues; earlier rounds found
# no real issues" holds them too, and it is not a clean round.
verdict_lc="${best_verdict,,}"
if [[ "$verdict_lc" != "no real issues"* ]]; then
    echo "the last review round (round $best_n) did not end clean:"
    echo "    $best_verdict"
    bad=1
fi

if [ "$bad" -eq 0 ]; then
    # The report of that round must be a PR comment. Its first non-empty line
    # names the round and the head commit; it names a model and a clean verdict.
    # Lines may be indented: the prompt shows the report as an indented block.
    found=0
    heading='^[[:space:]]*#*[[:space:]]*Review round[[:space:]]+([0-9]+)[[:space:]]*@[[:space:]]*`?([0-9a-fA-F]{7,40})`?[[:space:]]*$'
    while IFS= read -r -d '' c || [ -n "$c" ]; do
        c="${c//$'\r'/}"
        first="$(printf '%s\n' "$c" | awk 'NF { print; exit }')"
        if [[ "$first" =~ $heading ]] \
            && [ "$((10#${BASH_REMATCH[1]}))" -eq "$best_n" ] \
            && [ "${head_lc#"${BASH_REMATCH[2],,}"}" != "$head_lc" ] \
            && grep -qiE '^[[:space:]]*Reviewer model:[[:space:]]*[^[:space:]]' <<<"$c" \
            && grep -qiE '^[[:space:]]*Verdict:[[:space:]]*no real issues' <<<"$c"; then
            found=1
        fi
        c=""
    done <"$comments"
    if [ "$found" -eq 1 ]; then
        echo "round $best_n read the head commit ${head_lc:0:7} and found no real issues;"
        echo "its report is a comment on the pull request."
        exit 0
    fi
    cat <<EOF
round $best_n is recorded in the body, but no comment on the pull request holds
its report. Post the reviewer's full report as a comment that starts with:

    ## Review round $best_n @ ${head_lc}
    Reviewer model: <model name>
    Verdict: no real issues

then edit the body (or re-run this check).
EOF
    exit 1
fi

cat <<'EOF'

Start a new round: a fresh reviewer reads the whole diff at the head commit.
Fix every real issue it finds, push, and repeat. Record each round in the
section `## Review rounds`; the loop ends with a line like this one:

    **Round 3 @ <head commit>: no real issues.**

Editing the body re-runs this check. You need no new commit.
EOF
exit 1
