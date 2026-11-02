# Per-job TEMP ADR: per-job TEMP folders

**Date:** 2026-11-02
**Status:** proposed; accepted when the owner merges the PR that adds this file
**Context:** [PORT_PLAN.md](../PORT_PLAN.md), "ADRs" row "1 (alongside)"; evidence in [02-parallel-workload-windows.md](../../decision-gate/02-parallel-workload-windows.md). Source citations are at commit `288f85f` (main after PR #98 at 4fa4472 and PR #99 merged). This ADR takes no number: "ADR 2" to "ADR 5" already name other ADRs in PORT_PLAN.md and elsewhere.

## Context

Three copies of one Rust project, built and tested at the same time on this PC with nothing coordinating them, gave 40 to 66 false test failures per worktree in two of the first three parallel runs, and in all three runs of a later same-day control. ripgrep's test harness writes to `%TEMP%\ripgrep-tests\<test>\<n>`, with a counter that starts at 0 in every process, so the three runs wrote the same paths ("os error 32"). With a TEMP and TMP of their own, three of three parallel runs were clean (437 of 437 tests in every worktree), with no loss of speed (02, Follow-up and Control). Memory-based admission alone would not have prevented it.

The SPEC keeps the submitted environment as it is: goal 3.1.1 preserves the command's environment (docs/fork/SPEC.md:37), V1 does not rewrite environment limits (docs/fork/SPEC.md:52), default execution sets no runtime variables and "such changes require explicit user configuration" (docs/fork/SPEC.md:191), and no runtime limit is introduced implicitly (S08, docs/fork/SPEC.md:691). It also allows "documented coordinator-owned additions" to the environment (docs/fork/SPEC.md:1007), fingerprints the environment before coordinator additions (docs/fork/SPEC.md:302), and asks for owner-only state with existing ownership validated (docs/fork/SPEC.md:668) and for "handle-relative/no-follow operations where available" (docs/fork/SPEC.md:671).

Today the supervisor gives the job the captured environment (`env_clear().envs(&job_env)`, src/supervisor.rs:447 to src/supervisor.rs:464) and then sets the device variables, which "REPLACE the captured environment" (src/supervisor.rs:466). The CPU and memory hints fill only missing values and only when asked for (`export_claim`, src/spec.rs:1164).

The owner settled D1 to D7 and three conditions on 2026-11-02, and answered the final review's two design questions (cleanup through a handle; per-platform proof that no descendant lives). This ADR records them with the facts they rest on.

Numbers in prose are checked by review against the logs, not by a command.

## Decisions

### D1 Off by default; config turns it on; a per-job flag overrides

- Config: a new table `[temp]` with `private = false` by default (beside the other tables of `Config`, src/config.rs:954).
- CLI: `--private-temp` and `--no-private-temp`, each overriding the other, the last one given wins. Both are new flags, so this is a CLI-compatibility change decided here.
- Job file: `private_temp: Option<bool>`, like `no_limit_env_hints` (src/spec.rs:53).
- Precedence: a flag, else the job-file key, else the config.
- With the feature off, Linux and macOS behave exactly as today. `[submit]` was rejected as the home for the switch: it is about capture, not launch.

### D2 When on, the values replace the submitted ones

Windows always sets TEMP and TMP, so filling only missing values would do nothing there. The feature therefore replaces them, in the same block that sets the device variables (src/supervisor.rs:466). That makes it the second replace case after device assignments, for the same reason: the coordinator owns the value, and a value from the user's shell would send two jobs to one place.

### D3 TMPDIR, TMP and TEMP, on every platform

All three are set on every platform. Rust's `temp_dir` reads TMPDIR on Unix ([std::env::temp_dir](https://doc.rust-lang.org/1.85.0/std/env/fn.temp_dir.html): "On Unix, returns the value of the TMPDIR environment variable"); Windows' [GetTempPath2W](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-gettemppath2w) "checks for the existence of environment variables in the following order": TMP, TEMP, USERPROFILE; Python's `tempfile` and other cross-platform tools read TEMP and TMP on Unix too. Tools that ignore all three are D6's case.

### D4 Under the state root, one folder per attempt

- Layout: `<state root>/jobs/<uuid>/tmp/<attempt>`, with `<attempt>` the 1-based attempt number in decimal. The folder lives in the job directory, which is already owner-only (`paths::ensure_dir(&dir, 0o700)`, src/daemon.rs:2906) and which `qex clean` (src/lifecycle.rs:424) and the orphan sweep of `qex gc` (src/commands.rs:6865) remove whole. A separate `<state root>/tmp/<uuid>` tree was rejected: it would need its own sweep.
- Ownership and links: `tmp` and `<attempt>` are made and checked through a no-follow handle (A2). An existing `tmp` not owned by the user fails the job before launch; one with any group or other bit is narrowed to 0700 through that handle.
- Windows root bound: on Windows, a private-temp job is refused before launch (S09) when the state root's path is longer than 111 characters (A1). Linux and macOS have no such bound (owner's answer).

### D5 A fresh folder per attempt; deleted only when no descendant lives

- The folder is named `status.attempts + 1` where the variables are set, because the spawn (src/supervisor.rs:580) comes before the increment (src/supervisor.rs:648). It must not exist before (`mkdirat` fails with EEXIST, and the job fails).
- **Deletion** happens at attempt end only when the platform's evidence shows no descendant of the job is alive. Every failure, and every case the evidence cannot settle, keeps the folder and sets `temp_kept`.
  - Linux: the supervisor sets `PR_SET_CHILD_SUBREAPER` on itself before the spawn, so every orphaned descendant of the job is re-parented to it ([PR_SET_CHILD_SUBREAPER](https://man7.org/linux/man-pages/man2/PR_SET_CHILD_SUBREAPER.2const.html): "When a process becomes orphaned ... then that process will be reparented to the nearest still living ancestor subreaper"; "not inherited by children created by fork(2) and clone(2)"; "preserved across execve(2)"). The job's end is read only from waits that name the job pid, as today: `wait_without_reaping` (src/supervisor.rs:700) is `waitid(P_PID, pid, ...)` with `WNOWAIT` (src/supervisor.rs:1288 to src/supervisor.rs:1292), and `child.wait()` (src/supervisor.rs:737) is std's wait on that one child. No wait in src/ takes any child (`git grep -n -E 'waitpid\(-1|waitpid\(0|P_ALL|P_PGID|SIG_IGN' 288f85f -- src` prints nothing), so those waits stay as they are. A new collector reaps adopted children during the attempt, each only by its own pid (found with a non-reaping `WNOWAIT` peek), never with a reaping wait on any child, which could take the job's status. At attempt end, after the job is reaped and a last collection, the folder is deleted only when the supervisor has no children left: `waitid(P_ALL, 0, WEXITED | WNOHANG | WNOWAIT)` fails with ECHILD ([wait(2)](https://man7.org/linux/man-pages/man2/wait.2.html): "ECHILD ... does not exist or is not a child of the calling process"), within a bound the implementing PR measures. A failing `prctl` means keep.
  - #85 (recovery counted a zombie supervisor alive) is fixed by #99 (288f85f): `own_pid_alive` is now `kill(pid, 0) == 0 && !process_is_zombie(pid)` (`process_is_zombie`, src/os/unix.rs:208). The subreaper changes who collects the job's orphans, not who collects the supervisor, and the no-children test reads ECHILD, never `kill(pid, 0)`, which a zombie passes.
  - Windows: the job runs in the per-job Job Object of the launch design (docs/fork/SPEC.md:352: "create a user-private Job Object and root process suspended, assign it successfully, then resume. ... do not enable breakaway"; docs/fork/SPEC.md:358: the supervisor owns the only Job handle). ADR 1 has no Job Object; PORT_PLAN's "named Job Object, if" the signal ADR "chooses one" is this same object given a name. It carries neither `JOB_OBJECT_LIMIT_BREAKAWAY_OK` ([JOBOBJECT_BASIC_LIMIT_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information)) nor `JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK`, which lets children out without asking ([Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)). A child's `CREATE_BREAKAWAY_FROM_JOB` is then refused, as the probe showed ("Access is denied (os error 5)", docs/decision-gate/01-jobobject-probe.md:29). The folder is deleted only when [QueryInformationJobObject](https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-queryinformationjobobject) reports `ActiveProcesses` 0 ([JOBOBJECT_BASIC_ACCOUNTING_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_accounting_information): "The total number of processes currently associated with the job"; child jobs' processes included, [Nested Jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs)).
  - macOS has no subreaper (the page's STANDARDS line: "Linux.") and no Job Object. Its only evidence is the job's process group and output pipes, and a descendant that left the group (`setsid`) and closed both streams is invisible to both (docs/fork/QEX_ASSESSMENT.md:80: "Process groups do not contain descendants that escape into new sessions"). So after a spawn that worked, nothing can rule escapees out: macOS keeps every attempt folder and sets `temp_kept`. Only the spawn-failure path, where nothing ran, deletes.
  - On every platform the folder is also kept when the supervisor sent no stop signal (`reserved` false, src/supervisor.rs:700) or a pipe stayed open past the 30 s limit (`incomplete`, src/supervisor.rs:757; src/supervisor.rs:962).
- **Order:** the deletion runs before the attempt's status write and before every `hook::fire` in the supervisor's `main`: the spawn-failure path (src/supervisor.rs:594, src/supervisor.rs:596), the OOM path (src/supervisor.rs:845, src/supervisor.rs:848), and the write at src/supervisor.rs:877 that serves both the retry (src/supervisor.rs:889) and the final hook (src/supervisor.rs:909). The implementing PR finds every exit with `grep -n -E 'hook::fire|fire_detached|write_status|return main\(' src/supervisor.rs` and `grep -rn -E 'hook::fire|fire_detached' src/` and gives one verdict per hit.
- **A supervisor gone without a terminal record** (a signal, the OOM killer, a failing `?`): the coordinator writes Failed and fires the hook itself, in `reap` (src/supervisor.rs:237, src/supervisor.rs:243) and, after a coordinator restart, in `recover` (src/daemon.rs:1616 and src/daemon.rs:1619, job still alive; src/daemon.rs:1647 and src/daemon.rs:1654, neither alive). Before each of those three writes it looks at `jobs/<uuid>/tmp` (an lstat; a `tmp` that is not a directory sets the flag without being read through; a directory, its listing) and sets `temp_kept` in the record it writes when anything is there. It never deletes: it cannot run the supervisor's checks, so it keeps and lists. `recover`'s branch for a terminal record written after its first read (src/daemon.rs:1576, hook at src/daemon.rs:1583, from #99) writes nothing and fires the hook on the supervisor's own record, written after its deletion or flag, so it needs no look.
- **The record:** `status.json` gains `temp_kept: bool` (serde default false, so older records read unchanged). It is set when a deletion was not confirmed and never set back to false. While it is true, `qex status` lists the kept folders of a finished attempt: every folder under `tmp/` once the record is terminal; while it is not, only folders named below the record's `attempts`, so the current attempt's folder is never listed as kept. The listing lstats `tmp` first and never reads through a link: a `tmp` that is not a directory is printed as itself (its path and kind), with nothing read through it.
- **What holds:** a folder the supervisor deleted is gone before that attempt's status write and every hook; every folder of a finished attempt that still exists when a stop hook runs or `qex status` reads the record is flagged by `temp_kept` in that record. The folder of an attempt still running is not covered.
- No keep-for-debugging option and no disk quota in V1. S05 (docs/fork/SPEC.md:688) is the same rule for reservations: root exit alone frees nothing while a known descendant lives.

### D6 Nothing new for tools that ignore TEMP

Tools that ignore TMPDIR, TMP and TEMP, or write to fixed paths, are served by the existing `--lock NAME` (src/cli.rs:282): jobs that name the same lock never run at the same time.

### D7 SPEC amended in this PR

Three pointer lines in SPEC: after §8.1's runtime-variables sentence (docs/fork/SPEC.md:191), after the capture sentence (docs/fork/SPEC.md:304), and inside §27.3 item 5 (docs/fork/SPEC.md:1007), each "Amended by the per-job TEMP ADR". The values are added at launch by the supervisor and never written to `spec.json`, so `--show-env`, the learn key and any future fingerprint (docs/fork/SPEC.md:302) see the submitted environment. No environment fingerprint exists in the code today.

### A1 The longest path, and the Windows root bound

Windows tools that are not long-path aware are held to [MAX_PATH](https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation): "the maximum length for a path is MAX_PATH, which is defined as 260 characters", so 259 for a file path after the NUL, and "the directory name cannot exceed MAX_PATH minus 12", so 248 for a directory path. The long-path opt-in needs the registry setting and each tool's manifest, so a job's tools do not get it.

The parts of the longest default per-attempt folder, in characters:

- `C:\Users\`: 9.
- The profile folder: 40, a SAM account name of at most 20 ([sAMAccountName](https://learn.microsoft.com/en-us/windows/win32/adschema/a-samaccountname)), a `.`, a NetBIOS domain of at most 15 ([naming conventions](https://learn.microsoft.com/en-us/troubleshoot/windows-server/active-directory/naming-conventions-for-computer-domain-site-ou)) and `.000`.
- `\AppData\Local\`: 15, `FOLDERID_LocalAppData` ([KNOWNFOLDERID](https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid)).
- The product folder: 24, ADR 1's bound on the product part.
- `\jobs\`: 6; the job id: 36, a UUID (src/job.rs:422); `\tmp\`: 5; the attempt: 10, a u32 at most `4294967295`.

- Longest default per-attempt folder: `echo $((9+40+15+24+6+36+5+10))` = 145.
- Headroom at the default root: `echo $((248-145))` = 103 for directories, `echo $((259-145))` = 114 for files. ripgrep's `\ripgrep-tests\<test>\<n>` with a 40-character test name is `echo $((15+40+2))` = 57, a directory chain, so it fits.
- Root bound on Windows: the fixed tail is `echo $((6+36+5+10))` = 57, and 80 characters are kept for the directories a tool makes: `echo $((248-57-80))` = 111. At the bound the per-attempt path is 168, leaving 80 for directories and 91 for files. The refusal names the length, the bound and the two fixes (a shorter state root, or the feature off).
- The final product folder name (#28) must be 24 characters or fewer; the placeholder `local-resource-coordinator` is 26.
- Linux and macOS: no bound. PATH_MAX is 4,096 on Linux ([limits.h](https://github.com/torvalds/linux/blob/v6.12/include/uapi/linux/limits.h), line 13) and 1,024 on macOS ([syslimits.h](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/sys/syslimits.h), line 111).

### A2 Making the folders the #88 way

Added by PR #98 at 4fa4472 and read at 288f85f (`git show 288f85f:src/os/unix.rs`), the #88 helpers give different guarantees, and none works relative to a descriptor (`git grep -c -E 'openat|unlinkat|O_NOFOLLOW|fchmod|mkdirat|fstatat|AT_REMOVEDIR' 288f85f -- src/os/unix.rs src/os/mod.rs src/paths.rs src/daemon.rs` prints nothing):

- `make_one_level` (src/os/unix.rs:721) is one `mkdir` with mode 0700 (src/os/unix.rs:723). It never follows a final link ([mkdir](https://pubs.opengroup.org/onlinepubs/9799919799/functions/mkdir.html): "If path names a symbolic link, mkdir() shall fail and set errno to [EEXIST]"), and it takes EEXIST as success, so alone it does not say what is there.
- `lstat_entry` (src/os/unix.rs:523) reports a path's last component, kind and owner, without following it (`symlink_metadata`, src/os/unix.rs:525).
- `ensure_dir` (src/os/unix.rs:427) and `check_state_dir` (src/os/unix.rs:456) run `Walk` (src/os/unix.rs:578, "following only owned links"), whose `Kind::Link` branch (src/os/unix.rs:654) checks the owner, then follows the link (src/os/unix.rs:668). They refuse another user's link and follow the user's own.
- `narrow_owned_dir` (src/os/unix.rs:475) checks the owner on an opened handle (src/os/unix.rs:489) and narrows through it (src/os/unix.rs:494), but it opens by path with `O_DIRECTORY` alone (src/os/unix.rs:479), so it follows a final link, and it narrows only group or other write (src/os/unix.rs:493).

So, for the two levels this feature creates (owner condition 2: one level at a time, owner-only, refusing links):

1. `tmp` is made with `make_one_level`, then opened once with `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC` ([open](https://pubs.opengroup.org/onlinepubs/9799919799/functions/open.html): "O_NOFOLLOW If path names a symbolic link, fail and set errno to [ELOOP]"). That handle stays open from its creation until the supervisor exits, across the retry, and `tmp` is never reopened by path.
2. Through that handle: `fstat` must show a directory owned by the effective uid, else the job fails before launch with #98's `refusal` text (src/os/unix.rs:733), also as root; a `tmp` with any group or other bit (`mode & 0o077`) is narrowed to 0700 with `fchmod` on the handle ([fchmod](https://pubs.opengroup.org/onlinepubs/9799919799/functions/fchmod.html)). Narrowing, not refusing: a 0700 `mkdir` never makes a wider `tmp`, so a wide one is a leftover the user owns, and a change through the verified handle cannot be redirected.
3. `<attempt>` is made with `mkdirat(tmp_fd, "<n>", 0700)`, where EEXIST fails the job, and checked with `openat(tmp_fd, "<n>", O_NOFOLLOW | O_DIRECTORY)` and `fstat`, whose identity (`st_dev`, `st_ino`) the supervisor keeps.
4. Before every launch, retries included, the supervisor lstats the paths `jobs/<uuid>/tmp` and `jobs/<uuid>/tmp/<n>` without following them and compares their identity with the held `tmp` handle and the kept `<n>` identity. A mismatch fails the job before its program starts and sets `temp_kept`. The variables are paths, so this binds them to the folders that were made and checked.

Above them, the job directory is made by `ensure_dir` and the state root is checked by `check_state_dir` in `state_dir()` (src/paths.rs:41); owned links there are followed, as #88 decided (owner's answer: #88 unchanged). Reuse: `make_one_level`, `refusal` and `lstat_entry` (for the path-only looks of the coordinator and `qex status`) are called unchanged; `narrow_owned_dir` is not called, because it would follow a final link, so its pattern is repeated on the no-follow handle; the descriptor-relative calls are new code beside them in src/os/unix.rs, and no #88 helper changes. Rejected: `ensure_dir` for the two new levels (it follows an owned link), and a "refuse every link" switch in `Walk` (a change to #88's helper). PR #98 merged, so the implementing PR builds on it directly.

Windows: the backend opens `tmp` with `FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS` ([CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)), refuses a reparse point there, keeps the handle, and makes and checks `<attempt>` relative to it ([NtCreateFile](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntcreatefile) with a `RootDirectory` handle, [InitializeObjectAttributes](https://learn.microsoft.com/en-us/windows/win32/api/ntdef/nf-ntdef-initializeobjectattributes), and `FILE_OPEN_REPARSE_POINT`). Its owner and ACL rule is PORT_PLAN's (PORT_PLAN.md:116, PORT_PLAN.md:132).

### A3 Cleanup never follows a link (owner condition 3)

The cleanup is handle-relative, from the `tmp` handle of A2:

- Linux: `openat2(tmp_fd, "<n>", ...)` with `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC` and `RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS` ([openat2](https://man7.org/linux/man-pages/man2/openat2.2.html): RESOLVE_NO_XDEV "Disallow traversal of mount points during path resolution (including all bind mounts)"), then a walk that opens every directory level the same way relative to its parent's descriptor, removes files and links with `unlinkat(dir_fd, name, 0)` and directories with `unlinkat(dir_fd, name, AT_REMOVEDIR)` ([unlinkat](https://pubs.opengroup.org/onlinepubs/9799919799/functions/unlinkat.html)), and never follows a link: a link at any level fails the open and is unlinked as an entry. A mount point at any level fails with EXDEV. EXDEV, ENOSYS (a kernel before 5.6, which has no `openat2`) or any other refusal stops the walk, keeps the folder and sets `temp_kept`, so such a kernel behaves like macOS: it keeps every folder after a spawn that worked, and on the spawn-failure path, where nothing ran, it deletes with the macOS walk below. Why the mount rule: the risk is an accidental leftover mount from a root build or test tool, not an attack, since a root job could delete those files itself.
- macOS: the same walk with `openat` and `O_NOFOLLOW`; it runs only on the spawn-failure path, where nothing ran (D5).
- Windows: the same walk through handles, each entry opened relative to its parent's handle with `FILE_OPEN_REPARSE_POINT`, so a junction or symlink is opened as itself and deleted through its own handle ([SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle), a disposition), never traversed.
- Model: Rust 1.85.0's walk below the directory it opened ([unix/fs.rs](https://github.com/rust-lang/rust/blob/1.85.0/library/std/src/sys/pal/unix/fs.rs), lines 2176 to 2185, 2238 to 2250 and 2288; [windows/fs/remove_dir_all.rs](https://github.com/rust-lang/rust/blob/1.85.0/library/std/src/sys/pal/windows/fs/remove_dir_all.rs), lines 74 to 101 and 143 to 150). It does this, but it is private. The public `remove_dir_all` is not the entry point: it opens its root by the whole path, with `AT_FDCWD` (unix/fs.rs, lines 2179 and 2301), so `O_NOFOLLOW` guards only the last component, and a job that swaps `tmp` for a link to an outside folder holding a matching `<n>` would have that folder emptied. ([remove_dir_all](https://doc.rust-lang.org/1.85.0/std/fs/fn.remove_dir_all.html): "This function does not follow symbolic links and it will simply remove the symbolic link itself" holds below the root only.)
- Before deleting, the supervisor compares the directory that the path `jobs/<uuid>/tmp` now names with the held handle; when they differ, it deletes nothing, sets `temp_kept`, and leaves the folder for `qex clean`. It also compares `<n>`: when it is missing, or its identity differs from the one kept at its creation (A2), it deletes nothing and sets `temp_kept`. After a deletion, any entry left in `tmp` (listed through the handle) that is not an earlier attempt's kept folder also sets `temp_kept`. The comparisons decide only the report; what is deleted is bounded by the handle, so a swap after them reaches nothing outside.
- Rejected: an identity check followed by a delete by path, because it leaves a check-then-delete race; narrowing the promise to same-user code (docs/fork/SPEC.md:51), because that is false when the coordinator runs as root or elevated.
- `qex clean` and `qex gc` keep std's `remove_dir_all` on the job directory: inside the directory it opened, std unlinks a link without entering it, so a swapped `tmp` link is removed there, not followed.

### A4 Interactions

- Hooks: `on_stop` runs with the supervisor's environment plus its own variables (src/hook.rs:420, `variables` at src/hook.rs:654), never the job's, so the feature does not reach hooks.
- Nested submissions: a `qex submit` inside a job captures that job's environment (`[submit] env_capture`, src/config.rs:657). With the feature off for the inner job, it inherits a TEMP that goes away at the outer attempt's end; with it on, the inner job gets its own.
- A coordinator autostarted from inside a job inherits that job's TMPDIR, so its long-path socket fallback (`std::env::temp_dir()`, src/paths.rs:118) would land in a folder that goes away. That fallback reads the coordinator's own environment, never a job's.
- `qex du` counts no subfolder (`directory_size`, src/commands.rs:6907), so a live or kept temp folder is invisible to it (#101).
- `qex clean` refuses a job that has not finished (src/lifecycle.rs:192), so a live folder is never swept; the orphan sweep of `qex gc` touches only directories with no record.
- PORT_PLAN's "Acceptance check for the port" already walks the per-job TEMP folders for their ACL (PORT_PLAN.md:116, PORT_PLAN.md:132).

## Tests the implementing PR must show

Linux e2e through the CLI (root-only variants gated as tests/e2e.rs:3900 gates its root-only test, `if unsafe { libc::geteuid() } != 0 { return; }`; tests/e2e.rs:1745 is the opposite gate):

1. Two concurrent jobs with the feature on see different, empty TMPDIR, TMP and TEMP (printed through `qex logs`, as tests/e2e.rs:12564 does); each folder is gone after its attempt, and an `on_stop` hook that lists the job's `tmp/` finds it empty.
2. With the feature off, the environment is byte-equal to the submitted one; with it on and `--env TMPDIR=...` submitted, the value is replaced.
3. D1's precedence, each case read from the job's TMPDIR: with `[temp] private = true` in the config, `--no-private-temp` gives off; a job file (`--job`, as tests/e2e.rs:4253 does) with `private_temp = false` gives off under that config, and one with `private_temp = true` gives on under the default config; `--no-private-temp` with a job file holding `private_temp = true` gives off, and `--private-temp` with one holding `private_temp = false` gives on.
4. `--retries 1` with a failing first attempt gets folders `1` and `2`, each fresh. A program that does not exist (the spawn-failure path) leaves no folder.
5. A link or a file planted at `tmp` or `<attempt>` before launch fails the job before its program starts; a `tmp` planted with mode 0777 is 0700 when the job runs; as root, a `tmp` owned by another user fails the job before its program starts. Each is planted while the job waits behind another that holds the same `--lock`.
6. The swap test: the job renames `tmp` away, puts in its place a link to an outside folder that holds a `<n>` folder with a file, plants a symlink to a second outside folder inside `<attempt>`, and exits. Both outside folders keep their listings and file bytes, the job folder is kept, `temp_kept` is true and `qex status` prints the `tmp` entry as a link, reading nothing through it; after `qex clean` both outside folders are still unchanged. Root-only variant with root-owned targets.
7. The plant test: a symlink to an outside file and one to an outside directory inside `<attempt>` are gone with the folder, their targets unchanged, `temp_kept` false. Root-only variant.
8. The escaped-descendant test: the job starts a descendant that calls `setsid`, closes stdout and stderr, and writes into TEMP after the job's root exits. The test first asserts that the descendant is alive at the attempt's end (the comment at src/supervisor.rs:991 records a `setsid` holder that did not survive on the ubuntu-24.04 runner, so a run where it died must fail, not pass), then that the folder is kept, `temp_kept` is true, `qex status` names it, and `qex clean` removes it.
9. The supervisor killed with SIGKILL while its job runs (as tests/e2e.rs:4394 does) leaves the attempt folder, `temp_kept` true, the folder named by `qex status`, and an `on_stop` hook that sees `temp_kept` true; the same after the coordinator and the supervisor are both killed and the next command's coordinator recovers the job (as tests/e2e.rs:4445 does).
10. The bind-mount test, root-only: the job bind-mounts an outside folder holding a file inside `<attempt>` and exits. The outside folder and its file are unchanged, the attempt folder is kept and `temp_kept` is true.
11. The retry-swap test: with `--retries 1`, the failing first attempt replaces `tmp` with a link to an outside folder that holds a `2` folder. The second attempt fails before its program starts, `temp_kept` is true and the outside folder is unchanged.
12. The attempt-swap test: the job renames `tmp/<n>` to `tmp/saved`, makes an empty `tmp/<n>` in its place, and exits. `temp_kept` is true and `tmp/saved` is still there with its file. A second case leaves `tmp/<n>` in place and adds `tmp/extra`: `temp_kept` is true.

Windows backend: the swap and plant tests (with a junction too), and a descendant that tries `CREATE_BREAKAWAY_FROM_JOB` and keeps using TEMP after the root exits: the folder is kept and `temp_kept` true while it lives. If that backend ends every job process at the root's exit, the test asserts instead that `ActiveProcesses` read at least 1 after the root's exit and that the folder went only after it read 0. A private-temp job under a state root of 112 characters is refused before launch with a message that names the length, the bound 111 and the two fixes (a shorter state root, or the feature off); one under a root of 111 runs. The elevated variant is PORT_PLAN's "Acceptance check for the port", not evaluated until it runs.

macOS (the e2e suite on the macOS runner): a job that finished with no descendant left keeps its attempt folder, `temp_kept` is true and `qex status` names the folder (D5's macOS rule); there tests 1 and 7 expect the folder kept, not gone. The same holds on a Linux kernel without `openat2` (ENOSYS): tests 1 and 7 expect the folder kept and `temp_kept` true. On both, test 4's spawn-failure case still expects no folder.

## Evidence

One row per measurement this ADR uses, from 02's three runs. Failed and passed counts are the sums of the `test result:` lines of one worktree's log.

| Fact | Used by | Value | from |
|---|---|---|---|
| Shared TEMP, run 1, rep 1, worktree 1: failed tests | Context | 63 | from: docs/decision-gate/logs/parallel-workload-windows/rep1-P-w0.log |
| Shared TEMP, run 1, rep 2, worktree 1: failed tests | Context | 40 | from: docs/decision-gate/logs/parallel-workload-windows/rep2-P-w0.log |
| Shared TEMP, run 1, rep 1, worktree 1: lines with error code 32 (`os error 32` or `Os { code: 32`) | Context | 24 | from: docs/decision-gate/logs/parallel-workload-windows/rep1-P-w0.log |
| Per-job TEMP, rep 1: exit code of each workload | Context | 0 / 0 / 0 | from: docs/decision-gate/logs/parallel-workload-windows-temp/rep1-P-summary.txt |
| Per-job TEMP, rep 1, worktree 1: tests passed and failed | Context | 437 and 0 | from: docs/decision-gate/logs/parallel-workload-windows-temp/rep1-P-w0.log |
| Shared TEMP control, rep 1: exit code of each workload | Context | 101 / 101 / 101 | from: docs/decision-gate/logs/parallel-workload-windows-shared-temp-control/rep1-P-summary.txt |
| Shared TEMP control, rep 1, worktree 3: failed tests | Context | 37 | from: docs/decision-gate/logs/parallel-workload-windows-shared-temp-control/rep1-P-w2.log |

## Consequences

- SPEC is amended in three places (D7), and PORT_PLAN's row links this ADR and its acceptance check gains the elevated cleanup step.
- Linux and macOS behave as today unless the feature is turned on.
- The new flag pair, the `[temp]` table, the job-file key and `temp_kept` are CLI and persistence changes decided here.
- The implementing PR is a feature PR, not an R-PR. It builds on #98, which merged. It is wider than first planned: new descriptor-relative create and delete code, the Linux subreaper with its collector, the macOS keep rule, and the coordinator's `temp_kept` checks. It is sized in its own plan. The Windows Job Object count and handle-relative delete go with the Windows backend after R7.

## Known limits

- One test harness (ripgrep's), three repetitions per side.
- Tools that ignore TMPDIR, TMP and TEMP, or use fixed paths, still collide; `--lock` is their answer (D6).
- tmpfs both ways: a Linux state root is usually on disk while `/tmp` is often tmpfs, so temp I/O can get slower with the feature on; a state root on tmpfs puts temp bytes in RAM that admission does not count.
- The job directory and the state root above `tmp` are path-resolved, as #88 decided; a link the user owns there is followed.
- macOS keeps every attempt folder after a spawn that worked, because its group check cannot rule escapees out (docs/fork/QEX_ASSESSMENT.md:80); folders gather there until `qex clean`.
- On Linux and Windows, work started outside the tree or the Job Object is not seen (a service manager, a container engine: docs/fork/SPEC.md:181; WMI: docs/decision-gate/01-jobobject-probe.md:32), and a child left from one attempt keeps the next attempt's folder too.
- The listing gap: a kept folder named `attempts` is not listed until the next attempt's Running write (src/supervisor.rs:649), though `temp_kept` is already true.
- Nested submissions with the feature off inherit a TEMP that goes away (owner's answer: a known limit only).
- A coordinator autostarted from inside a job puts its fallback socket folder in that job's TEMP.
- `qex du` does not count the folders (#101).
- The elevated Windows cleanup variant is not evaluated until PORT_PLAN's acceptance check runs.
- The Windows handle-relative delete, the junction cases and the `ActiveProcesses` count rest on Microsoft's pages, Rust's source and the Job Object probe until the Windows backend's tests run; whether Rust's Windows walk treats a junction root correctly was not traced.
- The Linux subreaper rule is not run in this PR; the implementing PR's escaped-descendant test is its evidence.
- A swap of `tmp` or `<attempt>` after the pre-launch check (A2) can only come from the job's own processes and is not detected then; cleanup stays bounded by the handle either way.
- On a Linux kernel before 5.6 (no `openat2`), every attempt folder is kept after a spawn that worked, as on macOS.

## Left to the implementing PRs

- The reuse of #98's helpers and the new descriptor-relative create, check and delete code (A2, A3).
- The subreaper, its collector and the no-children bound (D5); the coordinator's `temp_kept` check on its three Failed paths.
- The tests above; help text (code).
- The Windows Job Object count, handle-relative delete and acceptance step, with the Windows backend.
