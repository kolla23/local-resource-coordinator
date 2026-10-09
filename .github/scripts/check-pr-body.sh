#!/usr/bin/env bash
# Checks the PR-body rules the reviewer leaves to CI (~/.claude/standards/REVIEWER_PROMPT.md).
# Usage, inside the repository: check-pr-body.sh <base commit> <head commit> < body

set -euo pipefail

base="${1-}"
head="${2-}"
if [ -z "$base" ] || [ -z "$head" ]; then
    echo "usage: check-pr-body.sh <base commit> <head commit> < body" >&2
    exit 2
fi
# The size count reads .gitattributes at the head commit with check-attr --source (git 2.40+).
if [[ ! "$(git version)" =~ ([0-9]+)\.([0-9]+) ]] \
    || (( BASH_REMATCH[1] < 2 || (BASH_REMATCH[1] == 2 && BASH_REMATCH[2] < 40) )); then
    echo "check-pr-body.sh needs git 2.40 or later (git check-attr --source); found: $(git version)" >&2
    exit 2
fi

# The browser saves a body with CRLF line ends.
body="$(tr -d '\r')"
problems=()

closing='(close|closes|closed|fix|fixes|fixed|resolve|resolves|resolved)[[:space:]]*:?[[:space:]]+'
ref='(https://github\.com/[^/[:space:]]+/[^/[:space:]]+/issues/[0-9]+|([[:alnum:]_.-]+/[[:alnum:]_.-]+)?#[0-9]+)'
keyword_re="(^|[^[:alnum:]_])${closing}${ref}"
size_re='^[-[:space:]>]*Size:[[:space:]]*(small|normal|big-risky)[[:space:]]*(—|–|-|:)[[:space:]]*[^[:space:]]'
item_re='^[[:space:]]*([-*+]|[0-9]+[.)])[[:space:]]+(.*)$'

has_size=0
has_box=0
in_found=0
found_seen=0
declare -A closed=()
while IFS= read -r raw; do
    line="${raw//\*/}"
    line="${line//\`/}"
    lc="${line,,}"
    if [[ "$line" =~ $size_re ]]; then
        has_size=1
    fi
    if [[ "$line" =~ ^[[:space:]]*[-*+][[:space:]]+\[[[:space:]xX]\] ]]; then
        has_box=1
    fi
    if [[ "$lc" =~ ^[#[:space:]]*found,[[:space:]]+not[[:space:]]+done ]]; then
        in_found=1
        found_seen=1
        continue
    fi
    [[ "$line" =~ ^# ]] && in_found=0

    rest="$lc"
    while [[ "$rest" =~ $keyword_re ]]; do
        issue="${BASH_REMATCH[3]}"
        rest="${rest#*"${BASH_REMATCH[0]}"}"
        if [[ "$issue" =~ ^https://github\.com/([^/]+/[^/]+)/issues/([0-9]+)$ ]]; then
            issue="${BASH_REMATCH[1]}#${BASH_REMATCH[2]}"
        fi
        closed[$issue]=$(( ${closed[$issue]:-0} + 1 ))
        if [ "$in_found" -eq 1 ]; then
            problems+=("\"Found, not done\" closes $issue when this PR merges: link it as plain #N, without Fixes/Closes/Resolves.")
        fi
    done

    if [ "$in_found" -eq 1 ] && [[ "$line" =~ $item_re ]]; then
        item="${BASH_REMATCH[2]}"
        if [[ ! "${item,,}" =~ ^none ]] && [[ ! "$item" =~ (^|[^[:alnum:]_/\&])#[0-9]+ ]]; then
            problems+=("\"Found, not done\" item has no plain #N issue link: $item")
        fi
    fi
done <<<"$body"

[ "$has_size" -eq 1 ] || problems+=("no line \"Size: small | normal | big-risky — <reason>\".")
[ "$has_box" -eq 1 ] || problems+=("no acceptance-criteria checkbox list (\"- [ ] ...\").")
[ "$found_seen" -eq 1 ] || problems+=("no \"Found, not done\" section.")
for issue in "${!closed[@]}"; do
    if [ "${closed[$issue]}" -gt 1 ]; then
        problems+=("$issue is closed by a Fixes/Closes/Resolves keyword ${closed[$issue]} times; say it once.")
    fi
done

# The one counting policy for PR size: no logs, lockfiles or linguist-generated paths.
numstat="$(mktemp)"
trap 'rm -f "$numstat"' EXIT
if ! git diff --numstat -z -M "$base...$head" >"$numstat"; then
    echo "cannot diff $base...$head" >&2
    exit 2
fi
size=0
while IFS= read -r -d '' rec; do
    added="${rec%%$'\t'*}"
    rec="${rec#*$'\t'}"
    deleted="${rec%%$'\t'*}"
    path="${rec#*$'\t'}"
    if [ -z "$path" ]; then
        IFS= read -r -d '' _old
        IFS= read -r -d '' path
    fi
    [ "$added" = "-" ] && continue
    case "${path##*/}" in
        *.log | *.lock | package-lock.json | pnpm-lock.yaml) continue ;;
    esac
    generated="$(git check-attr --source "$head" linguist-generated -- "$path")"
    case "${generated##*: }" in
        set | true) continue ;;
    esac
    size=$(( size + added + deleted ))
done <"$numstat"

if [ "$size" -gt 800 ] && [[ ! "${body,,}" == *"owner ok for size"* ]]; then
    problems+=("$size changed lines (past 800) and no \"Owner OK for size\" in the body.")
fi

if [ "${#problems[@]}" -gt 0 ]; then
    printf '%s\n' "${problems[@]}"
    exit 1
fi
echo "PR body checks pass; $size counted changed lines."
