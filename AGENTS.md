# local-resource-coordinator (placeholder name)

An independently maintained fork of qex (Apache-2.0). It is a local coordinator that decides RUN/WAIT for commands that humans and AI agents submit in parallel, so that shared work doesn't exhaust CPU or RAM. The users are developers running several agents or builds on one machine.

## Stack
- Rust (edition 2021, MSRV 1.85), a single crate `qex`. Dependencies: clap, serde/serde_json, toml, serde_yaml_ng, uuid, libc, anyhow.
- No database. State lives in atomic job files. IPC runs over Unix sockets.
- Upstream builds only on Unix (Linux, macOS). On Windows, build and test inside WSL2.
- Upstream: https://github.com/stephenc/qex. We fork from v0.33.0 (`78b4e86`, tag `base/qex-v0.33.0`).

## Commands (run in WSL2 or on Linux/macOS)
- Format check: `cargo fmt --all --check`
- Lint: `cargo clippy --features test-fixtures --all-targets -- -D warnings`
- Unit tests: `cargo test --bins`
- End-to-end tests: `cargo test --features test-fixtures --test e2e -- --test-threads=2` (about 6 minutes). The feature builds `testjob`, the portable job some tests run; without it `--test e2e` stops with an error and a plain `cargo test` skips the suite.
- Fast check while working: `cargo test --features test-fixtures --test e2e a_job_that_succeeds_gives_the_exit_code_zero` (in WSL2; about 4 s once built, measured 2026-10-09).
- Release build: `cargo build --release --locked`
- MSRV check: `cargo +1.85 check --features test-fixtures --all-targets --locked`
- The upstream baseline, with logs: `docs/baseline/run-baseline-linux.sh <clone> <log-dir>` (it checks out upstream v0.33.0, which has no `test-fixtures` feature, so it keeps the upstream commands)
- Control characters: stage your changes (by name), then `python3 .github/scripts/check-control-chars.py`. It checks staged content only and names any unstaged or untracked files it skipped. Tests: `check-control-chars-test.py`
- Move checker (the R-PRs of `docs/fork/PORT_PLAN.md`): `python3 .github/scripts/check-moves.py <base> [<head>]`. It pairs each function with a byte-identical copy and lists every other line, in every changed file: a changed line is left out only when it belongs to a paired function, and every line of a new function that pairs with nothing (a forwarder, an edited copy left behind) is listed. It exits 1 when a changed function has no byte-identical copy, 2 on a usage error and 3 when it fails itself; at any exit code, the PR explains every listed line. The authoritative run is CI's Move check job on `refactor/` PRs. Tests: `check-moves-test.py`

## Structure
- `src/`: upstream qex (coordinator `daemon.rs`, scheduler `sched.rs`, per-job `supervisor.rs`, platform metrics `sys.rs`).
- `tests/e2e.rs`: upstream end-to-end suite.
- `docs/fork/`: our handoff, SPEC (target V1 contract), qex assessment and protocol schema.
- `docs/baseline/`: recorded baseline results and raw logs. Update these; don't rewrite history.
- `docs/*.md` at the top level is upstream's shipped documentation, and an e2e test scans it.

## Rules for this project
- Run the checks above before saying done, and show the command and its result. Never claim a build, test or benchmark passed unless it ran in this session.
- Never label an untested platform as passing. Missing access means "not evaluated".
- Never commit secrets. Job records and logs can contain them.
- Keep to the V1 scope in `docs/fork/SPEC.md` and `docs/fork/CLAUDE_HANDOFF.md`. V2 items (Windows/WSL shared budgets, containers, PTY, GPU, result reuse, rich integrations) are out of scope; ask before adding features.
- Every independent request executes. Don't add dedupe or result reuse.
- License (Apache-2.0):
  - Keep `LICENSE` and all notices.
  - Every upstream file we modify gets a header line: `// Modified by the local-resource-coordinator fork, <date>: <what>`.
  - Never use "qex" as our product name.
- Git:
  - `upstream` is fetch-only (push URL `DISABLED`); never contact or contribute upstream.
  - One purpose per branch and PR. Split work into PRs at planning time, aiming for about 400 changed lines each (not counting logs or generated files). Finish the agreed task in the same PR; if it grows past about twice the estimate (~800 lines, not counting logs or generated files), stop and ask the owner whether to continue or split. Never move planned work to a later PR without the owner's OK. Never commit directly to `main`; never merge without the owner's OK.
  - Every PR description lists the task's acceptance criteria as checkboxes, ticked only when done and verified, and a "Found, not done" section for unplanned work found along the way (ask the owner before including it). Each "Found, not done" item gets a GitHub issue in this repo (`--repo kolla23/local-resource-coordinator`, never upstream), labelled `found-in-pr`, with its number next to the item; list only work that still needs doing. Search open issues first and link an existing one instead of opening a duplicate. Issue text is public, so no local paths or usernames (repo file:line is fine). In the "Found, not done" list, link each issue as plain "#N", never with Fixes/Closes/Resolves (those words close the issue when this PR merges); only the PR that does the work says "Fixes #N", once per issue.
  - Start each branch from a fresh `origin/main` (`git fetch` first).
  - Every PR runs the review loop in `~/.claude/standards/REVIEW_LOOP.md` (the one copy). Here, the CI check "Reviewed at the head commit" (`.github/scripts/check-reviewed.sh`) runs only on ready PRs and confirms the body's `Reviewed at` line names the head commit.
- Don't add fork documents directly in `docs/`; use `docs/fork/` or another subfolder.
- Decisions that change upstream interfaces (persistence, IPC framing, CLI compatibility, launch protocol) need an ADR first.
- Scripts may write code only for pure moves that check-moves.py verifies; every other change uses the normal edit tools.

## Gotchas
Add one line each time the same mistake happens twice.
- On Windows, Git Bash's `/usr/bin/link.exe` can shadow the MSVC linker. From Git Bash, pass Linux paths to `wsl.exe` with `MSYS_NO_PATHCONV=1`, and git `rev:path` arguments too (`MSYS_NO_PATHCONV=1 git cat-file -e base/qex-v0.33.0:<path>`): Git Bash rewrote one into `base\...;...`, the lookup failed, and an upstream file was taken for a fork file (twice).
- When a change replaces what a test runs (a port, a de-flake, a faster fixture), list each assertion and the quantity it depends on (CPU time, wall time, bytes, exit code, output), and measure that quantity before and after: a replacement that only keeps the command's shape can leave an assertion guarding nothing.
- Keep this repo's `core.autocrlf=false`, so upstream files stay LF.
- PR titles must match `type(scope): summary` (lower-case type, scope optional), be at most 72 characters and not end with a full stop (`.github/scripts/check-title.sh`); count before `gh pr create`.
- A Rust string with a `\` line continuation, written through a Python edit script, came out as one long line with the spaces inside the string (twice). Edit those lines with the Edit tool, then check the added lines: `git diff -U0 | grep '^+[^+]' | awk 'length > 101'` (the fork header lines are expected to show up).
- A `\0` written through a Python edit script inside a shell heredoc came out as a raw NUL byte (three times). Write escapes with the Write or Edit tool; CI rejects raw control bytes and bare CRs (`.github/scripts/check-control-chars.py`).
- A Unicode escape such as the one for U+2028, typed into the Edit or Write tool, landed as the raw invisible character (three times). Name the character as U+XXXX in prose, and write UTF-8 byte escapes (`b"\xe2\x80\xa8"`) or `chr(0x2028)` in code; `check-control-chars.py` rejects C1, U+2028/2029 and format characters (issue #50).
