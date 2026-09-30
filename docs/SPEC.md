# [PROJECT_NAME]: Local Development Resource Coordinator

**Document version:** 0.4.0 — proposed V1 specification with independent fork decision  
**Prepared:** 2026-09-29; revised 2026-09-30  
**Status:** design draft for POC implementation; native backend behavior and benchmark targets remain unvalidated.  
**Name:** `[PROJECT_NAME]` is deliberately unselected. `coord` is a documentation-only CLI token, not a proposed trademark or package name.  
**Audience:** maintainers, contributors, agent integrators, security reviewers, and developers evaluating the POC.

## 1. Purpose and interpretation

Build an open-source, local resource coordinator that lets independent software-development clients share a machine without each assuming that all currently available resources belong to it. Claude, Codex, Cursor, other agents, CI scripts, and humans submit ordinary commands to one admission authority. The coordinator decides deterministically whether each command may **RUN** or must **WAIT**, supervises its process lifetime, and records what it actually consumed.

The intended outcome is fewer workload-induced memory-pressure incidents, less destructive contention, predictable cancellation, and understandable queue decisions. Faster total completion is desirable but subordinate to correctness and responsiveness. The project does not promise to make every workload faster or prevent every OS out-of-memory event.

MUST/MUST NOT are release requirements; SHOULD is the recommended behavior with a documented exception; MAY is optional. A proposed default in this document is normative for the initial implementation until replaced through an architecture decision record (ADR). Unresolved questions in §25 do not permit silent implementation choices that contradict these defaults.

This document is the source of truth before repository creation and implementation. It incorporates the prior discussion's agreed scope and supplies explicit defaults where that discussion left choices open. It does not assert that an implementation or POC already exists.

## 2. Problem statement

Parallel agents and terminals launch test workers, compilers, language servers, dependency installers, browser automation, containers, and local model jobs. They often operate from different Git worktrees and cannot observe one another's imminent launches. Independent checks of free RAM race: several clients see the same headroom and consume it simultaneously. Static worker counts also fail when workloads have very different peaks.

The consequences include swapping, long interactive stalls, OS-selected OOM victims, abandoned descendants, misleading validation records, and repeated work after agents lose their terminals. Parent-process measurements undercount subprocess-heavy tools. Worktree identity alone does not establish identical source, dependencies, or execution environment.

A useful solution must coordinate before launching, observe after launching, preserve command semantics, and explain its limitations. It must work without recognizing the programming language or consulting an LLM.

### 2.1 Scope of “global”

V1 has **one coordinator per OS user per resource domain**, shared across repositories, worktrees, shells, and agent products. It is not one daemon per repository or agent. The resource domain is the native OS environment or an independently scheduled VM/container environment.

V1 is not a privileged, machine-wide multi-user scheduler. Other users and unwrapped processes are external load. Windows and a WSL2 guest do not become one accounting domain merely because they share hardware. Cross-domain coordination is explicitly limited in §10.4; no UI may label independent host/guest daemons as a globally enforced host budget.

## 3. Goals, non-goals, and compatibility

### 3.1 Goals

1. Execute arbitrary local, noninteractive commands with their arguments, working directory, environment, streams, and meaningful exit status preserved.
2. Offer the same CLI contract to humans and agents; no agent-specific integration is required.
3. Combine CPU capacity, usable RAM, swap activity, memory pressure, current workload consumption, and future claims into admission decisions.
4. Account for whole owned workload groups where supported and disclose uncertainty where it is not.
5. Learn conservative resource estimates from local historical measurements, with useful cold-start behavior for unknown commands.
6. Provide bounded queue bypass, observable fairness, and explanations for every wait.
7. Preserve lifecycle truth across client disconnects, daemon failure, supervisor failure, reboot, and partial launches.
8. Record Git repository, worktree, and observed source-state context without skipping or reusing requested validation.
9. Run unprivileged by default on Windows, macOS, Linux, and WSL2; expose capability differences honestly.
10. Make all safety decisions through deterministic code and explicit policy, without network or LLM availability.

### 3.2 Non-goals for V1

- Distributed/remote scheduling, cluster orchestration, GPU/VRAM allocation, or an agent planner.
- A sandbox against malicious code running as the same user.
- Automatic rewriting of commands, worker counts, environment limits, or test selection.
- Intercepting every process on the machine or controlling programs never submitted to the coordinator.
- Worktree creation, branch management, file editing, merging, PR review, or deployment.
- Cross-worktree result reuse, command coalescing, build caching, or declaring an unexecuted test successful.
- Universal hard memory limits or transparent pausing/checkpointing on every OS.
- Full terminal emulation/PTY/ConPTY, interactive shells, password prompts, or durable interactive stdin.
- Owning remote services, Docker-daemon containers, or system services merely because a submitted CLI contacted them.

“Arbitrary commands” means no stack allowlist for ordinary executable jobs. Platform execution rules, permissions, noninteractive I/O, and external-service boundaries still apply. Unknown stacks receive generic scheduling and supervision. Long-running foreground services are permitted but hold resources until stopped; finite completion and fairness bounds do not apply while capacity is permanently occupied.

## 4. Competitive boundary and build-versus-contribute decision

These are repository-level observations checked during drafting, not an exhaustive code audit or a claim about future releases. Pin exact versions and commits before benchmarking. In particular, do not present shared admission control or historical learning as novel.

| Project | Documented focus | Boundary for this proposal |
|---|---|---|
| [qex — stephenc/qex](https://github.com/stephenc/qex) | Shared local resource admission, CPU/memory claims, resource learning, queue bypass protection, command supervision and agent-facing CLI. | Closest comparator and possible upstream destination. Proposed value must be demonstrated in platform behavior, pressure response, lifecycle guarantees, or source-context observability. Feature overlap alone does not justify a new project. |
| [Cordon — uncoalesced/cordon](https://github.com/uncoalesced/cordon) | Agent tool-call characterization and control; hooks and Linux cgroup-based enforcement are prominent. | This proposal's primary interface is arbitrary command submission across independent clients. Tool-call instrumentation could complement it; V1 does not require semantic classification or agent hooks. |
| [Pueue — Nukesor/pueue](https://github.com/Nukesor/pueue) | Persistent command queues, parallelism, groups, process interaction, and daemon-based execution across platforms. | A strong lifecycle/queue baseline. This proposal centers measured host pressure and changing resource claims rather than only configured parallel counts. Prefer reuse or integration where appropriate. |
| [Orbit — orbcli/orbit](https://github.com/orbcli/orbit) | Multi-repository Git workspaces for coding agents. | Workspace organization is adjacent; this project schedules processes submitted from those workspaces. |
| [Orbit — constellation-works/orbit](https://github.com/constellation-works/orbit) | Agent task delivery with sandboxed worktrees, locks, and review gates. | Workflow orchestration is adjacent; it can submit heavy commands to this coordinator. |
| [wotr — gtmax/wotr](https://github.com/gtmax/wotr) | Worktree sessions, hooks, and management of shared infrastructure resources such as exclusive services. | Infrastructure ownership is not interchangeable with CPU/RAM admission. V1 does not replace service leases or worktree management. |

“Orbit” was not disambiguated in the supplied conversation; both identifiable projects are listed instead of attributing features to the wrong one. This ambiguity does not affect V1 design.

**Decision gate:** after the POC, publish a gap analysis against pinned qex, including favorable and unfavorable results. The user has selected an independent qex-derived fork. Evaluate whether its improvements justify ongoing maintenance; do not substitute an upstream-contribution plan without a new user decision. Record maintenance and interoperability tradeoffs.

### 4.1 Source assessment and proposed POC foundation

The September 30 source audit reviewed qex at commit `afd869eef8ab9085dcaa4376a9f8f8b77397ba16`; v0.33.0 has the same runtime source with release-version metadata changes. See [QEX_ASSESSMENT.md](QEX_ASSESSMENT.md), [FOUNDATION_DECISION.md](FOUNDATION_DECISION.md), and [QEX_AUDIT_EVIDENCE.json](QEX_AUDIT_EVIDENCE.json) for evidence, limitations, and the proposed next milestone.

Selected direction: begin the POC from qex in Rust and maintain our own modified version with proper attribution. The user chose an independent fork without upstream contributions, superseding the assessment's initial recommendation. qex already provides substantial scheduling, supervision, learning, recovery, configured resource pools, and a terminal dashboard. It does not yet satisfy this V1 contract, including native Windows execution and the proposed whole-workload estimation and recovery guarantees. No benchmark advantage or maintainer acceptance has been established. The independent-fork direction is selected; technical feasibility, benchmark value, and release readiness remain post-POC gates.

This specification and its schemas describe the target contract, not qex's existing interfaces. Before adapting qex, record ADRs for CLI compatibility, IPC framing/versioning, file persistence versus SQLite, and private launch semantics. Reuse proven mechanisms where they meet the requirements; do not silently weaken safety obligations or require an unmotivated storage rewrite.

## 5. Requirements and priority

| ID | V1 MUST requirement | Verification |
|---|---|---|
| R01 | One active admission leader per user/domain; no launch on an unverified second leader. | Contended startup and fencing tests. |
| R02 | Direct argv execution; shell evaluation only when explicitly requested. | Argument/quoting fixtures on all platforms. |
| R03 | Unknown and non-Git commands execute without plugins. | Generic binary, script, and no-repository fixtures. |
| R04 | CPU, memory, swap, and pressure telemetry includes provenance, age, units, and capability flags. | Backend conformance and stale-data injection. |
| R05 | RUN/WAIT decisions atomically reserve claims before execution. | Concurrent admission property tests. |
| R06 | Every wait has a stable reason code and current limiting quantity. | CLI/IPC contract tests. |
| R07 | Learning never treats incomplete measurements as successful low peaks. | Censored-observation tests. |
| R08 | Cancellation targets verified ownership only and remains idempotent. | PID reuse and descendant fixtures. |
| R09 | Crash recovery never automatically reruns a possibly executed command. | Fault injection at each durable transition. |
| R10 | Git context is observational; each accepted execution request is independent. | Same-HEAD/different-dirty-worktree tests. |
| R11 | All core policy works offline without an LLM or agent account. | Network-disabled end-to-end run. |
| R12 | Unsupported requested hard enforcement fails before launch. | Capability-negative tests. |
| R13 | Queue, history, output, and IPC have finite size limits and user-private access. | Flooding, disk pressure, and permission tests. |
| R14 | V1 runs natively on the release platforms and in WSL2 with explicit domain limitations. | Native runner matrix and host/guest tests. |

Optional agent hooks, specialized command recognizers, GUI, dashboards, and extra package managers cannot block the generic CLI release. No plugin may be necessary to make a supported ordinary executable work.

## 6. Architecture and component boundaries

```text
Human / Claude / Codex / Cursor / CI / other client
                    |
             coord CLI (argv, cwd, env)
                    | private versioned local IPC
                    v
          Per-user/domain coordinator
       +------------------------------------+
       | Admission scheduler + durable queue|
       | Resource estimator + history       |
       | Git/context collector              |
       | OS monitor + capability registry   |
       | Event journal + recovery manager   |
       +-----------------+------------------+
                         | one launch grant per job
                         v
                Per-job supervisor
          streams / exit receipt / heartbeat
                         |
          OS-owned group or best-effort tree
                         |
                 executable + descendants
```

| Component | Responsibility | Must not do |
|---|---|---|
| CLI | Resolve submission context, forward streams, present decisions, wait or detach. | Independently grant capacity or silently run outside a failed daemon. |
| Coordinator | Own admission, state transitions, policy snapshots, recovery and queries. | Block its event loop on Git, output consumers, or process enumeration. |
| Scheduler | Pure decision function over validated snapshots, queue, and reservations. | Spawn processes, invoke shells, or use nondeterministic model output. |
| Estimator | Generate versioned claims and confidence from hints/history/defaults. | Declare source equivalence or lower claims from censored samples. |
| Platform backend | Monitor, establish ownership, launch, signal, and enumerate capabilities. | Pretend identical measurement or enforcement semantics on different OSes. |
| Supervisor | Own a single launched workload; collect exit and output; survive daemon restart. | Admit a second independent workload or accept unauthenticated control. |
| Context collector | Bounded read-only Git/environment observations. | Execute repository hooks or mutate the index/source tree. |
| Store | Transactional state, ordered events, bounded history and spool references. | Persist secrets in ordinary diagnostic events. |

### 6.1 Architecture alternatives and implementation technology

Recommended: a persistent coordinator plus a small supervisor per running job. It separates scheduling failure from command lifetime, at the cost of extra processes and a recovery protocol.

Rejected for V1: independent wrappers with a shared lock file. They simplify deployment but make reservations, liveness, output ownership, and crash reconciliation harder. Also rejected: a privileged system broker as the default. It could coordinate users and enforce more policy, but substantially expands the trust boundary and installation burden.

Rust is the proposed implementation default: one distributable executable with CLI, daemon, and internal supervisor modes; typed state machines; isolated native-API bindings. Go remains a valid alternative if the POC demonstrates materially simpler maintenance and comparable native behavior. An ADR must compare Windows Job Objects, Unix spawning, macOS bindings, idle overhead, packaging, and contributor accessibility before the first implementation milestone is frozen. The protocol and requirements do not depend on either language.

## 7. Resource model and measurement

### 7.1 Terms

- **Claim:** predicted peak resource need used for admission, not an OS limit.
- **Usage:** measured current resource consumption with a backend-specific metric.
- **Limit:** optional OS-enforced bound; distinct from a claim.
- **Reserve:** capacity deliberately left for the OS, foreground apps, and uncertainty.
- **Domain capacity:** effective accessible capacity after affinity, VM boundaries, and observable ancestor limits, not simply physical-machine specifications.
- **Owned workload:** processes in a verified kernel group or verified descendants tracked by the supervisor.
- **External load:** all load not reliably attributed to owned workloads, including accounting uncertainty.

All persisted memory values are integer bytes. CLI units are binary (`MiB`, `GiB`); CPU is a positive fractional count of logical cores, where `1.0` is one fully utilized logical core. CPU time is stored separately in nanoseconds. Duration comparisons use monotonic clocks; UTC timestamps serve audit display. Every metric includes `backend`, `metric_kind`, `sample_time`, and `quality` (`exact_group`, `sampled_tree`, `host_only`, or `unavailable`). “Exact group” describes membership/accounting scope, not perfect physical-RAM attribution.

### 7.2 Required observations

Host/domain: effective memory capacity, conservative available-memory estimate, swap usage and page-out rate when observable, pressure indicators, CPU utilization, CPU capacity/affinity, and memory commit headroom where relevant. Optional load average and I/O PSI are diagnostic. Swap capacity MUST NOT be added to admissible RAM.

In a constrained Linux domain, `host_available` means the minimum of host available memory and every readable finite ancestor memory-limit headroom (`limit - current`, floored at zero). Account for sibling usage at those ancestors. CPU capacity similarly respects the most restrictive observable quota and affinity. Unreadable ancestor constraints are reported as uncertainty; a visible VM or container total is not evidence of exclusive ownership of that capacity.

Workload: current and sampled peak memory, cumulative CPU time, recent core-equivalent utilization, live process count, containment identity, root exit status, and completeness. Track platform commit and resident metrics separately. Do not add RSS, private bytes, compressed memory, and cgroup charged memory as if they were independent quantities.

Sampling defaults: host every 500 ms; workload groups every 1 s; 250 ms host sampling during HIGH/CRITICAL pressure. Events supplement polling. Admission never uses host samples older than 2 s. Batch process enumeration once per tick in fallback backends. Short-lived peaks may be missed; use kernel high-water counters where available and label sampled peaks otherwise.

### 7.3 Accounting rules

Prefer a kernel group metric. Without one, sum verified resident footprints conservatively and state that shared pages may be counted repeatedly and short-lived/escaped descendants missed. Do not claim system used-memory equals the sum of workload measurements. Estimation histories are partitioned by metric kind and backend.

If a child daemonizes, escapes a process group, launches via a service manager, invokes a privileged helper, or contacts a container engine, complete ownership may be impossible. Mark coverage incomplete. Treat unattributed host usage as external load, preserve reservations conservatively, and never fabricate an empty tree from failed enumeration.

## 8. Scheduler specification

### 8.1 Default budgets

Let `M` be effective domain RAM. Reserve `R = max(2 GiB, 0.15*M)`. Maximum aggregate memory claims `B = min(0.75*M, M-R)`. If `B <= 0`, admit nothing and explain the invalid budget. A user may explicitly configure lower reserves for a small device; there is no automatic safety bypass.

Let `N` be effective logical-core capacity. CPU budget `C = max(1, N-1)` for `N >= 1`, capped at `N`; fractional quotas below one core use that quota. Default maximum parallel jobs is `max(1, min(8, floor(N)))`. These limits apply to active reservations, including STARTING and unresolved recovery jobs.

CPU claims are advisory scheduling reservations, not promises about runtime thread count. Default execution does not set `OMP_NUM_THREADS`, `GOMAXPROCS`, test workers, runtime heap flags, or similar variables. Such changes require explicit user configuration in a future adapter or the submitted command itself.

### 8.2 Admission math

For active job `i`, let `q_i` be its current memory claim and `u_i` its allowed materialized-memory credit on the backend's accounting basis. Credit is distinct from displayed raw usage: shared-page overcounts or incomparable footprint metrics cannot reduce future reservations. Raise `q_i` to at least observed usage immediately, regardless of credit; never reduce an active job's memory claim. For STARTING, stale/incomplete workload data, and the initial macOS/Windows backend policies in §10, use `u_i = 0` when calculating unmaterialized reservations; this intentionally errs toward waiting. Linux sampled-tree fallback also uses zero credit; fresh delegated cgroup accounting may use its current charged-memory value, subject to the documented metric limitations.

```text
growth_reserved = sum(max(q_i - u_i, 0))
future_headroom = max(0, host_available - R - growth_reserved)
claimed_memory = sum(q_i)

memory_fit(j) =
    q_j <= future_headroom
    AND claimed_memory + q_j <= B

cpu_fit(j) = sum(cpu_claim_i) + cpu_claim_j <= C
```

This subtracts only the unmaterialized portion from current availability: subtracting complete running claims again would double-count already resident memory. The separate aggregate-claims ceiling remains intentionally conservative. Available memory and workload usage are imperfectly comparable, so this formula is an admission heuristic, never a physical guarantee.

Windows additionally requires predicted incremental commit plus a commit reserve to fit system commit headroom. Maintain a separate commit claim `k_i` for each job, initially `max(memory_claim, learned_commit_claim)`; without commit history, use the full memory claim. Use a commit reserve of `max(1 GiB, 10% of commit limit)`. The exact ledger and worked example are in §10.3.1. Commit exhaustion independently closes admission.

A job may RUN only when all of the following hold: fresh host telemetry; leader and store healthy; no reconciliation uncertainty that exhausts the budget; pressure NORMAL; memory fit; CPU fit; available job slot; configured per-group cap; no fairness reservation blocking it. Persist the grant and reservations in one transaction before starting a supervisor. Re-evaluate after every grant; serialize admissions even when requests arrive concurrently.

**Oversized jobs:** reject a submission with `RESOURCE_REQUEST_EXCEEDS_BUDGET` when its initial claim exceeds the configured memory or CPU budget. Do not silently run it alone over budget. The error distinguishes static infeasibility from temporary external load and suggests explicit configuration or workload changes. A later capacity reduction puts queued jobs into WAIT with `CAPACITY_CHANGED`; it does not rewrite their request.

### 8.3 Pressure state machine

Use the most severe valid signal. The initial policy thresholds below are tuning defaults to validate in the POC, not universal OS facts.

| Signal | HIGH trigger | CRITICAL trigger |
|---|---|---|
| Available RAM | Below `R` for 2 s | Below `max(512 MiB, 3% of M)` for 1 s |
| Linux memory PSI | `some avg10 >= 10%` for 2 s | `full avg10 >= 5%` for 2 s |
| macOS pressure event | Native warning | Native critical |
| Windows commit headroom | Below commit reserve for 2 s | Below `max(256 MiB, 2% of commit limit)` for 1 s |
| Windows low-memory notification | HIGH immediately | Critical only when another critical condition holds |
| Page-out rate, when available | At least 64 MiB/s for 5 s, with available RAM below `2R` | Not independently critical |

CRITICAL blocks new admissions immediately on a native critical event; other conditions use the durations above. HIGH also blocks new admissions. Return from CRITICAL to HIGH only after all critical triggers have cleared for 5 s. Return to NORMAL only after 10 s with no HIGH trigger and available RAM above `1.2R`. Resume with at most one launch per second for the next 5 s. High CPU utilization (`>90%` for 5 s) independently blocks launches until utilization is below 80% for 3 s; it does not imply memory pressure.

If PSI or swap counters are unavailable, use valid remaining signals and report the capability loss. If essential available-memory/CPU sampling fails or is stale, enter UNKNOWN and WAIT. Missing metrics are never zero. On resume from system sleep, invalidate samples and collect fresh data before admission.

### 8.4 Overruns and emergency behavior

On observed usage above a claim, enlarge its reservation, emit `ESTIMATE_EXCEEDED`, increase its history floor, and stop admitting if the new total does not fit. Sustained high memory-growth rates can raise a claim further using the estimator's margin. Do not resize other jobs downward to accommodate an overrun.

Default pressure response is **hold new work and warn**. V1 does not automatically kill, freeze, or retry running jobs because pressure might come from unrelated applications and terminating development commands can leave partial side effects. Optional explicit hard limits act through the OS and are reported as such. This leaves residual OOM risk; the product must say so. CPU saturation and memory pressure cannot always be fixed after admission.

### 8.5 Fairness and bounded bypass

Default scheduling group is the local repository identity, or `ungrouped` outside Git. An explicit `--group` can group agent sessions or other work; agent labels alone do not create more shares. Groups have weight 1 by default and use weighted round-robin admission opportunities; within each group select the oldest pending job. User-configured weights are integers 1–16. Each successful admission consumes one opportunity; unused opportunities do not accumulate. This is admission fairness, not equal CPU-time allocation.

Select among group heads. A fitting head may bypass an older non-fitting eligible head at most twice globally, or until that older job has accrued 30 s of fairness-eligible waiting, whichever comes first. Then reserve the next available capacity for the oldest protected head and admit no other job until it fits, is cancelled, times out, or ceases to be fairness-eligible. Reconsider on each resource update; no preemption occurs. This deliberately favors a clear starvation boundary over maximal utilization.

#### 8.5.1 Deterministic selection and tie-breaking

The coordinator assigns an increasing durable `submission_seq` at acceptance; this, not wall-clock time, defines oldest. Store each group's first-registration sequence. Construct a ring ordered by that sequence, with `weight` consecutive slots per group. Persist the next-slot cursor with each admission. A new group joins at the end; rebuilding after a group/config change preserves the next surviving slot, otherwise starts at the first slot. Empty groups remain registered until restart, when their durable order is still preserved; inactive slots are simply skipped.

For each decision, first identify group heads whose only blockers are resource headroom/claims or the global parallel cap. These are fairness-eligible. Exclude heads blocked by a group-specific running cap, missing payload, recovery uncertainty, failed executable validation, or an explicit precondition; they cannot freeze other groups. Global pressure/telemetry/pause gates suspend all admission without advancing the cursor.

Each eligible head has a durable `bypass_count` and an eligible-wait elapsed counter. Eligible-wait time accrues only while the head is fairness-eligible; global pressure pauses do not reset it. If any head has reached the bypass or 30 s threshold, select the smallest `submission_seq` among those heads as the reservation owner. Once chosen, that owner persists until admission/removal or loss of fairness eligibility; no new job from any group may launch while it cannot fit. Other owners cannot displace it. Loss of eligibility suspends its protection without resetting its counters; eligibility restoration makes it a candidate again.

Without a reservation owner, scan at most one full ring from the cursor, considering only each group's FIFO head and selecting the first fitting, eligible head. Skipped slots are not banked. If nothing fits, keep the cursor unchanged. After a grant, move it to the slot following the selected slot. Increment `bypass_count` once for every older eligible head that could not fit before that grant; a fitting older head skipped solely by weight does not accrue a resource-bypass count. Recompute protection before another grant. Forced admission of a reservation owner advances the cursor after the first slot for its group found from the old cursor. All counters and selection changes commit atomically with the grant.

Examples with NORMAL pressure and ample CPU:

- **Weights:** A has weight 2, B weight 1, all jobs fit, and the ring starts at A. Admission order is A1, A2, B1, A3, A4, B2. This does not promise equal resource-time use.
- **Large head:** A1 needs 6 GiB, is oldest, and only 2 GiB is currently free for claims. B1 and C1 each need 1 GiB. They can both pass A1, making its count 2; D1 then waits behind A1 even if D1 fits. Once running work frees 6 GiB, A1 runs next.
- **Two blocked heads:** A1 and B1 each reach protection; A1 has the smaller sequence and owns the reservation. After A1 starts, B1 is selected before any younger fitting head. No two independent reservations compete for the same freed capacity.
- **Group cap:** A1 cannot run because group A already reached its own cap. It does not stop B1. Once the cap clears, A1 regains eligibility and its existing eligible-wait/bypass counters.

The 30 s rule measures fairness-eligible waiting, not total wall time. A statically infeasible head after a capacity change remains visibly WAITing but is excluded from reservation ownership; otherwise it could block the entire queue forever.

Persistent external load, nonterminating running jobs, or unavailable capacity can prevent all progress. There is no finite waiting guarantee in those cases. Emit a stuck-reservation event after 60 s and every material change, not every sampling tick. An explicit `--queue-timeout` is available; the default is unlimited waiting with visible reason. The finite-work fairness test assumes eventually sufficient capacity and terminating jobs.

### 8.6 Decision procedure

```text
on queue change, resource sample, completion, or recovery change:
    reconcile ownership and retain uncertain reservations
    update pressure state and enlarge observed-overrun claims
    expire queue deadlines
    if leader/store/telemetry unhealthy or pressure != NORMAL: WAIT
    choose candidate by group rotation and bounded-bypass rule
    if no candidate: idle
    if candidate cannot fit: record WAIT reasons; consider permitted bypass
    otherwise:
        atomically store STARTING + unique launch token + reservations
        dispatch one launch request
        account for the grant before considering another candidate
```

For reproducibility, log policy version, input snapshot ID, candidate, decision, limiting quantities, and reservation totals. Reasons include `MEMORY_HEADROOM`, `MEMORY_CLAIMS`, `COMMIT_HEADROOM`, `CPU_CLAIMS`, `CPU_BUSY`, `PRESSURE_HIGH`, `PRESSURE_CRITICAL`, `TELEMETRY_STALE`, `FAIRNESS_RESERVATION`, `PARALLEL_LIMIT`, `GROUP_LIMIT`, `RECOVERY_PENDING`, and `ADMISSION_PAUSED`.

## 9. Resource estimation and local learning

### 9.1 Cold start and hints

Without history, use `memory = max(512 MiB, min(2 GiB, 0.25*B))` and `cpu = min(2, C)`. Permit only one running cold-start job at a time; known jobs can still run if all budgets fit. An impossible cold-start claim is rejected explicitly on tiny configurations.

`--memory` and `--cpu` are user-provided admission hints. They never change the command or silently establish a hard limit. A memory hint is a floor relative to relevant historical evidence: `claim = max(hint_or_default, learned_claim)`. Users needing to discard stale history use `history forget <profile-id>` explicitly. Malicious or incorrect low hints must not bypass floors or aggregate checks.

### 9.2 History keys

Use a versioned, length-delimited encoding and local keyed hash of: executable identity, exact argv, canonical cwd/worktree identity, OS/architecture, resource-domain class, accounting metric, environment fingerprint, optional selected dependency/lockfile fingerprints, and user profile label. Include relevant execution limits and whether the run was cold/warm if explicitly known. No parser guesses that two different commands are equivalent.

A narrower profile improves precision but causes cold starts. A coarser repository-level history may supply only a conservative fallback floor. V1 does not pool fine-grained learning across worktrees by default. Source changes invalidate confidence, not execution correctness; history is a prediction aid, never validation evidence.

Fingerprint the original submitted environment before adding coordinator-internal job IDs, channel references, or launch tokens. These per-run values must not force every execution into a new cold profile. On a detected source/dependency/executable change within a profile, retain its previous conservative peak floor but reset the confidence/sample-count eligibility for downward adjustment; no change may reset the claim to a smaller cold default automatically.

Capture the environment actually submitted, not the daemon's environment. Persist only keyed fingerprints in historical profiles; do not log environment values. An environment hash establishes byte equality of captured values, not equivalence of remote state, file contents, or secrets' semantics.

### 9.3 Update algorithm

Retain up to 50 complete observations per profile over 30 days. Each includes peak, CPU core-seconds, duration, metric quality, resource limits, outcome, and context. For fewer than 5 complete samples, use `max(cold_default, 1.5 * maximum_peak)`. Thereafter use `max(256 MiB, 1.25 * nearest_rank_p95(peaks), recent_failure_floor)`. Round memory upward to 64 MiB. New evidence may increase the next claim immediately; decreases are limited to 10% per five additional complete samples. Active claims never decrease.

For CPU, estimate recent p90 core utilization with a 20% margin, bounded below by 0.25 cores and above by the effective domain capacity. If a CPU quota constrained measurement, retain the requested claim and mark the observation constrained rather than learning unrestricted demand from it.

OOM, cancellation, hard-limit termination, supervisor loss, incomplete accounting, and timeout samples are censored: they may raise a floor but never lower it. For a likely memory-limited failure, next memory floor is `max(previous_claim, observed_peak) * 1.5`. Attribute an OOM to the job only with kernel/group evidence; otherwise say `suspected_memory_failure`. Preserve censored floors for 30 days or until explicit reset. Reject an infeasible new estimate rather than repeatedly retrying unchanged work.

Expose sample count, last observation, confidence, estimator version, and why a claim was chosen. `history forget` deletes selected prediction records, not job outcomes. No training upload, remote model, or network telemetry is needed.

## 10. OS abstraction and capability contracts

Each backend implements `probe`, `sample_domain`, `create_group`, `launch_blocked`, `release_launch`, `sample_group`, `request_stop`, `force_stop`, `verify_identity`, and `reconcile`. Capabilities are individual flags, including group membership, memory metric, peak metric, pressure source, hard-memory limit, CPU quota, tree termination, and recovery support. Do not collapse them into one misleading “supported” boolean.

### 10.1 Linux

Implementation design: use `/proc` for domain/process observations, PSI when exposed, and a dedicated delegated cgroup v2 per job where available. Launch into the group before executing user code; prefer an atomic placement primitive where supported, otherwise use a stopped/bootstrap child with a verified placement handshake. Never launch first and move an already-forking workload later.

Cgroup v2 exposes hierarchical accounting/control, including `memory.current`, `memory.events`, `cpu.stat`, `memory.high`, and `memory.max`. Feature-probe peak counters and group termination support. `memory.high` is a pressure/throttling mechanism, whereas `memory.max` is a hard bound that can lead to OOM behavior. [Linux cgroup v2 documentation](https://docs.kernel.org/admin-guide/cgroup-v2.html)

PSI describes time stalled on constrained resources and offers system/per-cgroup interfaces where enabled; it is not a RAM-utilization percentage. [Linux PSI documentation](https://docs.kernel.org/accounting/psi.html)

V1 policy: do not mount controllers, request root, edit system settings, or change OOM scoring automatically. Without delegation, use an isolated process session/group plus tracked descendants and expose weaker coverage. Respect visible ancestor memory/CPU ceilings and affinity. A requested hard limit requires verified delegation; otherwise reject before spawn. Process identity uses boot identity, PID start time, and pidfd when available.

### 10.2 macOS

Implementation design: use native host VM statistics and swap observations, native process enumeration/resource information, and dispatch memory-pressure notifications. The native dispatch API includes a memory-pressure event source. [Apple DispatchSourceMemoryPressure](https://developer.apple.com/documentation/dispatch/dispatchsourcememorypressure)

The backend must calibrate a conservative available-memory estimate against observed pressure and compression; it must not equate “free pages” alone with useful headroom. Track physical footprint/resident measurements with their specific metric names. Use a new process group/session and independently tracked PID/start-time identities for descendants.

#### 10.2.1 Initial macOS measurement contract

The POC MUST implement estimator `macos_available_v1` before tuning alternatives. Read `HOST_VM_INFO64`, the native page size, and physical capacity. Let `F = free_count`, `I = inactive_count`, and `P = page_size`. The candidate is `A = clamp(P * (F + floor(0.5 * I)), 0, M)`. Take the minimum of the last three valid samples (or all valid samples during startup); that value is `host_available`. Arithmetic is checked for overflow. This is a deliberately discounted reclaimability heuristic, not an Apple-defined available-memory value.

Do not add `speculative_count` to `free_count`: Apple's structure documents their overlap. Do not add purgeable, external/file-backed, compressor, or swap counters as extra available pages. Inactive pages are not guaranteed immediately reclaimable; native warning/critical events override an apparently generous estimate. The underlying counters and overlap are documented in [Apple's VM statistics definitions](https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/vm_statistics.h).

Use physical footprint for per-process claim history when available, otherwise resident bytes under a different profile key; never silently merge those series. Initially use `u_i=0` for materialized-RAM credit in the admission formula because summed footprints do not establish unique resident bytes. This intentionally retains the full growth reservation while still displaying measured usage. Page-out rate is `max(0, delta(pageouts)) * P / elapsed_seconds`; a counter reset invalidates the rate for that interval. A failed required host read yields UNKNOWN, not a zero/free-memory guess. Pressure observation must be initialized through a supported native state query/event path and tested; inability to establish its state is reported explicitly.

Example: `M=16 GiB`, free pages represent 1 GiB, inactive pages 6 GiB. Candidate availability is 4 GiB. With `R=2.4 GiB` and 0.5 GiB reserved future growth, only 1.1 GiB is available for a new claim, subject to the other gates. A pressure warning still blocks it. A POC must test both overoptimistic admission and excessive waiting under cache-heavy but healthy conditions. Failure to calibrate this estimator blocks macOS GA rather than silently relaxing the reserve.

V1 does not advertise a Linux-style aggregate hard memory bound on macOS. `--memory-limit` must return `CAPABILITY_UNAVAILABLE` there. POSIX per-process limits are not an equivalent process-tree memory cap. Escaped/reparented descendants and supervisor death can prevent complete attribution; retain uncertainty and never kill a PID merely because its command name matches. Admission and early pressure response are the primary controls.

### 10.3 Windows

Implementation design: create a user-private Job Object and root process suspended, assign it successfully, then resume. Restrict handle inheritance; do not enable breakaway. Test compatibility with an already job-contained parent; fail launch cleanly if required ownership cannot be established.

Job Objects provide group accounting, limits, and termination; descendants normally inherit association, with documented exceptions. Nested jobs have constraints, and kill-on-close acts when the last relevant handle closes. [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects), [Nested Jobs](https://learn.microsoft.com/en-us/windows/win32/procthread/nested-jobs)

Use native memory status, performance, low-memory notifications, and job/process counters. Keep resident memory distinct from commit charge; system performance information is available through `GetPerformanceInfo`. [Microsoft GetPerformanceInfo](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getperformanceinfo)

The supervisor owns the only kill-on-close Job handle; children must not inherit it. Daemon restart therefore preserves the workload, while supervisor death terminates contained descendants. CPU rates and hard memory caps are optional capabilities with explicit semantics. A job memory limit is commit-oriented and must never be labelled an exact resident-RAM cap. Graceful console signaling is best effort; hard cancellation uses verified Job ownership. Retain native 32-bit exit codes in structured outcomes.

#### 10.3.1 Windows resident and commit ledgers

For each fresh `GetPerformanceInfo` sample, multiply `PhysicalTotal`, `PhysicalAvailable`, `CommitTotal`, and `CommitLimit` by `PageSize` with checked arithmetic. Use `PhysicalAvailable` for physical headroom; do not also add the system cache. Microsoft documents these fields in page units and distinguishes committed from physically charged memory. [PERFORMANCE_INFORMATION](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-performance_information)

For resident observations, sum working sets of currently verified Job members, labelling possible shared-page overcount. Use this value for display and conservative claim increases. Because summed shared pages are not reliable materialized-RAM credit, the initial Windows backend uses `u_i=0` in the future-headroom calculation; this can deliberately double-reserve resident usage. A later backend may credit a measured lower bound of private resident pages only after native validation and a policy-version change. For attributed commit `c_i`, sum `PrivateUsage` from complete verified member samples; label this `sampled_private_commit`, not exact total job commit. If enumeration/counters are incomplete or stale, set `c_i=0` solely for credit calculation so no uncertain consumption is credited back. Keep the last observed measurement separately; zero here means “no accounting credit,” not “zero real usage.” Job peak commit counters may raise a claim but must never be substituted for current usage credit.

```text
commit_limit = CommitLimit * PageSize
commit_used  = CommitTotal * PageSize
K = max(1 GiB, 0.10 * commit_limit)
k_i = max(previous_k_i, current_memory_claim_i, observed_private_commit_i)
commit_growth = sum(max(k_i - c_i, 0))
commit_room = max(0, commit_limit - commit_used - K - commit_growth)
commit_fit(j) = k_j <= commit_room
```

Evaluate this independently of physical-RAM fit. Do not add `sum(k_i)` again to `commit_used`; that double-counts materialized usage. Claims reserve unmaterialized growth, while the live system counter already includes owned and external consumption. Re-read a changed commit limit; do not anticipate future pagefile growth. Unattributed/shared committed memory remains in `commit_used` without attribution credit.

Example: limit 40 GiB, used 28 GiB, reserve 4 GiB; A claims 6 GiB commit and has 4 GiB private commit, B claims 3 GiB with no trustworthy sample. Growth is `2+3=5 GiB`, leaving `40-28-4-5=3 GiB`. A 2 GiB commit claim fits this gate; a 4 GiB claim waits even if physical RAM is abundant. The job must also pass RAM/CPU/fairness gates. Report independent resident and commit estimates in `inspect`.

### 10.4 WSL2 and other virtualization boundaries

WSL2 guests use the Linux backend, with runtime detection of cgroups, PSI, limits, and init/service support. Windows-side memory includes VM consumption; guest memory and host VM footprint must not be added as independent physical memory. WSL configuration includes VM memory, processor, and swap settings; these are operator-controlled settings, not values this project silently edits. [Microsoft WSL configuration](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)

V1 requires two clearly labelled modes:

1. **Guest-only:** one supported guest coordinator with a user-configured budget consistent with the VM allocation. Guarantees apply inside that domain only; Windows native work and other distributions remain external.
2. **Static partition:** operator assigns nonoverlapping conservative budgets to Windows and guest coordinators, leaving host reserve. `doctor` reports configured partitions and unverified assumptions. This reduces conflict but does not dynamically enforce a host-wide total.

Multiple WSL distributions may share underlying VM resources; independent per-distribution budgets are not independently owned physical RAM. V1 documentation must recommend a single coordinated development domain or explicit total-budget partitioning. It must not infer safe capacity by summing per-distribution reports.

Windows `coord run -- wsl.exe ...` owns the Windows launcher, not reliably the guest workload. Detect the known wrapper and warn; recommend running `coord` inside the guest. Arbitrary shell wrappers can hide such delegation, so detection is advisory. Host/guest dynamic leases, an authenticated bridge, aggregate caps, and cross-distro membership are V2 work requiring their own failure protocol.

### 10.5 Containers and external services

A submitted `docker build`, `docker run`, or similar client can ask an existing daemon to do work outside its process group. V1 runs the client and observes total host pressure, but does not claim complete container accounting or cleanup. User hints can reserve expected external consumption; a successful client exit does not prove external work has ceased. Detected external delegation receives `EXTERNAL_EXECUTION_UNTRACKED` and an incomplete-coverage marker.

Automatic Docker socket access, container termination, and service adoption are excluded from V1. A future adapter must own explicit immutable container IDs, account without double-counting, and define cancellation separately. Unknown stacks still execute, with these generic ownership limits.

## 11. CLI contract

```text
coord run [options] -- PROGRAM [ARG...]
coord submit [options] -- PROGRAM [ARG...]
coord status [--json]
coord jobs [--state STATE] [--group GROUP] [--json]
coord inspect JOB [--json]
coord wait JOB [--json]
coord logs JOB [--follow] [--stream stdout|stderr|both] [--raw]
coord cancel JOB [--grace DURATION] [--force] [--json]
coord doctor [--json]
coord daemon start|status|stop [--cancel-running]
coord admission pause|resume
coord config validate|show [--json]
coord history list|show|forget [PROFILE_ID]
coord recovery list [--json]
coord recovery inspect JOB [--json]
coord recovery resolve JOB --action recheck|terminate-owned|release-absent [--revision N] [--json]
coord version [--json]
```

`run` submits and blocks, streaming child stdout/stderr to corresponding streams. Queue messages go to stderr and include the job ID once; callers needing clean diagnostics use a machine-readable event file. `submit` queues detached work and prints one opaque job ID; success means durable acceptance, not job success. `wait` observes an existing job and never launches another execution.

Common execution options: `--cwd PATH`, `--memory SIZE`, `--cpu CORES`, `--group NAME`, `--label KEY=VALUE`, `--queue-timeout DURATION`, `--timeout DURATION`, `--memory-limit SIZE`, `--require-containment kernel`, `--idempotency-key KEY`, `--result-file PATH`, and `--shell SHELL -- SCRIPT`. Options before `--` belong to the coordinator; everything after is executable data. Labels are metadata and never authorization.

An explicit memory limit must be positive, supported by the backend, and at least the effective memory claim; otherwise reject the submission with a typed error. Show whether the limit bounds cgroup charges or Windows job commit. `--result-file` is written atomically by the submitting CLI in its own context; an existing file is not overwritten without an explicit overwrite option. Detached callers obtain durable results through `inspect`/`wait` instead of depending on a vanished submit client. A missing result file never implies job failure or success.

Examples (replace `coord` with the eventual binary name):

```sh
coord run -- npm test
coord run --memory 4GiB --cpu 4 -- cargo test --workspace
coord submit --group integration -- python -m pytest tests/integration
coord run --shell /bin/sh -- 'make build && make test'
coord status --json
coord inspect JOB_ID --json
coord cancel JOB_ID --grace 10s
```

Windows direct execution supports native executables with correctly encoded Unicode arguments. Batch files and shell built-ins require an explicit shell; for example `coord run --shell cmd.exe -- 'npm test'` in a caller shell that passes that script as one argument. PowerShell scripts similarly use an explicit executable/script invocation or `--shell pwsh`. The CLI must not fall back to `cmd.exe` implicitly after a direct spawn failure. Shell startup flags and script-passing rules are documented and tested per supported shell.

V1 default stdin is closed. `run --stdin` forwards a byte stream only after RUNNING and is permitted only for attached execution; do not consume pipeline data while queued. Detached `submit` rejects `--stdin`. No PTY is allocated. Loss of an attached stdin channel closes child stdin and records that event.

### 11.1 Disconnects, signals, deadlines, and exit status

- Ctrl-C requests cancellation of that job; a second Ctrl-C requests immediate forced cancellation.
- An abrupt client crash or terminal disconnect does not terminate accepted work by default. Reattach with `wait`, `logs`, or `inspect`.
- `--queue-timeout` starts at durable acceptance and expires only before STARTING. `--timeout` measures execution from launch authorization until the owned group drains; it invokes cancellation when exceeded. Output finalization has a separate bounded deadline.
- Normal root exit is preserved when the workload has drained successfully: Unix exit values as reported; Windows native exit value via the native CLI exit mechanism. Root nonzero becomes FAILED, even if cleanup succeeds.
- Wrapper/management errors use 125; not-executable and not-found use 126 and 127. Cancellation uses 130, timeout 124, and Unix signal termination is conventionally `128 + signal`. These values can collide with legitimate child exits. `inspect --json` and `--result-file` are authoritative about origin and native status.
- A failed `wait` transport is not a failed workload. Print its ID and reconnect instructions. Never invent a successful status after losing the terminal receipt.

Persist queue deadline UTC alongside the same-boot monotonic deadline. During a daemon restart in the same boot, retain the monotonic deadline. After reboot, use the persisted UTC deadline conservatively: an expired deadline becomes QUEUE_EXPIRED, while a detected large backward clock jump holds affected jobs for explicit deadline renewal. Runtime deadlines remain the supervisor's responsibility and are never restarted by a coordinator reconnect. Backends must use a suspend-inclusive elapsed clock where available; otherwise detect resume and account for the suspension explicitly.

Management commands return 0 on successful operation, 2 on usage/configuration error, and 125 on operational failure. `cancel` on an already terminal job is a successful no-op with its existing outcome; unknown IDs are errors.

### 11.2 Structured output

JSON responses contain `schema_version`, `request_id`, `daemon_version`, and typed data or a typed error. A job record includes state, execution ID, group, timestamps, context, requested hints, effective claims, limits, capability/coverage, WAIT reasons, native exit status, terminal reason, and log truncation status. Byte counts are integers, unavailable fields are null with a reason, and state values are stable strings.

Event streaming uses NDJSON records with monotonically increasing event sequence IDs and resume cursors. A retention gap yields an explicit `EVENT_GAP` followed by a current snapshot; it must not silently omit transitions. Human output is not a parsing API. Optional agent instructions merely teach this contract; they do not bypass the scheduler.

### 11.3 Executable resolution and queued changes

At acceptance, resolve a direct command from the submitted cwd and submitted PATH, never the daemon's PATH/cwd. On Unix, a path containing a separator is resolved relative to submitted cwd; otherwise search PATH in order. Relative and empty PATH entries resolve against that cwd. On Windows, resolve explicit paths similarly; for bare names search submitted PATH in order, trying an exact executable name and then `.exe` when no extension is present. V1 does not apply PATHEXT batch/script fallback or implicitly search the current directory outside the submitted PATH. Reject NULs and names that cannot be represented by the native API. Windows case-insensitive environment-key duplicates are invalid.

Persist the absolute resolved path, file identity (device/inode or Windows file ID), size, modification timestamp, and SHA-256 content digest of the executable. Bound fingerprinting to 2 s and 256 MiB; an unhashable/large executable may still run with an explicitly incomplete identity and a conservative cold estimate. Do not resolve a new earlier PATH entry while the job waits: the accepted absolute target remains its target. A deleted target fails launch; a changed target triggers the policy below.

Recheck immediately before the STARTING grant and again in the blocked launch helper before user-code release. If identity changes, return to prelaunch estimation with a new profile and `claim = max(previous_claim, new_profile_claim, cold_default)`; release the unconsumed launch attempt only after proving its blocked child absent. If it fits, issue a fresh launch token. Otherwise WAIT; if now statically oversized, REJECT with `EXECUTABLE_CHANGED_RESOURCE_REQUEST`. After three target changes before a successful launch, fail `EXECUTABLE_UNSTABLE`. There is no silent execution using the old smaller claim.

For explicit shells, fingerprint the shell binary and retain the script bytes exactly. For directly executable Unix scripts, record the shebang and fingerprint a fixed interpreter path where accessible. For `env`-selected interpreters or dynamically loaded modules, mark the dependency closure incomplete. Windows scripts require an explicit interpreter. The coordinator does not attempt to discover every library, package, configuration file, or shell-selected binary.

Use an opened executable handle/fd for launch where the OS and script semantics support it; otherwise record `path_rechecked`, recognizing a remaining check-to-execution race. V1 is not a same-user tamper-proof execution system. Neither executable hashing nor an unchanged PATH proves environment equivalence, and neither permits result reuse. Changes to dependencies remain part of the conservative source/history confidence policy in §9.2.

## 12. Configuration schema

User configuration is TOML with `schema_version = 1`. Platform locations follow user configuration/data conventions; state never defaults into the repository. This example defines the initial default policy and field types:

```toml
schema_version = 1

[daemon]
autostart = true
max_queued_jobs = 1000
max_request_bytes = 1048576

[resources]
memory_budget_fraction = 0.75
memory_reserve_fraction = 0.15
memory_reserve_min = "2GiB"
cpu_reserve_cores = 1.0
max_parallel = "auto"                  # integer >= 1 or "auto"
commit_reserve_fraction = 0.10
commit_reserve_min = "1GiB"

[sampling]
host_interval = "500ms"
job_interval = "1s"
pressure_interval = "250ms"
stale_after = "2s"

[pressure]
policy_version = 1                      # thresholds and hysteresis in section 8.3
action = "hold"                        # only V1 automatic action

[scheduler]
max_bypass = 2
reservation_after = "30s"
oversized = "reject"                   # only V1 behavior
unknown_parallel = 1

[learning]
enabled = true
window_samples = 50
retention = "30d"
margin = 1.25
cross_worktree = false                 # must remain false in V1

[execution]
disconnect = "continue"                # V1 default and supported policy
cancel_grace = "10s"
drain_timeout = "30s"
containment = "best_available"          # or "kernel_required"
shell = "none"

[storage]
job_retention = "30d"
log_retention = "7d"
max_log_per_job = "64MiB"
max_log_total = "1GiB"
min_free_disk = "256MiB"

[privacy]
telemetry = false
store_environment_in_history = false

[groups.integration]
weight = 1
max_running = 2
```

Optional `resources.memory_budget_bytes` and `resources.cpu_budget_cores` specify explicit ceilings, still bounded by observable domain capacity and reserve. Group names must match a bounded safe identifier grammar; unspecified groups inherit weight 1 and the global running limit. Sizes reject negative, zero where invalid, overflow, and ambiguous units. Durations are positive integers with `ms`, `s`, `m`, `h`, or `d`. Fractions are in `(0,1)` and must produce a valid budget. `stale_after` must exceed two host sampling intervals. Unknown keys are errors, not silently ignored.

Precedence: built-in defaults < user config < explicitly selected trusted profile < permitted per-job CLI hints. CLI hints cannot override global safety budgets. Environment variables may select the config path but do not silently supply safety policy. `config show` includes effective values, provenance, and policy digest without secrets.

Repository configuration is ignored automatically in V1. A user may explicitly select a trusted file with `--config`; daemon-global policy cannot be replaced by an ordinary submission. The coordinator snapshots job policy at acceptance; changing global budgets affects future admission immediately after validated reload but never silently kills running work or lowers existing claims. Invalid reloads retain the previous policy and emit an error. Config does not contain arbitrary executable hooks.

## 13. Git, worktrees, and source-state semantics

Git is optional. Commands outside repositories and machines without Git must still work. Resolve Git context with bounded read-only commands, machine-readable NUL-delimited output, and timeouts. `git rev-parse` provides repository/worktree path context and `git status --porcelain=v2 -z` supplies a stable status format. [Git rev-parse](https://git-scm.com/docs/git-rev-parse), [Git status](https://git-scm.com/docs/git-status)

Record local repository identity from the canonical common Git directory plus a locally allocated stable ID, and a distinct worktree ID from its Git directory/root. Remote URLs are optional metadata, not identity; clones with the same remote are separate local repositories. Respect filesystem case sensitivity and canonical path identity; a path rename requires re-resolution, not guessing.

At submission, immediately before execution, and after termination, capture:

- HEAD object ID or unborn/detached state; current branch if available.
- Index/worktree dirty status, conflicts, untracked presence, submodule status, and collection completeness.
- Repository/worktree roots, cwd relative to the worktree, observation timestamps, and collection errors.
- Optional bounded content digest of tracked diffs and selected lockfiles; byte limits, skipped files, and unstable reads are explicit.

Do not invoke custom diff/text conversion programs, hooks, credential helpers, network fetches, or arbitrary repository scripts for metadata. Bound Git work to 2 s per observation and 16 MiB of collected metadata by default. Failure yields `context_unknown`; it never silently skips the executable.

A queued command normally runs against files present when it starts. If HEAD or observed dirty state changed since submission, record `SOURCE_CHANGED_WHILE_QUEUED` and expose both observations. An optional `--require-head OID` rejects launch if HEAD differs; this is only a HEAD check, not a proof of unchanged dirty files. V1 does not snapshot or lock the source tree, so concurrent changes can occur during execution. Describe outcomes as “command exited under these observations,” never “this immutable source state is fully validated.”

**V1 prohibition:** no result reuse, test deduplication, or coalescing even for identical argv, HEAD, paths, or status summaries. An idempotency key retries one submission transaction only; it is not cross-request test reuse.

V2 may consider reuse only with an explicit hermetic execution contract: immutable content-addressed input closure including tracked, untracked, generated, ignored, submodule and LFS inputs; toolchain/dependency identities; environment and platform; command and working directory semantics; external-service state or isolation; outputs; and test nondeterminism policy. If any required input cannot be proven equivalent, execute again. A Git hash, clean flag, or lockfile match alone is insufficient.

## 14. Process supervision and state machine

### 14.1 Durable states

```text
QUEUED -> STARTING -> RUNNING -> DRAINING -> SUCCEEDED | FAILED
   |          |          |          |
   +----------+----------+----------+--> CANCELLING -> CANCELLED | TIMED_OUT

STARTING/RUNNING/DRAINING/CANCELLING -> RECOVERING
RECOVERING -> prior live state | known terminal state | LOST
QUEUED -> CANCELLED | QUEUE_EXPIRED | REJECTED
STARTING -> LAUNCH_FAILED
```

Rejected submissions before durable acceptance receive an error and no executable job. A persisted job can become REJECTED during prelaunch validation. RUN/WAIT are scheduler decisions, not replacements for lifecycle states. `LOST` means the outcome cannot be established; it is never success. `RECOVERING` and `LOST` may retain resource reservations through a separate unresolved-ownership record until absence is verified or the user explicitly resolves it.

### 14.2 Launch protocol and at-most-once intent

1. Validate request, capacity feasibility, environment size, executable, and cwd. Persist QUEUED plus a unique submission ID and policy snapshot.
2. Admission transaction allocates a cryptographically random launch token and STARTING reservation.
3. Create a supervisor with an authenticated control channel. It creates ownership primitives and a blocked root child; user code has not executed.
4. Supervisor persists the root identity and a launch-intent receipt, then receives the matching fenced authorization. Immediately before releasing the root it durably marks that token consumed. A crash after this mark but before release can lose an execution, but must not duplicate one.
5. Supervisor releases the root, reports RUNNING, streams output, and periodically reports ownership/usage.
6. On root exit, enter DRAINING while verified descendants remain. Persist native root status separately from workload completion.
7. Persist the terminal receipt before releasing the reservation. Coordinator ingestion is idempotent.

Each supervisor also holds an exclusive per-job/token OS lock before it can create a root child. Duplicate supervisor requests attach to the existing owner or fail; they must not each create a blocked root and race to consume a token. If the owner disappears with a consumed token, no replacement supervisor may execute that token. Fencing, the supervisor lock, and durable token consumption are separate required protections.

Retries with the same client idempotency key and identical payload return the same accepted job; different payloads conflict. Retain key mappings for the job-retention period and document that expiry ends this guarantee. Duplicate launch tokens never execute twice. There is no exactly-once guarantee for arbitrary command side effects; ambiguous launches become LOST rather than rerun.

### 14.3 Drain and cancellation

The root exiting does not free the workload reservation while descendants still run. Default drain timeout is 30 s. After expiry, terminate verified owned descendants using the normal cancellation sequence and finalize FAILED with `DESCENDANTS_OUTLIVED_ROOT`, retaining the root's exit value separately. A foreground long-lived service keeps its root alive and is not subject to this post-root deadline.

Queued cancellation removes it from admission transactionally. Running cancellation records intent, requests cooperative stop (Unix SIGTERM or supported Windows console event), waits the configured grace, then force-stops the verified group. `--force` skips grace; it does not relax ownership checks. Continue observing until group emptiness is confirmed. Escaped or unverifiable processes lead to incomplete cleanup/LOST, not a false CANCELLED success.

Resolve cancellation/completion races by durable ordering: if a verified terminal receipt already exists, retain its outcome and record cancellation as a no-op. Otherwise a committed cancellation intent owns the terminal classification even if the root subsequently exits zero; preserve that native exit separately. A timeout commits its own reason before entering the same cancellation path. Cleanup uncertainty always takes precedence over claiming successful cancellation.

Do not identify targets by executable name, parent PID alone, port number, or a persisted PID without birth identity. Unix fallback retains a group leader/reaping relationship while signaling; when identity cannot be established atomically enough to avoid PID reuse, prefer leaving a process unresolved over signaling a possible unrelated process. Kernel ownership handles are preferred.

### 14.4 Nested invocations

Pass a private authenticated supervisor-channel reference to descendants. If a running job invokes `coord run` again, do not enqueue a child reservation behind its own waiting parent. V1 rejects it with `NESTED_SUBMISSION_UNSUPPORTED` and explains that the outer command already owns the workload. This avoids nested-queue deadlock and double accounting. Environment markers alone are not proof of ownership; verify the channel and process relationship. Resource subleases are V2.

## 15. Persistence, IPC, startup, and recovery

Use SQLite with a single coordinator writer, transactional schema migrations, durable launch-state commits, and a bounded append-only event sequence. Use WAL only on a supported local filesystem. Network-shared state directories are unsupported. Tables: `jobs`, `job_events`, `reservations`, `launch_tokens`, `supervisors`, `contexts`, `profiles`, `observations`, `idempotency_keys`, and `schema_migrations`. Foreign keys and monotonic transition validation are mandatory.

Supervisors write private per-job receipts/spool files atomically, with restrictive permissions and durable replacement where required. They never become concurrent SQLite writers. The coordinator reconciles receipts by job ID, token, boot identity, and supervisor identity. Schema migrations take an exclusive lock, make a recoverable backup, and never run implicitly from an older binary against a newer schema.

IPC uses user-private Unix domain sockets on Linux/macOS and local-only ACL-restricted named pipes on Windows. No TCP listener in V1. Authenticate OS peer identity plus supervisor capabilities where applicable; do not trust a PID supplied in JSON. Use length-prefixed, versioned messages, a 1 MiB request limit, bounded stream frames, timeouts, and backpressure. Major protocol mismatch refuses mutating calls with upgrade guidance.

Autostart acquires an OS lock and establishes a leader epoch before binding the endpoint. Concurrent clients attach to the winner. Use process-held locks/mutexes, not a TTL file that can expire while a paused leader remains alive. Supervisors reject stale-epoch control. No production option creates multiple independent budgets for the same domain; isolated test instances must explicitly identify themselves as test mode.

### 15.1 Recovery sequence

1. Acquire exclusive leader ownership, load policy, and block admission.
2. Validate the store and boot identity. Enumerate persisted live/starting records and private supervisor receipts.
3. Reconnect to surviving supervisors and verify launch tokens and process birth identities. Preserve their reservations; ingest terminal receipts if available.
4. Never infer “not running” from a missing heartbeat alone. If a supervisor is absent, verify kernel group/process ownership and either clean up verified owned survivors or mark uncertainty.
5. Jobs proved never released to user code, with an unconsumed token and a verified intact payload, may return to QUEUED after any blocked child is verified absent. Any consumed/ambiguous launch is not automatically replayed. A missing/corrupt payload for a proved-unlaunched job becomes LAUNCH_FAILED with `PAYLOAD_UNAVAILABLE`; it requires a fresh submission. After reboot, previously executing jobs become LOST with `HOST_RESTART`; queued jobs remain queued if their payloads are intact and context checks pass.
6. Resume admissions only with fresh telemetry and conservative accounting for every unresolved live reservation.

Daemon crash: supervisors keep jobs alive and continue bounded logging. Supervisor crash: Windows kill-on-close terminates contained jobs; on Unix, cleanup/recovery is best effort unless a kernel/service mechanism proves ownership. Reboot destroys live OS processes but does not prove whether a command's external side effects completed.

`daemon stop` stops admission and exits only after jobs drain; if jobs are active, it reports waiting and can be interrupted without killing them. `--cancel-running` explicitly cancels before stopping. Restarting the daemon does not rerun jobs.

### 15.2 Environment and output persistence

Full executable payloads may include credentials in argv and environment. Pending jobs require a private execution spool containing this data to survive restart. V1 uses owner-only filesystem protection, not a claim of encryption against the same user. The coordinator MUST retain the complete payload until it has durably ingested the supervisor's consumed-token receipt (the boundary after which replay is prohibited), or a proved-unlaunched terminal outcome. Mere supervisor receipt/acknowledgement is insufficient. Write payload, length/digest, and spool-directory entry durably before acknowledging acceptance. Deletion is idempotent and occurs only after the durable boundary; a crash before deletion leaves a private file for recovery cleanup, not permission to rerun. Do not retain environment values in history or support exports. Running jobs cannot be replayed from history without a fresh submission. Explain this storage behavior in CLI help and security documentation.

Drain output continuously even when clients are slow. Spool raw bytes up to per-job and global quotas; once a quota is reached, continue draining but discard excess with explicit byte counts and a truncation event. Raw output remains raw only under explicit raw display/export; ordinary terminal views escape control sequences. Output logging failure must not deadlock the child or turn a nonzero exit into success.

When free disk is below the configured floor, stop new admissions. If terminal receipt persistence fails, keep the in-memory outcome and retry; a later unrecoverable outcome is LOST. Never acknowledge a submission as durable before the acceptance transaction commits.

### 15.3 Operator recovery and reservation resolution

Every uncertain workload has a durable recovery record with `revision`, reason, last verified identities, boot ID, reserved RAM/CPU/commit, evidence timestamps, supported actions, and cleanup status. `recovery list` lists these records; `inspect` includes them for a single job. They never expose launch capabilities or raw environment values.

`recovery resolve --action recheck` is read/refresh-only: reconnect to the supervisor, inspect ownership, and ingest evidence. It cannot launch user code or kill a process. `terminate-owned` explicitly requests normal cancellation of only currently verified ownership; unresolved members remain unresolved. `release-absent` releases an unresolved reservation only after the backend proves no owned workload remains. Mutating actions require the inspected `--revision`; mismatch returns `RECOVERY_REVISION_CONFLICT` with no side effects, so stale automation cannot act on a newer identity.

Acceptable absence evidence: verified empty kernel group plus consumed/retired launch token and no live launch helper, or a changed boot identity proving native processes from the earlier boot cannot survive. In fallback tree mode, tracked-process absence is insufficient when escaped descendants cannot be excluded. That record remains unresolved. The user can retain the conservative reservation or reboot the resource domain and recheck; V1 has no force-forget command that disguises uncertainty as proven cleanup. Confirmed external-service delegation remains explicitly outside owned-process guarantees even after native reboot/cleanup.

Release changes accounting only. A LOST job remains LOST; an operator cannot turn it into SUCCEEDED or erase its audit trail. Store the evidence digest, actor's authenticated local identity, action, old/new revision, and released quantities transactionally. A repeated action using the current revision of an already resolved record returns its state without releasing resources twice; a stale revision still conflicts. Successful termination with a missing original exit receipt may resolve ownership but still leave the outcome LOST.

Example: J17 has a 4 GiB reservation and a missing supervisor. `recovery inspect J17` returns revision 3, `ownership=unverified`, and only `recheck` as currently useful. A replacement PID with a different birth identity is not killed. After a verified domain reboot, `recheck` records absence and revision 4; `release-absent --revision 4` releases exactly 4 GiB and leaves J17's outcome LOST. Fresh host telemetry is still required before new admission.

## 16. Security and threat model

### 16.1 Trusted and untrusted boundaries

Trust the OS kernel, the installed signed/verified executable, and explicit user configuration. Treat commands, repository contents, output bytes, agent-provided labels/hints, IPC payloads, stale disk state, and external processes as untrusted inputs. A command executes with the submitting user's privileges; this tool is not a security sandbox. Another malicious process with the same user identity can often alter configuration, inspect secrets, or bypass the tool. Do not promise isolation from it.

Threats in scope: another user controlling the daemon through IPC, malformed/flooded submissions, shell injection, output escape injection, path traversal/symlink replacement, PID reuse, forged supervisor receipts, resource-estimate poisoning, stale locks, and accidental exposure through logs or diagnostics.

### 16.2 Required controls

- Owner-only state/spool directories and endpoints; Unix mode 0700/0600 equivalents and Windows SID ACLs. Validate existing ownership before use.
- Direct argv execution by default. Shell scripts are explicit payloads and never assembled by interpolating unescaped arguments.
- Private randomly named endpoints and unguessable supervisor capabilities; local-peer verification remains required.
- Refuse symlink/reparse-point redirection into unsafe state locations; use handle-relative/no-follow operations where available.
- Bounded request size, queued jobs, labels, environment size, log sizes, per-client connection counts, and parser depth. Default max 32 simultaneous IPC connections per user daemon.
- Do not load arbitrary repository plugins or execute project configuration during inspection.
- Redact metadata by default; retain only hashes/allowlisted values in diagnostics. Raw command output can contain secrets, so permission protection and explicit export choices matter more than promises of perfect redaction.
- No automatic network telemetry, update execution, root escalation, firewall changes, or Docker socket access.
- Dependencies and release artifacts have auditable provenance; report vulnerabilities privately through a documented channel.

Denial of service by a same-user command remains possible, especially through fast allocations, fork storms, external daemons, or deliberately bypassing the wrapper. Optional kernel PID/CPU/memory limits may reduce exposure, but unsupported enforcement is never simulated by a success message.

## 17. Safety invariants

| ID | Invariant |
|---|---|
| S01 | Every reported successful execution has a corresponding authorized launch and verified terminal receipt; no skipped command is success. |
| S02 | Two admission decisions cannot spend the same unreserved capacity snapshot independently. |
| S03 | Only verified owned processes/groups receive cancellation signals. |
| S04 | Missing/stale telemetry or uncertain ownership does not become zero usage. |
| S05 | Root exit alone does not free a known live descendant's reservation. |
| S06 | A crash never causes automatic replay of a possibly executed command. |
| S07 | No source fingerprint or history record authorizes V1 result reuse. |
| S08 | No shell, runtime limit, worker-count change, or retry is introduced implicitly. |
| S09 | Unsupported hard limits fail before execution; best-effort monitoring is labelled. |
| S10 | No LLM, network service, or semantic stack recognizer is necessary for safe admission. |
| S11 | Claims, measurement, and OS enforcement are represented separately. |
| S12 | Admission does not promise immunity to external load, escaped work, sampling gaps, or kernel OOM. |
| S13 | Client loss never becomes an untracked execution: a durable job ID remains queryable. |
| S14 | Full job results retain failure origin, native exit information, and measurement completeness. |

A successful shell exit is evidence about that shell command, not proof that every test passed or that an external asynchronous service finished. The coordinator preserves what the process reported and documents the scope of that evidence.

## 18. Failure modes and required behavior

| Failure | Required response |
|---|---|
| Daemon unavailable/autostart fails | Fail submission with actionable error; never silently execute unmanaged. |
| Duplicate daemon startup | Only lock owner admits; others connect or fail. |
| Missing PSI/cgroups/optional counters | Report degraded capability and use documented fallback; required limits fail. |
| Essential telemetry stale | WAIT with `TELEMETRY_STALE`; existing jobs continue. |
| Sudden external memory spike | Close admission; alert; do not kill unrelated processes. |
| Job rapidly exceeds estimate | Raise claim and learning floor; hold admissions; record possible sampling gap. |
| OS kills a job | Preserve signal/native status and available OOM evidence; no automatic retry. |
| Spawn/assignment fails | Dispose blocked child safely; LAUNCH_FAILED; release only verified empty ownership. |
| Client loses acceptance response | Idempotency-key lookup returns original job; no duplicate submission. |
| Daemon dies during launch handshake | Reconcile token/receipt; ambiguous work becomes LOST, never replayed. |
| Supervisor disappears | Verify and clean up only owned remnants; preserve unresolved accounting. |
| PID recycled | Reject stale identity; do not signal replacement. |
| Disk full/database corruption | Stop admission; protect running supervision; preserve files for repair; no reset-on-error. |
| Output flood/slow reader | Bounded spool, continued drain, explicit truncation. |
| Git unavailable/metadata races | Mark unknown/unstable context; execute unless an explicit precondition fails. |
| cwd deleted or executable replaced | Fail nonexistent cwd; revalidate executable identity at launch and update context/estimate when changed. |
| Sleep/resume/clock adjustment | Use monotonic deadlines; invalidate stale samples; document platform suspend-clock semantics. |
| Global budget reduced | Hold new jobs until claims fit; keep running claims intact. |
| Nested wrapper | Reject nested submission; explain outer ownership. |
| Host/guest bridge absent | Domain-scoped operation only; no claim of dynamic host-wide coordination. |

Operational recovery never deletes user source, caches, worktrees, or unrelated processes. Diagnostic repair commands, if introduced later, must show their target state before mutation.

## 19. Observability and diagnostics

`status` shows capacity, reserve, available headroom, pressure state, telemetry age, active/queued totals, committed/future growth reservations, containment quality, and domain identity. `inspect` explains effective claims, prediction samples, queue blockers, source observations, native exit reason, and cleanup status. `doctor` checks endpoint ownership, filesystem suitability, OS capabilities, effective ancestor limits, clock/sampling health, and WSL/container boundaries without mutating the system.

Structured events cover accepted, waiting-reason-changed, grant, launch, overrun, pressure-transition, cancellation, root-exit, drain, terminal, recovery, capability-loss, and output-truncation. Log wait events on reason/quantity-class changes rather than every poll. Assign a correlation ID across CLI, daemon, supervisor, and durable receipt.

Metrics include queue wait/run/drain duration, p50/p95 job latency, resource estimate error, pressure time, prevented admission count, cancellation latency, unresolved jobs, launch failures, telemetry age, sampler cost, coordinator RSS, log loss, and fairness bypass counts. Expose a local JSON snapshot; a network metrics server is not required in V1.

Support bundles exclude raw output, argv, environment, absolute paths, and repository content by default. An explicit opt-in export lists included fields. Use keyed local identifiers to correlate jobs without exposing paths. Keep raw high-frequency samples for at most 24 hours by default; aggregate history follows configured retention.

## 20. Testing strategy

### 20.1 Deterministic core

Use a virtual clock and reproducible resource traces. Property/model tests must cover no over-admission under the stated input model, nonnegative reservations, one launch per token, valid transitions, bounded bypass, censored learning, hysteresis, timeout races, stable reasons, and monotonic active memory claims. Scheduler tests operate independently of real OS timing.

### 20.2 Native integration fixtures

Build a small portable fixture executable capable of allocating/touching memory, ramping allocations, burning specified CPU, forking/spawning workers, exiting before children, flooding both streams, ignoring graceful termination, requesting stdin, and exiting with selected codes. Use it to verify actual process-tree ownership, sampling, cancellation, and release of reservations on each OS.

Include shell quoting, Unicode/space paths, case-sensitive paths, Windows batch rules, deleted cwd, simultaneous startup, hundreds of short children, PID reuse stress, root/descendant exit races, inherited handles, nested Job Objects, missing delegation, partial proc visibility, and nested coordinator invocations. Test supported shells separately from direct execution.

### 20.3 Fault and security testing

Inject crashes before/after each durable launch transition, after OS spawn but before acknowledgement, during exit receipt persistence, and during migration. Test reboot reconciliation, stale epoch messages, malformed IPC, forged receipts, endpoint ACL failures, path replacement, environment secrecy, log escapes, quota exhaustion, and corrupted database handling. Verify no unrelated sentinel process is terminated.

### 20.4 Platform and real-workload matrix

Initial release targets: Windows 11 x64, macOS 14+ arm64 and x64 where a maintained runner is available, and Linux x86_64/arm64 with a documented libc baseline. WSL2 on Windows 11 with a supported Ubuntu distribution is a separately tested configuration. Exact minimum builds and packaging ABI baselines are frozen in the release ADR after native tests, not inferred from compilation alone.

Run Linux with delegated cgroup v2 and without delegation; macOS with pressure events; Windows under normal and nested jobs; WSL2 guest-only and static partition cases. Test Node, Python, Rust, Go, JVM, and a generic unrecognized binary. Toolchains are fixtures, not product dependencies. Include two worktrees at the same HEAD with different dirty/untracked/generated inputs and assert both requested test jobs execute.

Hosted CI verifies logic and routine process behavior. Dedicated disposable VMs or test machines run pressure/OOM/freeze tests; never deliberately exhaust a contributor's interactive workstation. An 8-hour mixed-workload soak is required per primary OS family before GA.

## 21. POC benchmark methodology

### 21.1 Questions and preregistration

Measure whether coordination reduces memory-pressure disruption while preserving every requested execution, and whether the design offers material value beyond qex. Publish the protocol, versions, workloads, seeds, configuration, and pass criteria before measuring. Do not select only workloads that favor this proposal.

### 21.2 Arms

1. **Uncoordinated baseline:** identical commands launched at the same scheduled arrival times without a wrapper.
2. **Static baseline:** identical workload with a fixed concurrency cap, including cap 1 and a tuned cap. This distinguishes adaptive value from simple serialization.
3. **Proposed coordinator:** documented default policy; cold-history and warmed-history runs separately.
4. **qex:** pinned release/commit and its documented invocation; default and a comparable explicitly configured resource-budget arm where supported.

For qex, validate behavior against its pinned help/source. Record claim defaults, learning state, queue policy, oversized policy, and any environment mutation. Report semantic differences rather than pretending the two products received identical effective settings. Missing support or installation failures are “not evaluated,” not a performance loss. [qex repository](https://github.com/stephenc/qex)

### 21.3 Workloads

| Suite | Purpose |
|---|---|
| Simultaneous cold burst | 4–8 independent clients submit unknown commands with large overlapping peaks. |
| Mixed arrivals | Short low-memory tasks interleaved with long builds and tests; test fairness and tail latency. |
| Learned repetition | Repeat exact profiles, then change peak demands to test adaptation and stale predictions. |
| Adversarial estimates | Underreported hints, burst growth, brief worker spikes, fork-heavy tests. |
| External load | Add a separately controlled browser-like memory/CPU consumer not submitted to any scheduler. |
| Worktree mix | Several repositories and dirty worktrees, same and different commands. |
| Lifecycle stress | Cancel, disconnect, kill daemon/supervisor, and recover at reproducible checkpoints. |
| Virtualization | WSL2 guest load plus Windows load; disclose domain scope and static partitions. |
| External engine | Container-client workload, scored separately for incomplete ownership. |

Use both synthetic touched-memory/CPU fixtures and pinned real projects with deterministic dependency installation. Real command lists, test counts, exit outcomes, and output digests must be compared across arms. Exclude any apparent speedup caused by omitted work, altered tests, failure, or hidden caching.

### 21.4 Controls and instrumentation

Use at least 16 GiB and 32 GiB memory configurations, record CPU topology, OS/kernel, swap/pagefile, storage, power mode, background services, VM limits, and thermal conditions. Constrain disposable VMs consistently rather than comparing unrelated laptops. Report per-platform results; do not pool incomparable memory metrics.

Randomize arm order in blocks. Use at least 10 measured repetitions per suite/arm, with predeclared seeds and separate warm-up. Preserve identical arrival traces. Report median, p95 where sample count permits, raw distributions, and bootstrap confidence intervals for paired differences. Do not present an unstable p99 from ten observations as reliable.

Separate dependency/build-cache state from scheduler-history state. Run cold and warm build-cache scenarios explicitly; reset or preserve the same caches for all arms. Never warm only one scheduler. External monitoring must be identical across arms and its overhead measured.

Measure independently of the coordinator at 100–250 ms where practical: host memory availability, commit, swap/page-outs, pressure time, CPU, per-arm makespan, per-job queue/run/end-to-end latency, completion/failure counts, owned and stray process counts, and scheduler overhead. Use an external responsiveness probe scheduled every 100 ms. Record probe lateness p95/p99 and the count/duration of gaps over 2 s; use a host/hypervisor watchdog to distinguish guest stalls from observer failure. Do not equate CPU utilization with responsiveness.

Define a freeze proxy as a responsiveness gap over 2 s, and an abort threshold as a gap over 10 s or an unsafe test-VM condition. Count aborted runs as adverse outcomes; never drop them from charts. Record kernel OOM evidence separately from test failures and supervisor-requested termination.

### 21.5 POC decision thresholds

The predeclared test envelope uses finite workloads whose isolated peaks fit the budget; deliberate oversize/unbounded workloads test failure behavior separately.

- **Correctness gate:** all intended commands execute once per request, outcomes preserved; zero unrelated-process kills, silent skips, or automatic replay of uncertain launches.
- **Safety gate:** zero coordinator-attributable OOMs in the within-envelope suite; at least 50% lower aggregate HIGH/CRITICAL-pressure time or responsiveness-stall time than the uncoordinated baseline in the contention suite. Report absolute values when baseline incidents are zero; do not claim percentage improvement then.
- **Cost gate:** median makespan no more than 20% worse than the fastest comparison arm that also meets the safety gate in contention tests. On unconstrained workloads, overhead should be below 5% for jobs lasting at least 10 s.
- **qex gate:** document at least one reproducible material advantage relevant to the stated goals, or recommend contributing/integrating instead of claiming a new product is justified. No mandatory claim of speed superiority.
- **Portability gate:** POC proves launch, monitoring, cancellation, and one pressure-limited queue scenario natively on Windows, macOS, Linux, and WSL2; results disclose fallback coverage.

If thresholds fail, publish the failure and revise the design or project direction. A manually tuned showcase is insufficient evidence for a production release.

## 22. V1 acceptance criteria

GA requires every R01–R14 requirement and S01–S14 invariant to be traceable to tests or an explicitly scoped capability contract. In addition:

| Area | Release criterion |
|---|---|
| Admission | 100 concurrent submissions do not produce duplicate leadership or exceed reservations under deterministic feasible traces. |
| Fairness | A blocked eligible head is bypassed no more than twice before reservation; eventually runs in finite feasible traces. |
| Responsiveness | Pressure/stale-telemetry admission gate changes within 2 s of the configured trigger duration, excluding documented OS scheduling stalls. |
| Cancellation | Queued job removed within 1 s; kernel-contained running fixtures stop by grace + 2 s under normal test load. Fallback failures are explicit. |
| Crash recovery | Each injected launch boundary recovers to verified live/terminal/LOST state with no duplicate execution. Healthy surviving supervisors reconnect within 10 s after daemon readiness. |
| Resource learning | Underpredictions enlarge claims; censored observations never lower them; exact reset and provenance are inspectable. |
| Command fidelity | Direct argv, cwd, Unicode, environment, binary output, and native outcomes pass all platform fixtures. |
| Git correctness | Same-HEAD worktrees never share results; context races are labelled and no Git metadata collection mutates source/index. |
| Idle overhead | On documented reference hardware: coordinator plus idle monitor median CPU below 0.5% of one logical core and RSS below 64 MiB over 10 min. |
| Active overhead | With eight fixture jobs: coordinator/supervision monitoring below 3% of one core and 128 MiB combined overhead, excluding output buffers and child workloads. Investigate failures rather than hiding them in workload totals. |
| Output and storage | A 1 GiB output flood obeys quotas, does not deadlock, and reports truncation; low disk stops admission. |
| Security | Peer/ACL/path/IPC fuzz tests pass; support exports exclude secret fixture values by default. |
| Portability | Native tests and published capability matrix pass for every advertised release target. |
| Soak | Eight-hour mixed run has no leaked owned workload groups, unbounded logs, or lost outcomes under supported normal operation. |

Performance thresholds are engineering targets subject to a documented pre-GA ADR if evidence warrants revision. Safety invariants cannot be waived by a performance ADR. “No OOM in tests” is a bounded empirical result, not a universal guarantee.

## 23. Repository, contribution, and release structure

Proposed Rust layout (adapt package names if the implementation ADR selects Go):

```text
SPEC.md
README.md
LICENSE
SECURITY.md
CONTRIBUTING.md
CODE_OF_CONDUCT.md
CHANGELOG.md
crates/
  cli/                 # human and machine-facing contract
  protocol/            # versioned messages and schema
  coordinator/         # leader, event loop, recovery
  scheduler/           # pure decisions and fairness
  estimator/           # history keys and conservative learning
  supervisor/          # launch/receipt/output lifetime
  platform/            # common capabilities and metric types
  platform-linux/
  platform-macos/
  platform-windows/
  context-git/
  store/
tests/
  fixtures/
  integration/
  fault-injection/
  security/
benchmarks/
  manifests/
  arrival-traces/
  harness/
  results/             # sanitized summaries; raw-data links
docs/
  adr/
  cli.md
  configuration.md
  capabilities.md
  security-model.md
  recovery.md
  benchmarks.md
  integrations/        # optional agent and worktree-tool recipes
packaging/
  linux/
  macos/
  windows/
.github/workflows/
```

Keep unsafe/native bindings isolated and reviewed. Core scheduler tests must not depend on privileged runners. Every behavior-changing contribution updates schema/help/spec or an ADR as appropriate. Favor small backend contracts over a plugin framework in V1. Use a permissive open-source license; Apache-2.0 is the proposed default, subject to maintainer choice and dependency compatibility review before publication.

### 23.1 Distribution

Ship versioned standalone binaries with checksums, SBOMs, build provenance, and signatures where supported. Publish Windows ZIP, macOS archives, and Linux archives first; add Homebrew, winget, and distribution packages after the artifact pipeline is reliable. Publish x64/arm64 only where native tests support the stated compatibility; Windows arm64 can follow as a separately qualified target.

No root install is required for basic operation. Daemon autostart is per-user and on-demand. Optional persistent user-service registration (systemd user unit, launch agent, Windows user-session startup mechanism) is explicit and reversible. Never register a SYSTEM service for ordinary use or silently enable cgroup delegation.

Use semantic versioning for CLI/config guarantees and separately version the IPC/database schema. Upgrades drain or reconnect supervised jobs through a documented compatible path; incompatible upgrades require a drain. Never replace a running supervisor binary in place. Rollback must respect schema compatibility and use a pre-migration backup only with an explicit operator procedure. Uninstall stops admission and requires a deliberate choice about running jobs and retained history; it never deletes source repositories.

## 24. Roadmap and phase gates

### Phase 0 — feasibility POC

Start with the recommended qex-derived experimental branch and an unchanged pinned qex baseline. Reuse its generic command, coordinator, and supervisor infrastructure; prototype native metric probes, whole-workload estimates, platform ownership, and deterministic synthetic workloads. Validate the hardest OS/lifecycle assumptions early on all four environments. Compare uncoordinated/static/qex arms. Deliver raw evidence and a build-versus-contribute ADR before committing to product expansion.

### V1 alpha

Complete durable queue/supervisor receipts, crash reconciliation, CLI/IPC schemas, generic cold-start rules, pressure gating, bounded bypass, conservative learning, Git observations, security controls, and bounded logging. Agent compatibility is ordinary CLI usage; optional instruction recipes may follow without changing safety policy.

### V1 beta to GA

Complete native platform matrix, WSL2 domain documentation, opt-in hard limits on capable platforms, fault/security testing, benchmarks, soak tests, packaging, migration tests, operator documentation, and signed releases. No platform may be labelled fully supported merely because its binary builds. Publish limitations alongside installation instructions.

### V2 candidates, each separately specified

- Authenticated Windows/WSL2 and multi-domain resource leases with explicit partition-failure behavior.
- Owned container/service adapters with immutable identity and complete lifecycle contracts.
- PTY/ConPTY interactive jobs and explicitly designed nested resource subleases.
- GPU/VRAM, disk-I/O and device pools; richer workload priorities or weighted resource-time fairness.
- Opt-in runtime worker hints and stack-specific estimators that preserve generic fallback.
- Proven hermetic validation reuse; otherwise continued execution of every request.
- Optional tool-call hooks, local UI, MCP adapter, or IDE integration using the same admission API.
- Privileged multi-user coordination only after a distinct security design.

## 25. Unresolved decisions and owners

These are explicit open decisions and engineering gates. Defaults define intended V1 behavior; they do not establish that the behavior is implemented or validated. The private protocol still requires implementation-level design. Native measurement, containment, recovery, calibration, and qex comparison remain required V1/POC checks, not deferred V2 features.

| Question | Default in this specification | Evidence/decision required |
|---|---|---|
| Final project/CLI/package name? | `[PROJECT_NAME]`, documentation token `coord`. | Maintainers check naming and availability before publication. |
| Independent fork feasibility and maintenance? | User selected our own qex-derived Rust version; no upstream contribution plan. | Native experiments, comparative benchmarks, and sustainable maintenance plan; see FOUNDATION_DECISION.md. |
| Implementation language? | Rust for the proposed qex-derived POC; target interfaces remain language-neutral. | Native lifecycle prototype and ADR before a permanent implementation commitment or language change. |
| Private daemon–supervisor protocol? | Separate authenticated, fenced launch channel required by §§14–15; public schemas do not define it. | V1 prerequisite: exact message schemas, capability lifecycle, sequence/replay rules, generation fencing, launch receipts, deadlines, cancellation acknowledgements, reconnect and terminal reconciliation; fault-inject every durable transition on native backends. |
| Exact supported OS/ABI versions? | Targets in §20.4. | Native CI availability and minimum-version test results before release claims. |
| macOS estimator calibration? | Explicit `macos_available_v1` formula in §10.2.1 plus native pressure gates. | Validate compression/cache traces; change policy version through an ADR if calibration fails. |
| Pressure thresholds/margins? | Versioned defaults in §§8–9. | Preregistered benchmark tuning, holdout workloads, recorded policy revision. |
| Hard-limit default? | Disabled, opt-in where supported. | No change without documenting command-failure semantics and user control. |
| Environment-at-rest encryption? | Private short-lived spool; no same-user secrecy claim. | Threat-model and cross-platform key-management ADR if encryption is added. |
| Advanced fairness? | Weighted admission turns plus bounded bypass. | Workload evidence before resource-time fairness complexity. |
| Full host-wide Windows/WSL coordination? | Explicit domain scope/static partition in V1. | Separate bridge/lease design and failure tests for V2. |
| Validation result reuse? | Prohibited throughout V1. | Hermeticity/equivalence proof and independent correctness review for V2. |
| Open-source license? | Apache-2.0 proposed. | Maintainer adoption and dependency audit before repository release. |

## 26. Review checklist

- Does the scope solve independent-client contention without requiring agents to cooperate beyond submission?
- Are claimed platform capabilities supported by native tests, including degraded modes?
- Can every state after a crash be explained without automatic duplicate execution?
- Are resource estimates clearly distinguished from hard limits and source validation?
- Does qex already solve the relevant need sufficiently, and does the POC justify any additional project?
- Are WSL2/container boundaries visible to users instead of hidden behind a “global” label?
- Can an unfamiliar command run correctly without an adapter or LLM?
- Can a contributor implement and test each normative requirement without inventing safety policy?

Approval of this document should authorize a separate implementation plan and POC design. It should not be interpreted as evidence that the benchmark gates have passed or that the proposed guarantees have already been implemented.

## 27. Machine-readable contracts and worked message sequences

Companion artifacts distributed with this specification:

- [protocol.schema.json](protocol.schema.json): JSON Schema Draft 2020-12 structural contract for 21 public CLI-to-daemon methods, including requests and success/error responses.
- [protocol.examples.json](protocol.examples.json): 16 ordered example messages showing acceptance, idempotent submission retry, observation, cancellation, wait, recovery release, and revision conflict. These are illustrative fixtures, not captured execution results.

The schema and prose are jointly normative. The schema checks field shape; the rules below cover temporal and cross-field semantics that JSON Schema cannot establish. Schema changes MUST accompany protocol changes. The internal supervisor channel remains separate, capability-authenticated, and subject to the launch/receipt state machine; clients cannot manufacture supervisor receipts through a public method.

### 27.1 Framing, types, versioning, and redaction

Each IPC frame is a 4-byte unsigned big-endian payload length followed by one UTF-8 JSON object. Reject frames above 1 MiB before allocation, duplicate object keys, nesting beyond 32 levels, invalid UTF-8, nonfinite numbers, and unknown object fields except explicitly extensible metadata. A partial frame closes only that connection and does not launch a job. Secrets in rejected payloads must not be echoed in an error message or daemon log.

All byte/count integers are in `[0, 2^53-1]` for cross-language JSON precision. Values outside the range fail validation; do not round them. CPU claims are positive finite decimal numbers rounded upward to the next 0.001 core before admission. UTC timestamps use the schema's syntax plus calendar validation. JSON strings preserve Unicode; NUL arguments and native byte sequences that cannot round-trip through this protocol are rejected explicitly. Support for opaque Unix argv bytes requires a later protocol extension, not lossy transcoding.

Every request has `schema_version`, `request_id`, `kind`, `method`, and `params`; every response repeats the request ID/method and adds `daemon_version` and `result`. A result has exactly one success body or typed error. Request IDs correlate messages within a connection and are not job deduplication keys. Submission uses a separate required `idempotency_key`, generated by the CLI before the first transmission and reused after an ambiguous response. Keys expire under §14.2; a fresh user invocation gets a fresh key unless explicitly overridden.

Unknown schema versions fail before mutation. Major-version negotiation occurs on connection setup using the `version` method at the supported bootstrap version. Clients may ignore fields only in the explicit `extensions` object; unknown normative fields require an updated schema version. No `extensions` entry changes execution or safety policy.

Default responses omit raw argv, environment, absolute source paths, and secret capabilities. Full source observations can be retained privately, but the public `Context` object summarizes them. `complete_observation` means the configured collection completed; it is not an immutable-source or hermeticity proof. A job's `recovery_required` is independent of its terminal outcome, allowing LOST plus an unresolved reservation.

### 27.2 Public method semantics

| Methods | Contract beyond field shape |
|---|---|
| `job.submit` | Validate payload and idempotency before QUEUED. Same key plus same canonical payload returns the current job with `replayed_acceptance=true`; mismatched payload returns `IDEMPOTENCY_CONFLICT`. Keys are scoped to user/domain. |
| `job.inspect`, `job.list` | Consistent committed snapshots. Cursors bind to a query/filter snapshot; expired cursors return `CURSOR_EXPIRED`, never silently restart pagination. |
| `job.wait` | A bounded observation wait of 0–60,000 ms. Expiry sets `observation_timed_out=true` and does not cancel the job. CLI waiting can renew this call. |
| `job.cancel` | Returns cancellation acknowledgement/current state, not a claim that cleanup already finished. Observe with `job.wait`. Root/terminal races follow §14.3. |
| `stdin.write` | Only the authenticated connection that submitted attached stdin may use this method, and only once RUNNING. Decoded chunks are at most 64 KiB; `offset` must equal the acknowledged next offset. Reject mismatches rather than replay ambiguous input. `close=true` sends remaining bytes then EOF. A disconnect closes stdin; no automatic reconnect/replay. |
| `status`, `doctor`, `version` | Read-only snapshots/diagnostics. Missing metrics are null with an explicit reason. They never alter cgroups, services, or OS settings. |
| `recovery.list`, `recovery.inspect`, `recovery.resolve` | §15.3 defines ownership evidence, revision preconditions, and no-replay behavior. Structural validity is not authorization to release an uncertain reservation. |
| `events.subscribe` | Bounded long-poll batches from a durable event cursor. `gap=true` requires a fresh domain snapshot; consumers refresh affected job snapshots. Sequence numbers advance only for returned events. |
| `logs.read` | Returns base64 bytes from a per-job capture-order stream. Chunk sequence preserves observation order, not a claim about the original cross-stream write timing. Cursor/offset advances across captured and discarded ranges; `gap` and discarded counts identify missing output. |
| `admission.set`, `daemon.stop` | Pause/resume or drain semantics from §§11/15. Stop acknowledgement precedes shutdown; it is not evidence that all jobs have finished. |
| `config.show`, `config.validate` | Flattened effective scalar entries with provenance, or validation of supplied TOML text. Validation does not install/reload that text. CLI file reads happen under the calling user's access. |
| `history.list`, `history.show`, `history.forget` | Prediction records only. Forget affects subsequent estimation, not active claims, audit outcomes, or result reuse. |

`daemon start` is a local CLI startup operation followed by IPC version/status checks; `run` is submission plus stream/wait operations; `logs --follow` repeatedly reads new offsets. Internal supervisor authentication and binary bootstrap mechanisms are never public JSON methods.

### 27.3 Required semantic checks

1. A metric with `quality=unavailable` has null `value` and a non-null `unavailable_reason`; available values have a sample timestamp and non-null age. Admission applies freshness independently of structural validity.
2. Terminal states have a terminal reason/outcome and finish time. Nonterminal states have no final outcome; a previously captured root status remains private until represented in a final outcome. LOST may have unknown native status. SUCCEEDED requires native root exit 0, verified receipt, and completion within the advertised ownership scope.
3. Non-Windows jobs use null commit claims. Windows jobs use positive commit claims and the independent ledger. A zero released reservation is represented by `reservation_released=true` retaining the original `reserved` quantities for audit; it is not subtracted twice.
4. Recovery mutation revision must match the current record and the action must be supported by current evidence. A valid schema cannot prove either fact.
5. A shell request contains only `shell_path` and script in its execution object; a direct request contains only argv. The environment is complete for the child, apart from documented coordinator-owned additions; the daemon does not fill missing values from its own environment.
6. `memory_limit_bytes` requires a capable backend and an effective claim not larger than the limit. An estimate that later grows beyond a fixed limit does not silently raise that limit; record the risk and hold new work according to the normal overrun policy.
7. Permission, executable/cwd checks, aggregate request bytes, base64 validity/decoded size, environment rules, and policy budget checks are mandatory after schema validation and before launch.

### 27.4 Reading the example sequence

Messages r1/r2 submit the same payload twice with the same idempotency key and return J17 both times. Only one launch is possible. r3 observes RUNNING. r4 acknowledges CANCELLING; r5 observes CANCELLED with the original signal and verified cleanup. Cancellation acknowledgement itself is not a completed outcome.

Messages r6/r7 operate on a separate LOST job J18 after absence is verified. Releasing its 4 GiB reservation changes ownership accounting but does not create an exit result. r8 deliberately repeats a stale revision and receives `RECOVERY_REVISION_CONFLICT`. A client must inspect the current record rather than guess a revision.

### 27.5 Revision verification checklist

Before using these contracts for implementation, validate the schema with a standards-compliant Draft 2020-12 validator; validate every example and reject negative fixtures for missing recovery revisions, mixed shell/direct fields, zero claims, unknown fields, wrong method response bodies, and invalid enum values. Then test the semantic rules against an implementation. Parsing JSON alone does not establish protocol correctness.

This revision resolves the six design gaps identified in review: operator recovery, launch-payload retention, explicit platform accounting, deterministic fairness, executable identity, and machine-readable public contracts. It does not claim native backend feasibility or benchmark success; those remain the POC's responsibility.
