#!/usr/bin/env bash
# Tests `check-pr-body.sh` against a throwaway git repository; no network, so
# the workflow runs it before it reads the pull request.

set -uo pipefail

check="$(cd "$(dirname "$0")" && pwd)/check-pr-body.sh"
fail=0
repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT

g() { git -C "$repo" -c user.name=t -c user.email=t@example.com -c core.hooksPath=/dev/null -c core.autocrlf=false "$@"; }
lines() { seq 1 "$1"; }
commit() { g add -A >/dev/null && g commit -q -m "$1" && g rev-parse HEAD; }

g init -q
echo base >"$repo/README"
base="$(commit base)"
lines 100 >"$repo/small.txt"
small="$(commit small)"
lines 900 >"$repo/big.txt"
big="$(commit big)"
g checkout -q "$small"
lines 500 >"$repo/src.txt"
lines 400 >"$repo/package-lock.json"
lock="$(commit lock)"
g checkout -q "$small"
lines 500 >"$repo/src.txt"
lines 400 >"$repo/build.log"
log="$(commit log)"
g checkout -q "$small"
lines 500 >"$repo/src.txt"
mkdir "$repo/gen"
lines 400 >"$repo/gen/api.ts"
echo 'gen/** linguist-generated' >"$repo/.gitattributes"
gen="$(commit generated)"
g checkout -q "$big"
g mv big.txt moved.txt
lines 899 >"$repo/moved.txt"
moved="$(commit rename)"

good="## Summary
Size: normal — one script and its test

## Acceptance criteria
- [x] the check runs
- [ ] the docs are updated

Fixes #12

## Found, not done
- Flaky timer in the e2e suite: #14
"

# expect <name> <exit code> <head> <body> [base]
expect() {
    local got
    printf '%s' "$4" | (cd "$repo" && "$check" "${5:-$base}" "$3") >/dev/null 2>&1
    got=$?
    if [ "$got" = "$2" ]; then
        echo "ok   $1"
    else
        echo "FAIL $1: exit $got, want $2"
        fail=1
    fi
}

expect "a good body passes" 0 "$small" "$good"
expect "a good body with CRLF line ends passes" 0 "$small" "${good//$'\n'/$'\r\n'}"
expect "an empty body fails" 1 "$small" ""
expect "no Size line fails" 1 "$small" "${good/Size: normal — one script and its test/}"
expect "a Size line with no reason fails" 1 "$small" "${good/Size: normal — one script and its test/Size: normal}"
expect "an unknown size word fails" 1 "$small" "${good/Size: normal/Size: medium}"
expect "Size: big-risky with a reason passes" 0 "$small" "${good/Size: normal/Size: big-risky}"
expect "a bold Size line with a colon passes" 0 "$small" "${good/Size: normal — /**Size:** small: }"
expect "no checkbox list fails" 1 "$small" "${good//- \[?\] /- }"
expect "no Found, not done section fails" 1 "$small" "${good/Found, not done/Later}"
expect "an item with no issue number fails" 1 "$small" "${good/: #14/}"
expect "an item linked by URL instead of #N fails" 1 "$small" \
    "${good/: #14/: https://github.com/o/r/issues/14}"
expect "Closes inside Found, not done fails" 1 "$small" "${good/: #14/: Closes #14}"
expect "lowercase resolved inside Found, not done fails" 1 "$small" "${good/: #14/: resolved #14}"
expect "a Found, not done section saying none passes" 0 "$small" \
    "${good/- Flaky timer in the e2e suite: #14/- None}"
expect "Fixes #N twice fails" 1 "$small" "${good/Fixes #12/Fixes #12, fixes #12}"
expect "Fixes for two different issues passes" 0 "$small" "${good/Fixes #12/Fixes #12, Fixes #13}"
expect "over 800 counted lines fails" 1 "$big" "$good"
expect "over 800 counted lines with the owner's OK passes" 0 "$big" "$good
Owner OK for size: one generated schema"
expect "a lockfile past 800 raw lines, under 800 counted, passes" 0 "$lock" "$good"
expect "a .log file is not counted" 0 "$log" "$good"
expect "a linguist-generated path is not counted" 0 "$gen" "$good"
expect "a rename with a one-line edit counts as small" 0 "$moved" "$good" "$big"
expect "a missing head argument is a usage error" 2 "" "$good"

fake="$(mktemp -d)"
trap 'rm -rf "$repo" "$fake"' EXIT
printf '#!/bin/sh\necho "git version 2.39.5"\n' >"$fake/git"
chmod +x "$fake/git"
printf '%s' "$good" | (cd "$repo" && PATH="$fake:$PATH" "$check" "$base" "$small") >/dev/null 2>&1
got=$?
if [ "$got" = 2 ]; then
    echo "ok   git older than 2.40 (no check-attr --source) stops with exit 2"
else
    echo "FAIL git older than 2.40 (no check-attr --source) stops with exit 2: exit $got, want 2"
    fail=1
fi

exit "$fail"
