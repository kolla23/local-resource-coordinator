# Decision gate for ADR 1: IPC message sizes

**Question:** how large do the coordinator's IPC lines get, request and reply, as the job count, the record size, an error list or the standing pause requests grow? ADR 1 sets per-message limits and says what SPEC's 1 MiB request figure ([SPEC.md](../fork/SPEC.md) §15 and §27.1) becomes; this note gives it measured bytes to size them on.

**Answer from this machine (Linux, WSL2):**
- Nothing caps a line today, and the current CLI already sends and gets lines far past 1 MiB. The largest replies come from error text: one Resolve, CancelMany or KillMany with 500 copies of a name that 1000 jobs share gets a single **84.7 MB** reply line, because every handle lists every match (`Resolution.error`, `StopOutcome.error`; src/resolve.rs:236-248).
- Full-record replies grow with every record kept, live or stopped: the List reply of `qex gc` and the Found reply of `qex list --all` are 1.73 MB at 1000 jobs of the smallest record, and 13.3 MB at 100 jobs whose argv holds one 131,071-byte string.
- Pause collections grow with the standing requests and their reasons, which nothing bounds: Info is 26.3 MB at 100 locks with the largest single-argument reason, and one queue reason is copied into every queued job's `blocked_reason`, which makes a 100-job List reply 11.8 MB.
- The exec ceilings bound what the CLI can put in one Submit from its own argv (about 1.97 MB), but not `qex submit --job FILE`: a 2 MiB argument goes over the wire as a 2.10 MB request, and the coordinator accepts it.
- JSON escaping expands a control byte 6 times and `"` 2 times; UTF-8 text goes over as is.

Every row below comes from the committed logs of one full run (3 reps) of [measure.sh](../../experiments/ipc-size-measure/measure.sh) at commit `6bf931d`; [environment.log](logs/ipc-sizes-wsl/environment.log) records the sha256 of the script and the proxy that ran, which match that commit.

## Setup
- Tools: [experiments/ipc-size-measure/](../../experiments/ipc-size-measure/), a bash script and a stdlib-only Python proxy, **not part of qex**. The script builds the release `qex` (`cargo build --release --locked`) and drives the real CLI against the real coordinator over its real Unix socket.
- Byte counter: the coordinator runs at state root A; the proxy listens at root B's socket and forwards every line unchanged to A's, writing per line only its connection, direction, `op` (from the request's `op` field), byte length without the newline, and job-state counts. No payload byte, environment value or path is written. B's `jobs` folder is a symlink to A's, because the CLI reads job folders locally; after every CLI call through B the script checks that no coordinator started at B and that A's coordinator pid did not change.
- Every dataset is preceded by a count check through A (`qex list --all --json`) that asserts the exact finished/running/queued split; the run aborts on a mismatch. Job counts 0, 100 and 1000 use the splits 0/0/0, 10/1/89 and 100/1/899; the unfinished jobs are `sleep 3600` at `--cpu full`, so one runs and the rest queue. Each destructive step (cancel, kill, abort) gets a fresh set.
- Every CLI call runs with a small fixed environment (`env -i`), so the env the CLI captures, and with it the sizes, repeat run to run; only the real-env Submit row uses the login shell's environment.
- A rep is one full pass: [run-1.log](logs/ipc-sizes-wsl/run-1.log), [run-2.log](logs/ipc-sizes-wsl/run-2.log) and [run-3.log](logs/ipc-sizes-wsl/run-3.log) hold each pass's count checks and boundary lines. [sizes.log](logs/ipc-sizes-wsl/sizes.log) has one row per op, direction, job count, record shape, step and rep; [summary.log](logs/ipc-sizes-wsl/summary.log) the largest line over the 3 reps and their spread. The largest spread across reps is 1.75% (a 114-byte Abort request); 1,154 keys have all 3 reps.
- Rerun check: a second full run of the same script (same sha256 for both files) on 2026-11-02, compared with `measure.sh --compare docs/decision-gate/logs/ipc-sizes-wsl/summary.log <rerun>/summary.log`, printed only `largest spread in the committed summary: 1.75%; tolerance used: 1.75%` and exited 0: every key in both summaries, every largest line within 1.75%. The rerun's logs are not committed.
- Environment: WSL2 kernel 5.15.167.4, rustc 1.98.1, `ARG_MAX` 2,097,152, `PAGESIZE` 4096, `MAX_ARG_STRLEN` = `PAGE_SIZE * 32`, `PATH_MAX` 4096.

## Job-count sweep (record shape `base`)
Largest reply line, in bytes, at 0, 100 and 1000 jobs; "lines" is the number of lines of that kind in one rep at 1000.

| Step (CLI) | Request → reply | 0 | 100 | 1000 | lines at 1000 | from |
|---|---|---|---|---|---|---|
| `qex gc --dry-run` | List → Jobs | 27 | 172,733 | 1,729,557 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex list --all --json` | Query → Found | 63 | 172,769 | 1,729,593 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex list --state queued --json` | Query → Found | 63 | 155,552 | 1,571,521 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex clean <unfinished ids>` | Resolve → Resolved | - | 15,152 | 151,232 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex clean <unfinished ids>` | Clean → Error (refused) | - | 191 | 191 | 900 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex cancel <all ids>` | CancelMany → Stopped | - | 19,325 | 100,855 | 2 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex kill --grace 1s <all ids>` | KillMany → Stopped | - | 29,609 | 150,533 | 2 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex clean <group id>` | Resolve → Resolved | - | - | 84,044 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex cancel <group id>` | CancelMany → Stopped | - | - | 96,275 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex status --json <id>` | Status → Status | - | 1,596 | 1,597 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex events --since start` | Events → Event lines | 219 | 1,947 | 1,945 | 515 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex info --json` | Info → Info | 486 | 544 | 545 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex abort --all --json` | Abort → Aborted | 414 | 453 | 455 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex resume --all --json` | Resume → PauseState | 331 | 331 | 332 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex pause --json` | PauseState → PauseState | 103 | 103 | 103 | 1 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

- Parts: `qex cancel` and `qex kill` send CancelMany/KillMany in parts of `STOP_MANY_PART = 500` handles (src/commands.rs:3317, :3327): at 1000 ids each rep has 2 request lines of up to 19,557 bytes, each answered by its own Stopped. `qex clean <ids>` sends one Resolve with every handle (src/commands.rs:3516, :3991-4004): 35,128 bytes for 900 ids. A group handle names all 1000 jobs of its pipeline in one request line of one handle.
- The events replay is capped by the ring, `RETAINED = 512` (src/events.rs:87), so its line count stays near 512 and its rows show the ring's bound, not growth with the count.
- Derived from these rows: a ≈ 27 bytes of envelope, b ≈ 1,727 bytes per smallest record in Jobs and Found; about 39 bytes per handle in a request (a quoted 36-character id and a comma), about 168 per entry in Resolved, about 201 per outcome in a cancel Stopped and 301 in a kill Stopped (a refused line carries its sentence). Spread across reps: under 1% for every row above (the largest, 0.91%, is the 219-byte events line at count 0).
- The proxy note: Info, Pause, Resume, PauseState and Abort read the peer's pid (src/daemon.rs:1885-1996), and the peer through B is the proxy, so these replies lack the issuer chain they would carry for a direct CLI. Their rows are a lower bound; the pause sets below measure the chain inside stored requests, made through A.

## Error lists: ambiguous handles and held records

| Step | Reply | h = 1 | h = 10 | h = 500 | job count | from |
|---|---|---|---|---|---|---|
| `qex clean <name> × h` | Resolved | 17,353 | 173,251 | 8,661,031 | 100 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex clean <name> × h` | Resolved | 169,454 | 1,694,261 | 84,711,531 | 1000 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex cancel <name> × h` | Stopped | 17,356 | 173,263 | 8,661,533 | 100 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex cancel <name> × h` | Stopped | 169,457 | 1,694,273 | 84,712,033 | 1000 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex kill --grace 1s <name> × h` | Stopped | 17,356 | 173,263 | 8,661,533 | 100 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex kill --grace 1s <name> × h` | Stopped | 169,457 | 1,694,273 | 84,712,033 | 1000 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `qex clean <H id>`, H needed by every other job | Clean refusal (Error) | 14,105 at 100 | 141,005 at 1000 | - | 100, 1000 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

- `dup`: every job carries the same 128-character `--name` (the CLI's maximum). Each handle comes back with an error that lists every match, one line each with the id and the name, in `Resolution.error` (src/daemon.rs:1918-1936) or `StopOutcome.error` (src/daemon.rs:2489-2498), so nothing is cancelled, killed or deleted (the count check after the last step equals the one before). The CLI sends handles as given, with no deduplication, so the reply is h × m match lines: about 169 bytes per match line (169,454 / 1000), linear in h and in m. h = 500 is the CLI's own part size, not an extreme.
- `held`: one finished job H, needed by every unfinished job (split 1/1/98 and 1/1/998). The refusal names every holder (src/lifecycle.rs:213-240): about 141 bytes per holder ((141,005 - 14,105) / 900).
- These are the largest lines measured anywhere, and they need no unusual input: 1000 jobs that share a name and a 500-handle cancel.

## Submit, the exec ceilings and escaping

| Row | Request bytes | Detail | from |
|---|---|---|---|
| real environment (`bash -l`, env capture All) | 3,706 | its `env` was 2,469 bytes; submitter chain 6 ancestors | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| `--env K=` (empty value) | 1,361 | the baseline for the escape factors | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `--env K=<x × 1,024>` | 2,385 | | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `--env K=<x × 65,536>` | 66,897 | | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `--env K=<x × 131,069>` | 132,430 | the largest one variable (`MAX_ARG_STRLEN` less `K=` and the NUL) | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| one argv string of 131,071 bytes | 132,552 | 131,072 fails with E2BIG in the CLI's own exec | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| 15 `--env` strings of 131,000 bytes | 1,966,480 | the 16th fails with E2BIG (argv 2,096,244 bytes) | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| 15 argv strings of 131,000 bytes | 1,966,523 | the 16th fails with E2BIG (argv 2,096,093 bytes) | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| escape: 131,069 × byte 0x01 | 787,775 | factor 6.000 (each byte is `\u0001`) | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| escape: 131,069 × `"` | 263,499 | factor 2.000 | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| escape: 65,534 × U+00E9 (131,068 bytes) | 132,429 | factor 1.000 (sent as UTF-8) | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| `qex submit --job FILE`, one 2 MiB argument | 2,098,633 | sent and accepted; the job then fails (its own exec) | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| `qex submit --needs <100 ids>` | 5,253 | after 100 one-handle Resolves (196-byte replies) | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

- The factor is (request bytes - the empty-value row) / the value's bytes. A job's environment is not in its status record: the Status reply of the 131,069-byte `--env` job is 1,447 bytes, against 132,646 for the 131,071-byte argv job.
- `--job FILE` and the `--needs`/`--after` id lists are bounded by nothing on the exec path, so only a request limit bounds them.

## Record shapes (100 jobs, split 10/1/89)

| Shape | b, bytes per job | List (Jobs) | Found, `--all` | Status | Event line | Submit request | from |
|---|---|---|---|---|---|---|---|
| base (`true` / `sleep 3600`) | 1,727 | 172,733 | 172,769 | 1,596 | 1,947 | - | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| argv with a 16-byte string | 1,788 | 178,794 | 178,830 | 1,661 | 2,031 | 1,432 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| argv with a 4 KiB string | 5,962 | 596,237 | 596,273 | 5,834 | 6,297 | 5,605 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| argv with a 131,071-byte string | 132,937 | 13,293,734 | 13,293,770 | 132,809 | 133,272 | 132,580 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `--name` of 128 characters | 1,847 | 184,716 | 184,752 | 1,717 | 2,180 | 1,488 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `--cwd` of 3,838 bytes | 5,547 | 554,752 | 554,788 | 5,418 | 5,763 | 5,189 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

- b = (the List reply at 100 jobs - the List reply at 0 jobs) / 100. Each shape adds about its extra bytes once per record (the cwd shape: 5,547 - 1,727 = 3,820 for 3,819 more cwd bytes); an Event line carries the whole record plus about 350 bytes of envelope. Per-job record max is therefore f(request limit): roughly the Submit request's argv, name and cwd plus about 1.7 KB.

## Pause collections (count 0 unless noted)
P standing requests; the first P - 1 go through A, so their stored issuer chains are real (6 ancestors each in this shell); the P-th goes through B. Largest reply line in bytes.

| Set | Info | PauseState | bare `qex resume` (Error) | resume all / lock `--all` | from |
|---|---|---|---|---|---|
| queue, P = 1, 10, 100, reason 1 KiB | 2,760 / 16,575 / 154,725 | 2,502 / 16,317 / 154,467 | 1,602 / 13,146 / 128,528 | 1,579 / 15,394 / 153,544 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| queue, P = 10, reason none / 1 KiB / 64 KiB / 131,062 | 5,333 / 16,575 / 726,207 / 1,446,993 | 5,075 / 16,317 / 725,949 / 1,446,735 | 2,976 / 13,146 / 658,266 / 1,313,526 | 5,174 / 15,394 / 660,514 / 1,315,774 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| queue, P = 100, reason 131,062 | 13,288,563 | 13,288,305 | 13,132,402 | 13,157,344 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| lock_many, P = 1, 10, 100, reason 1 KiB | 2,948 / 27,698 / 275,198 | 2,564 / 27,314 / 274,814 | 1,647 / 1,621 / 1,621 | 1,347 / 26,096 / 273,596 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| lock_many, P = 10, reason none / 1 KiB / 64 KiB / 131,062 | 7,258 / 27,698 / 1,317,938 / 2,628,458 | 6,874 / 27,314 / 1,317,554 / 2,628,074 | - | 6,678 / 26,096 / 1,251,824 / 2,496,818 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| lock_many, P = 100, reason 131,062 | 26,282,798 | 26,282,414 | - | 26,151,158 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| lock_many, P = 10, 4 KiB lock names, no reason | 48,058 | 47,674 | - | 43,398 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| lock_one, P = 1, 10, 100, reason 1 KiB | 2,948 / 16,763 / 154,913 | 2,564 / 16,379 / 154,529 | 1,647 / 13,191 / 128,573 | 1,347 / 15,162 / 153,312 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| `blocked_reason`, 100 jobs, one queue request of 1 KiB / 131,062 | List 263,548 / 11,836,929 | Found 263,584 / 11,836,965 | Status 2,800 / 132,839 | Event line 2,967 / 133,000 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

- Info carries `pauses`, `paused_reason` and `paused_locks`; PauseState carries `queue`, `locks`, `pauses` and `resumed`; Aborted carries `pauses`; a bare resume's Error lists each standing request of its target (src/proto.rs:541-557, 590-607, 640; src/daemon.rs:2239-2261).
- Bytes per reason byte, between the 1 KiB and the 131,062-byte sets at P = 10 (the COEF lines of `measure.sh --summarise`, reason bytes read from the run logs): Info and PauseState 11.000 for `queue` (P views plus the copy of the oldest request) and 20.000 for `lock_many` (each lock's view plus its LockPause record), as the plan's analysis predicted. A 4 KiB lock name adds 1.0 byte per name byte per lock ((48,058 - 7,258) / 10 / 4,080).
- Per request, without its reason: about 511 bytes per queue request in Info ((154,725 - 16,575) / 90 - 1,024) and about 702 per lock with its record ((275,198 - 27,698) / 90 - 2 × 1,024), each including a stored issuer chain of 6 ancestors; the 64-ancestor worst case (src/os/unix.rs:303-304) is derived, not measured.
- `blocked_reason`: while one queue request stands, each queued job's record carries its reason (src/pause.rs:1119-1124, src/sched.rs:1650-1653): the List reply grows by 89.0 bytes per reason byte ((11,836,929 - 263,548) / 130,038), one copy per queued job.

## Every wire field, with its verdict
The fields of the Request and Response variants that are a Vec, a String, a Box or a nested payload (39, from `awk '/^pub enum (Request|Response) /,/^}/ {print FNR": "$0}' src/proto.rs | grep -E '^[0-9]+: +([A-Z][A-Za-z]* \{ )?[a-z_]+: (Option<)?(Box<|Vec<|String|crate::|PauseTarget|AbortScope)'`, src/proto.rs at this commit):
- Measured by the sweep, the shapes or the Submit rows: Submit `spec` and `submitter` (:46, :52); Query `filter` (:77, fixed per command); Resolve, CancelMany and KillMany `handles` (:84, :90, :93); Events `since` (:132, a cursor); Jobs `jobs` (:470); Status `status` (:472); Found `jobs` (:481); Resolved `found` (:490); Stopped `outcomes` (:492); Capabilities `names` (:583, 224 bytes); Event `event` (:588); Error `message` (:643, the Clean refusal and the bare-resume Error).
- Measured by the pause sets: Pause `target` and `reason` (:139, :141); Resume `target` and `pause_id` (:155, :163); Info `pauses`, `paused_reason`, `paused_locks` (:548, :551, :557); PauseState `queue`, `locks`, `pauses`, `resumed` (:593, :594, :598, :607); Aborted `pauses` (:640).
- Bounded and derived, not measured: Abort `scope` (:183, a tag and a path); Submitted `warning` (:456, one line per own lock a person holds, src/daemon.rs:2752-2785); Found `rows` (:479, the short rows of `qex list` without `--json`, a subset of the record's fields, src/proto.rs:237); Info `version` and `queue_state` (:496, :537, short words), `config_error` and `pools` (:516, :580, from the owner's config file, left at its defaults here), `health` (:572, the head job's id and safe name and a scheduler sentence whose maximum is not traced); Aborted `not_cancelled`, `signalled`, `not_stopped`, `continues` (:623-630): per job, an id and a short sentence (src/proto.rs:623-630), estimated at the size of a kill Stopped outcome (about 301 bytes per job, from the sweep); not measured, as the sweep cannot produce these lists.
- The resolver's other lists (pipelines that share a name, src/resolve.rs:132-156; a name of both a job and a pipeline, :212-229) give at most two short entries per record, less than the measured 169-byte match line; derived, not measured.
- History is not an IPC message: `grep -n -i history src/proto.rs` prints nothing (exit 1); history's text reaches the wire only inside the NoSuchJob error sentence (src/daemon.rs:2344-2349).

## How ADR 1 turns these into limits
- Request limit ≥ the OS ceiling × the worst expansion factor (6 for control bytes) + the envelope. Linux: one string at most 131,071 bytes, all argv and env at most `ARG_MAX`. Windows, analytic (no Windows CLI exists yet): a command line of at most 32,767 characters including the NUL ([CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)), one variable of at most 32,767 characters and "no technical limitation on the size of the environment block" ([Environment Variables](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables)), both pages as retrieved 2026-11-01, so the ADR picks a product cap for the environment; up to 3 UTF-8 bytes per UTF-16 unit and 6 per escaped control character.
- Per message, sized on the largest message the current CLI sends: Jobs and Found = a + R × per-job record max (R = every record kept, as nothing is deleted until `qex gc --older-than` runs); Resolved = a + h × per-handle entry + n × per-job entry (one Resolve carries every handle); CancelMany/KillMany = a + 500 × handle max, with Stopped = a + 500 × per-outcome entry + n × per-line entry; error lists = a + h × (per-handle entry + m × per-match line), and the Clean refusal a + H × per-holder entry; pause collections = a + P × (view + reason + chain) + L × (lock record + the oldest reason + its views), plus the queue's oldest reason once; the stream's line limit = per-job record max + the Event envelope, with a replay of at most `RETAINED = 512` lines.
- N_max, R_max, h_max, the matches listed per handle, the standing-request maximum and the reason and lock-name bounds are ADR decisions, as is what each refusal says and how `qex gc`, `qex clean` and `qex resume --all`, the remedies, still answer past a limit.

## Known limits
- Linux under WSL2 only, one machine. The Windows ceilings and factors are analytic.
- The peer-dependent replies (Info, Pause, Resume, PauseState, Abort) are measured through the proxy, so they lack the reader's issuer chain; stored chains are only as long as this shell's process tree.
- One Resolve's handle count h is measured up to 500; the CLI's argv is its only bound. The events replay is capped by the ring.
- Derived, not measured: the fields listed as such above, and the 64-ancestor chain.
- `measure.sh --compare` checks the largest line per summary key, not every row of sizes.log.
- The `from:` cells show that each row names a log, not that its numbers are in it. Numbers in prose are checked by review against the logs, not by a command.
