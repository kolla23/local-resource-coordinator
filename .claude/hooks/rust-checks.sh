#!/usr/bin/env bash
# Stop hook: when .rs files changed since HEAD, run fmt and clippy; on failure exit 2, so Claude sees the
# output and keeps working. Test: bash .claude/hooks/rust-checks-test.sh
input=$(cat)
printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true' && exit 0
cd "$(git rev-parse --show-toplevel)" || exit 0
[ -z "$(git diff --name-only HEAD -- '*.rs'; git ls-files --others --exclude-standard -- '*.rs')" ] && exit 0

cargo_run() {
  case $(uname -s) in
    Linux|Darwin) cargo "$@" ;;
    # --exec skips WSL's default shell, which would re-split the arguments; the toolchain is in Ubuntu.
    *) MSYS_NO_PATHCONV=1 wsl.exe -d Ubuntu --cd "$(pwd -W)" --exec bash -lc 'cargo "$@"' cargo "$@" ;;
  esac
}

for check in "fmt --all --check" "clippy --features test-fixtures --all-targets -- -D warnings"; do
  # shellcheck disable=SC2086
  if ! out=$(cargo_run $check 2>&1); then
    printf 'cargo %s failed:\n%s\n' "$check" "$out" >&2
    exit 2
  fi
done
exit 0
