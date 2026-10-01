# Decision: native Windows is the core V1 platform

**Date:** 2026-10-01
**Status:** decided by the owner
**Context:** the decision gate in [START_HERE_CLAUDE_CODE.md](START_HERE_CLAUDE_CODE.md); evidence in [../decision-gate/](../decision-gate/)

## Decision
- The owner's agents and builds run in **native Windows**: PowerShell, VS Code on Windows, Claude Code in a normal terminal.
- **Native Windows support is the core V1 requirement.** This matches SPEC R14 / §3.1 goal 9: native Windows, macOS and Linux.
- **Unchanged qex in WSL2 is a reference only.** It serves as a comparison arm and a measurement baseline, not the product and not the answer for the owner's own work. This removes option (b), "use unchanged qex in WSL2", as an outcome of the decision gate. The remaining options are (a) continue the qex-based fork with a Windows backend, or (c) a new core.

## Consequences
- The decision gate measures native Windows workloads first. WSL2 runs of qex are kept for comparison.
- Early evidence is in [01-jobobject-probe.md](../decision-gate/01-jobobject-probe.md): Job Objects supply ownership, whole-tree memory and cancellation.
- Still open: how much of qex must be rebuilt for Windows (gate step 3), and the two ADRs listed in that document.
