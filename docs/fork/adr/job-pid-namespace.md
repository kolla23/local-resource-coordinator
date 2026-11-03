# Job pid namespace ADR: the job record names the pid namespace of its pids

**Date:** 2026-11-03
**Status:** proposed; accepted when the owner merges the PR that adds this file (PR #141)
**Context:** issue #115 and its fix in PR #141; the owner's decisions on that PR's review round 1, finding 2, and round 2, finding 3 (2026-11-03). Source citations are at commit `d726884` unless they say otherwise. This ADR takes no number, as the per-job TEMP ADR takes none: the numbers of PORT_PLAN.md's "ADRs" table already name other ADRs.

## Context

Once the system has collected a job's leader, the fix of #115 lets recovery SIGKILL group N when pid N is free, group N exists, and every live member belongs to this user and started no earlier than the recorded leader (`kill_what_the_job_left`, src/supervisor.rs:340). The proof that group N is the job's is a kernel rule: neither Linux nor macOS gives number N to a new process while group N exists.

That rule holds within one pid namespace. A coordinator inside a pid-namespaced sandbox or container can share the state directory with the host (docs/sandbox.md, "Give the sandbox the state directory and the socket", asks for one `XDG_STATE_HOME` inside and outside). The job record it writes holds pids of its own namespace. A later coordinator in another namespace that recovers the record reads number N there, where group N can be any group of this user whose leader has ended, and every member check passes. The only scope check that recovery has, `boot_id` (src/os/unix.rs:137), is the same in every namespace of one kernel.

Before #115's fix, every group signal after a restart needed the leader's exact start token (`same_process_start`), which a process in another namespace almost never matches. The collected case compares tokens by order (`>=`), so it needs the scope stated in the record.

Reap and qex's own TERM path are not exposed: reap's supervisor is the coordinator's own child, and the TERM path matched the exact start token in this namespace just before the TERM. The exposed call is recovery's final branch (src/daemon.rs:1652), the main case of #115.

### A namespace's name is not enough

The name `/proc/self/ns/pid` gives, `pid:[<inode>]`, is reused once a namespace ends. Measured on WSL2 Ubuntu (kernel 5.15.167.4-microsoft-standard-WSL2), 2026-11-03:

- Eight runs of `unshare -Urpf --mount-proc readlink /proc/self/ns/pid`, 1 s apart, gave `pid:[4026532263]` six times and `pid:[4026532264]` twice.
- WSL Ubuntu itself does not run in the initial pid namespace (inode 4026531836). Each start of the distro gets a new namespace inside the WSL VM, and `wsl --terminate Ubuntu` (not `--shutdown`) keeps `boot_id`:

| Read | `readlink /proc/self/ns/pid` | `boot_id` | start token of pid 1 (systemd) |
|---|---|---|---|
| before any terminate | `pid:[4026532255]` | `6fbe66fe-fc23-48e9-b6c2-df4bc305e964` | 161 |
| after one `wsl --terminate Ubuntu` | `pid:[4026532257]` | the same | 2256 |
| after a second one | `pid:[4026532256]` | the same | 2827 |

`pid:[4026532256]` was also the name of the distro's namespace earlier the same day, under the same `boot_id`. So a record left before a `wsl --terminate` can name the next session's namespace. The start token of each namespace's pid 1 differed every time.

## Decision

1. **Two new record fields in `status.json`: `pid_ns: Option<String>` and `pid_ns_init_start: Option<u64>`.** They hold the namespace's name (`sys::pid_namespace()`, src/os/unix.rs:398) and the start token of that namespace's pid 1 (`sys::process_start_token(1)`), both from `sys::pid_namespace_identity()`. The supervisor reads them when it writes `pid` and `pid_start_token`, and they are cleared wherever those two are cleared. This follows the pause record's `issuer_ns` (src/pause.rs:88, written at src/daemon.rs:2142, compared at src/pause.rs:767): an existing helper and a pattern the code already trusts, with the start of pid 1 added because the name alone is reused.
2. **The NSpid test.** pid 1 in `/proc` is this namespace's pid 1 only when `/proc` is this namespace's own mount. A sandbox that shows its parent's `/proc` would give the parent's pid 1, and two such sessions would compare equal again. The `NSpid:` line of `/proc/self/status` lists this process's pid in each namespace from that `/proc`'s down to its own, so it has exactly one entry when `/proc` is this namespace's own. With more than one entry, or when any read fails, `pid_namespace_identity()` gives `None` and nothing is recorded.
3. **The collected case runs only when both fields equal the recovering coordinator's `pid_namespace_identity()`.** On Linux, a record without them is refused, as a record with no start token is, and so is a coordinator that gets `None` itself. That covers a record written before this change, one written where `/proc` was not the namespace's own, and one whose read failed: no collected-case cleanup, only the refusal line. On macOS, which has no pid namespaces, both sides are empty and the comparison passes.
4. **The zombie case and every exact-token test stay as they are.** They do not read these fields.

Tests: `recovery_stops_the_leftovers_of_a_collected_job_process` checks on Linux that a new record holds this namespace's name and its pid 1's start, and the group is stopped (matching). `recovery_leaves_a_collected_leaders_group_alone_for_a_record_of_another_pid_namespace` (another name) and `recovery_leaves_a_collected_leaders_group_alone_when_the_namespaces_pid_1_started_later` (the same name, another pid 1 start) check that the group is left alone with one refusal line. The unit test `only_a_single_nspid_entry_is_this_namespaces_own_proc` (src/os/unix.rs) checks the NSpid rule: one entry is accepted; two entries, an empty line or no line are refused. It is a unit test because making a pid namespace that sees its parent's `/proc` needs a user namespace, which a test machine may refuse.

## Compatibility

- Both fields are additive and `#[serde(default)]`: an older coordinator ignores them, and this one reads an older record as `None`.
- No IPC, CLI or launch change. `qex status --json` shows the fields as `status.json` does.
- Rollback: a revert of the PR stops writing the fields. Records that hold them stay readable by the older code, which ignores unknown fields.

## Rejected

- **The name alone (`pid_ns` only, the first version of this ADR).** The name is reused (Context, above).
- **Run the collected case only in the initial pid namespace (inode 4026531836).** It would turn the cleanup off in WSL entirely, where the distro never runs in that namespace, and in every container.
- **Narrow the collected case to reap and the TERM path, with no record change.** It leaves #115's main case unfixed: a coordinator restart after the supervisor died and the system collected the leader.
- **Accept the gap as a known limit.** A shared state directory between a sandbox and the host is the setup docs/sandbox.md recommends, and on WSL a distro restart alone reuses names.
- **Compare the namespace of group N's members with the coordinator's.** A stranger in the coordinator's own namespace has that namespace, so the comparison passes exactly when it must refuse.
