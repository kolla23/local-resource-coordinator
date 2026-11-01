#!/usr/bin/env bash
# Measures qex IPC line sizes through a passive proxy; see docs/decision-gate/07-ipc-message-sizes.md.
# Usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$HOME/.cache/lrc-ipc-size-measure/target}
Q=$target/release/qex
A='' B='' proxy_pid='' coord_pid='' label_file='' raw=/dev/null log=/dev/null
rep=0 job_count=0 shape=base

die() {
  echo "measure.sh: $*" | tee -a "$log" >&2
  exit 1
}
say() { echo "$*" | tee -a "$log"; }

build() {
  (cd "$repo" && CARGO_TARGET_DIR=$target cargo build --release --locked >/dev/null 2>&1) ||
    die "cargo build --release --locked failed"
}

# A fixed, small environment keeps the captured env (and so the sizes) the same on every run.
in_root() {
  local root=$1
  shift
  (cd "$A" && env -i HOME="$HOME" PATH=/usr/local/bin:/usr/bin:/bin LANG=C.UTF-8 \
    XDG_CONFIG_HOME="$A/cfg" XDG_STATE_HOME="$root" XDG_RUNTIME_DIR="$root/run" \
    QEX_IDLE_EXIT_SECS=3600 QEX_PEERS_DIR="$A/peers" "$Q" "$@")
}
qa() { in_root "$A" "$@"; }

# One measured CLI call through root B; the proxy tags its lines with the label written here.
qb() {
  local step=$1 rc=0
  shift
  printf '%s\t%s\t%s\t%s\n' "$step" "$rep" "$job_count" "$shape" >"$label_file"
  in_root "$B" "$@" || rc=$?
  [ ! -e "$B/qex/run/pid" ] || die "a coordinator started at root B (step $step)"
  [ "$(cat "$A/qex/run/pid")" = "$coord_pid" ] || die "the coordinator at root A changed (step $step)"
  return "$rc"
}

jget() {
  python3 -c 'import json, sys
v = json.load(sys.stdin)
for k in sys.argv[1:]:
    v = v[int(k)] if isinstance(v, list) else v[k]
print(json.dumps(v) if isinstance(v, (list, dict)) else v)' "$@"
}

procs_of() {
  local p
  for p in /proc/[0-9]*; do
    if tr '\0' '\n' 2>/dev/null <"$p/environ" | grep -qxF "XDG_STATE_HOME=$1"; then
      echo "${p#/proc/}"
    fi
  done
}

teardown() {
  set +e
  if [ -n "$proxy_pid" ]; then
    kill "$proxy_pid" 2>/dev/null
    wait "$proxy_pid" 2>/dev/null
  fi
  if [ -n "$A" ]; then
    if [ -n "$coord_pid" ]; then
      kill "$coord_pid" 2>/dev/null
      for _ in $(seq 50); do
        [ -e "/proc/$coord_pid" ] || break
        sleep 0.1
      done
    fi
    # The coordinator's jobs and supervisors inherit XDG_STATE_HOME=$A; nothing else here has it.
    procs_of "$A" | xargs -r kill -9 2>/dev/null
    rm -rf "$A" "$B"
  fi
  A='' B='' proxy_pid='' coord_pid=''
  set -e
}
trap teardown EXIT

setup() {
  A=$(mktemp -d /tmp/lrc-ipc.XXXXXX)
  B=$(mktemp -d /tmp/lrc-ipc.XXXXXX)
  mkdir -p "$A/cfg" "$A/peers" "$B/qex"
  mkdir -m 0700 "$B/qex/run"
  ln -s "$A/qex/jobs" "$B/qex/jobs"
  local sock=$A/qex/run/s
  [ "${#sock}" -le 100 ] || die "socket path $sock is over 100 bytes (src/paths.rs MAX_SOCKET_PATH)"
  coord_pid=$(qa info --json | jget pid)
  label_file=$B/label
  printf 'setup\t0\t0\t-\n' >"$label_file"
  python3 "$here/proxy.py" "$B/qex/run/s" "$sock" "$label_file" "$raw" &
  proxy_pid=$!
  for _ in $(seq 50); do
    [ -S "$B/qex/run/s" ] && break
    sleep 0.1
  done
  [ -S "$B/qex/run/s" ] || die "the proxy did not open $B/qex/run/s"
  local via_b
  via_b=$(qb setup info --json | jget pid)
  [ "$via_b" = "$coord_pid" ] || die "through B: coordinator pid $via_b, at A: $coord_pid"
}

smoke() {
  build
  setup
  local id a b
  id=$(qa submit -- true)
  qa wait "$id" >/dev/null
  say "coordinator pid direct=$coord_pid via-B=$(qb smoke info --json | jget pid)"
  a=$(qa status --json "$id" | python3 -c 'import json, sys
v = json.load(sys.stdin); print(v["id"], v["state"], json.dumps(v["command"]))')
  b=$(qb smoke status --json "$id" | python3 -c 'import json, sys
v = json.load(sys.stdin); print(v["id"], v["state"], json.dumps(v["command"]))')
  [ "$a" = "$b" ] || die "status differs: direct [$a] via-B [$b]"
  say "status equal through A and B: id, state and command ($(echo "$a" | cut -d' ' -f2))"
  teardown
}

case ${1:-} in
--smoke) smoke ;;
*) die "usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW" ;;
esac
