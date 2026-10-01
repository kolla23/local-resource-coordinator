# qex source assessment

**Audit date:** 2026-09-30  
**Purpose:** decide how to begin the resource coordinator POC against [SPEC.md](SPEC.md).  
**Conclusion:** use qex as the proposed POC foundation, pursue compatible improvements upstream, and defer a permanent fork or separate product decision until native tests and comparative measurements justify it. This is a recommendation, not an implemented port or an agreement from qex maintainers.

> **Subsequent user decision:** maintain an independent qex-derived fork without contributing upstream. The original recommendation below is retained as audit history and is superseded on project direction by [FOUNDATION_DECISION.md](FOUNDATION_DECISION.md). Technical findings and unrun validation gates remain applicable.

## Evidence and limits

The source reviewed is stephenc/qex at commit `afd869eef8ab9085dcaa4376a9f8f8b77397ba16`. The latest release observed during this audit was [v0.33.0](https://github.com/stephenc/qex/releases/tag/v0.33.0), published September 20, 2026, at `78b4e86c5ff8be0c7f4bc26bea809aa75fe230f0`. Comparing those commits found only Cargo package-version changes; the reviewed runtime source is therefore also applicable to that release.

The audit used 52 source, test, documentation, license, and workflow snapshots. Every snapshot was checked against its Git blob hash. [QEX_AUDIT_EVIDENCE.json](QEX_AUDIT_EVIDENCE.json) records those inputs. This was focused static review of relevant paths, not exhaustive security verification.

No qex executable was built or run here. Rust tooling was unavailable on PATH, and access to WSL distribution enumeration was denied. Native Windows, Linux, macOS, WSL2, crash injection, and comparative performance results remain unverified. The [upstream CI run](https://github.com/stephenc/qex/actions/runs/35533299050) reported success for its jobs, including Linux and macOS tests; that is upstream evidence, not a local rerun or proof of our proposed guarantees. No maintainer was contacted.

## What qex already provides

qex is a substantial foundation. It already separates the CLI, coordinator, and per-job supervisor; queues arbitrary commands; accounts for declared CPU and memory claims; checks available memory and Linux pressure; learns resource suggestions; protects against unlimited queue bypass; preserves supervisors across coordinator restart; records job status and output; and includes recovery and cancellation tests. Its core does not need an LLM. See [docs/design.md](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/docs/design.md), [src/sched.rs:702](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sched.rs#L702), and [src/supervisor.rs](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs).

It also has capabilities beyond this project's proposed V1: configured resource pools and GPU device allocation, a terminal dashboard, and cooperative peer claims. Those should not be mistaken for complete GPU enforcement, privileged multi-user coordination, or Windows/WSL shared accounting. See [src/sched.rs:96](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sched.rs#L96), [src/peers.rs](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/peers.rs), and [src/top.rs](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/top.rs).

The reason to extend qex would be stronger measured behavior and platform coverage. Recreating its queue and CLI under a new name would not establish useful differentiation.

## Coverage against the proposed V1

“Partial” means useful existing implementation with a remaining contract difference. It does not mean the existing product is defective.

| Required capability | Source finding | Work before claiming our V1 |
|---|---|---|
| Arbitrary noninteractive commands | Direct executable, arguments, working directory, captured environment, and null stdin are supported. | Verify quoting, signals, exit mapping, output bounds, and executable-change policy on every supported OS. |
| Claude, Codex, Cursor, and human use | Generic CLI is suitable for any client able to launch a command. | Document and test integrations without claiming every agent product was tested. |
| Shared local coordinator | Per-user state and Unix IPC; shared across repositories using that state namespace. | Define one admission authority per user/domain even under conflicting configuration and startup races. |
| CPU and RAM admission | Aggregate declared claims plus available memory checks. | Incorporate conservative live usage, future growth, effective OS/container limits, and CPU-load policy. |
| Swap and pressure | Linux PSI support; no equivalent native macOS pressure gate in the reviewed path. | Add required swap/pageout/commit observations, freshness, hysteresis, and conservative unavailable-metric behavior. |
| Whole workload accounting | Process-group RSS is available to the dashboard; historical learning uses child resource usage. | Feed validated whole-workload peaks into admission and history; expose uncertainty and escapes. |
| RUN/WAIT and fairness | Priority, FIFO, resource checks, bounded bypass, and oversized policies. | Reconcile repository-group admission turns, starvation behavior, overrun handling, and reasons with the spec. |
| Historical learning | Completed-job suggestions keyed by working directory and command. | Add environment/toolchain identity, censored samples, conservative floors, and retention controls. |
| Git/worktree/source context | Process-ancestor context exists; Git source-state recording was not found in reviewed paths. | Add observational repository/worktree/HEAD/dirty context. No V1 result reuse. |
| Safe lifecycle and recovery | Sessions/groups, identity checks, durable state, orphan recovery, cancellation fixtures. | Prove launch fencing and recovery under injected failures; implement unresolved-ownership handling. |
| Native Windows | Explicitly excluded from compilation. | Implement Windows process creation, Job Objects, IPC, signals/cancellation adaptation, and metrics. |
| Linux and macOS | Existing implementations and CI coverage. | Validate the stronger target accounting, pressure, containment, and release matrix. |
| WSL2 | Linux execution inside WSL is the documented Windows path. | Test WSL-specific limits and lifecycle. Host and guest remain separate V1 domains. |
| Security and operability | Local state, bounded stream storage, status, and troubleshooting facilities. | Reconcile secret retention, redaction, disk bounds, IPC validation, recovery operations, and diagnostics. |

## Findings that affect safety and correctness

### Native Windows is a backend project

The source rejects non-Unix builds. The coordinator uses Unix sockets and the supervisor uses Unix process APIs. Removing a compile guard cannot deliver a Windows port. WSL support also does not confer ownership of native Windows jobs. See [src/main.rs:16](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/main.rs#L16), [src/daemon.rs:20](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/daemon.rs#L20), and [src/supervisor.rs:16](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L16).

The first platform experiment should launch a Windows child suspended, attach it to an appropriate Job Object before allowing user code to run, and exercise cancellation and owner failures. Validate inherited handles, breakaway behavior, nested jobs, and descendants separately. This experiment is required evidence, not functionality completed in this audit.

### Admission uses claims without the spec's full live growth model

The scheduler combines held CPU/memory claims, peer claims, configured pools, available memory, and Linux PSI. The available-memory test considers reserve plus the candidate's claim. It does not implement the proposed subtraction for the unmaterialized growth of already admitted jobs. See [src/sched.rs:299](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sched.rs#L299), [src/sched.rs:702](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sched.rs#L702), and [src/sched.rs:1502](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sched.rs#L1502).

The default oversized policy permits execution when idle; a reject policy is available. The default pressure threshold and bounded bypass are useful existing controls, but are not the spec's complete pressure state machine or weighted group policy. See [src/config.rs:412](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/config.rs#L412) and [src/config.rs:485](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/config.rs#L485). Fair comparisons must disclose these differences and use both default and matched configurations.

### Telemetry failure and macOS estimation need validation

When the platform-specific available-memory read fails, the reviewed helper falls back to total memory. That can overstate headroom and conflicts with our conservative unavailable-metric rule. On macOS, the helper sums free, inactive, purgeable, and speculative pages. Apple's VM header states that speculative pages are already included in free pages, so that sum contains an overlap. See [src/sys.rs:36](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sys.rs#L36), [src/sys.rs:95](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sys.rs#L95), and [Apple's VM statistics definition](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/vm_statistics.h).

These are source-grounded risks, not measured incidents or a claim that qex froze a machine. Add failure-injection fixtures and compare each component with native observations before choosing a replacement formula. Pressure, compression, reclaimability, and effective container limits must be considered separately.

### Learned memory is not a simultaneous process-tree peak

The supervisor obtains learning data using `getrusage(RUSAGE_CHILDREN)`. On Linux, the reported maximum RSS is the largest child's value, not the simultaneous memory of the whole tree. Parallel workers can therefore require substantially more RAM than this history value describes. See [src/supervisor.rs:1409](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L1409) and [Linux getrusage semantics](https://man7.org/linux/man-pages/man2/getrusage.2.html).

qex also implements process-group RSS sampling, but the reviewed call site is in the dashboard. Its presence does not establish that scheduler admission or historical learning uses that aggregate. See [src/sys.rs:685](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/sys.rs#L685) and [src/top.rs:725](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/top.rs#L725).

Learning currently records completed jobs with a nonzero peak; failed or interrupted observations do not provide the conservative lower bounds required by our design. The history key is based on working directory and argv. Learned memory can be clipped to the configured budget, and explicit memory claims can replace the learned suggestion. These are policy differences to resolve explicitly, rather than treating a suggestion as a guaranteed safe bound. See [src/usage.rs:200](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/usage.rs#L200), [src/usage.rs:324](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/usage.rs#L324), [src/usage.rs:467](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/usage.rs#L467), and [src/spec.rs:581](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/spec.rs#L581).

### Existing lifecycle protections deserve preservation and stronger tests

The supervisor records identity information, creates a child process group, and uses a wait-without-reaping sequence before cleaning up remaining group members. Recovery checks boot/process identity and avoids blindly treating every interrupted launch as safe to retry. These are important protections. See [src/supervisor.rs:325](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L325), [src/supervisor.rs:547](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L547), [src/supervisor.rs:689](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L689), and [src/daemon.rs:1322](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/daemon.rs#L1322).

The user process can begin after spawn and before its PID is durably recorded. The implementation is not the specification's fully fenced launch/receipt protocol. This identifies a fault-injection boundary; it does not demonstrate that a duplicate launch actually occurs. Existing recovery guards must be included in that analysis.

Process groups do not contain descendants that escape into new sessions. The reviewed enforcement module does not create per-job cgroups; it observes OOM information from the containing environment. Do not infer stronger containment from a comment or old issue. Cleanup also differs from the proposed descendant-drain contract. See [src/enforce.rs:1](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/enforce.rs#L1), [src/enforce.rs:86](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/enforce.rs#L86), and [src/supervisor.rs:689](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/supervisor.rs#L689).

Before releasing reservations after supervisor failure, our target must represent unresolved ownership explicitly or establish that all owned work has stopped. Tests should interrupt each durable transition and cover escaped children, reboot, PID reuse, and partial state writes. Existing fixtures in [tests/e2e.rs:2828](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/tests/e2e.rs#L2828) and [tests/e2e.rs:3848](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/tests/e2e.rs#L3848) are useful starting points, not substitutes for running the target matrix.

### qex dedupe is not source equivalence or strict request idempotency

The opt-in dedupe key can select an existing job without comparing its command. It may also match a recently successful job when a reuse window is requested. That is an explicit user feature; it does not prove that two source states or environments are equivalent. See [src/daemon.rs:704](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/daemon.rs#L704) and [src/daemon.rs:2577](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/daemon.rs#L2577).

Our V1 must execute every independent validation request. A transport retry may refer to the same request only when strict idempotency rules match its execution payload. Do not enable result-window reuse for that purpose. qex's ancestor-process context also must not be presented as Git/worktree identity.

### Command and storage policies need deliberate configuration

qex may export runtime limit hints when explicit resource claims are provided, and has configurable process priority behavior. The proposed V1 preserves environment semantics unless users opt in to changes. Disable hints where required and record priority in benchmark arms. See [src/spec.rs:665](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/spec.rs#L665) and [src/config.rs:560](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/config.rs#L560).

Per-stream output limits exist, but they do not establish a global disk budget or short-lived environment retention. Job records and command output can contain secrets; filenames and command arguments can also be sensitive. Reconcile cleanup, permissions, redaction, and environment deletion with the target contract. See [docs/security.md](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/docs/security.md) and [src/config.rs:853](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/src/config.rs#L853).

## Foundation and license assessment

qex is Rust and declares Apache-2.0. Its license permits modification and redistribution subject to its conditions. A derivative must preserve applicable license and attribution material, mark modified files as required, and handle NOTICE obligations if applicable. No NOTICE file was present in the audited tree. Transitive dependency licensing and bundled artifacts still require review before release. See [Cargo.toml](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/Cargo.toml), [LICENSE](https://github.com/stephenc/qex/blob/afd869eef8ab9085dcaa4376a9f8f8b77397ba16/LICENSE), and the [Apache license](https://www.apache.org/licenses/LICENSE-2.0).

There is no need to choose a final project name to run a private development POC. Use Rust for the qex-derived experiment, preserve upstream attribution, and pin the starting revision. Do not spend the first milestone translating the code to a new language.

| Approach | Assessment |
|---|---|
| Focused upstream contributions | Preferred for compatible telemetry corrections, test coverage, diagnostics, and backend seams. Maintainer acceptance is unknown. |
| qex-derived experimental branch | Recommended POC vehicle. Allows measurable changes while keeping a clean upstream comparison. It is not a commitment to a permanent fork. |
| Companion only | Suitable for Git metadata or reporting. Cannot alone replace qex's internal admission, native process ownership, or recovery behavior. |
| Permanent fork | Conditional option if required semantics or Windows architecture cannot be accepted upstream and maintenance capacity is credible. |
| Separate implementation | Last option if native experiments show that adaptation is materially worse; overlap alone is not justification. |

qex uses atomic job files rather than the target specification's SQLite design, and an additive JSON protocol rather than its proposed strict public framing. These differences require an ADR. Preserve safety outcomes and public compatibility where possible; do not rewrite persistence solely for architectural uniformity. Conversely, do not silently relax durability, identity, or launch requirements just to match existing code.

## Required experiments and decision gates

1. **Freeze reproducible inputs.** Pin v0.33.0 and the source commit, record machine/OS/toolchain/configuration, collect clean upstream baseline results, and identify the exact test commands before changes.
2. **Measure workload memory.** Use simultaneous workers, short bursts, failures, cancellations, and OOM-like termination. Compare independent whole-workload observations with qex history and the proposed estimator.
3. **Exercise platform ownership.** Test Windows Job Objects, Linux cgroup availability and fallback, macOS group limitations, and WSL domain boundaries. Document unsupported guarantees.
4. **Specify and fault-test the private protocol.** Define schemas, authentication, generations, launch receipts, sequence/replay rules, deadlines, reconnect, cancellation acknowledgement, and terminal reconciliation. Interrupt every launch/persistence boundary.
5. **Compare scheduling.** Run uncoordinated, static-limit, unchanged qex, and the experimental coordinator arms from SPEC §21. Compare requested executions, failures, memory pressure, responsiveness, wait fairness, and completion time, with cold and learned histories.
6. **Choose the long-term home.** Publish favorable and unfavorable results. Prefer upstream if the improvements fit. A permanent fork or separate product needs a demonstrated gap and maintenance rationale.

No benefit over qex has been demonstrated by this source audit. It establishes where to test and where implementation work is likely. It does not close the native validation, calibration, or product-justification gates.

## Outstanding work classification

**V1 design and engineering gates:** exact private daemon–supervisor protocol, native measurement and ownership validation, launch/crash recovery tests, threshold calibration, supported OS versions, dependency/license inventory, and the foundation decision after benchmarks.

**Open product decisions:** final name; final license adoption; permanent upstream/fork/separate home. Rust is recommended for the qex-derived POC, while any later language change requires evidence.

**Deferred V2 scope:** dynamic Windows/WSL coordination, complete container/external-service ownership, child PTY/ConPTY support, nested resource subleases, GPU/VRAM policy, privileged multi-user coordination, proven-equivalence result reuse, and optional GUI/IDE/MCP integrations. qex already having an adjacent feature does not automatically promote it into this project's V1 support contract.

