# Job pid namespace ADR: the job record names the pid namespace of its pids

**Date:** 2026-11-03
**Status:** proposed; accepted when the owner merges the PR that adds this file (PR #141)
**Context:** issue #115 and its fix in PR #141; the owner's decision on that PR's review round 1, finding 2 (2026-11-03). Source citations are at commit `d726884`. This ADR takes no number, as the per-job TEMP ADR takes none: the numbers of PORT_PLAN.md's "ADRs" table already name other ADRs.

## Context

Once the system has collected a job's leader, the fix of #115 lets recovery SIGKILL group N when pid N is free, group N exists, and every live member belongs to this user and started no earlier than the recorded leader (`kill_what_the_job_left`, src/supervisor.rs:340). The proof that group N is the job's is a kernel rule: neither Linux nor macOS gives number N to a new process while group N exists.

That rule holds within one pid namespace. A coordinator inside a pid-namespaced sandbox or container can share the state directory with the host (docs/sandbox.md, "Give the sandbox the state directory and the socket", asks for one `XDG_STATE_HOME` inside and outside). The job record it writes holds pids of its own namespace. A later coordinator on the host that recovers the record reads number N in the host's namespace, where group N can be any group of this user whose leader has ended, and every member check passes. The only scope check that recovery has, `boot_id` (src/os/unix.rs:137), is the same in every namespace of one kernel.

Before #115's fix, every group signal after a restart needed the leader's exact start token (`same_process_start`), which a process in another namespace almost never matches. The collected case compares tokens by order (`>=`), so it needs the scope stated in the record.

Reap and qex's own TERM path are not exposed: reap's supervisor is the coordinator's own child, and the TERM path matched the exact start token in this namespace just before the TERM. The exposed call is recovery's final branch (src/daemon.rs:1652), the main case of #115.

## Decision

1. **A new record field, `pid_ns: Option<String>`, in `status.json`.** It holds `sys::pid_namespace()` (the target of `/proc/self/ns/pid`, src/os/unix.rs:398; `None` on macOS, which has no pid namespaces), read by the supervisor when it writes `pid` and `pid_start_token`. It is cleared wherever those two are cleared. This follows the pause record's `issuer_ns` (src/pause.rs:88, written at src/daemon.rs:2142, compared at src/pause.rs:767), so the field reuses an existing helper and a pattern the code already trusts.
2. **The collected case runs only when the record's `pid_ns` equals the recovering coordinator's `sys::pid_namespace()`.** On Linux a record with no `pid_ns` is refused, the same rule as a record with no start token: a record written before this change, or by a supervisor that could not read its namespace, gets no collected-case cleanup, only the refusal line. On macOS both values are `None`, and the comparison passes.
3. **The zombie case and every exact-token test stay as they are.** They do not read `pid_ns`.

## Compatibility

- The field is additive and `#[serde(default)]`: an older coordinator ignores it, and this one reads an older record as `pid_ns: None`.
- No IPC, CLI or launch change. `qex status --json` shows the field as `status.json` does.
- Rollback: a revert of the PR stops writing the field. Records that hold it stay readable by the older code, which ignores unknown fields.

## Rejected

- **Narrow the collected case to reap and the TERM path, with no record change.** It leaves #115's main case unfixed: a coordinator restart after the supervisor died and the system collected the leader.
- **Accept the gap as a known limit.** A shared state directory between a sandbox and the host is the setup docs/sandbox.md recommends, so the case is not rare.
- **Compare the namespace of group N's members with the coordinator's.** A stranger in the host's namespace has the host's namespace, so the comparison passes exactly when it must refuse.
