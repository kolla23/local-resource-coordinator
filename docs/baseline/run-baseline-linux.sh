#!/usr/bin/env bash
# Milestone 1 baseline: run upstream qex v0.33.0 CI commands, unchanged, on Linux.
# Usage: run-baseline-linux.sh <repo-clone> <log-dir>
# Checks out the pinned release commit (detached) and records every command's
# output, exit code and wall time. Does not stop on failure.
set -u
REPO=${1:?repo clone path}
LOGS=${2:?log dir}
PIN=78b4e86c5ff8be0c7f4bc26bea809aa75fe230f0
mkdir -p "$LOGS"
cd "$REPO" || exit 1
git checkout -q --detach "$PIN" || exit 1

{
  echo "date_utc: $(date -u +%FT%TZ)"
  echo "commit: $(git rev-parse HEAD)"
  echo "describe: $(git describe --tags --always)"
  echo "uname: $(uname -a)"
  grep PRETTY_NAME /etc/os-release
  echo "nproc: $(nproc)"
  free -b | head -2
  swapon --show 2>/dev/null || true
  rustc --version --verbose
  cargo --version
  rustfmt --version
  cargo clippy --version
  gcc --version | head -1
} > "$LOGS/00-environment.txt" 2>&1

run() {
  local name=$1; shift
  local start end rc
  start=$(date +%s)
  { echo "\$ $*"; "$@"; } > "$LOGS/$name.txt" 2>&1
  rc=$?
  end=$(date +%s)
  echo "exit_code: $rc  wall_seconds: $((end - start))" >> "$LOGS/$name.txt"
  printf '%-22s exit=%-3s %ss\n' "$name" "$rc" "$((end - start))" | tee -a "$LOGS/summary.txt"
}

: > "$LOGS/summary.txt"
run 01-fmt          cargo fmt --all --check
run 02-clippy       cargo clippy --all-targets -- -D warnings
run 03-test-bins    cargo test --bins
run 04-test-e2e     cargo test --test e2e -- --test-threads=2
run 05-build-release cargo build --release --locked
run 06-msrv-check   cargo +1.85 check --all-targets --locked
