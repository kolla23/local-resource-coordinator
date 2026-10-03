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
- Release build: `cargo build --release --locked`
- MSRV check: `cargo +1.85 check --features test-fixtures --all-targets --locked`
- The upstream baseline, with logs: `docs/baseline/run-baseline-linux.sh <clone> <log-dir>` (it checks out upstream v0.33.0, which has no `test-fixtures` feature, so it keeps the upstream commands)
- Control characters: stage your changes (by name), then `python3 .github/scripts/check-control-chars.py`. It checks staged content only and names any unstaged or untracked files it skipped. Tests: `check-control-chars-test.py`

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
  - Every PR description lists the task's acceptance criteria as checkboxes, ticked only when done and verified, and a "Found, not done" section for unplanned work found along the way (ask the owner before including it).
  - Start each branch from a fresh `origin/main` (`git fetch` first).
  - Every PR, however small, opens as a draft (`gh pr create --draft`) and runs the review loop until a round finds no real issues; any fix starts a new round. Keep each round's report in `$(git rev-parse --git-common-dir)/reviews/<branch>/` (shared by all worktrees, so removing one keeps the reports), never on GitHub. Then add one line to the PR body, `Reviewed at <full head commit id>: no real issues`, and mark the PR ready (`gh pr ready`). A push to a ready PR first turns it back into a draft (`gh pr ready --undo`) and needs a new round. The CI check "Reviewed at the head commit" (`.github/scripts/check-reviewed.sh`) runs only on ready PRs and confirms that line names the head commit.
- Don't add fork documents directly in `docs/`; use `docs/fork/` or another subfolder.
- Decisions that change upstream interfaces (persistence, IPC framing, CLI compatibility, launch protocol) need an ADR first.

## Gotchas
Add one line each time the same mistake happens twice.
- On Windows, Git Bash's `/usr/bin/link.exe` can shadow the MSVC linker. From Git Bash, pass Linux paths to `wsl.exe` with `MSYS_NO_PATHCONV=1`, and git `rev:path` arguments too (`MSYS_NO_PATHCONV=1 git cat-file -e base/qex-v0.33.0:<path>`): Git Bash rewrote one into `base\...;...`, the lookup failed, and an upstream file was taken for a fork file (twice).
- Keep this repo's `core.autocrlf=false`, so upstream files stay LF.
- PR titles must match `type(scope): summary` (lower-case type, scope optional), be at most 72 characters and not end with a full stop (`.github/scripts/check-title.sh`); count before `gh pr create`.
- A Rust string with a `\` line continuation, written through a Python edit script, came out as one long line with the spaces inside the string (twice). Edit those lines with the Edit tool, then check the added lines: `git diff -U0 | grep '^+[^+]' | awk 'length > 101'` (the fork header lines are expected to show up).
- A `\0` written through a Python edit script inside a shell heredoc came out as a raw NUL byte (three times). Write escapes with the Write or Edit tool; CI rejects raw control bytes and bare CRs (`.github/scripts/check-control-chars.py`).
