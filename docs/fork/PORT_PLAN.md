# Plan: the backend layer, the ADRs and the Windows port

**Date:** 2026-10-02
**Status:** approved by owner 2026-10-02; accepted when the owner merges the PR that adds this file
**Context:** [04-recommendation.md](../decision-gate/04-recommendation.md) option (a), continue the fork; the inventory in [03-unix-inventory.md](../decision-gate/03-unix-inventory.md)

## Owner decisions
1. The test fixture `testjob` is a feature-gated binary (see "Portable test fixtures").
2. Job records and the coordinator's pipe allow the owner, SYSTEM and Administrators, and deny everyone else.
3. The per-job TEMP ADR is written alongside ADR 1, not last.
4. SQLite is dropped from V1 ([DECISION_PERSISTENCE.md](DECISION_PERSISTENCE.md)). ADR 3 covers CLI compatibility for signals.
5. Work starts with two separate PRs: the fixture sample and R1.

## Order of work
1. This plan (docs only).
2. The fixture-sample PR and R1, as separate PRs. R1 also adds the move checker.
3. In parallel:
   - R2–R6, which wait for no ADR, in any order the reviews allow.
   - The ADRs in the order of the "ADRs" table: ADR 1 and the per-job TEMP ADR first, ADR 4 drafted alongside them if useful.
4. R7–R10, each after its ADR.
5. The Windows backend, starting with removing the `compile_error!` and the `#[cfg(unix)]` module gates in `main.rs`.

Every step is its own PR under about 400 changed lines and runs the review loop.

## The backend layer: small PRs that change no behaviour on Linux

**Shape:**
- A module `src/os/` with `linux.rs` and `macos.rs` now, and `windows.rs` later.
- Its functions are plain functions chosen at compile time by `cfg`. No trait objects and no plugin framework (SPEC.md:885, "small backend contracts").
- Callers name what they need (`os::available_memory()`, `os::lock_exclusive(&file)`) and never `libc` or `std::os::unix`.
- Every modified upstream file gets the fork header line.

**Evidence in every R-PR description:**
1. **Tests unchanged.** No assertion in `tests/` or in a unit test changes. A unit test may only move with its code; `git diff --color-moved` shows that.
2. **Code only moves.** The move checker, `.github/scripts/check-moves.py` (added in R1 with its own test, reused by every R-PR), compares each moved function body before and after and reports it byte-identical apart from the call path. The PR lists and explains every line that is not a move.
3. **Public surface unchanged.** The help text for every subcommand and `qex schema <name>` for each of `schema::NAMES` (job, status, pipeline, event) give identical output before and after.
4. **Full Linux checks in WSL2:** fmt, clippy, unit tests, the e2e suite twice (to catch flakes), release build, MSRV; and CI passes.
5. **macOS CI started by hand** on every PR that moves a `cfg(target_os = "macos")` branch. If it was not run, the PR says macOS is "not evaluated".

**Order.** Seams that depend on no ADR come first. The seams of the five design areas wait for their ADR, so that the interface does not decide the ADR in advance.

| # | PR | Contents | Callers touched | Waits for |
|---|---|---|---|---|
| R1 | Memory, CPU and clocks | Move the `sys.rs` memory, CPU and clock bodies into `os/{linux,macos}.rs`. `sys.rs` keeps its signatures and forwards to them. Adds the move checker. | none | — |
| R2 | Process identity | `pid_alive`, `job_pid_alive`, `own_pid_alive`, `process_start_token`, `same_process_start`, `process_info`, `process_exe`, `boot_id`, `pid_namespace`, `submitter_chain`, `group_usage` | none (still through `sys::`) | — |
| R3 | File modes and locks | `ensure_dir(mode)`, owner-only writes, `flock` → `os::fs::{set_owner_only, lock_exclusive}`. e2e already pins the job folder (0700), `spec.json` and the stdout/stderr logs (0600). R3 first adds characterization tests for every file, folder and socket that the code creates owner-only and no test pins yet. The list comes from a grep of the mode calls (`ensure_dir`, `.mode(0o…)`, `write_atomic`, `restrict_socket`), and the PR shows that grep. At the time of writing it is: history, `usage.json`, `paused.json`, `daemon.log`, `status.json`, `supervisor.log`, `supervisor.pid`, `hook.log`, `hook.ran`, `update.json`, `update.lock`, the coordinator's `run/pid`, the control socket and the logcap tail file. These tests pass today, so they change no behaviour. | every caller of `ensure_dir`, owner-only modes or `flock`: client, commands, daemon, history, hook, job, logcap, paths, pause, supervisor, update, usage (peers waits for R10). Likely split into R3a (modes) and R3b (locks). | — |
| R4 | Terminal and console | `isatty`, terminal size, raw keys (`keys.rs`), the `SIGPIPE` gates in `main.rs`, Ctrl-C while waiting | style, top, keys, main, client | — |
| R5 | Detached start and reaping | Starting the supervisor and the coordinator (`setsid`), the `waitpid` reaper → `os::spawn_detached`, `os::wait_child` | supervisor, client, daemon | — |
| R6 | Sub-processes with a time limit | The stop hook and the update check's `curl`: `poll` and kill → `os::run_bounded` | hook, update | — |
| R7 | IPC transport | `os::ipc::{Listener, Stream, connect, peer_pid}`; `UnixStream` no longer leaks into `Connected::Open`. Framing stays newline-based on Linux. | paths, daemon, client, events | ADR 1 |
| R8 | Stopping and signalling | `killpg`, the graceful-then-forced stop → `os::stop_job(...)` | lifecycle, supervisor | ADRs 2, 3 |
| R9 | Memory metric input | Peak-memory measurement behind `os::`. The field stays `max_rss` until ADR 4 changes it. | supervisor, usage | ADR 4 |
| R10 | Peers | The peer directory becomes a backend capability (`os::peers_supported()`), or what ADR 5 decides | peers, sched | ADR 5 |

- **Size.** Moved code counts twice in a diff. R2 and R3 are near the limit and may split in two.
- **Tripwire.** 04-recommendation names this refactor as the test of whether the layer can be drawn cleanly: recommend (c) instead if "the scheduler or daemon state needs pids or process groups in its core types rather than at the edges".
  - R2 moves code without touching callers, so it cannot answer this by itself. Its PR description therefore also lists every pid or process-group field and use in the core types of `sched` and `daemon`, with where each is used. That gives the owner an early answer.
  - R5, R7 and R8 confirm it, because they change `daemon`, `lifecycle` and `supervisor`.
  - If either step meets the trigger, work stops and goes back to the owner.
- **Not in these PRs:** the `compile_error!` and the 37 `#[cfg(unix)]` module gates in `main.rs` stay until the Windows backend exists.

## ADRs
All ADRs go in `docs/fork/adr/`, one docs-only PR each, written while R2–R6 are in progress.

| Order | ADR | Why here |
|---|---|---|
| 1 | **IPC transport and launch protocol**: named pipes, who may start a coordinator, liveness without a socket file; SPEC's framing gap decided at the same time | The largest area (≈800 lines). AGENTS.md requires an ADR before any launch change. Unblocks R7. Sets the pipe security that the acceptance check below tests: the ACL, local-only clients (SPEC.md:615), and protection against another user creating the pipe name first. |
| 1 (alongside) | **Per-job TEMP** | Depends on no other ADR. Changes Linux behaviour too: it overrides the submitted environment, against SPEC.md:37 and :999, so it is a product decision. |
| 2 | **Who may signal a job**: a named Job Object, or a request routed through the supervisor | The routed option needs a channel to the supervisor, which ADR 1 defines. |
| 3 | **Exit classification, `--signal` and CLI compatibility for signals** | Depends on 2: how a job is stopped decides what its exit code means. Settles HUP, QUIT and USR1/2. |
| 4 | **Memory metric in history and admission**: commit vs working set, metric-kind labels, the commit ledger and gate, `max_pressure` | Independent of 1–3 and may be drafted alongside them. Changes the on-disk history, so it comes before peers. |
| 5 | **Peer directory** | Peer claims carry ADR 4's units. The likely answer is "off on Windows" (single-user machine, *inferred*). |

## Portable test fixtures: a sample to measure the test-port cost early
This answers 04-recommendation's other tripwire: does the test port cost more than the product port?

- **Fixture:** a small std-only Rust program, `testjob`, with subcommands `exit N`, `sleep SECS`, `print TEXT [--stderr]` and `hold-mem MiB SECS`.
  - It is a `[[bin]]` with `required-features = ["test-fixtures"]`, so `cargo install` never installs it.
  - The e2e suite reaches it through `env!("CARGO_BIN_EXE_testjob")`. Without the feature, that line no longer compiles, so the fixture PR also gives `tests/e2e.rs` a `[[test]]` entry with the same `required-features`. It then updates every command that builds or runs the e2e suite, so none of them silently skips it:
    - `ci.yml` (clippy `--all-targets`, the e2e job, the MSRV `check --all-targets`)
    - `macos.yml` and `release.yml` (their e2e steps)
    - `docs/baseline/run-baseline-linux.sh` and the commands in AGENTS.md
    - the test commands in README.md and CONTRIBUTING.md (upstream's shipped docs, which e2e scans; a plain `cargo test` would silently skip e2e)
    - any other hit of a repo-wide grep for `cargo test`, `cargo clippy` and `--all-targets`, which the PR shows
- **Sample of 10 tests.** In each category, the first tests in file order, skipping any with an open flaky-test issue. The occurrence counts are multi-line-aware matches in `tests/e2e.rs` at the time of writing; they are not test counts:

  | Category (occurrences) | Tests in sample |
  |---|---|
  | `true` / `false` (200) | 3 |
  | `sleep` (98) | 2 |
  | `sh -c` (90) and `bash` (12) | 2 (one simple, one with pipes or redirects) |
  | `echo` (27) | 1 |
  | `libc::kill` (57) | 1 (its Windows meaning waits for ADRs 2 and 3; the sample records that cost) |
  | `/tmp` (12) | 1 |

- **Not sampled, but counted:** every other pattern that `experiments/unix-inventory/e2e-unix.py` scans for, plus the Unix tools that tests start. They form an "other Unix" category in the estimate:
  - other `libc::` uses (163 in all, 57 of them `kill`), `SIG*` names and `"kill"`
  - `std::os::unix` (`PermissionsExt`, `UnixListener`/`UnixStream` fake coordinators, `ExitStatusExt`, `CommandExt`, `MetadataExt`)
  - `/proc`, `"/bin/` paths, `.sh"` scripts and `"cat"`
  - other tools: `ps`, `mkfifo`, `bwrap`, `lsof`, `chmod`, `cp`, `touch`, `printf`

- **Measured per test:** changed lines, time spent, whether the semantics shifted (shell features with no portable equivalent), whether it passes on Linux in 3 of 3 runs, runtime before and after.
- **On Windows:** `testjob` is built natively and its subcommands are run. The converted e2e tests cannot run on Windows until the coordinator builds, and the write-up says so.
- **Output:** `docs/decision-gate/05-test-fixture-sample.md`.
  - It counts **tests** per category (a test whose body names the pattern, as `experiments/unix-inventory/e2e-unix.py` does), not occurrences, and multiplies by the sampled cost per test.
  - The "other Unix" category has no sample, so it is costed at the highest sampled per-test cost and labelled as an estimate.
  - Helpers aren't followed, so the total is a lower bound, and the write-up says so.
  - It compares the total with the product estimate (≈3,635 lines). If the test port comes out larger, that goes to the owner as the "re-check option (c)" trigger.

## Acceptance check for the port: other Windows users cannot read job records
The Windows backend is not done, and no Windows release is made, until this check passes. It is not a "verify later" note.

- **Covers:**
  - the state root, every job record and log or output capture, history, usage and the pause file
  - the per-job TEMP folders, wherever the per-job TEMP ADR puts them
  - the named Job Object, if ADR 2 chooses one
  - the coordinator's named pipe
- **Rule:**
  - "The user" means the SID of the user who runs the coordinator, read from its process token. It is not read from the descriptor being checked.
  - Every such object gets an explicit, protected, non-NULL DACL with inheritance turned off. The inherited profile ACL is *not verified* and is not relied on.
  - The DACL holds allow ACEs only for the user, SYSTEM and the Administrators group. Everyone else is denied because no ACE allows them; there is no deny ACE for Everyone, since that would also deny the user (owner decision 2).
  - The descriptor's owner is the user, even when the coordinator runs elevated (an elevated process would otherwise make Administrators the owner).
  - Atomic replace: a rename keeps the *source* file's DACL, so the temporary file is created with the restricted DACL, not fixed after the rename.
  - The pipe also:
    - accepts local clients only (SPEC.md:615)
    - is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so the coordinator never joins a pipe that someone else created
    - is used by clients only after they check that the server process runs as the same user. Otherwise another user could create the name first and receive submitted environments.
- **Automated Windows e2e test:**
  1. Submit a job whose record holds a known secret value in its environment.
  2. Walk the state root, and the per-job TEMP folders if they are elsewhere. Read each object's owner and DACL with `GetNamedSecurityInfoW`; read the pipe's and the Job Object's with `GetSecurityInfo`.
  3. Fail if:
     - a DACL is NULL or inheritance is on
     - any ACE grants access to a SID other than the user, SYSTEM or Administrators
     - an owner is not the user
  4. Create the pipe name first from a process that runs as a second local account, which the test setup creates. Then start a coordinator: it must refuse to serve, and a client must refuse to send to that process. (A squatter running as the same user is out of scope: same-user processes are trusted, as on Unix.)
  5. Connect to the pipe through the SMB loopback path `\\127.0.0.1\pipe\<name>`, which arrives as a remote client. The connection must be refused. If the test machine cannot make such a connection at all (for example, SMB is off), the test reports "local-only: not evaluated" and does not pass that step.
  6. After the job ends, search for the secret value from step 1 in the user's `%TEMP%`, `%LOCALAPPDATA%`, `%ProgramData%` and the job's working folder. Fail if it appears anywhere outside the objects checked in step 3. This search covers the likely places, not the whole disk, and the test says so.
- **Manual check, recorded in `docs/baseline/`:**
  1. Log in as a second, standard (non-admin) local account.
  2. Try to read a job file, its log and the history with `type` and `Get-Content`, and try to connect to the pipe. Each attempt fails with "access denied".
- **Linux counterpart:** the existing e2e mode tests plus the characterization tests R3 adds (see R3) pin 0700 on folders and 0600 on every private file in the state root, so both backends are held to the same rule.
