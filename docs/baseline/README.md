# Milestone 1 baseline: unchanged qex v0.33.0

Recorded 2026-09-30. Every result below comes from a run in this session. The raw output is in [logs/](logs/). A platform with no run is **not evaluated**, which is different from passing.

## What was built

| Item | Value |
|---|---|
| Upstream | https://github.com/stephenc/qex (Apache-2.0) |
| Release | `v0.33.0`, commit `78b4e86c5ff8be0c7f4bc26bea809aa75fe230f0` (local tag `base/qex-v0.33.0`) |
| Audit commit | `afd869e`, the direct parent of the release. `git diff --stat afd869e 78b4e86` shows only the version number changing in `Cargo.toml` and `Cargo.lock` (1 line each), so the audit's source findings apply to the release. |
| Source used for the runs | A detached checkout of `78b4e86`, with none of our commits applied |
| NOTICE file upstream | None at `78b4e86` |

## Machine and tools

| | Windows (native) | Linux (WSL2 guest) |
|---|---|---|
| OS | Windows 11 Home 10.0.26200.9457 | Ubuntu 24.04.1 LTS, kernel 5.15.167.4-microsoft-standard-WSL2 |
| CPU / RAM | i7-1255U, 10 cores / 12 threads, 15.7 GiB | 12 vCPU, 7.6 GiB, 2 GiB swap |
| Rust | rustc / cargo 1.98.1, `x86_64-pc-windows-msvc` | rustc / cargo 1.98.1, plus 1.85 for the MSRV check; rustfmt 1.9.0; clippy 0.1.98 |
| C toolchain | VS 2019 Build Tools 16.11, MSVC 14.29.30133, Windows SDK 10.0.22621.0 | gcc 13.3.0 (build-essential), pkg-config 1.8.1 |

The full environment dump is in [logs/linux-wsl2/00-environment.txt](logs/linux-wsl2/00-environment.txt) and in the header of [logs/windows-build.txt](logs/windows-build.txt).

## Results

### Linux (WSL2), with upstream's CI commands

These were run by [run-baseline-linux.sh](run-baseline-linux.sh) on a clone in the WSL filesystem (`~/src/lrc`), with the command `run-baseline-linux.sh ~/src/lrc ~/baseline-logs`.

| Step | Command | Result | Time |
|---|---|---|---|
| Format | `cargo fmt --all --check` | ✅ pass | 1 s |
| Lint | `cargo clippy --all-targets -- -D warnings` | ✅ pass | 25 s |
| Unit tests | `cargo test --bins` | ❌ **587 passed, 1 failed** | 29 s |
| End-to-end tests | `cargo test --test e2e -- --test-threads=2` | ✅ **325 passed, 0 failed** | 379 s |
| Release build | `cargo build --release --locked` | ✅ pass; the binary is 3.5 MB and reports `qex 0.33.0` | 105 s |
| MSRV | `cargo +1.85 check --all-targets --locked` | ✅ pass | 22 s |

**The unit-test failure depends on running in the full suite.**
- The failing test was `sched::tests::a_job_that_another_user_holds_back_says_so_and_never_keeps_capacity` (`src/sched.rs:3315`, "the test peer must count": left 0, right 1).
- It failed in both full `cargo test --bins` runs: here on the release, and again on this branch's head `700290b`.
- Re-run alone three times, it passed 3 out of 3 ([logs/linux-wsl2/07-rerun-failing-unit-test.txt](logs/linux-wsl2/07-rerun-failing-unit-test.txt)).
- It passed in GitHub's hosted runs. The root cause has not been investigated.
- The test builds a fake peer-coordinator record, and one of qex's peer filters discarded it. Those filters check the file owner, boot ID, staleness and whether the process is alive.

### Windows (native)

| Step | Command | Result |
|---|---|---|
| Build | `cargo build --locked` | ❌ **expected failure**. Exit code 101 after 69 s. |

- All 46 dependency crates compiled and linked.
- qex then stops at its own guard, `src/main.rs:23–24`, a `#[cfg(not(unix))] compile_error!`: *"qex builds for Linux and macOS only … ON WINDOWS, USE WSL2."* The audit cites `main.rs:16`; at the release, the guard's doc comment starts at line 17 and the macro is at lines 23–24.
- The guard's own comment says that without it a Windows build gives 114 errors.
- As agreed, no Windows tests and no further Windows work were done in milestone 1.

### macOS

**Not evaluated**, because no macOS machine is available.

### GitHub Actions on the private repo (upstream workflows, unmodified)

Pushing `main` (release + our docs commit `15e5b15`) ran upstream's workflows on GitHub-hosted runners. This is upstream CI running on our repo; it is not a local result.

| Workflow | Result | Cause |
|---|---|---|
| CI › style, MSRV, musl and darwin builds, unit tests | ✅ | |
| CI › "The version that a build reports" (`build-version-test.sh`) | ❌ | The test expects the development version `0.0.0-dev`, but the release commit carries `0.33.0`. Upstream never runs CI on the release commit: its passing run 35533299050 was on `afd869e`. |
| CI › e2e on ubuntu-22.04 / 24.04 | ❌ 319 passed, 6 failed | See below |
| CI › e2e on macos-14 | ❌ 320 passed, 2 failed | See below |
| Pages / Release | Pages ❌ because Pages isn't enabled; Release did nothing | Both are now **disabled** on the private repo. Release can publish GitHub releases and to crates.io; Pages publishes the upstream website. |

The e2e failures on GitHub, with the local result for comparison:
- `no_shipped_word_promises_the_limit_that_went` (all 3 OSes). **Our fault.**
  - The test scans every top-level `docs/*.md` file and found "lower bound" in our `QEX_ASSESSMENT.md`.
  - Fixed by moving the fork's documents to [docs/fork/](../fork/), which the scan doesn't read.
  - It passed locally because the local run used the pristine release.
- `a_config_fault_in_the_record_of_a_job_stays_short` (all 3 OSes). **Not explained yet.**
  - It passed locally.
  - The job note showed the long config-error message where the test expects the short form.
- `a_kill_of_a_group_reaches_every_stage`, `a_paused_queue_starts_no_job…`, `every_output_shows_the_safe_name` and `the_completion_candidates_start_no_coordinator` (Linux runners only).
  - These showed 45 s timeouts, and "Permission denied" reading `/proc/<pid>/fd` on the hosted runner.
  - All passed locally. A runner-environment cause is likely, but that hasn't been proven.

The CI on this PR's branch will show which of these remain after the docs move.

## Capability matrix (unchanged qex v0.33.0)

| Environment | Builds | Tests run | Outcome |
|---|---|---|---|
| Windows 11 native | ❌ (by design) | none | Unsupported upstream: stops at the compile-time guard |
| Linux, WSL2 Ubuntu 24.04 | ✅ | unit and e2e | 912 of 913 passed. One unit test fails in the full parallel run and passes alone. |
| Linux, GitHub ubuntu-22.04 / 24.04 | ✅ | unit and e2e | Unit tests pass. 6 e2e failures, one caused by our docs (see above). |
| macOS, GitHub macos-14 | ✅ | unit and e2e | Unit tests pass. 2 e2e failures, one caused by our docs. |
| macOS, local | — | — | **Not evaluated** |

## Caveats carried forward (from the audit; static findings, not observed here)
- When the available-memory read fails, qex falls back to *total* memory, which overstates headroom (`src/sys.rs`).
- On macOS the available-memory formula counts speculative pages twice.
- The learned memory figure is the peak of the largest child (`getrusage(RUSAGE_CHILDREN)`), not the simultaneous peak of the whole process tree.
- Some qex defaults conflict with the V1 spec, and they must be disabled or matched in any benchmark arm:
  - the dedupe key and result-reuse window
  - exporting runtime limit hints into the job environment
  - running oversized jobs when the queue is idle
  - peer claims
- This machine can't run the spec's §21 benchmark configurations of 16 GiB and 32 GiB: it has 15.7 GiB, and WSL gets 7.6 GiB.
