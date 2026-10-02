# Decision gate: recommendation

**For the owner's decision.** Option (b), unchanged qex in WSL2, was already removed by [DECISION_NATIVE_WINDOWS.md](../fork/DECISION_NATIVE_WINDOWS.md). The choice left is:
- **(a)** continue the qex-based fork with a Windows backend, or
- **(c)** build a new core.

## Recommendation: (a), continue the fork, with two conditions

1. **First, a backend layer on Unix, before any Windows code.**
   - Move the OS calls behind one interface. Today they are free `#[cfg]` functions called from 18 files.
   - On Linux this refactor changes no behaviour, so the existing 610 unit tests and 326 e2e tests check it on CI.
   - The Windows backend then fills in that interface.
2. **The five design-level changes go through ADRs before their code.** Each is listed in step 3, and each changes a qex contract:
   - the IPC transport and launch protocol
   - who may signal a job
   - exit classification and `--signal`
   - the memory metric in history and admission
   - the peer directory

   If those ADRs end up rewriting most of the process, IPC and state layers, re-check this recommendation; see "What would change it" below.

## Why: the evidence from the three steps

**Step 1: [Job Object probe](01-jobobject-probe.md).**
- The mechanisms qex lacks on Windows exist and behave well:
  - ownership from the first instruction
  - kernel-measured whole-tree peak memory
  - forced cancellation of the whole tree in about 6 ms
  - cleanup when the owner crashes
- They fit qex's existing shape: one supervisor process per job, which writes the result (qex's guarantee 2). The supervisor would hold the Job Object handle.
- This favours neither option on its own, but it shows **qex's process model doesn't have to be thrown away**. It ports to something stronger.

**Step 2: [parallel workloads on Windows](02-parallel-workload-windows.md).**
- Three uncoordinated build+test runs pushed available RAM as low as 1 MiB, 418 MiB and 1,337 MiB in the first run.
- On a shared `TEMP`, 5 of the 6 parallel reps across both shared-TEMP runs had collisions, with **9 to 66 false test failures in each worktree that collided**. Per-job `TEMP` removed them (3 of 3 clean).
- One at a time cost 58% more time.
- So the product needs:
  - admission against a memory budget, not serial running
  - whole-tree memory
  - a per-job `TEMP`
- qex already has the first: the scheduler admits work up to a memory budget, and its logic is OS-independent. Only its inputs (memory, pressure, peer claims) need the backend. The second comes from step 1. The third neither option has yet; it's an ADR either way, because SPEC limits automatic environment changes.

**Step 3: [Unix inventory](03-unix-inventory.md).**
- About 10% of the production code (≈3,635 of 37,867 lines) is in code that changes for Windows:
  - ≈625 lines are thin call swaps
  - ≈1,435 lines are rework inside one module
  - ≈1,575 lines, in five areas, are design changes
- The other ≈90% is logic that a new core would have to rebuild: the scheduler, claims and pools, learned history, restart recovery, the CLI and protocol, `top`, the hooks, and the tested behaviour behind 610 unit and 326 e2e tests.
- The five design changes aren't fork-only costs. A new core would make the same decisions (pipes, signalling, the metric, peers) from a blank page.
- The Unix code is spread over many files: 24 of 39 files have a hit, and 22 fail to compile. But the weight is at the OS-facing edge: 7 files (paths, supervisor, keys, client, hook, sys, update) hold 72% of the compile errors. The scheduler's logic is OS-independent; only its inputs go through the OS.

## The case for a new core, weighed
- **Windows-only gains:**
  - A new core would be clean on Windows: no pid/process-group identity, no `max_rss` in the formats and no Unix socket files.
  - It would have a backend layer from day one, and the fork would have to retrofit one.
- **qex also differs from SPEC's V1 target beyond Windows** ([QEX_ASSESSMENT.md:110](../fork/QEX_ASSESSMENT.md); `SPEC.md:611–617`):
  - atomic job files instead of SQLite
  - newline-separated JSON instead of length-prefixed messages
  - no OS peer authentication
  - no leader epoch at autostart
- A new core could build to SPEC directly, where the fork needs an ADR for each difference.
- **Why this doesn't change the recommendation:**
  - These gaps are on every platform, so closing them is the same design work either way.
  - QEX_ASSESSMENT.md:110 already advises against rewriting persistence "solely for architectural uniformity" and says to keep safety outcomes and compatibility where possible.
  - A new core pays for that cleanliness by rebuilding and re-proving the other ~34k lines, without the existing tests.

**The trade-off in plain terms:**
- **The fork** keeps ~34k lines of working, tested behaviour. It pays with a refactor (the backend layer), five ADRs, and a large test-port job: at least 281 of the 326 e2e tests use Unix commands, APIs or paths.
- **A new core** starts clean. It pays by rebuilding and re-proving everything else. Only the black-box e2e tests could carry over, and only if it kept qex's CLI; even then most of them need the same portable fixtures as the fork's.

## What would change it
Recommend **(c)** instead if any of these turns out true:
- **The backend layer can't be drawn cleanly**, for example if the scheduler or daemon state needs pids or process groups in its core types rather than at the edges. The first refactor PR will show this quickly.
- **The ADRs decide to break most qex contracts at once** (state files, protocol, CLI and history format, plus the SPEC gaps above), so that little beyond the scheduler's arithmetic survives.
- **The test port costs more than the product port.** A first sample of portable test fixtures (`sleep`/`true`/`sh -c` replacements) would measure this.

## What's not known yet
- Every Windows line count is an estimate. No Windows port was written, and the compile error count (291) is a lower bound.
- One Windows behaviour was tested: renaming over an open file works.
- Several others are *not verified*:
  - `Instant` across sleep
  - rename keeping the mtime
  - reading another process's command line
  - I/O priority
- Only one machine and one workload (ripgrep) were measured, with 3 repetitions per arm.
- macOS was not evaluated in any step.

The next steps (ADRs, then the backend plan) depend on this decision and haven't been started.
