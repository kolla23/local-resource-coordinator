# Decision gate, step 5b: two hard tests converted

[05-test-fixture-sample.md](05-test-fixture-sample.md) priced each category from its simplest tests and said the follow-up converts "the hardest example of each gap category". That is only partly true: this PR converts one hard test in each of two gaps, a Unix tool and an `sh -c` script with shell features. Neither is the hardest of its gap, and every gap in 05 stays open apart from these two tests, so the trigger is still not settled. The last section lists every group and what happened to it.

## What was converted
Before is `main` at c6d9344, after is 8e34772. Raw output is in [logs/fixture-hard-sample](logs/fixture-hard-sample/).

| Gap | Test | What changed | Lines | Linux runs before / after | Mean s before / after |
|---|---|---|---|---|---|
| Unix tool | `an_old_record_from_before_the_boot_is_dead` | `sleep 300` → `testjob sleep 300`; `touch -t 202001010000` → `File::set_modified` (2020-01-01 00:00 UTC) | 12 | 3/3 / 3/3 | 0.5 / 0.4 |
| `sh -c` with shell features | `a_job_that_starts_again_never_shows_the_attempt_that_failed` | the retry script (counter file, `if`, busy loop, `exit 3`, `sleep 5`) → `testjob count … if-count-below 2 spin 1.5 exit 3 end sleep 5` | 29 | 3/3 / 3/3 | 8.9 / 8.2 |

- **Lines:** `experiments/fixture-sample/cost-per-test.py c6d9344 8e34772 <the two tests>` ([cost-per-test.tsv](logs/fixture-hard-sample/cost-per-test.tsv)). rustfmt put each argument of the retry test's `submit` on its own line.
- **Runs:** `experiments/fixture-sample/time-sample.sh <label> <the two tests>` on WSL2 Ubuntu (kernel 5.15.167.4), then `summarize-times.py` ([times.tsv](logs/fixture-hard-sample/times.tsv)).
- **Fixture growth:** `testjob` gained three steps, `spin SECS`, `count FILE` and `if-count-below N … end`, all standard library only: +48/−2 lines (`git diff --numstat c6d9344 8e34772 -- tests/fixtures/testjob.rs`, [testjob-growth.tsv](logs/fixture-hard-sample/testjob-growth.tsv)).
- **CPU kept:** the test's last check is that the record's `cpu_secs` is under 0.5 s, so the first attempt must use more. `spin` runs for a time, not a count, and uses up to that time in CPU (less on a busy core): on Linux, run alone, the shell loop used 1.30 s of user CPU and `testjob spin 1.5` 1.54 s ([spin-cpu.txt](logs/fixture-hard-sample/spin-cpu.txt)).
- **Signals stay:** the touch test still stops the coordinator with `libc::kill(SIGKILL)` and checks processes with `kill(pid, 0)`. As for 05's tests 4 and 9, its Windows meaning waits for ADRs 2 and 3, so its 12 lines don't price that part.
- **On Windows:** `testjob` built natively at 8e34772 runs the new steps as on Linux ([windows-testjob.txt](logs/fixture-hard-sample/windows-testjob.txt)). The converted tests need the coordinator, which doesn't build on Windows yet: not evaluated.

## Every group, and what happened to it
One row per "other Unix" group and per category with a known gap in 05's [estimate.txt](logs/fixture-sample/estimate.txt) (at d73c760). "Not sampled" without a reason means no blocker was checked.

| Group (tests) | Status |
|---|---|
| `UnixListener` or `UnixStream` (7) | Wait for ADR 1 (IPC transport). Not sampled. |
| `/proc` (4) | Linux-only: three are `#[cfg(target_os = "linux")]`, one needs `bwrap`. Dropped or replaced on Windows. Not sampled. The two politeness tests among them also wait for the ADR that covers politeness; PORT_PLAN's ADR table has none yet. |
| A Unix tool (5) | `an_old_record_from_before_the_boot_is_dead` converted here. The two `mkfifo` tests (`a_command_refuses_a_configuration_path_that_is_not_a_regular_file`, `a_configuration_path_that_is_not_a_regular_file_does_not_stop_the_coordinator`): probably replaced on Windows; FIFOs have no direct equivalent. `bash_keeps_a_hostile_candidate_in_one_word` tests the bash completion script, so it needs bash. `a_line_with_a_space_a_quotation_mark_and_a_semicolon_is_one_argument` (`printf`): not sampled. |
| A libc item other than `kill` and the `SIG*` constants (10) | Not sampled. |
| Another `std::os::unix` item (7) | Not sampled. |
| A `"/bin/"` path, `.sh` script or `"cat"` (1) | Not sampled. |
| Only a signal name (33) | Wait for ADRs 2 and 3. So does the signal part of every test that names a signal: 38 tests across all categories. |
| `sh -c` / `bash` (46) | `a_job_that_starts_again_never_shows_the_attempt_that_failed` converted here. The others with shell features: not sampled. |
