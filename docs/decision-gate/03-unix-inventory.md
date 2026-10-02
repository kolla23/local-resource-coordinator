# Decision gate, step 3: how much of qex is Unix-only

**Question:** a native Windows coordinator needs a Windows backend. How much of the forked qex code depends on Unix, and how deep does each dependency go? Is it a thin OS call, a rework inside one module, or an assumption built into the design?

**Short answer:**
- **Production code is 37,867 lines**, not counting tests. About **3,635 lines (≈10%)** sit in code that has to change for Windows.
- **15 files (9,024 lines) contain no Unix API**, among them the scheduler. Counting two files whose only Unix content is text, that's **12,451 lines (33%)**.
- By depth, the affected code splits like this:
  - **THIN:** ≈625 lines. A call is swapped behind a function, and the callers don't change.
  - **MODULE:** ≈1,435 lines. A rework inside one module, whose interface holds.
  - **DESIGN:** ≈1,575 lines, in **5 areas**. Each changes a guarantee, a cross-module contract, the on-disk format, the IPC protocol or the CLI, so each needs an ADR.
- **The process model maps to Windows, and maps better.** A supervisor per job that owns a Job Object replaces "parent + `waitpid` on one pid" (step 1).
- **The tests are the larger port:**
  - At least 281 of the 326 e2e tests use Unix commands, APIs or paths (`sleep`, `true`, `sh -c`, `libc::kill`, `/tmp`, …). At least 79 of them run a shell.
  - The unit tests use `libc::SIG*`, FIFOs, mode bits and Unix sockets.

The Windows line counts are estimates from reading the code; each class total is the sum of its per-item estimates below. No Windows port was written. One Windows behaviour was tested directly (see "Tested on Windows").

## Method
Run all of these from the repository root. `src/` and `tests/` are unchanged since `f2d96f0`, and the outputs are in [logs/unix-inventory/](logs/unix-inventory/). Each log's first line names the script.

1. **Text scan:** [`experiments/unix-inventory/scan.py`](../../experiments/unix-inventory/scan.py).
   - It reads only production lines: every `#[cfg(test)]` item and every test-only module (`testutil.rs`) is removed.
   - It counts lines by Unix API category and records which functions they are in.
   - A hit is a place to read, not proof of a dependency.
   - Output: [scan.txt](logs/unix-inventory/scan.txt), [scan.json](logs/unix-inventory/scan.json).
2. **Windows compile:** [`compile-windows.sh`](../../experiments/unix-inventory/compile-windows.sh) and [`summarize.py`](../../experiments/unix-inventory/summarize.py).
   - The script copies the repository to a scratch folder and, only there, removes the `compile_error!`, the empty non-Unix `main` and the 37 `#[cfg(unix)]` module gates.
   - It then runs `cargo check` on this machine: rustc 1.98.1, host `x86_64-pc-windows-msvc`.
   - Result: **291 errors in 22 files.** rustc reports missing names before type errors, so this is a **lower bound**: it shows the first layer of breaks.
   - Output: [compile-windows-summary.txt](logs/unix-inventory/compile-windows-summary.txt).
3. **e2e tests:** [`e2e-unix.py`](../../experiments/unix-inventory/e2e-unix.py) counts the tests whose own body names a Unix command, API or path.
   - Helpers aren't followed, so the counts are lower bounds.
   - Output: [e2e-unix.txt](logs/unix-inventory/e2e-unix.txt).
4. **Depth classification:** three read-only reviews, one per area, read each hit and compile error.
   - They recorded what the code does, the Windows mechanism that would replace it, a class and a rough line count, taken from the function spans.
   - The Windows replacements named below come from Microsoft's documentation of each API, and none of them was tested here, apart from the one in "Tested on Windows". Where even the documentation leaves the behaviour open, the text says *not verified*.
   - I checked the key claims against the code (see "Spot checks").

## Where the breaks are (compile errors per file)

| File | Errors | | File | Errors |
|---|---|---|---|---|
| paths.rs | 71 | | commands.rs | 11 |
| supervisor.rs | 34 | | usage.rs, lifecycle.rs | 9 each |
| keys.rs | 29 | | config.rs, daemon.rs | 7 each |
| client.rs | 21 | | main.rs, history.rs, job.rs, logcap.rs, top.rs | 2 each |
| hook.rs, sys.rs, update.rs | 18 each | | sched.rs, style.rs, watchers.rs | 1 each |
| events.rs | 14 | | | |
| peers.rs | 12 | | | |

- **Where they cluster:** 7 files (paths, supervisor, keys, client, hook, sys, update) hold **209 of the 291 errors (72%)**. The other 15 files have 1 to 14 each.
- **Missing most often:** `std::os::unix` (50), `SIGKILL` (12), `mode` (12), `killpg` (10), `sockaddr_un` (8), `flock` (7) and `STDIN_FILENO` (7).

## THIN: swap a call behind an existing function (≈625 lines)
- **Memory and clocks** (`sys.rs`), ≈175 lines:
  - total and available memory → `GlobalMemoryStatusEx` (≈25)
  - boot id → boot time from `GetTickCount64` (≈30)
  - `pid_alive` → `OpenProcess` + `WaitForSingleObject(0)` (≈35)
  - process start token → `GetProcessTimes` (≈20)
  - executable path → `QueryFullProcessImageNameW` (≈15)
  - local time → the time-zone API (≈30)
  - terminal checks → `IsTerminal` and the console API (≈20)
- **Starting detached processes:** the per-job supervisor (≈38) and the coordinator (≈35), using process-creation flags instead of `setsid`.
- **Stops and waits:**
  - The graceful-then-forced stop of a group becomes Ctrl-Break, a grace period, then `TerminateJobObject` (step 1, scenario 4); ≈32.
  - A reaper thread waits on a handle instead of calling `waitpid`, and the same code works for a process that isn't our child; ≈20.
- **Ctrl-C and broken pipes:**
  - Ctrl-C while the client waits → `SetConsoleCtrlHandler` (≈75).
  - `SIGPIPE` goes away, so `BrokenPipe` is treated as a normal exit, and the gates in `main.rs` are removed (≈65).
- **File locks** (`flock`, for the spawn lock and the usage file) → `LockFileEx` (≈70).
- **Detecting an event reader that left** → `PeekNamedPipe` (≈40).
- **Owner-only file modes** on history, log and job files → rely on the profile's ACL (≈15). This is security-relevant, because job records can hold secrets: whether the inherited ACL keeps other users out is *not verified* and has to be checked in the port.
- **The config reload guard's file identity** (`dev`/`ino`) → volume serial + file ID (≈12; issue #20).
- **The peer pid used for attribution** (`SO_PEERCRED`) → `GetNamedPipeClientProcessId` (≈30).
- **Small items:**
  - job names split on `\` as well as `/` (≈5)
  - colour detection (≈10)
  - the scheduler's memory input (≈3)

## MODULE: rework inside one module (≈1,435 lines)
- **Job launch** (`supervisor.rs`):
  - `CreateProcessW` suspended, `AssignProcessToJobObject`, then `ResumeThread` (step 1, scenario 1).
  - std's `Command` gives no thread handle, so the pipes are set up by hand. ≈200 lines.
- **Timeout and leftover handling** in the supervisor (≈75 lines). An open process handle already stops pid reuse, so the `waitid(WNOWAIT)` trick goes away.
- **Restart recovery** in `daemon.rs` (≈110 lines). The start token becomes the creation time. Job-level kill-on-close (step 1, scenario 7) handles orphans.
- **Politeness** (nice, ionice, `oom_score_adj`) → the priority class (≈150 lines). Windows has no OOM score, and I/O priority needs an undocumented call (*not verified*).
- **Sub-processes with a time limit**: the stop hook in `hook.rs` and the update check's `curl` in `update.rs` (≈165 lines). Each gets a job object and pipes; `poll` becomes a reader thread.
- **Process inspection**:
  - `sys.rs` parent chain → Toolhelp32 (≈100 lines)
  - `sys.rs` group usage for `top` (≈60 lines)
  - `watchers.rs` monitor detection (≈110 lines). Reading another process's command line needs the PEB or WMI (*not verified*); a "not supported" stub is the cheap option.
- **OOM classification** (`supervisor.rs`, ≈50 lines). Windows has no OOM killer; the nearest signal is a job memory-limit message.
- **Raw-key input for `top`** (`keys.rs`, ≈120 lines) → `SetConsoleMode` with VT input, or the `crossterm` crate. Key decoding stays as is.
- **Client connection code** (`client.rs`, ≈250 lines): timeouts on a pipe and the sandbox probe. Its `UnixStream` type leaks into `Connected::Open`.
- **Paths:** HOME/XDG → Known Folders (≈30; user-visible, so it touches CLI compatibility). `ensure_dir(mode)` callers keep the argument and ignore it (≈15).

## DESIGN: assumptions built into a guarantee or a contract (≈1,575 lines, 5 areas, each needs an ADR)
1. **IPC transport and the launch protocol** (`paths.rs`, `daemon.rs`, `client.rs`, `events.rs`; ≈800 lines).
   - Today's rules:
     - Liveness comes from a socket *file*, a `flock`ed pid file and an `nlink` check.
     - Only "nobody listens" (ENOENT, ECONNREFUSED, ENOTSOCK) lets a client start a coordinator.
     - The socket path limit forces short `/tmp` folders and a sweep.
   - Named pipes have no file, and the OS removes them when the server exits, so most of this machinery goes away.
   - `SPEC.md:615` already names local-only, ACL-restricted named pipes for Windows. The *launch rules* still change, and AGENTS.md requires an ADR for that.
   - Today no peer is authenticated by identity: access comes only from file modes (0700 folder, 0600 socket).
2. **Who may signal a job** (`lifecycle.rs` kill and abort, ≈150 lines).
   - The coordinator calls `killpg` on the job's group directly.
   - On Windows it needs either a named Job Object that it can open, or a request routed through the supervisor.
   - This is the "job handle owned by the supervisor" ADR from step 1.
3. **Exit classification and the `--signal` option** (`supervisor.rs` `classify` plus the `signal` field in `job.rs` and `proto.rs`, ≈105 lines plus record fields).
   - "Killed by signal N" doesn't exist on Windows; it needs an exit-code convention.
   - `qex kill --signal HUP|QUIT|USR1|USR2|<number>` has no equivalent. Only INT/TERM can be approximated with Ctrl-C/Ctrl-Break, and only for console processes.
   - Pause does **not** use signals. `pause.rs` holds admission through a file, which ports as it is.
4. **The memory metric in history and admission** (`max_rss` in 10 files, among them `usage.rs`, `proto.rs`, `schema.rs`, `spec.rs`, `top.rs` and the hook variable `QEX_MAX_RSS`; ≈120 lines plus a new admission test).
   - Linux records the largest single child's peak *resident* memory. A Job Object gives the whole tree's simultaneous peak *commit*.
   - Step 1 showed these differ: 706 MiB for the tree against ≈100 MiB per process.
   - Swapping the call is ≈30 lines. Adopting it changes the units of learned claims, the on-disk history (SPEC §7/§9 want a labelled metric kind) and admission (SPEC §10.3.1 wants a separate commit ledger and commit gate).
   - The "commit versus working set" ADR from step 1 is still open.
   - Pressure is a PSI percentage today. Windows only offers a low-memory boolean, so `max_pressure` changes meaning.
5. **The shared peer directory** (`peers.rs`, ≈400 lines, called by `sched.rs` to publish and read claims).
   - It is built on world-writable `/tmp/qex`, the sticky bit and per-uid folders and owners, none of which exist on Windows.
   - An ACL-based `%ProgramData%` design would be new security work. Turning peers off on Windows is the likely answer: the owner's machine is single-user (*inferred*).

**Gaps on every platform, not caused by the OS** (found on the way):
- **Framing:** messages are separated by newlines (`daemon.rs:1824`), where `SPEC.md:615` asks for length-prefixed messages.
- **Admission:** the test has no SPEC §8.2 `growth_reserved` term.

## Tested on Windows: atomic replace works
Before this was tested, I had listed the atomic-replace contract (`job::write_atomic` and its readers) as a sixth design area. The test showed it isn't one.

- [`rename-over-open.rs`](../../experiments/unix-inventory/rename-over-open.rs) holds a file open with std's default `File::open`, then renames another file over it.
- Result, 3 of 3 runs on NTFS (Windows 11 10.0.26200, rustc 1.98.1):
  - the rename succeeds
  - the open reader still reads the old contents
  - the path holds the new contents
- Log: [rename-over-open.txt](logs/unix-inventory/rename-over-open.txt).
- std's default open allows a rename over the file. A reader that is *not* our std code, for example a tool that opens without delete sharing, could still block a replace. That case wasn't tested.

## Portable code
**15 files contain no Unix API**, together **9,024 lines**: `sched.rs` 2,081, `top.rs` 1,451, `spec.rs` 1,271, `proto.rs` 695, `logsel.rs` 549, `resolve.rs` 537, `schema.rs` 536, `capabilities.rs` 483, `fanout.rs` 369, `pipeline.rs` 284, `units.rs` 200, `deps.rs` 188, `context.rs` 173, `claim.rs` 152 and `version.rs` 55.

`help.rs` and `cli.rs` (3,427 lines) only mention Unix in text or defaults. (`style.rs` is not text-only: it calls `isatty`, a THIN item.)

"No Unix API" doesn't mean "untouched". Four of these files (`top`, `spec`, `schema`, `proto`) carry the `max_rss` field that DESIGN area 4 changes.

**The scheduler's logic is OS-independent; its inputs are not.**
- It reads memory and pressure through `Machine` (`sched.rs:358–375`).
- It publishes and reads peer claims through `peers::` (`sched.rs:371`, `:1072`; DESIGN area 5).
- It starts supervisors (`supervisor::spawn`, `sched.rs:2010`), runs hooks and writes job files through other modules, which are counted under those modules above.
- The admission decision itself is arithmetic over claims and pools.

Caveats:
- These files call `sys::` functions that are gated per OS. In all, 18 files call `sys::`, and there is no backend layer: the OS calls are free `#[cfg]` functions.
- The help text (2,406 lines) talks about signals, cgroups and PSI. The e2e suite scans the documentation, so Windows-correct help text is a large text change.

## Issue #20's checklist
These can't be tested until the coordinator builds on Windows. The likely answers come from documentation, reasoning or the rename test above.

| Item | Class | Likely Windows answer |
|---|---|---|
| `Instant` across sleep/hibernate | THIN | std uses QueryPerformanceCounter. Whether it counts during sleep is *not verified*. Either answer falls on the safe side, as the issue argues. |
| NTFS mtime and rename | THIN | `ftLastWriteTime` at 100 ns (FAT: 2 s). A rename probably keeps the mtime (*not verified*). The reload guard reads with std's default open. The rename test shows that this read doesn't block a *std* rename over the file; an editor's own atomic save (`MoveFileExW` or `ReplaceFileW`) is *not verified*. |
| `dev`/`ino` file identity | THIN | Volume serial + 128-bit file ID (`GetFileInformationByHandleEx`). Probably needs the `windows-sys` crate. |
| The 15 reload unit tests + the e2e test | — | Most inject their clocks, so they should port once `config` and `daemon` compile. |
| Manual sleep/wake check | — | Needs real hardware after the port. |
| `/tmp` in `testutil::temp_dir` | THIN (tests only) | Local AppData `\Temp` from the Known Folder API, which doesn't read `TEMP`/`TMP`. `GetTempPath2W` reads them, so it would bring back the fault fixed in PR #25. |

## Spot checks
I checked these claims against the code at `f2d96f0`:
- No `SIGSTOP`/`SIGCONT` anywhere in `src/`.
- `--signal` accepts TERM, KILL, INT, HUP, QUIT, USR1 and USR2 (`lifecycle.rs:17–40`).
- Framing is `reader.lines()` (`daemon.rs:1824`).
- The peer default is `/tmp/qex` (`config.rs:461`).
- `max_rss` appears in 10 files.
- `SPEC.md:615` names named pipes for Windows.

## Not done
- No Windows port was written, so every Windows line count is an estimate.
- The compile error count is a lower bound.
- I didn't measure how much the documentation or the 610 unit tests would change beyond the patterns above.
- macOS wasn't evaluated.
