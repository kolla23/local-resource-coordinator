# Foundation decision

**Date:** 2026-09-30  
**Status:** user selected an independently maintained qex-derived fork; technical feasibility and release readiness remain subject to validation.

Start the feasibility POC from qex v0.33.0 in Rust. Keep the unchanged release as a benchmark arm and make experimental changes in a separate development branch. Maintain our own modified version with proper upstream attribution. Contributions to upstream are not part of the selected plan. This user decision supersedes the earlier upstream-first recommendation; it does not establish that the proposed improvements are implemented or validated.

The [source assessment](QEX_ASSESSMENT.md) found substantial reusable scheduling, learning, supervision, recovery, and CLI infrastructure. It also found meaningful differences from the [V1 specification](SPEC.md), particularly native Windows support, whole-workload memory estimates, conservative admission, and launch/recovery semantics. A companion alone cannot resolve those internal differences.

## First implementation milestone

1. Establish a reproducible build and run unchanged upstream tests on Linux and macOS. Record exact versions, configurations, and failures.
2. Create independent measurement fixtures for simultaneous workers and short memory spikes. Record actual observations before tuning estimates.
3. Prototype native Windows process ownership and test Linux/macOS containment limits. Keep these probes small enough to invalidate the architecture early.
4. Write the private daemon–supervisor contract and a crash-injection matrix before implementing the strengthened launch path.
5. Implement the smallest scheduler/learning changes needed for a fair comparison, then execute SPEC §21 across the available native environments.

Deliver raw measurements, reproducible commands, a capability matrix, test results, and an updated decision. Missing platform access means that platform is not evaluated; it is not a passed gate.

## Decisions to record before changing interfaces

The current specification describes a target product contract. Its public schema is not an existing qex protocol and must not be presented as a compatible extension without implementation work. Record ADRs for public CLI compatibility, IPC versioning, file persistence versus SQLite, and the private launch/recovery contract. Keep safety requirements explicit while allowing an evidence-backed implementation to reuse upstream mechanisms.

Apache-2.0 is the upstream license. Preserve applicable attribution and redistribution requirements. Complete the dependency and release-artifact license inventory before distribution. No final product name is needed for the POC.

## Evaluation after the POC

Evaluate whether the independent fork meets its technical goals and whether measured benefit justifies continued maintenance. Compare it fairly with unchanged qex. Narrow scope or reconsider architecture if native experiments fail; do not switch to upstream contributions as the default without a new user decision.

The source audit is complete. No product implementation, native validation, benchmark advantage, or release readiness is claimed. See [CLAUDE_HANDOFF.md](CLAUDE_HANDOFF.md) for the conversation summary and next steps.
