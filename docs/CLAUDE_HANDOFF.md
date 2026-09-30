# Claude handoff

## Start here

Continue development planning and implementation for an independently maintained, qex-based local resource coordinator. Read this file first, then FOUNDATION_DECISION.md, QEX_ASSESSMENT.md, and SPEC.md. Read the public schemas before implementing corresponding interfaces.

This package transfers documents and a curated conversation summary. It is not a verbatim transcript, a working product, or a repository checkout. The user will supply this package to Claude; nothing has been sent to an external service automatically.

## Latest user decisions

- Maintain our own modified version of qex. Contributing upstream is not part of the chosen plan. Earlier upstream-first recommendations are superseded.
- Preserve upstream attribution and comply with Apache-2.0 and dependency licenses. Independent branding does not remove those obligations.
- Use a placeholder project name. Neither arm nor devguard is selected.
- Begin with a qex-derived Rust prototype. Do not rewrite the core from scratch without new evidence and an explicit design decision.
- Eventually provide a basic agent skill and convenient plugins/integrations. The executable and background coordinator own all safety decisions; a skill alone cannot enforce resource limits.
- Build and validate the core before expanding integrations. Ordinary CLI use must remain independent of any agent application.
- Explain progress plainly and distinguish completed work from proposed behavior and unrun tests.

## Product intent

A stack-agnostic, agent-agnostic local coordinator for parallel AI and software-development commands on Windows, macOS, Linux, and WSL2. Claude, Codex, Cursor, scripts, and humans submit ordinary commands to a shared local scheduler.

Core scope: arbitrary noninteractive commands; CPU, RAM, swap and memory-pressure observations; process-tree accounting; deterministic RUN/WAIT admission; conservative historical resource learning; observational Git repository/worktree/source context; safe cancellation and crash recovery; understandable diagnostics. Unknown stacks and non-Git directories must work. No LLM is needed for safety decisions.

V1 has one coordinator per OS user per resource domain. Windows and a WSL guest are separate domains; optional static partitioning is not dynamic host-wide enforcement. Only submitted workloads are controlled.

Every independent V1 validation request executes. Do not coalesce tests or reuse cross-worktree results. A strict retry of the same submission is different from skipping an independent request.

## What the conversation established

1. The user requested a production-quality specification covering architecture, scheduling, native APIs, CLI/configuration, lifecycle, security, testing, benchmarks, distribution, and roadmap.
2. The spec was created and expanded with public protocol schemas and examples.
3. The user asked whether anything remained. Open decisions, required engineering checks, and deferred features were separated. A spec does not prove native behavior.
4. A source/license assessment found qex to be a substantial foundation, with important remaining differences from the target.
5. The assistant initially recommended upstream-first work. The user then chose their own maintained modified version, without contributing upstream.
6. The user asked about legality. The audited qex code uses Apache-2.0, which allows modification and redistribution without requiring contribution back, subject to license conditions. Preserve applicable notices, include the license, identify modified files, carry NOTICE attributions if supplied, respect trademark limits, and review dependencies before distribution.
7. The user asked about skills/plugins. These are integration layers around the core, not substitutes for deterministic scheduling and native supervision.
8. The agreed next step is a small, tested qex-derived prototype. The user now requested this handoff.

## Existing artifacts

| File | Purpose |
|---|---|
| SPEC.md | Proposed target V1 requirements, platform contracts, algorithm, acceptance criteria, benchmark methodology, and roadmap. |
| FOUNDATION_DECISION.md | Latest selected direction: independent qex-derived fork; staged feasibility work. |
| QEX_ASSESSMENT.md | Dated static source audit, gaps, evidence links, and original recommendation, with a supersession note. |
| QEX_AUDIT_EVIDENCE.json | Source revision and snapshot hashes for reproducing the audit. |
| protocol.schema.json | Proposed public CLI-to-daemon JSON Schema; not qex's existing wire protocol. |
| protocol.examples.json | 16 illustrative messages, not captured runtime results. |
| CLAUDE_START_PROMPT.txt | A ready-to-paste continuation request. |
| PACKAGE_MANIFEST.json | File hashes for this handoff package. |

## Audit facts and limitations

Repository: https://github.com/stephenc/qex  
Reviewed commit: afd869eef8ab9085dcaa4376a9f8f8b77397ba16  
Release observed during audit: v0.33.0  
Release commit: 78b4e86c5ff8be0c7f4bc26bea809aa75fe230f0

The release comparison found version metadata changes only, with the same runtime source. Fifty-two downloaded source/test/documentation snapshots were Git-blob verified. The source snapshot itself is not included in this handoff; retrieve the pinned upstream source to reproduce it.

qex already has scheduling, resource claims, learning, supervision, recovery, bounded bypass, a terminal dashboard, configured pools, and cooperative peers. Main gaps relative to our target include:
- Native Windows support: the audited source explicitly requires Unix; WSL is not a native Windows backend.
- Whole-workload historical memory peaks: Linux child max RSS is not a simultaneous process-tree peak.
- Conservative available-memory failure behavior, macOS accounting, and pressure policy.
- Exact launch fencing, unresolved ownership, and strengthened recovery semantics.
- Git/source observations and strict payload-based submission idempotency.

Consult the assessment for qualifications. These are static findings and policy differences; no crash, duplicate execution, performance gain, or complete exploit was demonstrated.

No product code was implemented, no qex executable was built here, and no native benchmark was run. Rust tooling was unavailable on PATH and WSL enumeration was denied in the original environment. Those are prior environment limitations, not assumptions about Claude's environment. Document/schema checks succeeded, including all 16 public examples. Upstream CI status is not a locally reproduced result.

## Next implementation milestone

1. Inspect the available workspace and tools; preserve existing work. Establish an independent development checkout from the pinned release with upstream license/attribution intact. Do not publish a repository, send upstream contributions, or contact maintainers without user instruction.
2. Build the unchanged baseline and run meaningful existing tests on supported available environments. Record exact commands, versions, configurations, and outcomes.
3. Probe native Windows child ownership using Job Objects, and Linux/macOS ownership limits. Native Windows is an early feasibility gate, not something to defer until the end.
4. Measure multiple simultaneous workers and short memory bursts independently. Validate estimates before tuning scheduler thresholds.
5. Specify the private daemon–supervisor protocol and crash-injection matrix before strengthening its implementation.
6. Implement the smallest end-to-end slice: submit several commands, queue excess work, execute every independent request, expose status/results, and safely cancel owned processes.
7. Compare unchanged qex and the prototype using SPEC §21, including uncoordinated and static-limit controls where feasible. Publish failures and tradeoffs along with improvements.
8. Add a basic agent skill after the core slice is reliable; broaden plugin integrations later.

Use safety outcomes from the spec, but reconcile implementation choices through explicit ADRs. In particular, do not force SQLite or replace qex's public protocol solely because the target draft names them. Do not silently weaken durability or lifecycle guarantees either.

## Still required for V1

Exact private protocol schemas/authentication/generations/receipts/replay/reconnect/cancellation rules; native metric validation; containment and recovery fault tests; threshold calibration; supported OS/ABI matrix; dependency/license inventory; reproducible benchmark evidence. Final name and release packaging remain open. The independent-fork direction is selected, but technical feasibility and release readiness remain unproven.

## Deferred scope

Dynamic Windows/WSL shared budgeting; complete Docker/container and external-service accounting/cancellation; child PTY/ConPTY; nested reservation subleases; GPU/VRAM policy; privileged multi-user coordination; provably equivalent cross-worktree result reuse; advanced GUI/IDE/MCP integrations and stack tuning. A basic documentation skill may accompany an early release. Existing adjacent qex features do not automatically become guaranteed support.

## Completion discipline

Separate static review, schema validation, upstream CI, locally executed tests, and native platform results. Never label an untested platform as passing. Never claim a plugin can enforce processes launched outside the core. Never report requested tests as successful if they were not executed. Keep command safety deterministic and visible.

