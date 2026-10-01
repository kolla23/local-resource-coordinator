# Decision: the fork turns off upstream's automatic update check by default

**Date:** 2026-10-01
**Status:** proposed; accepted when the owner merges the PR that adds this file
**Context:** GitHub issue #4; SPEC goal 10 and R11 (core policy works offline), SPEC L669 ("No automatic network telemetry, update execution, …")

## Problem
Upstream qex's coordinator asks `https://api.github.com/repos/stephenc/qex/releases/latest` for the newest release every 7 days (`[update] check = "7d"`, `src/config.rs` `UpdateConfig::default`; called from `src/daemon.rs`, run with `curl` or `wget`).

For this fork that request is wrong:
- Upstream's releases aren't ours. Since PR #3 our build reports `0.0.0-dev+g<commit>`, so the "a newer qex exists" message is hidden, but **the request still happens**. Every fork user contacts a third party's API on a schedule, and gets nothing from it.
- The SPEC wants core work offline and no outbound data the user didn't ask for.

## Decision
- The default becomes `[update] check = "never"`. The coordinator then opens no connection of its own, writes no update record and prints no version message. This is upstream's existing, tested `never` behaviour; no new code path.
- Nothing replaces it for now. The fork has no releases yet. When it does, a later ADR can point `url` at the fork's own releases and decide whether any default check is wanted.
- Unchanged: `qex version --check` still asks when a person runs it, and a user can still opt in with `check = "7d"` (or any interval) in their config. The default `url` stays upstream's, so an explicit opt-in or `--check` behaves exactly as upstream documents.

## Consequences
- `src/config.rs` (default), `src/help.rs` (config template, a sentence that the check is off by default, and the "first interval" paragraph), `docs/reference.md` (example, a paragraph on the fork's default, and the "first interval" paragraph) and a comment in `src/daemon.rs` change, each with the fork's modified-file header.
- `src/update.rs`: tests of the message and the record turn the check on explicitly, since the default no longer does.
- New tests: a unit test that the default config gives no interval, and an e2e test that a coordinator whose config names no `check` writes no update record.
- The existing tests of an explicit interval (`300s`) and of `never` still apply unchanged.
