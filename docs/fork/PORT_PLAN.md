# Plan: the backend layer, the ADRs and the Windows port

**Date:** 2026-10-02
**Status:** approved by owner 2026-10-02; accepted when the owner merges the PR that adds this file
**Context:** [04-recommendation.md](../decision-gate/04-recommendation.md) option (a), continue the fork; the inventory in [03-unix-inventory.md](../decision-gate/03-unix-inventory.md)

## Owner decisions
1. The test fixture `testjob` is a feature-gated binary (see "Portable test fixtures").
2. Job records and the coordinator's pipe allow the owner, SYSTEM and Administrators, and deny everyone else.
3. The per-job TEMP ADR is written alongside ADR 1, not last.
4. SQLite is dropped from V1 ([DECISION_PERSISTENCE.md](DECISION_PERSISTENCE.md)). ADR 3 covers CLI compatibility for signals; no separate CLI-compatibility ADR.
5. Work starts with two separate PRs: the fixture sample and R1.

## Order of work
1. This plan (docs only).
2. The fixture-sample PR and R1, as separate PRs. R1 also adds the move checker.
3. ADR 1 and the per-job TEMP ADR, then ADRs 2–5 (see "ADRs").
4. R2–R6, which wait for no ADR, in any order the reviews allow; R7–R10 after their ADRs.
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
3. **Public surface unchanged.** The help text for every subcommand and `protocol.schema.json` give identical output before and after.
4. **Full Linux checks in WSL2:** fmt, clippy, unit tests, the e2e suite twice (to catch flakes), release build, MSRV; and CI passes.
5. **macOS CI started by hand** on every PR that moves a `cfg(target_os = "macos")` branch. If it was not run, the PR says macOS is "not evaluated".

**Order.** Seams that depend on no ADR come first. The seams of the five design areas wait for their ADR, so that the interface does not decide the ADR in advance.

| # | PR | Contents | Callers touched | Waits for |
|---|---|---|---|---|
| R1 | Memory, CPU and clocks | Move the `sys.rs` memory, CPU and clock bodies into `os/{linux,macos}.rs`. `sys.rs` keeps its signatures and forwards to them. Adds the move checker. | none | — |
| R2 | Process identity | `pid_alive`, `job_pid_alive`, `own_pid_alive`, `process_start_token`, `same_process_start`, `process_info`, `process_exe`, `boot_id`, `pid_namespace`, `submitter_chain`, `group_usage` | none (still through `sys::`) | — |
| R3 | File modes and locks | `ensure_dir(mode)`, owner-only writes, `flock` → `os::fs::{set_owner_only, lock_exclusive}`. If no test pins the 0700/0600 modes yet, add a characterization test first; it passes today, so it changes no behaviour. | paths, job, usage, history, logcap | — |
| R4 | Terminal and console | `isatty`, terminal size, raw keys (`keys.rs`), the `SIGPIPE` gates in `main.rs`, Ctrl-C while waiting | style, top, keys, main, client | — |
| R5 | Detached start and reaping | Starting the supervisor and the coordinator (`setsid`), the `waitpid` reaper → `os::spawn_detached`, `os::wait_child` | supervisor, client, daemon | — |
| R6 | Sub-processes with a time limit | The stop hook and the update check's `curl`: `poll` and kill → `os::run_bounded` | hook, update | — |
| R7 | IPC transport | `os::ipc::{Listener, Stream, connect, peer_pid}`; `UnixStream` no longer leaks into `Connected::Open`. Framing stays newline-based on Linux. | paths, daemon, client, events | ADR 1 |
| R8 | Stopping and signalling | `killpg`, the graceful-then-forced stop → `os::stop_job(...)` | lifecycle, supervisor | ADRs 2, 3 |
| R9 | Memory metric input | Peak-memory measurement behind `os::`. The field stays `max_rss` until ADR 4 changes it. | supervisor, usage | ADR 4 |
| R10 | Peers | The peer directory becomes a backend capability (`os::peers_supported()`), or what ADR 5 decides | peers, sched | ADR 5 |

- **Size.** Moved code counts twice in a diff. R2 and R3 are near the limit and may split in two.
- **Tripwire.** 04-recommendation names this refactor as the test of whether the layer can be drawn cleanly. R2 answers it: if `sched` or `daemon` state needs pid or process-group *semantics* in its core types (not just a pid stored in a record), work stops and goes back to the owner before R3.
- **Not in these PRs:** the `compile_error!` and the 37 `#[cfg(unix)]` module gates in `main.rs` stay until the Windows backend exists.

## ADRs
All ADRs go in `docs/fork/adr/`, one docs-only PR each, written while R1–R6 are in progress.

| Order | ADR | Why here |
|---|---|---|
| 1 | **IPC transport and launch protocol**: named pipes, who may start a coordinator, liveness without a socket file; SPEC's framing gap decided at the same time | The largest area (≈800 lines). AGENTS.md requires an ADR before any launch change. Unblocks R7. Sets the pipe ACL that the acceptance check below tests. |
| 1 (alongside) | **Per-job TEMP** | Depends on no other ADR. Changes Linux behaviour too: it overrides the submitted environment, against SPEC.md:37 and :999, so it is a product decision. |
| 2 | **Who may signal a job**: a named Job Object, or a request routed through the supervisor | The routed option needs a channel to the supervisor, which ADR 1 defines. |
| 3 | **Exit classification, `--signal` and CLI compatibility for signals** | Depends on 2: how a job is stopped decides what its exit code means. Settles HUP, QUIT and USR1/2. |
| 4 | **Memory metric in history and admission**: commit vs working set, metric-kind labels, the commit ledger and gate, `max_pressure` | Independent of 1–3 and may be drafted alongside them. Changes the on-disk history, so it comes before peers. |
| 5 | **Peer directory** | Peer claims carry ADR 4's units. The likely answer is "off on Windows" (single-user machine, *inferred*). |

## Portable test fixtures: a sample to measure the test-port cost early
This answers 04-recommendation's other tripwire: does the test port cost more than the product port?

- **Fixture:** a small std-only Rust program, `testjob`, with subcommands `exit N`, `sleep SECS`, `print TEXT [--stderr]` and `hold-mem MiB SECS`.
  - It is a `[[bin]]` with `required-features = ["test-fixtures"]`, so `cargo install` never installs it.
  - The e2e suite runs with `--features test-fixtures` and reaches it through `env!("CARGO_BIN_EXE_testjob")`. CI and the commands in AGENTS.md change to match.
- **Sample of 10 tests.** In each category, the first tests in file order, skipping any with an open flaky-test issue:

  | Category (uses in e2e) | Tests in sample |
  |---|---|
  | `true` / `false` (200) | 3 |
  | `sleep` (98) | 2 |
  | `sh -c` (44) | 2 (one simple, one with pipes or redirects) |
  | `echo` (27) | 1 |
  | `libc::kill` (57) | 1 (its Windows meaning waits for ADRs 2 and 3; the sample records that cost) |
  | `/tmp` (12) | 1 |

- **Measured per test:** changed lines, time spent, whether the semantics shifted (shell features with no portable equivalent), whether it passes on Linux in 3 of 3 runs, runtime before and after.
- **On Windows:** `testjob` is built natively and its subcommands are run. The converted e2e tests cannot run on Windows until the coordinator builds, and the write-up says so.
- **Output:** `docs/decision-gate/05-test-fixture-sample.md`. It multiplies the cost per category by the category counts and compares the total with the product estimate (≈3,635 lines). If the test port comes out larger, that goes to the owner as the "re-check option (c)" trigger.

## Acceptance check for the port: other Windows users cannot read job records
The Windows backend is not done, and no Windows release is made, until this check passes. It is not a "verify later" note.

- **Covers:**
  - the state root, every job record and log or output capture, history, usage and the pause file
  - the per-job TEMP folders, if the per-job TEMP ADR puts them under the state root
  - the coordinator's named pipe: another user's connection is refused
- **Rule:**
  - Every such object gets an explicit, protected DACL with inheritance turned off. The inherited profile ACL is *not verified* and is not relied on.
  - Allowed: the owner SID, SYSTEM and the Administrators group. Everyone else is denied (owner decision 2).
  - Atomic replace: a rename keeps the *source* file's DACL, so the temporary file is created with the restricted DACL, not fixed after the rename.
- **Automated Windows e2e test:**
  1. Submit a job whose record holds a known secret value in its environment.
  2. Walk the state root and read each object's DACL with `GetNamedSecurityInfoW`.
  3. Fail if any ACE grants access to a SID other than the owner, SYSTEM or Administrators, or if inheritance is on.
- **Manual check, recorded in `docs/baseline/`:**
  1. Log in as a second, standard (non-admin) local account.
  2. Try to read a job file, its log and the history with `type` and `Get-Content`, and try to connect to the pipe. Each attempt fails with "access denied".
- **Linux counterpart, added in R3:** a test that pins 0700 on folders and 0600 on files, so both backends are held to the same rule.
