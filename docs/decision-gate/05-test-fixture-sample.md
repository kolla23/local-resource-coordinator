# Decision gate, step 5: what porting the e2e tests costs (a sample)

**Question.** [04-recommendation.md](04-recommendation.md) names a reason to re-check option (c): "The test port costs more than the product port." [PORT_PLAN.md](../fork/PORT_PLAN.md) asks for a small sample of portable test fixtures to measure that early. #40 converted the sample; this records what it cost.

**Short answer.**
- Of ten sampled tests, nine now run a portable job, `testjob`, in place of Unix programs; the tenth needed no change. Each changed **0 to 11 lines** (mean 3.7). All ten pass 3 of 3 runs on Linux before and after, with no runtime change beyond noise.
- Scaled to the whole suite, the test port comes to **about 1,580 changed lines** (1,579), against **≈3,635** for the product port. The harness adds **60 lines** with a Unix pattern, which are counted but not priced.
- **That does not settle the trigger.** Nearly half of the estimate (737 lines, 47%) is 67 tests in an "other Unix" category. Half of them (33) are there only because they name a signal: cheap in lines, with their real cost deferred to ADRs 2 and 3. The other 34 are where the hard porting is. By the first pattern each matches (a pattern, not a reading of what the test does): 7 `UnixListener` or `UnixStream`, mostly fake coordinators; 4 `/proc`; 5 a Unix tool such as `mkfifo` or `chmod`; 10 another libc item (`flock`, `getuid`, `killpg`, …); 7 another `std::os::unix` item; 1 a `/bin/` path. **None was sampled for its Unix part**: two sampled tests fall in it, but only their `sleep` and `true` were converted, and their signal work was left for ADRs 2 and 3. Its 11 lines per test are borrowed from a sampled test whose 11 lines were mostly rustfmt layout. The test port reaches the product port if those 67 tests average **42 changed lines** each. With half of them signal-only, that needs the other half to average far more, which a fake coordinator rebuilt on named pipes might cost and a `getuid` swap would not. Only converting a few of them will tell.
- **Verdict: this is a partial measurement, and the whole estimate is likely low.** Each category is priced from its simplest tests, because PORT_PLAN's "first in file order" rule picks the easy ones first. Known gaps:
  - the 67 "other Unix" tests, none sampled for its Unix part;
  - many of the 46 `sh -c` tests use shell features `testjob` can't do (environment variables, loops and `seq`, `dd`, non-UTF-8 output, `/dev/zero`, background processes), but the category is priced from two one-line scripts;
  - fixture growth: `testjob` (102 lines at db1abd0) plus the steps the remaining conversions need are not counted;
  - the 38 signal tests, waiting on ADRs 2 and 3.

  **The trigger decision waits for the follow-up PR**, which converts the hardest example of each gap category rather than the first.

  The follow-up, [05b-hard-fixture-sample.md](05b-hard-fixture-sample.md), converts the hardest example of two gaps only, and lists the gaps still open.
- What lines don't capture: **38 tests** name a Unix signal (`libc::kill` or a `SIG*` name). Their Windows meaning waits for ADRs 2 and 3, so their real cost is not known yet.

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

3. **Cost per test:** `experiments/fixture-sample/cost-per-test.py d73c760 db1abd0` counts the changed lines of `tests/e2e.rs` inside each test, a test ending at its closing brace as in step 5. Output: [cost-per-test.tsv](logs/fixture-sample/cost-per-test.tsv).
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
- **Which tests:** a test counts when the test function itself (from its `#[test]` to its closing brace) names a Unix pattern of any category; full-line comments don't count. The helpers between tests are harness code, counted separately below. (Step 3's `e2e-unix.py` took everything up to the next `#[test]` and kept comments, so it counted a helper as part of the test above it.)
- **No double counting:** a test that matches several categories counts once, under its most expensive one.
- **Cost per category:** the mean changed lines of the sampled tests chosen for that category (the table in "Method"). No sampled test was chosen for "other Unix" (other `libc::` items, `SIG*` names, `std::os::unix`, `/proc`, `"/bin/` paths, `.sh` scripts, `"cat"` and the tools `ps`, `mkfifo`, `bwrap`, `lsof`, `chmod`, `cp`, `touch`, `printf`, whether a test starts them or submits them as a job), so it takes the highest cost measured on one test, 11.
- **Where the sampled tests are filed:** filing by most expensive match puts tests 4 and 9 under "other Unix", because both name `SIGINT`; their 2 and 8 lines leave the signal part unported, so they don't measure that category. The `libc::kill` category is then priced from test 9 although test 9 is filed elsewhere. Likewise, `/tmp` is priced from test 10, which is outside the category (its `/tmp` is only in a comment), and the one test filed under `/tmp`, `a_config_that_names_the_live_peer_directory_gets_a_private_one` (line 18287), isn't priced. `estimate.txt` lists where each sampled test lands.

| Category | Tests | Lines per test | Estimate |
|---|---|---|---|
| other Unix | 67 | 11.0 | 737 |
| `true` / `false` | 116 | 5.0 | 580 |
| `sh -c` / `bash` | 46 | 3.0 | 138 |
| `echo` | 16 | 4.0 | 64 |
| `sleep` | 22 | 2.0 | 44 |
| `libc::kill` | 2 | 8.0 | 16 |
| `/tmp` | 1 | 0.0 | 0 |
| **total** | **270** | | **1,579** |

- **What "other Unix" holds,** in its 67 tests, each in the first group whose pattern it matches (`estimate.txt` prints the groups and their sum): `UnixListener`/`UnixStream` 7, `/proc` 4, a Unix tool 5, a libc item other than `kill` and the `SIG*` constants 10, another `std::os::unix` item 7, a `"/bin/"` path, `.sh` script or `"cat"` 1, and only a signal name 33. The groups name patterns, not uses: one `/proc` test matches through `bwrap`'s `--proc /proc` argument, and one socket test is a client of the real coordinator rather than a fake one.
- **Not a bound either way:** pricing all 270 tests at 11 lines gives 2,970, but that is no upper bound, because the unsampled category isn't capped at the highest sampled cost. Helpers aren't followed, so a test that uses Unix only through a helper doesn't count, and that pulls the other way.
- **Break-even:** with the other categories as estimated (842 lines), the test port reaches the product port's 3,635 lines if the 67 "other Unix" tests average 42 changed lines each.
- **Why `libc::kill` has 2 tests:** "other Unix" is the most expensive category, and nearly every test that calls `libc::kill` also names a signal (`SIGKILL`, `SIGINT`, …), so it counts there.
- **What's not a Unix pattern:** `"kill"` in these tests is qex's own `kill` subcommand (`h.ok(&["kill", &id])`) or text qex prints, and no test runs a `kill` program; `SIGNAL` appears only as a word; neither counts. Step 3's `e2e-unix.py` counted `"kill"` and helper code, so its "281 tests using Unix" (at c693c90, the commit its log names; squashed into 7e4f9c6 on `main`; 326 tests) is too high; 270 here (at d73c760, 329 tests). `"bash"` counts only as `bash -c`, in a list (`"bash", "-c"`) or a builder (`Command::new("bash").arg("-c")`): in `the_completions_hold_the_commands_of_qex` a bare `"bash"` is a shell name for `qex completions`, not a shell that runs.
- **Harness:** the harness and helpers outside the tests have 60 lines with a Unix pattern (comments excluded). They are not priced, because the sample didn't touch them: `Harness::stopped` alone uses `libc::kill`, and `/proc` through `state_in_proc`.
- **What lines don't capture:** 38 tests name a signal (`libc::kill` or a `SIG*` name). Changing their lines is cheap; deciding what they test on Windows is the work that ADR 2 (who may signal a job) and ADR 3 (exit classification and `--signal`) must do first. This sample doesn't price that.

## Changes to the plan
1. **`testjob` takes a list of steps.** PORT_PLAN lists four subcommands. `sh -c "echo bad >&2; exit 9"` needs two in one job, so the arguments are a sequence (`testjob print bad --stderr exit 9`). The four subcommands are unchanged.
2. **`docs/baseline/run-baseline-linux.sh` keeps its commands.** PORT_PLAN listed it among the commands to update. It checks out upstream v0.33.0, which has no `test-fixtures` feature, so `--features test-fixtures` would fail there. Its AGENTS.md line says why.
3. **Not a lower bound.** PORT_PLAN expected the total to be a lower bound, because helpers aren't followed. The unsampled "other Unix" category isn't capped either, so the total is neither bound; the write-up says so and gives the break-even instead.
4. **Two PRs, not one.** The fixture and the conversions merged as #40; this measurement follows it, so every commit it names is on `main`.

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
- How the 38 signal tests port: that waits for ADRs 2 and 3.
- The cost of the 67 "other Unix" tests: none was sampled for its Unix part.
- The cost of the `sh -c` tests whose scripts use shell features `testjob` can't do (many of the 46): the category is priced from two one-line scripts.
- Fixture growth: `testjob` (102 lines at db1abd0) and the steps the remaining conversions need.
- The cost of the 60 harness lines.
- Only one machine was measured, with 3 runs per test (10 for the one whose mean moved).
