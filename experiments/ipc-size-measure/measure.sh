#!/usr/bin/env bash
# Measures qex IPC line sizes through a passive proxy; see docs/decision-gate/07-ipc-message-sizes.md.
# Usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$HOME/.cache/lrc-ipc-size-measure/target}
Q=$target/release/qex
A='' B='' proxy_pid='' coord_pid='' label_file='' raw=/dev/null log=/dev/null work=''
rep=0 job_count=0 shape=base
fin_ids=() unf_ids=()
name128=$(printf 'n%.0s' $(seq 128))

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

# One measured CLI call through root B, output in $B/out; the proxy tags its lines with this label.
qb() {
  local step=$1 rc=0
  shift
  printf '%s\t%s\t%s\t%s\n' "$step" "$rep" "$job_count" "$shape" >"$label_file"
  in_root "$B" "$@" >"$B/out" 2>&1 || rc=$?
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
finish() {
  teardown
  [ -z "$work" ] || rm -rf "$work"
}
trap finish EXIT

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
  qb setup info --json
  [ "$(jget pid <"$B/out")" = "$coord_pid" ] || die "through B: another coordinator than $coord_pid"
}

smoke() {
  build
  setup
  local id a b fields='import json, sys
v = json.load(sys.stdin); print(v["id"], v["state"], json.dumps(v["command"]))'
  id=$(qa submit -- true)
  qa wait "$id" >/dev/null
  qb smoke info --json
  say "coordinator pid direct=$coord_pid via-B=$(jget pid <"$B/out")"
  a=$(qa status --json "$id" | python3 -c "$fields")
  qb smoke status --json "$id"
  b=$(python3 -c "$fields" <"$B/out")
  [ "$a" = "$b" ] || die "status differs: direct [$a] via-B [$b]"
  say "status equal through A and B: id, state and command ($(echo "$a" | cut -d' ' -f2))"
  teardown
}

wait_load() {
  local load=''
  for _ in $(seq 120); do
    load=$(qa info --json | python3 -c 'import json, sys
v = json.load(sys.stdin); print(v["jobs_running"], v["jobs_queued"])')
    [ "$load" = "$1 $2" ] && return 0
    sleep 0.5
  done
  die "after 60 s the coordinator runs/queues $load, expected $1 $2"
}

# count_check DATASET F R Q (or DATASET -): asserts the records' states through A; sets job_count.
count_check() {
  local got expected=${2:-}
  got=$(qa list --all --json | python3 -c 'import json, sys
c = {"f": 0, "r": 0, "q": 0}
for j in json.load(sys.stdin):
    s = j["state"]
    c["q" if s == "queued" else "r" if s in ("starting", "running") else "f"] += 1
print("%d/%d/%d" % (c["f"], c["r"], c["q"]))')
  job_count=$(($(echo "$got" | tr '/' '+')))
  if [ "$expected" = - ]; then
    say "count-check $1 expected=- got=$got ok"
    return
  fi
  expected=$2/$3/$4
  if [ "$got" = "$expected" ]; then
    say "count-check $1 expected=$expected got=$got ok"
  else
    say "count-check $1 expected=$expected got=$got MISMATCH"
    die "count check $1 failed"
  fi
}

# submit_split F R Q [OPTIONS]: F finished `true` jobs, then R+Q at --cpu full, so one runs and Q queue.
submit_split() {
  local f=$1 r=$2 q=$3 i
  shift 3
  fin_ids=() unf_ids=()
  for ((i = 0; i < f; i++)); do fin_ids+=("$(qa submit "$@" -- true)"); done
  [ "$f" -eq 0 ] || qa wait "${fin_ids[@]}" >/dev/null
  for ((i = 0; i < r + q; i++)); do unf_ids+=("$(qa submit --cpu full "$@" -- sleep 3600)"); done
  wait_load "$r" "$q"
}

cleanup() {
  qa abort --all --grace 1s >/dev/null 2>&1 || true
  qa resume --all >/dev/null 2>&1 || true
  wait_load 0 0
  qa clean --all >/dev/null 2>&1 || true
  count_check cleanup 0 0 0
}

split_of() {
  case $1 in
  0) echo 0 0 0 ;;
  100) echo 10 1 89 ;;
  1000) echo 100 1 899 ;;
  esac
}

nondestructive() {
  qb gc gc --dry-run --json
  qb list_all list --all --json
  qb list_queued list --state queued --json
  qb info info --json
  qb pause_state pause --json
  local rc=0
  qb events events --since start --count 512 --timeout 10s --json || rc=$?
  [ "$rc" -eq 0 ] || [ "$rc" -eq 124 ] || die "qex events exited $rc"
  [ "$job_count" -gt 0 ] || return 0
  qb status status --json "${unf_ids[0]}"
  qb clean clean "${unf_ids[@]}" || true
  grep -qxF 'qex deleted 0 records' "$B/out" || die "qex clean of unfinished jobs deleted records"
}

destructive() {
  local f=$1 r=$2 q=$3 all
  if [ "$job_count" -gt 0 ]; then
    cleanup
    submit_split "$f" "$r" "$q"
    count_check "cancel_set" "$f" "$r" "$q"
    all=("${fin_ids[@]}" "${unf_ids[@]}")
    qb cancel cancel "${all[@]}" || true
    cleanup
    submit_split "$f" "$r" "$q"
    count_check "kill_set" "$f" "$r" "$q"
    all=("${fin_ids[@]}" "${unf_ids[@]}")
    qb kill kill --grace 1s "${all[@]}" || true
  fi
  cleanup
  submit_split "$f" "$r" "$q"
  count_check "abort_set" "$f" "$r" "$q"
  qb abort abort --all --json || true
  qb resume resume --all --json
  [ "$(qa info --json | jget queue_state)" = running ] || die "the queue is not running after resume --all"
}

group_rows() {
  local gid step
  seq 1000 >"$work/lines"
  for step in clean_group cancel_group; do
    cleanup
    gid=$(qa submit --each-line "$work/lines" --cpu full -- sh -c 'exec sleep 3600' sh '{}' 2>/dev/null)
    wait_load 1 999
    count_check "$step" 0 1 999
    qb "$step" "${step%_group}" "$gid" || true
  done
}

dup_rows() {
  local f=$1 r=$2 q=$3 h handles
  shape=dup
  cleanup
  submit_split "$f" "$r" "$q" --name "$name128"
  count_check dup "$f" "$r" "$q"
  qb status_dup_h1 status --json "$name128" || true
  for h in 1 10 500; do
    handles=()
    for ((i = 0; i < h; i++)); do handles+=("$name128"); done
    qb "clean_dup_h$h" clean "${handles[@]}" || true
    qb "cancel_dup_h$h" cancel "${handles[@]}" || true
    qb "kill_dup_h$h" kill --grace 1s "${handles[@]}" || true
  done
  count_check dup_after "$f" "$r" "$q"
  shape=base
}

held_rows() {
  local n=$1 held i
  shape=held
  cleanup
  held=$(qa submit --name "$name128" -- true)
  qa wait "$held" >/dev/null
  for ((i = 0; i < n - 1; i++)); do
    qa submit --cpu full --needs "$held" --name "$name128" -- sleep 3600 >/dev/null
  done
  wait_load 1 $((n - 2))
  count_check held 1 1 $((n - 2))
  qb clean_held clean "$held" || true
  count_check held_after 1 1 $((n - 2))
  shape=base
}

sweep() {
  local count f r q
  for count in 0 100 1000; do
    read -r f r q <<<"$(split_of "$count")"
    setup
    shape=base
    submit_split "$f" "$r" "$q"
    count_check "base_$count" "$f" "$r" "$q"
    nondestructive
    destructive "$f" "$r" "$q"
    [ "$count" -ne 1000 ] || group_rows
    if [ "$count" -gt 0 ]; then
      dup_rows "$f" "$r" "$q"
      held_rows "$count"
    fi
    teardown
  done
}

environment() {
  echo "date_utc $(date -u +%FT%TZ)"
  echo "kernel $(uname -r)"
  echo "rustc $(rustc --version)"
  echo "qex $("$Q" --version)"
  echo "ARG_MAX $(getconf ARG_MAX)"
  echo "PAGESIZE $(getconf PAGESIZE)"
  echo "PATH_MAX $(getconf PATH_MAX /)"
  echo "binfmts.h $(grep -n MAX_ARG_STRLEN /usr/include/linux/binfmts.h)"
  echo "login_env_bytes $(bash -lc 'env | wc -c')"
  (cd "$here" && sha256sum measure.sh proxy.py | sed 's/^/sha256 /')
}

# sizes.log: one row per (op, direction, job_count, record_shape, step, rep); setup lines left out.
aggregate() {
  python3 - "$raw" <<'PY'
import sys
rows = {}
for line in open(sys.argv[1], encoding="utf-8"):
    conn, step, rep, jc, shape, direction, op, size, states = line.rstrip("\n").split("\t")
    if step == "setup" or direction == "accept":
        continue
    key = (op, direction, jc, shape, step, rep)
    r = rows.setdefault(key, [0, 0, 0, states])
    size = int(size)
    if size > r[0]:
        r[0], r[3] = size, states
    r[1] += 1
    r[2] += size
print("kind\tdirection\tline_bytes\tlines\ttotal_bytes\tjob_count\trecord_shape\tstates\tstep\trep")
for (op, direction, jc, shape, step, rep), r in sorted(rows.items()):
    print("\t".join([op, direction, str(r[0]), str(r[1]), str(r[2]), jc, shape, r[3], step, rep]))
PY
}

summarise() {
  local dir=$1
  [ -f "$dir/sizes.log" ] || die "no $dir/sizes.log"
  python3 - "$dir/sizes.log" "$dir/summary.log" <<'PY'
import sys
keys = {}
for line in list(open(sys.argv[1], encoding="utf-8"))[1:]:
    op, direction, size, lines, total, jc, shape, states, step, rep = line.rstrip("\n").split("\t")
    k = keys.setdefault((step, op, direction, jc, shape), {"sizes": [], "lines": 0})
    k["sizes"].append(int(size))
    k["lines"] = max(k["lines"], int(lines))
with open(sys.argv[2], "w", encoding="utf-8") as out:
    out.write("step\top\tdirection\tjob_count\trecord_shape\tlargest_line\tspread_pct\tlines\treps\n")
    for (step, op, direction, jc, shape), k in sorted(keys.items()):
        hi, lo = max(k["sizes"]), min(k["sizes"])
        spread = 100.0 * (hi - lo) / hi if hi else 0.0
        out.write("%s\t%s\t%s\t%s\t%s\t%d\t%.2f\t%d\t%d\n"
                  % (step, op, direction, jc, shape, hi, spread, k["lines"], len(k["sizes"])))

def largest(step, op, jc, shape):
    k = keys.get((step, op, "reply", str(jc), shape))
    return max(k["sizes"]) if k else None

def check(name, points):
    vals = [largest(*p) for p in points]
    ok = None not in vals and all(a < b for a, b in zip(vals, vals[1:]))
    print("%s %s %s" % ("GROWS" if ok else "FLAT", name, " ".join(str(v) for v in vals)))

for jc in (100, 1000):
    for verb, op in (("clean", "resolve"), ("cancel", "cancel_many"), ("kill", "kill_many")):
        check("%s_dup_%s_reply_h1_h10_h500_at_%d" % (verb, op, jc),
              [("%s_dup_h%d" % (verb, h), op, jc, "dup") for h in (1, 10, 500)])
check("clean_held_reply_100_to_1000", [("clean_held", "clean", jc, "held") for jc in (100, 1000)])
PY
}

run_all() {
  local out_dir=$1
  mkdir -p "$out_dir"
  work=$(mktemp -d /tmp/lrc-ipcw.XXXXXX)
  raw=$work/raw.tsv
  : >"$raw"
  build
  environment >"$out_dir/environment.log"
  for rep in 1 2 3; do
    log=$out_dir/run-$rep.log
    : >"$log"
    sweep
  done
  log=/dev/null
  aggregate >"$out_dir/sizes.log"
  summarise "$out_dir"
}

case ${1:-} in
--smoke) smoke ;;
--summarise) summarise "${2:?usage: measure.sh --summarise DIR}" ;;
--out) run_all "${2:?usage: measure.sh --out DIR}" ;;
'') run_all "$repo/docs/decision-gate/logs/ipc-sizes-wsl" ;;
*) die "usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW" ;;
esac
