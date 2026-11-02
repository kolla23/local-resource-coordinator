# ADR 1: IPC transport and launch protocol

**Date:** 2026-11-02
**Status:** proposed; accepted when the owner merges the PR that adds this file
**Context:** [PORT_PLAN.md](../PORT_PLAN.md), "ADRs" row 1, which unblocks R7; evidence in [06-named-pipe-probe.md](../../decision-gate/06-named-pipe-probe.md) (Windows facts, this PC) and [07-ipc-message-sizes.md](../../decision-gate/07-ipc-message-sizes.md) (message sizes, Linux under WSL2). Source citations are at commit `0407a8e`.

## Context

Today the coordinator and the CLI talk over a Unix socket at `<XDG_STATE_HOME>/qex/run/s`, with a 100-byte path ceiling and a TMPDIR fallback (src/paths.rs:75, :88). A client's connect has three outcomes, `Open`, `NobodyListens` and `NoAnswer`, and only `NobodyListens` may start a coordinator (src/paths.rs:812-820). Two `flock` locks guard the launch: `spawn.lock` (src/paths.rs:987-988, src/client.rs:1310-1323) and the lock on the pid file, which `hold_pid_file` takes before it truncates the file and writes its own pid (src/paths.rs:479-566). The coordinator reads the peer's pid from the socket (`SO_PEERCRED`, src/sys.rs:100-111; src/daemon.rs:1813) and uses it only for attribution. Messages are newline-delimited JSON, one object per line, tagged by `op` (src/proto.rs:39-40), read with no bound on either side (`reader.lines()`, src/daemon.rs:1824; `read_line`, src/client.rs:788). Each connection gets its own thread (src/daemon.rs:1324).

Windows has no such socket path in this design, mandatory byte-range locks and no `SO_PEERCRED`. SPEC asks for "length-prefixed, versioned messages, a 1 MiB request limit" (docs/fork/SPEC.md:619) and a 4-byte length prefix with "Reject frames above 1 MiB before allocation" (docs/fork/SPEC.md:970). The measurement shows that the current CLI already sends and gets lines far past 1 MiB (07).

The owner settled D1-D8 and three additions on 2026-10-31. This ADR records them with the facts they rest on and the limits that D5 needs.

Numbers in prose are checked by review against the logs, not by a command.

## Decisions

### D1 Transport on Windows: named pipes, byte mode

The coordinator listens on a named pipe in byte mode (`PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`), the parameter set the probe used for every create (06, Setup). Linux and macOS keep the Unix socket. AF_UNIX on Windows is rejected: D2's name ownership and D7's descriptor rest on pipe calls that the probe showed on this PC (06), and a Windows AF_UNIX socket would need a socket file, an ACL and evidence of its own.

### D2 Pipe name and who owns it

- **Name:** `\\.\pipe\<product>-<user SID>-<hash>`, where `<user SID>` is the string SID of the coordinator's TokenUser and `<hash>` is the first 32 hex characters of SHA-256 over the canonical full path of the state root, upper-cased first, as Windows paths are case-insensitive. The product part is at most 24 characters, so the name stays inside the 256 characters that [pipe names](https://learn.microsoft.com/en-us/windows/win32/ipc/pipe-names) allow ("The entire pipe name string can be up to 256 characters long", retrieved 2026-11-02) even for the longest SID string. A random name published in a file is rejected: the file would be one more object to protect and a second source of truth beside the pid lock.
- **First instance:** the server creates its first instance with `FILE_FLAG_FIRST_PIPE_INSTANCE` and `PIPE_REJECT_REMOTE_CLIENTS`, with D7's descriptor ([CreateNamedPipeW](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew)). The flag refuses a name that another same-user process created first with 5, and the same call without the flag succeeds, so the flag closes the squat (P1a, P1b). A squatter at its instance limit makes the flagged create fail with 231, not 5 (P1d). So **every** failure of the first create means "name taken": the coordinator exits with a message that names the pipe, and never serves on a name it did not create. Later instances carry the same descriptor and `PIPE_REJECT_REMOTE_CLIENTS`, without the flag (the flag fails every instance after the first, per the CreateNamedPipeW page).
- **Client check before sending:** the client opens the pipe, reads the server's pid with `GetNamedPipeServerProcessId`, opens its token (`OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION`, `OpenProcessToken`, `GetTokenInformation(TokenUser)`) and compares the SID with its own TokenUser; it also reads the pipe's owner with `GetSecurityInfo` on its handle and requires the same SID. A same-user client gets all three (P2a, P2b). A non-elevated client identifies an elevated server that uses D7's descriptor (P2e, `-d7-`), and an elevated client identifies a non-elevated server (P2f). Any failed call or mismatch means refuse: the client sends nothing and never starts a coordinator (D4).
- **Relaunch:** a dead server's pipe does not block the next one. A killed server with no client leaves no name behind (P4e); a client that still holds a handle to the dead instance does not stop a new flagged create on this build (P4f), and that client gets 109 on read and 232 on write, which the CLI reports as "the coordinator stopped", as it reports a broken socket today.
- **Not evaluated:** a squatter that runs as another user (P1c, owner step O2 not run). PORT_PLAN's "Acceptance check for the port", step 4, tests it.

### D3 Launch guard: the same two locks, with `LockFileEx`

The Windows launch keeps Unix's order and files: `spawn.lock`, then the pid-file lock, both in the state folder, which carries the protected DACL of PORT_PLAN's acceptance check. A named mutex is rejected: it lives outside the protected folder, and the pid lock already answers "is a coordinator alive".

- **The call:** R7 calls `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)` on one byte at offset `0xFFFFFFFFFFFFFFF0`, past the end of the file, on a handle opened with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`. Both files use the same range. The retry loops and their patience stay (`PID_LOCK_PATIENCE`, src/paths.rs:387).
- **Production's order:** lock, then truncate and write the pid through the same handle (src/paths.rs:532-564). On this PC the holder's truncate and write succeed under its own lock, and another process reads the new pid with no leftover digit (P6a). A second exclusive try fails at once (P6c), and the lock goes when the holder is killed (P6d).
- **The `st_nlink == 0` test** (src/paths.rs:555-556; a sweep deleted the file after this process opened it) becomes: after the lock, `GetFileInformationByHandleEx(FileStandardInfo)`; `DeletePending` true or `NumberOfLinks` 0 means "deleted, open the path again". With `FILE_SHARE_DELETE`, a delete under the lock succeeds, the holder then sees `DeletePending` 1 and `NumberOfLinks` 0, and `CREATE_NEW` on the name succeeds at once (P6e, share=all). Without it the delete fails with 32 (P6e, share=nodelete), and the sweep (src/paths.rs:263, :292) could never remove a stale folder, so the share mode is part of the decision.
- A1 explains why the range sits past the end of the file.

### D4 Liveness: connect errors map onto the three outcomes

The Windows `connect` returns the existing `Connected` outcomes (src/paths.rs:812-820):

- the open succeeds and D2's check passes: `Open` (P2a, P2b);
- `CreateFileW` fails with 2, no pipe or a killed server: `NobodyListens`, which may start a coordinator (P4a, P4e);
- 231, all instances busy or the gap between instances: wait with `WaitNamedPipeW` inside the existing ceiling, then `NoAnswer` (P4b, P4c);
- 5, a failed identity call, or a server of another user: refuse with a message that names the pipe, and never start (P1a, P2e);
- any other code: `NoAnswer`, never start.

`WaitNamedPipeW` returns at once when no instance exists, whatever its timeout (P4a), so the client does not wait on a missing pipe; when all instances are busy it fails with 121 after its timeout (P4b). The server creates the next instance before it hands the connected one to its thread, which narrows the gap where a client sees 231 (P4c); a client that connects before `ConnectNamedPipe` is served, and the server takes 535 as success (P4d). `NobodyListens` only lets a client try to start: it takes `spawn.lock`, then the pid lock decides (D3), as on Unix.

### D5 Framing and message limits

- **Framing:** newline-delimited JSON on every platform, one UTF-8 JSON object per line, as today. serde_json escapes every control character inside a string, so a line holds no raw newline. The 4-byte length prefix of SPEC §27.1 is not adopted: it would change the Unix wire format that every installed CLI and coordinator speak, and a bounded line reader gives the same guarantee.
- **Bounded readers on both sides:** a reader reads at most the limit plus one byte while it looks for the newline, and never allocates past that. A request line past the limit gets a `TooLarge` error that does not echo the request, and the coordinator closes that connection; a reply line past the limit makes the CLI stop reading and report "the coordinator's reply is larger than <limit>". A partial line at end of file closes only that connection and launches nothing (SPEC §27.1 keeps that rule).
- **Request limit: 16 MiB (16,777,216 bytes) per line.** A2's method: the OS ceiling × the worst escape factor + the envelope. On Linux all argv and env strings fit in `ARG_MAX` = 2,097,152 bytes; a control byte goes over as 6 bytes, so 12,582,912 bytes, plus a submitter chain of at most 64 ancestors (src/os/unix.rs:303-304), each with a cwd of at most `PATH_MAX` = 4,096 bytes, escaped: about 1.6 MB (derived), plus about 1.4 KB of envelope (the 1,361-byte empty `--env` Submit): about 14.2 MB. On Windows the command line is at most 32,767 characters including the NUL ([CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw), "The maximum length of this string is 32,767 characters, including the Unicode terminating null character", retrieved 2026-11-02): at most 196,596 bytes when every unit is an escaped control character. The OS sets no cap on the environment block ([Environment Variables](https://learn.microsoft.com/en-us/windows/win32/procthread/environment-variables): "The maximum size of a user-defined environment variable is 32,767 characters. There is no technical limitation on the size of the environment block.", retrieved 2026-11-02), so **the Windows CLI caps the environment it captures at 2,097,152 bytes of UTF-8**, the same as Linux's `ARG_MAX`, and refuses a larger submit with a message that names the size and the `--env` and env-capture settings. Both platforms then fit in 16 MiB with room. SPEC's 1 MiB would refuse submits that work today: the measured exec-bounded Submit is 1,966,523 bytes, and `qex submit --job` with one 2 MiB argument sends 2,098,633 bytes (07).
- **Reply limit: 64 MiB (67,108,864 bytes) per line**, and the same per line of the event stream. One record must fit in a Status reply and an Event line: a record holds at most what its Submit carried (16 MiB, less the environment, which the record leaves out) plus about 1.7 KB and the 4,096-byte `blocked_reason` bound below (derived from the record-shape rows, where each shape adds its bytes once per record). 64 MiB holds about 38,850 records of the smallest measured size (b = 1,727 bytes) in one Jobs or Found line.
- **Requests the exec ceilings do not bound** (`qex submit --job FILE`, src/cli.rs:265-267; the `--needs` and `--after` id lists, src/cli.rs:255-263; Abort's `scope.cwd`, made with `to_string_lossy`, src/commands.rs:3047, its `scope.submitter` and `scope.tags`; Resume's `pause_id`, any string today, src/commands.rs:5184, :5193) are bounded by the request limit itself. The CLI measures the serialised request before it connects and refuses one past the limit with the same `TooLarge` text; the coordinator enforces it again.
- **Field bounds**, each checked by the CLI before sending and by the coordinator on receipt, each refused with `TooLarge` and a sentence that names the bound and the length, never the value:
  - a handle (an id, the start of an id, a name, a group id or a group name) is at most 256 bytes; Resume's `pause_id` at most 64 bytes;
  - `--reason` is at most 4,096 bytes of UTF-8 (src/cli.rs:811-812, :830 and :833-834 take any text today);
  - a lock name in a Pause request is at most 256 bytes; a Resume that names a lock takes any name up to the request limit, because a lock is matched by its exact stored name or its safe form (`lock_matches`, src/pause.rs:1044-1045), and a lock stored before this bound may have a longer one;
  - each ancestor's name and cwd in a submitter or issuer chain is cut to 4,096 bytes, with a mark that shows the cut (a Linux cwd never passes `PATH_MAX`; a Windows path can reach 32,767 characters);
  - at most 1,000 standing pause requests in all (queue and locks); `add_queue` and `add_lock` push with no cap today (src/pause.rs:273-292).
- **List bounds** inside replies:
  - an ambiguous handle's error lists at most the first 20 matches, then "and <n> more; `qex list --name <name>` lists them" (`Resolution.error`, `StopOutcome.error`; src/resolve.rs:236-248);
  - a Clean refusal names at most the first 20 holders, then "and <n> more" (src/lifecycle.rs:213-240);
  - the CLI sends Resolve in parts of at most 500 handles, as `qex cancel` and `qex kill` already do (`STOP_MANY_PART`, src/commands.rs:3317, :3327), instead of one Resolve with every handle (src/commands.rs:3516, :3991).
- **Past the reply limit**, two classes:
  - (a) record lists, which a person narrows: Jobs (List), Found (Query) and Resolved (Resolve) are refused with `TooLarge` before anything is sent. The text names the remedy: "delete old records with `qex gc --older-than <age>`, or ask for fewer with `qex list --limit <n>`". A Resolve part that is too large is halved and sent again by the CLI, as Resolve changes nothing; a single handle past the limit is refused.
  - (b) every other reply (Status, an Event line, Info, PauseState, Stopped, Aborted, Submitted, an Error) is bounded by the field and list bounds above. If one would still pass the limit, the coordinator keeps any change it made, cuts each list to its first entries, and adds the count of the entries it left out, in a new optional field that an older CLI ignores (the protocol never uses `deny_unknown_fields`, src/proto.rs:27). A change is never refused after it was made.
- **The remedies still answer.** `qex gc` (src/commands.rs:6761) and `qex clean` with a selecting option (src/commands.rs:3542) need the List reply, which class (a) can refuse once records pile up, as qex removes no record by itself. Under a new capability `query-page`, they read short rows instead (Query with `full: false`), a page of at most 1,000 rows at a time, through two new optional filter fields: `before`, a cursor, and `stopped_before`, an age. The cursor is a whole sort key, `(submitted_at, sequence, id)`, the key Query already orders by (src/daemon.rs:2381-2382); which row of a page sets it and the order a page comes back in are left to R7c's design (Known limits). The id makes the key unique: `sequence` alone is not, as a record is first written with `sequence` 0 (src/job.rs:685, src/daemon.rs:2876-2878) and gets its number only afterwards (:2891-2892), so many records can share a value. A record that an unfinished job holds is skipped as today (`held_by_unfinished`, src/commands.rs:6765); if one slips through, the coordinator's Clean refusal holds it, and `qex gc` counts it as held back. With the pause bounds, `qex resume --all` and `qex resume --pause <id>`, whose PauseState reply carries `resumed` (src/proto.rs:607), stay under the limit; a pause state that an older coordinator stored past the bounds is kept whole, new pause requests are refused while it is past them, the resume forms (`qex resume --all`, `qex resume --pause <id>` and `qex resume lock <name>` with or without `--all`) are always accepted, and class (b) cuts their reply if needed.
- **History is not an IPC message:** `grep -n -i history src/proto.rs` prints nothing (exit 1); history's text reaches the wire only inside the NoSuchJob error sentence, so no history limit is needed.
- **Error kind:** `TooLarge` is a new `ErrorKind` (src/proto.rs:678-685). An older CLI cannot read that kind and reports an unreadable reply, which is still a refusal: every `TooLarge` is sent before any change.
- **Memory:** a connection's thread holds at most one request line (16 MiB) and serialises a reply into a buffer that fails at 64 MiB, so a connection's message memory is bounded; the number of connections is not, as today (src/daemon.rs:1324).

### D6 Versioning: Capabilities stays

No new handshake. The `Capabilities` request and `CAPABILITY_FLOOR` (src/capabilities.rs:121) stay, and a CLI keeps asking once per connection (`Client::can`, src/client.rs:483-485). This ADR adds the capability words `limits` (the coordinator enforces D5 and sends `TooLarge`) and `query-page` (D5's remedy path). A new CLI that talks to a coordinator without `query-page` keeps today's List path; its own reader is bounded either way.

### D7 Access and attribution

- **Descriptor:** every pipe instance gets `O:<user SID>D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)(A;;GA;;;BA)`: owner the user, even when elevated; a protected DACL (`P`, which sets `SE_DACL_PROTECTED`, [security-descriptor-control](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-control): "Prevents the DACL of the security descriptor from being modified by inheritable ACEs", retrieved 2026-11-02); full access for the user, `SY` (local system) and `BA` (built-in administrators) ([SID strings](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings), [security descriptor string format](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format)); no Everyone entry. This is PORT_PLAN's owner decision 2 (docs/fork/PORT_PLAN.md:9, :119-123).
- **Why explicit:** the default descriptor grants "read access to members of the Everyone group and the anonymous account" ([named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights), retrieved 2026-11-02), and an elevated creator makes Administrators the owner (P2c, default descriptor). With it, a non-elevated client's read+write open is refused, and a read-only open succeeds (P2e, `-ro-`). With this descriptor the owner is the user (P2c, explicit descriptor) and the non-elevated client opens read+write (P2e, `-d7-`).
- **Attribution:** `GetNamedPipeClientProcessId` replaces `peer_pid` (src/sys.rs:100) for attribution only, in the same places (Info, Pause, Resume, PauseState, the issuer chain and Abort: src/daemon.rs:1885-1996, `issuer_chain` at :2041); it gives the client's pid, elevated or not (P2d, P2f). There is no client-SID check: the DACL already admits only the user, SYSTEM and Administrators.
- **Known gaps, left to ADR 2's supervisor channel:** the `submitter` chain is still sent by the client and trusted, and there is no leader epoch (docs/fork/SPEC.md:623).
- **Verified by:** the DACL read-back, with the coordinator not elevated and elevated, is PORT_PLAN's "Acceptance check for the port", steps 2-3. The probe does not read the DACL back (owner decision 2026-10-31; 06, "Facts moved out of the probe").

### D8 Binding: `windows-sys` directly, behind `os::ipc`

R7 adds `os::ipc::{Listener, Stream, connect, peer_pid}` (PORT_PLAN R7) and implements the Windows side with `windows-sys` calls. Every call D2-D4 and D7 need ran in the probe through `windows-sys` 0.61.2 (06, Setup); the overlapped calls of A3 are in the same crate. The `interprocess` crate is rejected: the decisions need the creation flags, the explicit descriptor and the identity calls one by one, and a wrapper adds a dependency between them and the code that R7's tests check.

### A1 Mandatory locks and the pid file

Windows byte-range locks are mandatory: a lock over the pid number blocks every reader, and `ReadFile` fails with 33 (P6b). A lock on a byte past the end of the file leaves the number readable (P6a), and [LockFileEx](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex) allows it: "Locking a region that goes beyond the current end-of-file position is not an error" (06). So D3 locks past the end. The fallback, if a later Windows build behaves otherwise, is a separate `pid.lock` file beside `pid`, at the cost of keeping two files consistent through the sweep.

Does anything read the pid file's content today? The search, rerun at `0407a8e`: `grep -rn 'PID_FILE\|join("pid")\|/pid"' src tests` prints 29 lines. One verdict per site:

- Production, src/paths.rs: :142 creates the file if missing (no read); :171 is the constant; :306 (`claim_unused_dir`, the sweep) opens it read+write to take the lock; :488 (`hold_pid_file`) locks, tests `nlink` (:555-556), truncates and writes (:562), and never reads.
- Test-only, src/paths.rs: :599 (`pid_file_state`, `#[cfg(test)]` at :589) only tries the lock; in `mod tests` (from :1012): :1163 (exists), :1333 (`a_held_pid_file` writes), :1462, :1706, :1830, :1864 (write), :1730 (writes, then reads the content back at :1741), :1762, :1772 (write, remove), :1906 (write), :2000 and :2016 (mode only).
- End-to-end, tests/e2e.rs: :3476 (a mode list), :3525 and :4000 (mode), :3962 (reads the content, inside `tmpdir_socket_dir`), :17146 (opens and tries `flock`, no read), :17268 (writes empty, mode 000), :17318 and :18947 (remove), :17356 and :17396 (`a_held_pid_file` at :17049, which writes after `flock`), :18976 (exists).
- Not the pid file: src/os/unix.rs:275 (`/proc/self/ns/pid`).

The hits inside functions that are not `#[test]` functions, followed to their callers (`grep -rnw <name> src tests`): `short_socket_dir` (:91), `claim_unused_dir` (:263 and three unit tests), `hold_pid_file` (src/daemon.rs:1189 and four unit tests), `pid_file_state` and `a_held_pid_file` (unit tests only) read no content; `tmpdir_socket_dir` (tests/e2e.rs:3950-3963) does, and its callers are :3995 (`the_socket_directory_in_tmpdir_is_private`) and :7729 (`a_long_runtime_directory_still_works`). So production never reads the pid number; its readers are the unit test at src/paths.rs:1741 and `tmpdir_socket_dir`, which serves the Unix-only TMPDIR fallback. A past-EOF lock keeps both working on Unix and keeps a Windows reader possible. Production does read another pid file, `supervisor.pid` (src/supervisor.rs:96); D3 puts no lock on it, and a future lock there needs the same past-EOF rule (ADR 2).

### A2 Limits measured before they were fixed

D5's numbers come from 07: the real CLI and coordinator, three reps, a lengths-only proxy (07, Setup). How each limit is sized, per message, on the largest message of its kind that the current CLI sends:

- **Jobs (List)** carries every record kept, live and stopped (sent by `qex gc` and by `qex clean` with a selecting option): a + R × record. 1,729,557 bytes at 1000 smallest records, 13,293,734 at 100 records with a 131,071-byte argv string. qex keeps records until `qex gc` or `qex clean` deletes them, so R has no bound, and D5 refuses it with class (a) and pages the remedy.
- **Found (Query):** the same shape, 1,729,593 bytes at 1000 records (`qex list --all --json`).
- **Resolved:** a + h × per-handle entry + n × per-job entry, where one Resolve carries every handle today and a group handle names every job of its pipeline (src/proto.rs:304-306, 342-344): 151,232 bytes for 900 ids, 84,044 for one group handle of 1000 jobs.
- **CancelMany/KillMany** go in parts of 500 handles: request lines of up to 19,557 bytes; each part's Stopped reply is a + 500 × per-outcome entry + n × per-line entry (about 201 bytes per outcome for cancel and 301 for kill, where a refused line carries its sentence).
- **Error lists:** a handle that m jobs share comes back with every match, about 169 bytes each, once per handle, so one reply holds h × m match lines: 84,711,531 bytes for 500 copies of a name that 1000 jobs share (Resolved), 84,712,033 (Stopped). These are the largest lines measured, and the input is ordinary: 1000 jobs with one name and a 500-handle cancel. With D5's bounds (20 matches, 256-byte handles, parts of 500) the same request gives about 1.84 MB (derived: 500 × (20 × 169 + about 300)).
- **Clean refusal:** a + H × holder, about 141 bytes per holder: 141,005 bytes at 1000 jobs. D5's 20 holders bound it at about 2.8 KB plus the sentence.
- **Pause collections** (Info's `pauses`, `paused_reason` and `paused_locks`; `PauseState.queue`, `PauseState.locks`, `PauseState.pauses`, `PauseState.resumed`; `Aborted.pauses`; the bare-resume Error, src/daemon.rs:2239-2261) grow with the standing requests and their reasons: Info is 26,282,798 bytes at 100 locks with 131,062-byte reasons. Each reason byte costs P + 1 bytes for P queue requests and 2 × P for P locks, as the COEF checks found (11.000 and 20.000 at P = 10). About 511 bytes per queue request and 702 per lock with its record, without the reason (6-ancestor chains). With D5's bounds (1,000 requests, 4,096-byte reasons, 256-byte lock names) the largest is about 1,000 × (702 + 2 × 4,096 + 256) ≈ 9.2 MB with such chains (derived); longer chains add bytes per view, and class (b) cuts the lists if they pass 64 MiB.
- **The `blocked_reason` cross-term:** while one queue request stands, its reason is copied into every queued job's `blocked_reason` (src/pause.rs:1119-1124, src/sched.rs:1650-1653): a 100-job List reply is 11,836,929 bytes with one 131,062-byte reason, 89.0 bytes per reason byte. D5's 4,096-byte reason caps the term at 4,096 bytes per queued record before escaping.
- **The event stream:** each Event line carries one whole record plus about 350 bytes (1,947 bytes for the smallest record); a `--since start` replay sends at most `RETAINED = 512` events (src/events.rs:87), and the stream has no total size, as it lasts as long as its connection.
- **Submit and the exec ceilings:** 1,966,523 bytes for 15 argv strings of 131,000 bytes (the 16th fails with E2BIG in the CLI's own exec); escape factors 6 for a control byte, 2 for `"` and 1 for UTF-8 text; about 11.8 MB when the same strings are control bytes (derived). `qex submit --job` with one 2 MiB argument: 2,098,633 bytes, accepted by the coordinator.
- **Windows** is analytic, as no Windows CLI exists yet: up to 3 UTF-8 bytes per UTF-16 unit, 6 bytes per escaped control character, and the ceilings quoted in D5.

### A3 Windows facts from the probe, and the facts that rest on docs

The probe (06) shows, on this PC, the facts D2, D3, D4 and D7's attribution use: the evidence table below has one row each. Three facts are not probed (owner decision 2026-10-31). Each rests on Microsoft's docs until the check named beside it runs:

- **The protected DACL for the user, SYSTEM and Administrators** (D7): [security descriptor string format](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format), [SID strings](https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings), [security-descriptor-control](https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-control), [named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights). Verified by PORT_PLAN's "Acceptance check for the port", steps 2-3, the elevated run included.
- **Remote clients refused** with `PIPE_REJECT_REMOTE_CLIENTS` (D2): [CreateNamedPipeW](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew), "Connections from remote clients are automatically rejected." (retrieved 2026-11-02), and [pipe names](https://learn.microsoft.com/en-us/windows/win32/ipc/pipe-names) for the `\\<server>\pipe\` form a remote client uses. Verified by PORT_PLAN's "Acceptance check for the port", step 5 (the SMB loopback path `\\127.0.0.1\pipe\<name>`).
- **Read and write timeouts and backpressure** (SPEC §15): a synchronous pipe handle has no timeout, so R7 opens every pipe handle with `FILE_FLAG_OVERLAPPED`, starts each `ReadFile` and `WriteFile` with an `OVERLAPPED` ([ReadFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-readfile), [WriteFile](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-writefile), [synchronous and asynchronous I/O](https://learn.microsoft.com/en-us/windows/win32/fileio/synchronous-and-asynchronous-i-o)), waits with `GetOverlappedResultEx` and its `dwMilliseconds` ([GetOverlappedResultEx](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresultex)), and on a timeout cancels with `CancelIoEx` ([CancelIoEx](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelioex)). That replaces today's socket read timeouts (src/client.rs:692, src/commands.rs:2016, :2259, :4162) and the event stream's write timeout (src/events.rs:401), which is its backpressure. Verified by R7's own tests.

## Evidence

One row per probe or measurement fact this ADR uses. Rows from 06 cite run 1 or the owner's elevated runs (O1); rows from 07 cite its summary, environment and run logs.

| Fact | Used by | Value | from |
|---|---|---|---|
| P1a: the flagged create on a name another same-user process created | D2 | err 5 | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P1b: the same create without the flag | D2 | ok | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P1d: the flagged create while a squatter sits at its instance limit | D2 | err 231 | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P2a: the server's pid and user from a client handle | D2 | pid matches; same user SID | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P2b: the pipe owner through the client handle, not elevated | D2 | owner = the user | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P2c: the pipe owner, elevated server, default descriptor | D7 | owner S-1-5-32-544 | from: docs/decision-gate/logs/named-pipe-probe-o1-i.log |
| P2c: the pipe owner, elevated server, explicit `O:<user>` descriptor | D7 | owner = the user | from: docs/decision-gate/logs/named-pipe-probe-o1-i.log |
| P2d: the client's pid on the server side | D7 | matches | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P2e: non-elevated client, elevated server, D7's descriptor | D2, D7 | read+write open ok; same user; owner the user | from: docs/decision-gate/logs/named-pipe-probe-o1-ii-d7-client.log |
| P2e: non-elevated client, elevated server, default descriptor | D7 | read+write err 5; read-only open ok | from: docs/decision-gate/logs/named-pipe-probe-o1-ii-ro-client.log |
| P2f: elevated client, non-elevated server | D2 | open ok; same user | from: docs/decision-gate/logs/named-pipe-probe-o1-iii-client.log |
| P2f: the server's view of an elevated client | D7 | client pid read | from: docs/decision-gate/logs/named-pipe-probe-o1-iii-server.log |
| P4a: no pipe | D4 | open err 2; wait returns at once | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P4b: all instances busy | D4 | open err 231; wait err 121 after 500 ms | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P4c: the gap between instances | D4 | open err 231, then connects at once | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P4d: client connects before ConnectNamedPipe | D4 | server err 535; client served | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P4e: server killed, no client | D2, D4 | open err 2; relaunch create ok | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P4f: server killed while a client holds a handle | D2 | relaunch create ok; old client read 109, write 232 | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6a: lock past EOF, then truncate and write; another process reads | D3, A1 | reads the holder's pid | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6b: lock over the content; another process reads | A1 | read err 33 | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6c: a second exclusive try | D3 | err 33 at once | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6d: the holder is killed | D3 | the lock is free at once | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6e: delete under the lock, holder shares delete | D3 | delete ok; DeletePending 1; NumberOfLinks 0; CREATE_NEW ok | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| P6e: delete under the lock, holder does not share delete | D3 | delete err 32; CREATE_NEW err 80 | from: docs/decision-gate/logs/named-pipe-probe-windows-1.log |
| Linux ceilings: `ARG_MAX`, `PATH_MAX` | D5 | 2,097,152; 4,096 | from: docs/decision-gate/logs/ipc-sizes-wsl/environment.log |
| Largest exec-bounded Submit (15 argv strings of 131,000 bytes) | D5, A2 | 1,966,523 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| Escape factors: control byte, `"`, UTF-8 | D5, A2 | 6.000, 2.000, 1.000 | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| `qex submit --job` with one 2 MiB argument | D5, A2 | 2,098,633 bytes, accepted | from: docs/decision-gate/logs/ipc-sizes-wsl/run-1.log |
| Jobs (List) and Found at 1000 smallest records | D5, A2 | 1,729,557 and 1,729,593 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Bytes per smallest record; per record with a 131,071-byte argv | D5, A2 | 1,727; 132,937 | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Resolved for 900 ids; for one group handle of 1000 jobs | A2 | 151,232; 84,044 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| CancelMany request part of 500 handles | A2 | up to 19,557 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Error lists: 500 copies of a name 1000 jobs share | D5, A2 | Resolved 84,711,531; Stopped 84,712,033 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Clean refusal at 1000 jobs | D5, A2 | 141,005 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Info, 100 locks with 131,062-byte reasons | D5, A2 | 26,282,798 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| List with one 131,062-byte queue reason in every queued `blocked_reason` | D5, A2 | 11,836,929 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |
| Event line of the smallest record | A2 | 1,947 bytes | from: docs/decision-gate/logs/ipc-sizes-wsl/summary.log |

## Consequences

- **SPEC is amended** in two places, each with a pointer line to this ADR: §15's "length-prefixed messages, a 1 MiB request limit" (docs/fork/SPEC.md:619) becomes newline-delimited JSON with a 16 MiB request limit and a 64 MiB reply limit, and §27.1's 4-byte length prefix and 1 MiB frame (docs/fork/SPEC.md:970) become the bounded line readers of D5. The rest of §27.1 stands.
- **R7 splits.** The transport and launch code is about 2,400 lines (about 1,400 code lines, by the inventory of 2026-10-31), past one PR. Planned at its own planning time, likely as: R7a, `os::ipc` and the Unix side as a pure move; R7b, the bounded readers, `TooLarge` and the `limits` capability on every platform; R7c, the message-shape changes (Resolve parts, list bounds, pause bounds, `query-page` and the remedy path), which change Linux behaviour and get e2e tests through the CLI; the Windows side follows with the Windows backend.
- **Linux changes too:** D5's limits, field bounds and list bounds apply on every platform. Lines that work today and pass a bound (a reason past 4,096 bytes, a 1000-match error list) change; each such change is named in its R-PR.
- **CLI compatibility of the path:** Windows has no socket path, so the 100-byte ceiling and the TMPDIR fallback (src/paths.rs:75-92; tests/e2e.rs:3950-3963) have no Windows counterpart, and a message that names the socket path names the pipe instead. Recorded here, implemented in R7.
- **Gaps left to ADR 2:** a trusted submitter chain, the leader epoch, and who may signal a job; any lock on `supervisor.pid`.

## Known limits

- One Windows PC (Windows 11 Home 10.0.26200.9711) and one Linux (WSL2) machine. The cross-user squatter (P1c) is not evaluated. The elevated rows come from one O1 run each.
- The DACL, remote-client refusal and timeout facts rest on Microsoft's docs until PORT_PLAN's acceptance check and R7's tests run.
- The `query-page` cursor is fixed only as a unique key, `(submitted_at, sequence, id)`. Which row of a page sets the cursor, and the page order, are R7c's design, and its e2e tests must show that the pages neither overlap nor skip a record.
- The Windows message sizes are analytic. The 64-ancestor chain and the bounded sizes in A2 are derived from the measured rows, not measured.
- The `from:` cells show that each row names a log, not that its numbers are in it. Whether each section points to the right row, and every number in prose, are checked by review against the logs. Numbers in prose are checked by review against the logs, not by a command.
