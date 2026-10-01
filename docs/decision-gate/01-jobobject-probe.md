# Decision gate, step 1: native Windows Job Object probe

**Question:** can a coordinator on native Windows own a whole process tree from its first instruction, measure the tree's memory, and cancel it reliably? qex does this on Unix with process groups and `waitpid`. Windows has neither, so a port depends on Job Objects.

**Answer from this machine: yes, with two limits.**
- It doesn't catch processes started *outside* the tree's parentage, such as through WMI.
- A process that has only just started can miss a graceful Ctrl-Break, so a forced stop must always follow it.

Every number below comes from 3 full runs in this session ([logs/jobobject-probe-windows.txt](logs/jobobject-probe-windows.txt)).

## Setup
- Probe: [experiments/jobobject-probe/](../../experiments/jobobject-probe/). It's a standalone Rust program (~600 lines) using `windows-sys` 0.61.2, and it's **not part of qex**.
- Environment: Windows 11 Home 10.0.26200.9457, i7-1255U, 15.7 GiB RAM, rustc 1.98.1 (MSVC).
- The probe itself was already running inside a job, because the terminal that started it put it in one. Nested jobs worked anyway.
- Test tree: one root process with 2 children, each of which has 2 children, so **7 processes**. Each process allocates and touches a fixed amount of memory, then holds it.
- Run it with: `cargo run --release --manifest-path experiments/jobobject-probe/Cargo.toml`

## Results (identical across all 3 runs unless a range is given)

| # | Scenario | Result | Meaning for the port |
|---|---|---|---|
| 1 | Start the root **suspended**, put it in the job, check, then resume | ✅ In the job before any of its code ran | No startup gap. On Unix, qex's process can start running before it's recorded. |
| 2 | 7 processes × 100 MiB, sampled every 100 ms | ✅ The job saw all 7 processes, the 6 descendants joining automatically. Kernel **peak job commit 705.8–706.0 MiB**; the sampled sum of private memory gave the same; the sampled sum of working sets was 732 MiB, which includes shared DLL pages. | **Measures the whole tree's simultaneous memory**, which is what qex's Linux history (`getrusage` max child) can't do. |
| 3 | A 400 MiB spike lasting 150 ms | ✅ Kernel peak commit **401.4 MiB**. In this run 250 ms sampling also caught it (401.4 MiB), because the spike happened while the probe was taking its first sample. | The kernel keeps a true running peak, so short spikes can't slip between samples. |
| 4a | Cancel: Ctrl-Break first, a 2 s grace period, then force | ✅ **7 of 7 stopped gracefully** in 23–34 ms; nothing forced; no survivors | Graceful stop works for console processes in the group. |
| 4b | Cancel: force only (`TerminateJobObject`) | ✅ The job's counter hit 0 after 0.08–0.11 ms, **every process had fully exited after 5.7–5.8 ms**, no survivors after 10 s | Reliable, fast hard stop of the whole tree. |
| 5 | A child tries to leave the job (`CREATE_BREAKAWAY_FROM_JOB`) | ✅ **Refused**: "Access is denied (os error 5)" | Children can't opt out unless we allow it. |
| 6 | A child puts a grandchild into its **own** nested job | ✅ It worked, the grandchild was still listed in our job, and **cancelling our job killed it** | Tools that use their own jobs, as some build systems do, stay under our control. |
| 7 | The owner, a stand-in coordinator, is killed with no chance to clean up | ✅ Kill-on-close stopped all 7 processes within 6.5–8.2 ms; no survivors | If the coordinator crashes, its jobs die with it. (Whether that's the *desired* policy is a separate design decision; see below.) |
| 8 | A child starts a process through **WMI** (`Win32_Process.Create`) | ⚠️ **Escaped:** the new process wasn't in the job and **kept running after cancel**. The probe then killed it. | A known limit. Anything launched through WMI, the Task Scheduler, services or out-of-process COM servers is outside the job, the same way a Unix process that leaves its session escapes. |

### A finding from building the probe: startup race on graceful cancel
- On the first versions of the cancel test, 1 to 4 of the 7 processes didn't react to Ctrl-Break, and the forced stop cleaned them up. They were always the newest leaf processes.
- When cancel first waited until every worker had installed its Ctrl-Break handler, all 7 stopped gracefully, 3 runs out of 3.
- So **a process that has only just started may miss the graceful signal**, and the coordinator must always follow a graceful request with a forced stop after a grace period. qex already follows the same pattern on Unix: a signal, then force.

## What was not tested
- Programs without a console, such as GUI apps or ones started with `DETACHED_PROCESS` or `CREATE_NEW_CONSOLE`. They don't receive Ctrl-Break, so they'd only get the forced stop.
- Handle inheritance, and limits on job count or nesting depth.
- Running elevated, or across users.
- `JOB_OBJECT_LIMIT_BREAKAWAY_OK` set deliberately.
- Memory limits enforced by the job. We only measured; we didn't cap.
- Commit (private) memory isn't the same as RAM in use. Working set and private commit can diverge for memory-mapped files and shared pages, and SPEC §10.3 needs a decision on which one admission should use.
- Accuracy under heavy paging, and per-process CPU accounting.
- Only one machine and one Windows build, so this isn't a support claim.

## Design questions this raises (for ADRs later, not decided here)
1. **Kill-on-close versus surviving a coordinator restart.** qex keeps a job running when its coordinator restarts, because a separate supervisor process owns the job. On Windows, the job handle should probably live in that per-job supervisor rather than the coordinator, which keeps both the restart behaviour and the crash cleanup in scenario 7.
2. **Escapes.** Detect and report them (for example, by flagging any process created during a job that's outside it), or document them as a V1 limit, matching SPEC's "best-effort, labelled" wording.
3. **Graceful-stop protocol.** Ctrl-Break to the process group, a grace period, then `TerminateJobObject`, with the outcome reported for each process.

## Bottom line for the gate
The Windows mechanisms qex is missing are available and behave well:
- ownership from the first instruction
- whole-tree peak memory measured by the kernel
- fast, complete cancellation
- cleanup when the owner crashes

This is **evidence for feasibility**, not a working port. Measuring how much qex code is Unix-specific (a later gate step) will show how much has to be rebuilt around these mechanisms.
