# Decision: no SQLite in V1

**Date:** 2026-10-02
**Status:** decided by owner
**Context:** SPEC's target design names SQLite; qex keeps state in atomic job files ([QEX_ASSESSMENT.md:110](QEX_ASSESSMENT.md), [FOUNDATION_DECISION.md](FOUNDATION_DECISION.md))

**Decision:** V1 keeps qex's atomic job files and does not move persistence to SQLite; no ADR for it is written during the Windows port.
