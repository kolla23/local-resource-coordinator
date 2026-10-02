# Decision gate, step 3: how much of qex is Unix-only

**Question:** a native Windows coordinator needs a Windows backend. How much of the forked qex code depends on Unix, and how deep does each dependency go? Is it a thin OS call, a rework inside one module, or an assumption built into the design?

**Short answer:**
- **Production code is 37,956 lines**, not counting tests. About **3,700 lines (≈10%)** sit in code that has to change for Windows.
- **13 files (8,274 lines) contain no Unix API at all**, among them the scheduler. Counting three files whose only Unix content is text, that's **11,831 lines (31%)**.
- By depth, the affected code splits like this:
  - **THIN:** ≈620 lines. A call is swapped behind a function, and the callers don't change.
  - **MODULE:** ≈1,500 lines. A rework inside one module, whose interface holds.
  - **DESIGN:** ≈1,600 lines, in **6 areas**. Each changes a guarantee, a cross-module contract, the on-disk format, the IPC protocol or the CLI, so each needs an ADR.
- **The process model maps to Windows, and maps better.** A supervisor per job that owns a Job Object replaces "parent + `waitpid` on one pid" (step 1).
- **The tests are the larger port:**
  - 281 of the 326 e2e tests use Unix commands or APIs (`sleep`, `true`, `sh -c`, `libc::kill`, …). 79 of them use `sh -c`.
  - The unit tests use `libc::SIG*`, FIFOs, mode bits and Unix sockets.

All line counts are estimates from reading the code, not from a port. No Windows code was written for this step.

## Method
Run all three from the repository root at commit `f2d96f0`. The outputs are in [logs/unix-inventory/](logs/unix-inventory/).

1. **Text scan:** [`experiments/unix-inventory/scan.py`](../../experiments/unix-inventory/scan.py).
   - It reads only production lines: every `#[cfg(test)]` item is removed.
   - It counts lines by Unix API category (process groups, wait, signals, fds, Unix sockets, permissions, file identity, `/proc`, macOS kernel calls, tty, rlimits, other `libc`) and records which functions they are in.
   - A hit is a place to read, not proof of a dependency.
   - Output: [scan.txt](logs/unix-inventory/scan.txt), [scan.json](logs/unix-inventory/scan.json).
2. **Windows compile:** [`compile-windows.sh`](../../experiments/unix-inventory/compile-windows.sh) and [`summarize.py`](../../experiments/unix-inventory/summarize.py).
   - The script copies the repository to a scratch folder and removes the `compile_error!` and the 37 `#[cfg(unix)]` module gates there.
   - It then runs `cargo check` on this machine: rustc 1.98.1, host `x86_64-pc-windows-msvc`.
   - Result: **292 errors in 22 files.** rustc reports missing names before type errors, so this is a **lower bound**: it shows the first layer of breaks.
   - Output: [compile-windows-summary.txt](logs/unix-inventory/compile-windows-summary.txt).
3. **Depth classification:** three read-only reviews, one per area, read each hit and compile error.
   - They recorded what the code does, the Windows mechanism that would replace it, a class (THIN, MODULE or DESIGN) and a rough line count.
   - Claims that rest on Windows behaviour nobody tested here are marked *not verified*.
   - I checked the key claims against the code (see "Spot checks").

## Where the breaks are (compile errors per file)

| File | Errors | | File | Errors |
|---|---|---|---|---|
| paths.rs | 71 | | commands.rs | 11 |
| supervisor.rs | 34 | | usage.rs, lifecycle.rs | 9 each |
| keys.rs | 29 | | config.rs, daemon.rs | 7 each |
| client.rs | 21 | | main.rs | 3 |
| hook.rs, sys.rs, update.rs | 18 each | | history, job, logcap, top | 2 each |
| events.rs | 14 | | sched, style, watchers | 1 each |
| peers.rs | 12 | | | |

The names missing most often are `std::os::unix` (50), `SIGKILL` (12), `mode` (12), `killpg` (10), `sockaddr_un` (8), `flock` (7) and `STDIN_FILENO` (7).

## THIN: swap a call behind an existing function (≈620 lines)
- **Memory and clocks** (`sys.rs`):
  - total and available memory → `GlobalMemoryStatusEx`
  - boot id → boot time from `GetTickCount64`
  - `pid_alive` → `OpenProcess` + `WaitForSingleObject(0)`
  - process start token → `GetProcessTimes`
  - executable path → `QueryFullProcessImageNameW`
  - local time → the time-zone API
  - terminal checks → `IsTerminal` and the console API
- **Starting detached processes:** the per-job supervisor and the coordinator, using process-creation flags instead of `setsid`.
- **Stops:** the graceful-then-forced stop of a group becomes Ctrl-Break, a grace period, then `TerminateJobObject` (step 1, scenario 4).
- **Waits:** a reaper thread waits on a handle instead of calling `waitpid`, and the same code works for a process that isn't our child.
- **Ctrl-C while the client waits** → `SetConsoleCtrlHandler`. `SIGPIPE` goes away; treat `BrokenPipe` as a normal exit.
- **File locks** (`flock`, for the spawn lock and the usage file) → `LockFileEx`.
- **Owner-only file modes** on history and log files → rely on the profile's ACL.
- **The config reload guard's file identity** (`dev`/`ino`) → volume serial + file ID (issue #20).
- **The peer pid used for attribution** (`SO_PEERCRED`) → `GetNamedPipeClientProcessId`.

## MODULE: rework inside one module (≈1,500 lines)
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
- **Raw-key input for `top`** (`keys.rs`, ≈120 lines) → `SetConsoleMode` with VT input, or the `crossterm` crate. Key decoding stays as is.
- **Client connection code** (`client.rs`, ≈250 lines): timeouts on a pipe and the sandbox probe. Its `UnixStream` type leaks into `Connected::Open`.
- **Paths:** HOME/XDG → Known Folders (user-visible, so it touches CLI compatibility). `ensure_dir(mode)` callers keep the argument and ignore it.

## DESIGN: assumptions built into a guarantee or a contract (≈1,600 lines, 6 areas, each needs an ADR)
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
4. **The memory metric in history and admission** (`max_rss` in 10 files; `usage.json`, `proto.rs`, the schema, the hook variable `QEX_MAX_RSS`).
   - Linux records the largest single child's peak *resident* memory. A Job Object gives the whole tree's simultaneous peak *commit*.
   - Step 1 showed these differ: 706 MiB for the tree against ≈100 MiB per process.
   - Swapping the call is ≈30 lines. Adopting it changes the units of learned claims, the on-disk history (SPEC §7/§9 want a labelled metric kind) and admission (SPEC §10.3.1 wants a separate commit ledger and commit gate).
   - The "commit versus working set" ADR from step 1 is still open.
   - Pressure is a PSI percentage today. Windows only offers a low-memory boolean, so `max_pressure` changes meaning.
5. **The shared peer directory** (`peers.rs`, ≈400 lines).
   - It is built on world-writable `/tmp/qex`, the sticky bit and per-uid folders and owners, none of which exist on Windows.
   - An ACL-based `%ProgramData%` design would be new security work. Turning peers off on Windows is the likely answer: the owner's machine is single-user (*inferred*).
6. **The atomic replace contract** (`job::write_atomic` and every reader; ≈60 lines plus the readers).
   - Rename-over works on Windows, but a reader that holds the file open without `FILE_SHARE_DELETE` (std's default) can make the replace fail (*not verified*).
   - This affects every state file the coordinator and supervisors share.

Not about the OS, but found on the way: newline-separated JSON framing (`daemon.rs:1824`) where SPEC §615 asks for length-prefixed messages, and an admission test without SPEC §8.2's `growth_reserved`. Both are gaps on every platform.

## Portable code
These files contain **no Unix API**: `sched.rs` 2,081 lines, `top.rs` 1,451, `spec.rs` 1,271, `logsel.rs` 549, `resolve.rs` 537, `schema.rs` 536, `capabilities.rs` 483, `fanout.rs` 369, `pipeline.rs` 284, `units.rs` 200, `deps.rs` 188, `context.rs` 173 and `claim.rs` 152. Together: **8,274 lines**.

`help.rs`, `cli.rs` and `style.rs` (3,557 lines) only mention Unix in text or defaults. `proto.rs` and `version.rs` have no hits either.

**The scheduler is OS-independent apart from its inputs.** It reads `available_memory`, `memory_pressure` and the clock through `Machine` (`sched.rs:358–375`); the rest is arithmetic over claims. Two caveats:
- "No Unix API" doesn't mean "compiles alone". These files call `sys::` functions that are gated per OS. In all, 18 files call `sys::`, and there is no backend layer: the OS calls are free `#[cfg]` functions.
- The help text (2,406 lines) talks about signals, cgroups and PSI. The e2e suite scans the documentation, so Windows-correct help text is a large text change.

## Issue #20's checklist
None of these can be tested until the coordinator builds on Windows. The likely answers below come from documentation and reasoning only.

| Item | Class | Likely Windows answer |
|---|---|---|
| `Instant` across sleep/hibernate | THIN | std uses QueryPerformanceCounter. Whether it counts during sleep is *not verified*. Either answer falls on the safe side, as the issue argues. |
| NTFS mtime and rename | THIN, with one risk | `ftLastWriteTime` at 100 ns (FAT: 2 s). A rename probably keeps the mtime (*not verified*). **New risk:** the reload guard's open handle without `FILE_SHARE_DELETE` can make an editor's atomic save fail while it reads. |
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
- No Windows code was written, so every Windows line count is an estimate.
- The compile error count is a lower bound.
- I didn't measure how much the documentation or the 610 unit tests would change beyond the patterns above.
- macOS wasn't evaluated.
