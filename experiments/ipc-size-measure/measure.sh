#!/usr/bin/env bash
# Measures qex IPC line sizes through a passive proxy; see docs/decision-gate/07-ipc-message-sizes.md.
# Usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
target=${CARGO_TARGET_DIR:-$HOME/.cache/lrc-ipc-size-measure/target}
Q=$target/release/qex
A='' B='' proxy_pid='' coord_pid='' label_file='' raw=/dev/null log=/dev/null work=''
rep=0 job_count=0 shape=base runner=in_root canary='' canaries=()
fin_ids=() unf_ids=() fin_cmd=(true) unf_cmd=(sleep 3600)
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

# The real login environment plus the canary, for the real-env Submit row only.
login() {
  (cd "$A" && env LRC_CANARY="$canary" XDG_CONFIG_HOME="$A/cfg" XDG_STATE_HOME="$1" \
    XDG_RUNTIME_DIR="$1/run" QEX_IDLE_EXIT_SECS=3600 QEX_PEERS_DIR="$A/peers" bash -lc "$2" "${@:3}")
}
in_login() {
  local root=$1
  shift
  # shellcheck disable=SC2016 # the login shell expands $0 and $@
  login "$root" '"$0" "$@"' "$Q" "$@"
}

fill() { printf "%${1}s" '' | tr ' ' "$2"; }

# One measured CLI call through root B, output in $B/out; the proxy tags its lines with this label.
qb() {
  local step=$1 rc=0
  shift
  printf '%s\t%s\t%s\t%s\n' "$step" "$rep" "$job_count" "$shape" >"$label_file"
  "$runner" "$B" "$@" >"$B/out" 2>&1 || rc=$?
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
  par_submit "$f" "$B/fin" "$@" -- "${fin_cmd[@]}"
  mapfile -t fin_ids <"$B/fin"
  [ "$f" -eq 0 ] || qa wait "${fin_ids[@]}" >/dev/null
  if [ $((r + q)) -gt 0 ]; then
    # The first unfinished job goes alone and starts before the rest arrive, so it is the one that runs.
    unf_ids=("$(qa submit --cpu full "$@" -- "${unf_cmd[@]}")")
    wait_load 1 0
    par_submit $((r + q - 1)) "$B/unf" --cpu full "$@" -- "${unf_cmd[@]}"
    mapfile -t -O 1 unf_ids <"$B/unf"
  fi
  wait_load "$r" "$q"
}

# par_submit N FILE ARGS...: N submits, 8 at a time; ids go to FILE (one short append each).
par_submit() {
  local n=$1 file=$2 i p pids=()
  shift 2
  : >"$file"
  for ((i = 0; i < n; i++)); do
    qa submit "$@" >>"$file" &
    pids+=($!)
    if [ "${#pids[@]}" -eq 8 ] || [ $((i + 1)) -eq "$n" ]; then
      for p in "${pids[@]}"; do wait "$p" || die "a parallel submit failed"; done
      pids=()
    fi
  done
  [ "$(grep -c . "$file")" -eq "$n" ] || die "$n submits gave $(grep -c . "$file") ids"
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
  par_submit $((n - 1)) "$B/unf" --cpu full --needs "$held" --name "$name128" -- sleep 3600
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

# req_bytes STEP [OP]: the largest request line of that step in this rep and shape.
req_bytes() {
  awk -F'\t' -v s="$1" -v r="$rep" -v sh="$shape" -v op="${2:-submit}" \
    '$2 == s && $3 == r && $5 == sh && $6 == "request" && $7 == op && $8 > m {m = $8} END {print m + 0}' "$raw"
}

wait_request() {
  for _ in $(seq 100); do
    if awk -F'\t' -v s="$1" -v r="$rep" -v sh="$shape" -v op="$2" \
      '$2 == s && $3 == r && $5 == sh && $6 == "request" && $7 == op {f = 1} END {exit !f}' "$raw"; then
      return 0
    fi
    sleep 0.1
  done
  die "the proxy saw no $2 request in step $1"
}

id_in_out() { grep -oE '^[0-9a-f]{8}-[0-9a-f-]{27}$' "$B/out" | tail -n1; }
argv_bytes() { printf '%s\0' "$@" | wc -c; }

# e2big_search KIND: adds 131000-byte strings (env vars or argv) until the CLI's own exec fails.
e2big_search() {
  local kind=$1 big k args=() ok_bytes=0
  big=$(fill 131000 x)
  for ((k = 1; k <= 64; k++)); do
    if [ "$kind" = env ]; then args+=(--env "K$k=$big"); else args+=("$big"); fi
    count_check "${kind}_many_k$k" -
    local cmd=(submit "${args[@]}" -- true)
    [ "$kind" = env ] || cmd=(submit -- true "${args[@]}")
    if qb "submit_${kind}_many_k$k" "${cmd[@]}"; then
      ok_bytes=$(argv_bytes "$Q" "${cmd[@]}")
      continue
    fi
    grep -q 'Argument list too long' "$B/out" || die "submit with $k $kind strings failed: $(head -c 300 "$B/out")"
    say "e2big $kind last_ok=$((k - 1)) argv_bytes_ok=$ok_bytes first_fail=$k argv_bytes_fail=$(argv_bytes "$Q" "${cmd[@]}") request_bytes_ok=$(req_bytes "submit_${kind}_many_k$((k - 1))")"
    return 0
  done
  die "no E2BIG after 64 $kind strings"
}

submit_rows() {
  local n rc id base_req req name oct value state
  shape=submit
  setup
  canary=lrc-canary-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')
  canaries+=("$canary")
  count_check submit_real_env -
  runner=in_login
  qb submit_real_env submit -- true
  runner=in_root
  id=$(id_in_out)
  say "submit-real-env env_bytes=$(login "$B" 'env | wc -c') request_bytes=$(req_bytes submit_real_env) chain_len=$(qa status --json "$id" | python3 -c 'import json, sys; print(len(json.load(sys.stdin)["submitter"]))')"
  count_check submit_env_empty -
  qb submit_env_empty submit --env K= -- true
  base_req=$(req_bytes submit_env_empty)
  for n in 1024 65536 131069; do
    count_check "submit_env_$n" -
    qb "submit_env_$n" submit --env "K=$(fill "$n" x)" -- true
  done
  id=$(id_in_out)
  qb status_env_max status --json "$id"
  e2big_search env
  e2big_search argv
  count_check submit_argv_max -
  qb submit_argv_max submit -- true "$(fill 131071 x)" || die "a 131071-byte argument failed"
  id=$(id_in_out)
  qb status_argv_max status --json "$id"
  rc=0
  qb submit_argv_over submit -- true "$(fill 131072 x)" || rc=$?
  grep -q 'Argument list too long' "$B/out" || die "a 131072-byte argument did not fail with E2BIG (rc $rc)"
  say "single-arg max_ok=131071 first_fail=131072"
  # Escapes made with printf/tr octal escapes, never typed as raw bytes (AGENTS.md Gotchas).
  for name in ctl:'\001' quote:'\042' plain:x eacute:; do
    oct=${name#*:} name=${name%%:*}
    if [ "$name" = eacute ]; then
      value=$(printf '\303\251%.0s' $(seq 65534))
    else
      value=$(fill 131069 "$oct")
    fi
    count_check "escape_$name" -
    rc=0
    qb "submit_escape_$name" submit --env "K=$value" -- true || rc=$?
    req=$(req_bytes "submit_escape_$name")
    n=$(printf %s "$value" | wc -c)
    say "escape $name os_bytes=$n rc=$rc wire_bytes=$((req - base_req)) factor=$(awk -v w=$((req - base_req)) -v o="$n" 'BEGIN {printf "%.3f", w / o}')"
  done
  python3 -c 'import json, sys
json.dump({"command": ["true", "x" * 2097152]}, open(sys.argv[1], "w"))' "$work/job.json"
  count_check submit_job_file -
  rc=0
  qb submit_job_file submit --job "$work/job.json" || rc=$?
  id=$(id_in_out) state=none
  if [ -n "$id" ]; then
    qa wait "$id" >/dev/null 2>&1 || true
    state=$(qa status --json "$id" | jget state)
  fi
  say "job-file arg_bytes=2097152 rc=$rc request_bytes=$(req_bytes submit_job_file) state=$state"
  cleanup
  submit_split 100 0 0
  count_check submit_needs_100 100 0 0
  qb submit_needs_100 submit --needs "$(IFS=,; echo "${fin_ids[*]}")" -- true
  teardown
}

events_line() {
  local id ev rc=0
  qb events events --since now --count 2 --timeout 30s --json &
  ev=$!
  wait_request events events
  id=$(qa submit --cpu full "$@" -- "${unf_cmd[@]}")
  qa cancel "$id" >/dev/null
  wait "$ev" || rc=$?
  [ "$rc" -eq 0 ] || [ "$rc" -eq 124 ] || die "qex events exited $rc"
}

shape_rows() {
  local s pad cwd opts
  for s in argv16 argv4k argvmax name128 cwd; do
    setup
    shape=$s opts=() fin_cmd=(true) unf_cmd=(sleep 3600)
    case $s in
    argv16) pad=$(fill 16 p) ;;
    argv4k) pad=$(fill 4096 p) ;;
    argvmax) pad=$(fill 131071 p) ;;
    name128) opts=(--name "$name128") ;;
    cwd)
      cwd=$A
      while [ $((${#cwd} + 201)) -lt 4000 ]; do cwd=$cwd/$(fill 200 d); done
      mkdir -p "$cwd"
      opts=(--cwd "$cwd")
      say "cwd-shape bytes=${#cwd}"
      ;;
    esac
    if [[ $s == argv* ]]; then fin_cmd=(true "$pad") unf_cmd=(sh -c 'exec sleep 3600' sh "$pad"); fi
    submit_split 10 1 89 "${opts[@]}"
    count_check "shape_$s" 10 1 89
    qb gc gc --dry-run --json
    qb list_all list --all --json
    qb list_queued list --state queued --json
    qb status status --json "${unf_ids[0]}"
    qb submit submit --cpu full "${opts[@]}" -- "${unf_cmd[@]}"
    events_line "${opts[@]}"
    teardown
  done
  shape=base fin_cmd=(true) unf_cmd=(sleep 3600)
}

reason_of() {
  case $1 in
  r1k) fill 1024 x ;;
  r64k) fill 65536 x ;;
  rmax) fill 131062 x ;;
  esac
}

pause_set() {
  local layout=$1 p=$2 r=$3 n=${4:-} reason d i got names=() ropt=()
  shape=pause_${layout}_P${p}_$r${n:+_$n}
  setup
  count_check "$shape" 0 0 0
  reason=$(reason_of "$r")
  [ -z "$reason" ] || ropt=("--reason=$reason")
  for ((i = 1; i <= p; i++)); do
    case $layout in
    lock_many)
      if [ -n "$n" ]; then
        names+=("$(printf 'L%08d' "$i")$(fill 4087 l)")
      else
        names+=("$(printf 'lock%012d' "$i")")
      fi
      ;;
    *) names+=(lock000000000001) ;;
    esac
  done
  for ((i = 0; i < p - 1; i++)); do
    if [ "$layout" = queue ]; then
      qa pause queue --json "${ropt[@]}" >/dev/null
    else
      qa pause lock "${names[$i]}" --json "${ropt[@]}" >/dev/null
    fi
  done
  d=$(export LC_ALL=C && echo "${#reason}")
  got=$(qa pause --json --verbose | python3 -c 'import json, sys
v = json.load(sys.stdin)
views = v["pauses"] + [p for lock in v["locks"] for p in lock["pauses"]]
print("got=%d chain_len=%d" % (len(views), max([len(p["issuer_chain"]) for p in views] or [0])))')
  if [ "${got%% *}" = "got=$((p - 1))" ]; then
    say "pause-check $shape expected=$((p - 1)) $got reason_bytes=$d ok"
  else
    say "pause-check $shape expected=$((p - 1)) $got reason_bytes=$d MISMATCH"
    die "pause check $shape failed"
  fi
  if [ "$layout" = queue ]; then
    qb pause_add pause queue --json "${ropt[@]}"
  else
    qb pause_add pause lock "${names[$((p - 1))]}" --json "${ropt[@]}"
  fi
  qb info info --json
  qb pause_state pause --json
  if [ "$layout" = queue ]; then
    qb resume_bare resume --json || true
    qb abort abort --all --json || true
    qb resume_all resume --all --json
  else
    qb resume_bare resume lock "${names[0]}" --json || true
    qb resume_lock_all resume lock "${names[0]}" --all --json
  fi
  teardown
}

pause_jobs() {
  local r=$1 reason ev i rc=0
  shape=pause_jobs_P1_$r
  setup
  submit_split 10 1 89
  count_check "$shape" 10 1 89
  reason=$(reason_of "$r")
  qb events events --since now --count 1 --timeout 30s --json &
  ev=$!
  wait_request events events
  qa pause queue --json "--reason=$reason" >/dev/null 2>&1
  wait "$ev" || rc=$?
  [ "$rc" -eq 0 ] || [ "$rc" -eq 124 ] || die "qex events exited $rc"
  for i in $(seq 61); do
    [ "$i" -le 60 ] || die "the queued records did not get the reason as blocked_reason in 30 s"
    qa list --state queued --json 2>/dev/null | python3 -c 'import json, sys
jobs = json.load(sys.stdin)
sys.exit(0 if jobs and all(len(j.get("blocked_reason") or "") >= int(sys.argv[1]) for j in jobs) else 1)' "${#reason}" && break
    sleep 0.5
  done
  qb gc gc --dry-run --json
  qb list list --all --json
  qb status status --json "${unf_ids[1]}"
  teardown
}

pause_rows() {
  local spec layout p r n
  for spec in queue:1:r1k queue:10:r1k queue:100:r1k queue:10:rnone queue:10:r64k queue:10:rmax \
    queue:100:rmax lock_many:1:r1k lock_many:10:r1k lock_many:100:r1k lock_many:10:rnone \
    lock_many:10:r64k lock_many:10:rmax lock_many:100:rmax lock_many:10:rnone:n4k \
    lock_one:1:r1k lock_one:10:r1k lock_one:100:r1k; do
    IFS=: read -r layout p r n <<<"$spec"
    pause_set "$layout" "$p" "$r" "$n"
  done
  pause_jobs r1k
  pause_jobs rmax
  shape=base
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
  python3 - "$dir/sizes.log" "$dir/summary.log" "$dir"/run-*.log <<'PY'
import re
import sys
keys = {}
for line in list(open(sys.argv[1], encoding="utf-8"))[1:]:
    op, direction, size, lines, total, jc, shape, states, step, rep = line.rstrip("\n").split("\t")
    k = keys.setdefault((step, op, direction, jc, shape), {"sizes": [], "lines": 0})
    k["sizes"].append(int(size))
    k["lines"] = max(k["lines"], int(lines))

# Bytes per job of a record shape: (Jobs reply at 100 jobs - Jobs reply at 0 jobs) / 100.
derived = []
empty = keys.get(("gc", "list", "reply", "0", "base"))
for shape in ("base", "argv16", "argv4k", "argvmax", "name128", "cwd"):
    k = keys.get(("gc", "list", "reply", "100", shape))
    if k and empty:
        b = (max(k["sizes"]) - max(empty["sizes"])) / 100.0
        derived.append(("b_per_job", "list", "derived", "100", shape, int(round(b)), 0.0, 0, len(k["sizes"])))
with open(sys.argv[2], "w", encoding="utf-8") as out:
    out.write("step\top\tdirection\tjob_count\trecord_shape\tlargest_line\tspread_pct\tlines\treps\n")
    rows = []
    for (step, op, direction, jc, shape), k in sorted(keys.items()):
        hi, lo = max(k["sizes"]), min(k["sizes"])
        spread = 100.0 * (hi - lo) / hi if hi else 0.0
        rows.append((step, op, direction, jc, shape, hi, spread, k["lines"], len(k["sizes"])))
    for row in rows + derived:
        out.write("%s\t%s\t%s\t%s\t%s\t%d\t%.2f\t%d\t%d\n" % row)

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

def pause(layout, p, r):
    return "pause_%s_P%d_%s" % (layout, p, r)

for layout in ("queue", "lock_many", "lock_one"):
    for step in ("info", "pause_state"):
        check("%s_%s_reply_P1_P10_P100" % (layout, step), [(step, step, 0, pause(layout, p, "r1k")) for p in (1, 10, 100)])
for layout in ("queue", "lock_one"):
    check("%s_resume_bare_reply_P1_P10_P100" % layout, [("resume_bare", "resume", 0, pause(layout, p, "r1k")) for p in (1, 10, 100)])
check("queue_abort_reply_P1_P10_P100", [("abort", "abort", 0, pause("queue", p, "r1k")) for p in (1, 10, 100)])
check("queue_resume_all_reply_P1_P10_P100", [("resume_all", "resume", 0, pause("queue", p, "r1k")) for p in (1, 10, 100)])
for layout in ("queue", "lock_many"):
    for step in ("info", "pause_state"):
        check("%s_%s_reply_P10_by_reason" % (layout, step),
              [(step, step, 0, pause(layout, 10, r)) for r in ("rnone", "r1k", "r64k", "rmax")])
check("pause_jobs_gc_list_reply_r1k_to_rmax", [("gc", "list", 100, "pause_jobs_P1_" + r) for r in ("r1k", "rmax")])
check("pause_jobs_list_query_reply_r1k_to_rmax", [("list", "query", 100, "pause_jobs_P1_" + r) for r in ("r1k", "rmax")])

# COEF: reply bytes per reason byte between the r1k and rmax sets at P = 10; d is read from the run logs.
reason = {}
for log in sys.argv[3:]:
    for line in open(log, encoding="utf-8"):
        m = re.match(r"pause-check (\S+) .* reason_bytes=(\d+) ", line)
        if m:
            reason.setdefault(m.group(1), set()).add(int(m.group(2)))
for layout, k in (("queue", 10 + 1), ("lock_many", 2 * 10)):
    for step in ("info", "pause_state"):
        name = "%s_%s_P10" % (layout, step)
        lo_shape, hi_shape = pause(layout, 10, "r1k"), pause(layout, 10, "rmax")
        d_lo, d_hi = reason.get(lo_shape, set()), reason.get(hi_shape, set())
        b_lo, b_hi = largest(step, step, 0, lo_shape), largest(step, step, 0, hi_shape)
        if len(d_lo) != 1 or len(d_hi) != 1 or b_lo is None or b_hi is None:
            print("COEF %s expected=%d got=- MISMATCH" % (name, k))
            continue
        s = (b_hi - b_lo) / float(min(d_hi) - min(d_lo))
        print("COEF %s expected=%d got=%.3f %s" % (name, k, s, "ok" if abs(s - k) < 0.01 else "MISMATCH"))
PY
}

# compare COMMITTED NEW: exit 0 clean, 1 on MISSING KEY or OUT OF TOLERANCE, 2 on a usage error.
compare() {
  if [ ! -f "${1:-}" ] || [ ! -f "${2:-}" ]; then
    echo "usage: measure.sh --compare COMMITTED_SUMMARY NEW_SUMMARY" >&2
    exit 2
  fi
  python3 - "$1" "$2" <<'PY'
import sys
def load(path):
    rows = {}
    for line in list(open(path, encoding="utf-8"))[1:]:
        f = line.rstrip("\n").split("\t")
        rows[tuple(f[:5])] = (int(f[5]), float(f[6]))
    return rows
old, new = load(sys.argv[1]), load(sys.argv[2])
spread = max(s for _, s in old.values())
print("largest spread in the committed summary: %.2f%%; tolerance used: %.2f%%" % (spread, spread))
bad = 0
for k in sorted(set(old) | set(new)):
    if k not in old or k not in new:
        print("MISSING KEY %s (only in the %s summary)" % (" ".join(k), "committed" if k in old else "new"))
        bad = 1
        continue
    a, b = old[k][0], new[k][0]
    diff = 100.0 * abs(b - a) / a if a else (0.0 if b == 0 else 100.0)
    if diff > spread:
        print("OUT OF TOLERANCE %s committed=%d new=%d diff=%.2f%%" % (" ".join(k), a, b, diff))
        bad = 1
sys.exit(bad)
PY
}

run_all() {
  local out_dir=$1
  mkdir -p "$out_dir"
  work=$(mktemp -d /tmp/lrc-ipcw.XXXXXX)
  raw=$work/raw.tsv
  : >"$raw"
  build
  environment | sed "s/$(whoami)/USERNAME/g" >"$out_dir/environment.log"
  for rep in 1 2 3; do
    log=$out_dir/run-$rep.log
    : >"$log"
    sweep
    submit_rows
    shape_rows
    pause_rows
  done
  log=/dev/null
  aggregate >"$out_dir/sizes.log"
  summarise "$out_dir"
  for canary in "${canaries[@]}"; do
    if grep -rqF "$canary" "$out_dir"; then die "the canary reached the output folder"; fi
  done
  echo "canary check: none of the ${#canaries[@]} canary values is in the output folder"
}

case ${1:-} in
--smoke) smoke ;;
--summarise) summarise "${2:?usage: measure.sh --summarise DIR}" ;;
--compare) compare "${2:-}" "${3:-}" ;;
--out) run_all "${2:?usage: measure.sh --out DIR}" ;;
'') run_all "$repo/docs/decision-gate/logs/ipc-sizes-wsl" ;;
*) die "usage: measure.sh --smoke | [--out DIR] | --summarise DIR | --compare COMMITTED NEW" ;;
esac
