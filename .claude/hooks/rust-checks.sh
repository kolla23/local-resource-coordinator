#!/usr/bin/env bash
# Stop hook: when .rs files or the root Cargo.toml, Cargo.lock or clippy.toml changed since HEAD, run fmt and
# clippy; on failure exit 2, so Claude sees the output and keeps working. Test: bash .claude/hooks/rust-checks-test.sh
input=$(cat)
printf '%s' "$input" | grep -Eq '"stop_hook_active"[[:space:]]*:[[:space:]]*true' && exit 0
cd "$(git rev-parse --show-toplevel)" || exit 0
# Root-level manifests only: the experiments/ crates are separate packages that cargo at the root never reads.
triggers=('*.rs' Cargo.toml Cargo.lock clippy.toml)
[ -z "$(git diff --name-only HEAD -- "${triggers[@]}"; git ls-files --others --exclude-standard -- "${triggers[@]}")" ] && exit 0

cargo_run() {
  case $(uname -s) in
    Linux|Darwin) cargo "$@" ;;
    # --exec skips WSL's default shell, which would re-split the arguments; the toolchain is in Ubuntu.
    # The target sits on WSL's own disk, one per worktree: on /mnt/c the checks ran 2-10x slower (#76).
    *) key=$(pwd | sha1sum | cut -c1-16)
       # shellcheck disable=SC2016 # $HOME and $1 expand in WSL's bash, not here.
       MSYS_NO_PATHCONV=1 wsl.exe -d Ubuntu --cd "$(pwd -W)" --exec bash -lc \
         'export CARGO_TARGET_DIR="$HOME/.cache/lrc-stop-hook/$1"; shift; cargo "$@"' bash "$key" "$@" ;;
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
