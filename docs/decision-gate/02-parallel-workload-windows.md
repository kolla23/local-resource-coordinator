# Decision gate, step 2: parallel build+test workloads on native Windows

**Question:** what happens on this laptop when three copies of one project build and test at the same time, with nothing coordinating them? This is the problem the coordinator exists to solve.

**Short answer:**
- **Memory:** the three workloads together pushed available RAM to **almost zero** (1 MiB at the lowest, in rep 1).
- **Correctness:** in **2 of 3** parallel runs, the tests gave **40–66 false failures per worktree**, because the three runs collided on shared temporary files. Run one at a time, those failures never happened.
- **Speed:** running all three at once was still **faster** than one at a time (median 97.5 s against 153.7 s).
- **No freeze:** the machine never stalled (worst timer delay 204 ms).

Raw data is in [logs/parallel-workload-windows/](logs/parallel-workload-windows/), and every number here comes from those files.

## Setup
- **Workload:** [ripgrep](https://github.com/BurntSushi/ripgrep) 15.2.0 (commit `e89fff8`), in 3 git worktrees.
  - Each worktree runs `cargo build --locked && cargo test --locked` from a clean `target/` directory.
  - Dependencies were fetched beforehand, so there was no network during the runs.
  - It was cloned into `%LOCALAPPDATA%\lrc-bench`, not this repo. ripgrep stands in for "a Rust project an agent builds and tests". It's a reference workload, not one of the owner's projects.
- **Two arms:**
  - **P (uncoordinated):** all 3 at once.
  - **S (static cap 1):** one after another. This is the simplest possible "coordination".
- **Repetitions:** 3 per arm, alternating the order (P,S / S,P / P,S), with 20 s of settling before each run.
- **Runner:** [experiments/parallel-workload/](../../experiments/parallel-workload/), run by [run-gate.sh](../../experiments/parallel-workload/run-gate.sh).
  - Each workload runs in its **own Job Object**, so its whole tree is measured: cargo, rustc, link.exe and the test binaries, 474–549 processes per workload. The probe's mechanism from step 1 is reused.
  - The runner samples system commit and available RAM every 250 ms, and records paging every second with `typeperf`.
  - A 100 ms timer measures responsiveness (how late it wakes).
  - A safety guard would abort on a stall over 10 s or commit within 2 GiB of the limit. **It never triggered.**
- **Machine** ([environment.txt](logs/parallel-workload-windows/environment.txt)):
  - i7-1255U (10 cores, 12 threads), 15.7 GiB RAM, commit limit 44.8 GiB.
  - **The owner's normal apps were running**, as on a real day. Just before each run, the system already had **36.4–37.3 GiB committed and only 2.2–4.0 GiB of RAM available**: Chrome 5.4 GiB, a `vmmem` VM 4.1 GiB, Python 3.4 GiB, Claude 2.9 GiB, ChatGPT 2.6 GiB, VS Code 1.5 GiB (a snapshot taken before the runs).

## Results

| Run | Makespan | Peak memory per workload (kernel job commit) | System commit increase | Lowest available RAM | Worst timer delay | Failed tests per worktree |
|---|---|---|---|---|---|---|
| rep1-P | **91.5 s** | 1702 / 1783 / 1702 MiB | +5155 MiB | **1 MiB** | 204 ms | **63 / 66 / 59** |
| rep1-S | 115.8 s | 1739 / 1704 / 1722 MiB | +1862 MiB | 1514 MiB | 30 ms | 1 / 1 / 1 |
| rep2-S | 155.3 s | (sum of peaks 5314 MiB) | +1788 MiB | 2123 MiB | 36 ms | 1 / 1 / 1 |
| rep2-P | **107.7 s** | (sum of peaks 5593 MiB) | +5496 MiB | **418 MiB** | 133 ms | **40 / 56 / 60** |
| rep3-P | **97.5 s** | (sum of peaks 5441 MiB) | +5288 MiB | 1337 MiB | 104 ms | 1 / 1 / 1 |
| rep3-S | 153.7 s | (sum of peaks 5314 MiB) | +2298 MiB | 2475 MiB | 44 ms | 1 / 1 / 1 |

**Median makespan:** P 97.5 s against S 153.7 s.

Paging from `typeperf`:
- Mean pages-in per second: P 1.9k–5.4k, S 1.1k–3.1k.
- In rep 1 of P, available RAM was under 100 MB for 2 seconds.
- Peak paging spikes were noisy in both arms, so I'm not claiming a difference there.

## What the numbers mean

1. **Memory is the binding constraint, and it's the whole tree that matters.**
   - One build+test peaks at about **1.7 GiB of commit**, spread across about 500 short-lived processes.
   - Three at once add **5.2–5.5 GiB of commit**, about the sum of their peaks, because they overlap.
   - On a machine already running normal apps, that drove available RAM to **1 MiB, 418 MiB and 1337 MiB** in the three runs, against 1.5–2.5 GiB when run one at a time.
   - A single `rustc` or `link.exe` process never shows this; only the job-level total does.
2. **The real failures came from shared files, not memory.**
   - In rep1-P and rep2-P, 40–66 tests per worktree failed with **"os error 32: the process cannot access the file because it is being used by another process"**, along with output mismatches.
   - Cause: ripgrep's test harness puts its scratch files at `%TEMP%\ripgrep-tests\<test>\<n>`, where `<n>` is a counter that starts at 0 in every process (`tests/util.rs:14–81`). So three worktrees' test runs write the **same paths**.
   - rep3-P happened to dodge the collision, so the problem is timing-dependent: a flaky false result.
   - **Implication for the coordinator:** memory-based admission alone wouldn't have prevented this. It needs either a per-job `TEMP`/`TMP` (an environment change, which SPEC §8.1 / S08 restricts) or a named exclusive lock (qex's `--lock`). This is an ADR candidate. It's also exactly the "every independent request must give a real result" problem in SPEC.
3. **Plain serialization costs time.** The static cap of 1 took about 58% longer (median 153.7 s against 97.5 s, a ratio of 1.58), because builds are partly single-threaded and overlap well. Good coordination should admit work up to a memory budget, not serialize it.
4. **No freeze on this hardware at this load.** The worst timer delay was 204 ms (SPEC's freeze proxy is >2 s). With more than three workloads, or bigger projects, the RAM floor would be crossed. That wasn't tested.

## A baseline failure unrelated to parallelism
- `feature::f1414_no_require_git` failed in **every** run, both arms.
- The test expects `.gitignore` to be ignored when there's no git repository. But `%TEMP%` (`C:\Users\kolla\AppData\Local\Temp`) is **inside the git repository at `C:\Users\kolla`**, so ripgrep finds a repository and respects it.
- That's an environment artefact of this machine, and it's counted as 1 failure per worktree in the table.
- *Update, 2026-10-01:* the owner renamed that repository's `.git`, and in the follow-up run below `f1414` passed in all 9 worktree runs.

## Follow-up (2026-10-01): a separate TEMP/TMP per workload
**Question:** were the false failures in rep1-P and rep2-P caused by the shared `%TEMP%`? To find out, I repeated the parallel arm only, giving each workload its own fresh, empty `TEMP`/`TMP`.

**Answer:** yes, very likely. **3 of 3 parallel reps were clean:** all 9 worktree runs passed 437 of 437 tests, and "os error 32" appeared **0** times.

- **Setup:** same ripgrep worktrees, command and runner as above. Two changes:
  - `--temp-root` gives workload *i* its own folder, `%LOCALAPPDATA%\lrc-bench\tmp\<rep>-w<i>`.
  - `run-gate.sh … P <temp-root>` runs the P arm 3 times.
- **Each workload used its own folder:** each of the 9 TEMP folders contains its own `ripgrep-tests` folder with 321 entries. The shared `%TEMP%\ripgrep-tests` was last modified on 2026-09-30 at 21:30, before this run.
- **Machine:** the owner closed apps first (`wsl --shutdown`, among others). Just before the run: 27.6 GiB committed of a 35.3 GiB limit, 4.0 GiB of RAM available ([environment.txt](logs/parallel-workload-windows-temp/environment.txt)). The commit limit was 44.8 GiB in the first run, so the page file had shrunk since.

| Run | Makespan | Peak memory per workload (kernel job commit) | System commit increase | Lowest available RAM | Worst timer delay | Failed tests per worktree |
|---|---|---|---|---|---|---|
| rep1-P | 103.2 s | 1607 / 1610 / 1594 MiB | +4848 MiB | 1373 MiB | 123 ms | 0 / 0 / 0 |
| rep2-P | 92.7 s | 1832 / 1723 / 1859 MiB | +4843 MiB | 2648 MiB | 106 ms | 0 / 0 / 0 |
| rep3-P | 91.2 s | 1993 / 2091 / 1820 MiB | +5836 MiB | 2024 MiB | 98 ms | 0 / 0 / 0 |

The median makespan was 92.7 s, against 97.5 s for P in the first run. Raw logs are in [logs/parallel-workload-windows-temp/](logs/parallel-workload-windows-temp/).

**What this means:**
- A per-job `TEMP`/`TMP` removed the collisions, without serializing anything or losing parallel speed.
- For the coordinator, this makes per-job temp directories the main candidate for the ADR named in point 2 above. That ADR needs to settle how this fits SPEC §8.1 / S08 (environment changes). A named lock is the fallback for tools that ignore `TEMP`.
- **How strong is the evidence?** If the original 2-in-3 collision rate still held, 3 clean reps would happen by chance about 4% of the time ((1/3)³). So this is strong evidence, but not proof. It covers one test harness (ripgrep's), and it doesn't cover tools that use fixed paths outside `TEMP`.
- **Lowest available RAM was 1.4–2.6 GiB this time**, against 1 MiB to 1.3 GiB in the first run. That's mainly because the owner freed memory first, so it isn't evidence that per-job TEMP saves memory.

**Two runner problems found on the way (both fixed, neither produced data):**
- `%PATH%` now contains `C:\Program Files (x86)\Windows Kits\…`. The `)` closed the runner's `( … )` group, and cmd failed with "\Windows was unexpected at this time". This affected the unchanged gate 2 command too. The fix quotes the assignment: `set "PATH=…"`.
- I passed a relative output directory, which the workloads, running with their worktree as the current directory, couldn't find. The output directory has to be an absolute path.

## Not tested
- **Through a coordinator.** Unchanged qex can't run natively here. A qex-in-WSL2 comparison is reference-only (see [DECISION_NATIVE_WINDOWS.md](../fork/DECISION_NATIVE_WINDOWS.md)) and wasn't run in this step.
- More than 3 workloads; larger projects; release builds; other toolchains (Node, Python, .NET).
- A quiet machine. Background apps weren't controlled, which makes the results realistic but noisier. The serial per-workload times varied from 36 to 60 s.
- SPEC §21's 16/32 GiB configurations, at least 10 repetitions, randomized blocks and bootstrap confidence intervals. With 3 repetitions these numbers are indicative, not statistically strong.
- Disk I/O and CPU pressure measured separately; only memory and paging were recorded.
- Whether a per-job `TEMP` fixes the collisions. It's very likely, but unverified. *(Tested on 2026-10-01: see the follow-up above.)*
