#!/usr/bin/env bash
# Tests rust-checks.sh, the Stop hook, in a scratch clone. Run: bash .claude/hooks/rust-checks-test.sh
set -u
root=$(git rev-parse --show-toplevel) || exit 2
hook="$root/.claude/hooks/rust-checks.sh"  # the working copy, so an uncommitted edit is what runs
tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
git clone -q --local "$root" "$tmp/repo" || exit 2
mkdir -p "$tmp/bin"
for fake in cargo wsl.exe; do
  printf '#!/bin/sh\necho called >> "%s/calls"\nexit 0\n' "$tmp" > "$tmp/bin/$fake"
  chmod +x "$tmp/bin/$fake"
done
failed=0
check() {
  if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: got $2, want $3"; failed=$((failed + 1)); fi
}
run() {
  (cd "$tmp/repo" && printf '%s' "$1" | bash "$hook" 2>"$tmp/err")
}

run '{"stop_hook_active": false}'
check "a clean tree exits 0" "$?" 0
(cd "$tmp/repo" && printf '{}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
check "a clean tree does not call cargo" "$([ -e "$tmp/calls" ] && echo called || echo none)" none

printf '\nfn  badly_formatted( ) {   }\n' >> "$tmp/repo/src/main.rs"
run '{"stop_hook_active": false}'
check "a badly formatted .rs change exits 2" "$?" 2
check "the fmt output reaches Claude on stderr" "$(grep -q 'badly_formatted' "$tmp/err" && echo yes)" yes

run '{"stop_hook_active": true}'
check "stop_hook_active exits 0 even with a bad change" "$?" 0

git -C "$tmp/repo" checkout -q -- src/main.rs
printf 'fn  untracked( ) {   }\n' > "$tmp/repo/src/scratch_untracked.rs"
(cd "$tmp/repo" && printf '{}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
check "an untracked .rs file counts as a change" "$([ -e "$tmp/calls" ] && echo called || echo none)" called

echo "$failed failed"
[ "$failed" -eq 0 ]
