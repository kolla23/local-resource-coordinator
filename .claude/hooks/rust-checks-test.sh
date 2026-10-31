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

reset() {
  git -C "$tmp/repo" checkout -q -- . && git -C "$tmp/repo" clean -fdq && rm -f "$tmp/calls"
}
mkdir -p "$tmp/bin-clippy"
for fake in cargo wsl.exe; do
  cat > "$tmp/bin-clippy/$fake" <<'EOF'
#!/bin/sh
for arg in "$@"; do
  if [ "$arg" = clippy ]; then echo zq_fake_clippy_lint; exit 1; fi
done
exit 0
EOF
  chmod +x "$tmp/bin-clippy/$fake"
done

reset
printf 'fn scratch_clippy() {}\n' > "$tmp/repo/src/scratch_clippy.rs"
(cd "$tmp/repo" && printf '{"stop_hook_active": false}' | PATH="$tmp/bin-clippy:$PATH" bash "$hook" 2>"$tmp/err")
check "a clippy failure exits 2" "$?" 2
check "the clippy output reaches Claude on stderr" "$(grep -q 'zq_fake_clippy_lint' "$tmp/err" && echo yes)" yes
check "the stderr names the clippy step" "$(grep -q 'cargo clippy' "$tmp/err" && echo yes)" yes
reset

reset
printf '\nA non-.rs change.\n' >> "$tmp/repo/README.md"
printf 'scratch\n' > "$tmp/repo/src/scratch_untracked.txt"
(cd "$tmp/repo" && printf '{"stop_hook_active": false}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
check "a non-.rs change exits 0" "$?" 0
check "a non-.rs change does not call cargo" "$([ -e "$tmp/calls" ] && echo called || echo none)" none
reset

for manifest in Cargo.toml Cargo.lock clippy.toml; do
  reset
  printf '\n# scratch\n' >> "$tmp/repo/$manifest"
  (cd "$tmp/repo" && printf '{"stop_hook_active": false}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
  check "a $manifest change calls cargo" "$([ -e "$tmp/calls" ] && echo called || echo none)" called
done

reset
mkdir -p "$tmp/repo/experiments/scratch"
printf '[package]\n' > "$tmp/repo/experiments/scratch/Cargo.toml"
(cd "$tmp/repo" && printf '{"stop_hook_active": false}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
check "a Cargo.toml under experiments/ does not call cargo" "$([ -e "$tmp/calls" ] && echo called || echo none)" none

reset
printf '\n# scratch\n' >> "$tmp/repo/experiments/parallel-workload/Cargo.toml"
(cd "$tmp/repo" && printf '{"stop_hook_active": false}' | PATH="$tmp/bin:$PATH" bash "$hook" 2>/dev/null)
check "an edited Cargo.toml under experiments/ does not call cargo" "$([ -e "$tmp/calls" ] && echo called || echo none)" none
reset

echo "$failed failed"
[ "$failed" -eq 0 ]
