# Decision gate, step 5: what porting the e2e tests costs (a sample)

**Question.** [04-recommendation.md](04-recommendation.md) names a reason to re-check option (c): "The test port costs more than the product port." [PORT_PLAN.md](../fork/PORT_PLAN.md) asks for a small sample of portable test fixtures to measure that early. #40 converted the sample; this records what it cost.

**Short answer.**
- Of ten sampled tests, nine now run a portable job, `testjob`, in place of Unix programs; the tenth needed no change. Each changed **0 to 11 lines** (mean 3.7). All ten pass 3 of 3 runs on Linux before and after, with no runtime change beyond noise.
- Scaled to the whole suite, the test port comes to **about 1,700 changed lines**, against **≈3,635** for the product port. The harness adds **56 lines** with a Unix pattern, priced separately.
- **That does not settle the trigger.** Half of the estimate (880 lines, 52%) is 80 tests in an "other Unix" category that has **no sample**: Unix sockets standing in for a coordinator, `/proc`, `ps`, `mkfifo` and other libc calls. These are the hardest tests to port, and their 11 lines each are borrowed from a sampled test whose 11 lines were mostly rustfmt layout. The test port reaches the product port if those 80 tests average **35 changed lines** each, which a fake coordinator rebuilt on named pipes could well cost. **Verdict: not met on this estimate, but not shown either way until a few "other Unix" tests are converted and measured.**
- What lines don't capture: **46 tests** send or check a Unix signal. Their Windows meaning waits for ADRs 2 and 3, so their real cost is not known yet.

## Method
"Before" is `main` at d73c760, the commit before #40; "after" is `main` at db1abd0, #40's squash commit. Everything below runs from the repository root. Measurements were taken on one machine: WSL2 Ubuntu (kernel 5.15.167.4), Linux build, 12 CPUs, 7 GB RAM.

1. **The fixture.** `tests/fixtures/testjob.rs` uses only the standard library. Its arguments are a list of steps run in order: `exit N`, `sleep SECS`, `print TEXT [--stderr]` and `hold-mem MIB SECS`. It is a `[[bin]]` with `required-features = ["test-fixtures"]`, so `cargo install` never installs it. The e2e suite reaches it as `env!("CARGO_BIN_EXE_testjob")` and has the same `required-features`.
2. **The sample.** PORT_PLAN's rule: in each category, the first tests in file order, each test taken once, skipping tests with an open flaky-test issue (there are none; the open issues are #20, #28 and #35).

   | # | Category | Test (line of its `#[test]` at d73c760) |
   |---|---|---|
   | 1–3 | `true` / `false` | `a_job_that_succeeds_gives_the_exit_code_zero` (963), `a_job_that_fails_gives_the_exit_code_one` (972), `a_usage_error_of_a_command_that_speaks_for_a_job_uses_the_band` (1225) |
   | 4–5 | `sleep` | `a_signal_to_the_wait_gives_the_code_of_a_broken_wait` (1057), `follow_obeys_the_time_limit_of_the_reader` (1095) |
   | 6 | `sh -c`, simple | `a_wait_gives_the_exit_code_of_the_job` (985) |
   | 7 | `sh -c`, redirect | `status_with_wait_ends_with_the_record_of_the_job` (1936) |
   | 8 | `echo` | `a_job_that_is_too_large_runs_when_the_queue_is_empty` (2797) |
   | 9 | `libc::kill` | `a_wait_for_many_jobs_says_why_the_later_job_waits` (1393); the category's first test, 1057, was already taken under `sleep` |
   | 10 | `/tmp` | `the_harness_stops_a_coordinator_on_a_long_socket_path` (18229) |

3. **Cost per test:** `experiments/fixture-sample/cost-per-test.py d73c760 db1abd0` counts the changed lines of `tests/e2e.rs` inside each test. Output: [cost-per-test.tsv](logs/fixture-sample/cost-per-test.tsv).
4. **Runs and runtime:** `experiments/fixture-sample/time-sample.sh` runs each test alone 3 times, on a checkout of d73c760 (built without the feature, which it doesn't have) and of db1abd0. `summarize-times.py` makes the table. Output: [before.tsv](logs/fixture-sample/before.tsv), [after.tsv](logs/fixture-sample/after.tsv), [times.tsv](logs/fixture-sample/times.tsv). A mean that moves is rerun 10 times with `repeat-one.sh`.
5. **Estimate:** `experiments/fixture-sample/estimate.py d73c760 docs/decision-gate/logs/fixture-sample/cost-per-test.tsv`. It reads the costs from step 3, not from numbers typed into the script. Output: [estimate.txt](logs/fixture-sample/estimate.txt). Its method is in "The estimate".
6. **Windows:** `cargo build --features test-fixtures --bin testjob` on native Windows at db1abd0, then each step run by hand. Output: [windows-testjob.txt](logs/fixture-sample/windows-testjob.txt).

## Results per test

| # | What changed | Lines | Semantics shift | Linux before / after | Mean s before / after |
|---|---|---|---|---|---|
| 1 | `true` → `testjob exit 0` | 2 | none | 3/3 / 3/3 | 0.2 / 0.2 |
| 2 | `false` → `testjob exit 1` | 2 | none | 3/3 / 3/3 | 0.2 / 0.2 |
| 3 | `true` → `testjob exit 0` | 11 | none. rustfmt split the longer array to one item per line; the edit itself is one line | 3/3 / 3/3 | 0.1 / 0.1 |
| 4 | `sleep 30` → `testjob sleep 30` | 2 | the job part none. The test's `libc::kill(SIGINT)` to the waiting `qex` is unchanged: its Windows meaning waits for ADRs 2 and 3 | 3/3 / 3/3 | 1.2 / 1.1 |
| 5 | `sleep 30` → `testjob sleep 30` | 2 | none | 3/3 / 3/3 | 2.4 / 2.4 |
| 6 | `sh -c "exit 42"` → `testjob exit 42` | 2 | none | 3/3 / 3/3 | 0.2 / 0.2 |
| 7 | `sh -c "echo bad >&2; exit 9"` → `testjob print bad --stderr exit 9` | 4 | the shell sequence needs steps in order (see "Changes to the plan") | 3/3 / 3/3 | 0.2 / 0.2 |
| 8 | `sleep 300` and `echo big` → `testjob` | 4 | none | 3/3 / 3/3 | 1.6 / 2.5, noise: 10 runs each give 1.74 / 1.62 ([too-large-10-runs.txt](logs/fixture-sample/too-large-10-runs.txt)) |
| 9 | `sleep 6`, `sleep 25`, `true` → `testjob` | 8 | as test 4: the `libc::kill(SIGINT)` stays | 3/3 / 3/3 | 13.1 / 12.9 |
| 10 | nothing | 0 | `/tmp` appears only in a comment. The test checks the Unix socket path limit, which Windows named pipes don't have; on Windows the test is dropped, not ported | 3/3 / 3/3 | 0.1 / 0.1 |

Shared lines, not in any test: 12 (the `TESTJOB` constant with its comment, the fork header and the run instruction in the file comment).

Time spent per test was not measured: an agent's editing time says nothing about a person's. The changed lines stand in for it.

## The estimate
`estimate.py` counts tests, not occurrences:
- **Which tests:** a test counts when its body (from its `#[test]` to the next one) names a Unix pattern, as `experiments/unix-inventory/e2e-unix.py` does.
- **No double counting:** a test that matches several categories counts once, under its most expensive one.
- **Cost per category:** the mean changed lines of that category's sampled tests. The "other Unix" category has no sample (other `libc::` calls, `SIG*` names, `std::os::unix`, `/proc`, `"/bin/` paths, `.sh` scripts, `"cat"`, `"kill"` and the tools `ps`, `mkfifo`, `bwrap`, `lsof`, `chmod`, `cp`, `touch`, `printf`). It takes the highest cost measured on one test, 11.

| Category | Tests | Lines per test | Estimate |
|---|---|---|---|
| other Unix | 80 | 11.0 | 880 |
| `true` / `false` | 114 | 5.0 | 570 |
| `sh -c` / `bash` | 44 | 3.0 | 132 |
| `echo` | 16 | 4.0 | 64 |
| `sleep` | 21 | 2.0 | 42 |
| `libc::kill` | 1 | 8.0 | 8 |
| `/tmp` | 1 | 0.0 | 0 |
| **total** | **277** | | **1,696** |

- **What "other Unix" holds,** in its 80 tests (a test can count in several): 48 call another libc function, 45 name a signal, 25 use `std::os::unix` (Unix-socket stand-ins for a coordinator, `PermissionsExt`, `ExitStatusExt`), 10 read `/proc` and 6 start another Unix tool.
- **Not a bound either way:** pricing all 277 tests at 11 lines gives 3,047, but that is no upper bound, because the unsampled category isn't capped at the highest sampled cost. Helpers aren't followed, so a test that uses Unix only through a helper doesn't count, and that pulls the other way.
- **Break-even:** with the sampled categories as measured (816 lines), the test port reaches the product port's 3,635 lines if the 80 "other Unix" tests average 35 changed lines each.
- **Why `libc::kill` has 1 test:** "other Unix" is the most expensive category, and nearly every test that calls `libc::kill` also names a signal (`SIGINT`, `SIGTERM`), so it counts there.
- **What's not a Unix pattern:** `"kill"` in these tests is qex's own `kill` subcommand (`h.ok(&["kill", &id])`), and `SIGNAL` appears only as a word; neither counts. Step 3's `e2e-unix.py` counted `"kill"`, so its "281 tests using Unix" is a few too high; 277 here.
- **Harness:** the harness and helpers outside the tests have 56 lines with a Unix pattern (comments excluded). They are not priced, because the sample didn't touch them: `Harness::stopped` alone uses `libc::kill` and `/proc`.
- **What lines don't capture:** 46 tests name a signal (`libc::kill` or a `SIG*` name). Changing their lines is cheap; deciding what they test on Windows is the work that ADR 2 (who may signal a job) and ADR 3 (exit classification and `--signal`) must do first. This sample doesn't price that.

## Changes to the plan
1. **`testjob` takes a list of steps.** PORT_PLAN lists four subcommands. `sh -c "echo bad >&2; exit 9"` needs two in one job, so the arguments are a sequence (`testjob print bad --stderr exit 9`). The four subcommands are unchanged.
2. **`docs/baseline/run-baseline-linux.sh` keeps its commands.** PORT_PLAN listed it among the commands to update. It checks out upstream v0.33.0, which has no `test-fixtures` feature, so `--features test-fixtures` would fail there. Its AGENTS.md line says why.
3. **Two PRs, not one.** The fixture and the conversions merged as #40; this measurement follows it, so every commit it names is on `main`.

## On Windows
- `testjob` builds natively (Windows 11 10.0.26200, rustc 1.98.1, at db1abd0), and every step behaves as on Linux:
  - exit codes 0, 1 and 42;
  - `print` goes to stdout, and `--stderr` to stderr only;
  - `sleep 1.5` took 1.57 s;
  - `hold-mem 64 1` exits 0;
  - an unknown step, or `exit 300`, exits 2 with a usage message.
- **Not evaluated:** the converted e2e tests on Windows. They need the coordinator, which doesn't build on Windows yet.

## Not evaluated
- macOS.
- How the 46 signal tests port: that waits for ADRs 2 and 3.
- The cost of the 80 "other Unix" tests: no test of that kind was sampled.
- The cost of the 56 harness lines.
- Only one machine was measured, with 3 runs per test (10 for the one whose mean moved).
