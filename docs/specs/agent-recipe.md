# Spec: agent recipe (Claude Code skill + AGENTS.md snippet)

**Status:** reviewed by the owner, 2026-10-01; decisions A–E recorded below. **Spec only.** Nothing gets built until the V1 `coord` CLI exists and works, because the recipe must document real commands, flags and exit codes. Every command below is the SPEC's proposed contract, not something that runs today.

## Goal
Coding agents on one machine run heavy commands through the coordinator, so their builds and tests queue instead of exhausting CPU and RAM or colliding with each other. The recipe teaches an agent the CLI contract. It enforces nothing: the coordinator owns every safety decision (CLAUDE_HANDOFF L15; SPEC L456, "Optional agent instructions merely teach this contract; they do not bypass the scheduler").

## Scope
- **V1, this spec:** two text artifacts with the same rules.
  1. **Claude Code skill**: `SKILL.md`, loaded when the agent is about to build, test, bundle or install.
  2. **AGENTS.md snippet**: the shared version for Codex, Cursor and any other agent that reads `AGENTS.md`. No separate Cursor or generic copies.
- **Location:** `docs/integrations/` (SPEC L877, "optional agent and worktree-tool recipes").
- **Not in V1:** tool-call hooks, a plugin that intercepts commands, an MCP adapter, IDE integration (SPEC L917, V2); a user config that overrides which commands are heavy (V2); agent identity flags.

## Prior art to reuse
Upstream ships the same idea: `docs/agents.md`, `skills/qex/SKILL.md` and `qex help agents`. The help text is compiled into the binary, so the skill and the binary can't drift apart. **Proposal:** use the same single-source pattern. `coord help agents` prints the recipe, and both artifacts are generated from it, with a test that fails when they differ. The fork must not call it "qex" (AGENTS.md license rule).

## Rules the recipe teaches

### 1. Which commands go through the coordinator
- **Heavy, always through `coord run`:** builds (`cargo build`, `go build`, `msbuild`, `gradle`, `make`), tests (`cargo test`, `pytest`, `npm test`, `go test`), bundlers (`webpack`, `vite build`, `tsc -b` on large projects) and dependency installs (`npm install`, `pip install`, `cargo fetch`).
- **Light, run directly:** git, file reads and searches, `ls`/`cat`/`grep`, formatters, and fast single-file linters. The agent uses judgement on borderline linters. A whole-repo `eslint` or `clippy` run counts as heavy.
- **Never:** nested `coord run` inside a coordinated job (rejected with `NESTED_SUBMISSION_UNSUPPORTED`, SPEC L607).

### 2. How to run them
- Use `coord run [options] -- PROGRAM [ARG...]`. It blocks and streams the child's stdout and stderr; queue messages go to stderr and include the job ID once (SPEC L417). Don't use detached `submit`.
- Shell syntax (`&&`, pipes) needs an explicit shell: `coord run --shell /bin/sh -- 'make build && make test'`.
- **Windows:** batch shims such as `npm` and `npx` need `--shell cmd.exe` or `--shell pwsh`; there is no implicit cmd fallback (SPEC L435).
- `--memory` and `--cpu` are optional floors (SPEC L292). The recipe tells the agent to pass them only when it knows the job is big, e.g. after an earlier out-of-memory failure.
- **Every `coord run` passes `--result-file PATH`** (SPEC L419), a fresh path per command. SPEC L445 makes it "authoritative about origin and native status", and human output "is not a parsing API" (L456). The agent reads the job ID and outcome from that file, never from stderr text.
- **Exit codes:** the job's code passes through. Coordinator codes (SPEC L444-450, e.g. 125 management error, 124 timeout, 130 cancelled) can collide with a child's own codes, so the agent uses the result file to tell "the build failed" from "the coordinator did something".

### 3. When the coordinator isn't available
- The fallback fires in exactly two cases:
  1. `coord` isn't on PATH;
  2. `coord run` exits with the dedicated **"coordinator unavailable"** code (ADR 2 below) **and** the result file shows no job was accepted (ADR 2 defines that proof). The code alone isn't enough: a child can exit with any value, so a build that happens to exit with the same number must not look like "unavailable".
- In those cases the agent runs the command directly and **warns the user once per session**, not once per command. Suggested wording: "The resource coordinator isn't available, so builds and tests in this session run without coordination."
- **Never fall back on any other exit code.** A failed build or test, a timeout, a cancellation or a queue timeout is a result, not "coordinator unavailable". So a failed build is never rerun uncoordinated.
- **The agent never starts the daemon itself.** It never runs `coord daemon start` and never edits coordinator config. The CLI's own on-demand autostart (SPEC L478, L617) is accepted (decision A): it's part of `coord run`, not an agent action.

### 4. When the job has to WAIT
- `coord run` waits in the foreground, so to the agent a queued command just looks slower.
- **The agent sets its tool timeout explicitly** for every `coord run` (in Claude Code the Bash tool defaults to 2 minutes, maximum 10), and **passes `--queue-timeout` shorter than that limit**, e.g. `--queue-timeout 8m` with a 10-minute tool limit (decision B). A job that is still queued then leaves the queue before the agent gives up, so it never starts later with nobody watching.
- Queue time plus run time must fit in the tool limit; otherwise the client is killed mid-run and ADR 1 cancels the job. For a command expected to run long, the agent leaves less room for queueing, and tells the user if the command can't fit in the limit at all.
- **If the queue timeout is reached, or the wait otherwise outlasts the tool limit**, the agent tells the user:
  - how long it waited,
  - what it's queued behind: the wait reason code (SPEC L284) and the jobs ahead of it, from `coord inspect --json JOB` (the jobs-ahead field is ADR 4),
  - the job ID, so the user can inspect it with `coord inspect JOB`.
- **Never:**
  - bypass the queue by running the command directly while the coordinator is up,
  - cancel or kill other jobs, or anyone's jobs except its own when the user asks,
  - retry in a tight loop or resubmit the same command (a resubmission is a second, independent job, and every request executes),
  - poll with `sleep` loops. Use `coord wait JOB`, which returns when the job ends, with the same explicit tool timeout.
- After a queue timeout the agent doesn't resubmit on its own. If the user asks it to try again, it runs `coord run` once more, which is a new job.

## Decisions (owner, 2026-10-01)
- **A. Autostart: accepted.** `coord run` may start the daemon on demand, as the SPEC says (L478, L617). The agent never starts it explicitly.
- **B. Queue timeout: yes.** The recipe passes `--queue-timeout` shorter than the tool limit (e.g. 8m vs 10m), with the tool timeout set explicitly. A killed client while the job is *running* is covered by ADR 1.
- **C. Unavailable code: yes.** The recipe falls back only on a dedicated "coordinator unavailable" exit code (ADR 2), never on a command failure. Because codes can collide, the fallback also needs the result file's proof that no job was accepted.
- **D. Per-job TEMP/TMP: fix it in the coordinator** via the pending per-job TEMP ADR (evidence: `docs/decision-gate/02-parallel-workload-windows.md`, PRs #9/#10; the SPEC doesn't cover TEMP yet). The recipe doesn't tell agents to set TEMP.
- **E. Binary name: keep `coord` as a placeholder. Rename before release.** The recipe can't ship until the name is final (SPEC L6, §25).

## ADRs this recipe depends on
These change upstream interfaces, so each needs an ADR before the CLI implements it (AGENTS.md rule).
1. **Cancel the job tree when the client is killed while the job runs.** If the process running `coord run` dies (for example, the agent's tool limit kills it) while the job is running, the coordinator cancels the whole job tree. This changes the SPEC default for `run` (`disconnect = "continue"`, L442/L515). The ADR must define how a killed client is told apart from a transient transport failure (SPEC L446), whether detached `submit` jobs keep today's behavior, and whether killing a `coord wait JOB` client (not `run`) also cancels the job. Proposal: no, since `wait` only attaches (SPEC L417).
2. **A dedicated "coordinator unavailable" exit code**, distinct from 125. No exit code can be distinct from every child code (a child can exit 0-255), so the ADR must also define machine-readable proof that no job was accepted: e.g. the `--result-file` is written with a "not accepted" record, or isn't created at all. It means "no job was accepted, nothing ran". The ADR picks the value and the proof, defines exactly which failures produce it (daemon unreachable, autostart failed, state directory unusable), and keeps L699's rule: the coordinator itself never runs work unmanaged.
3. **Per-job TEMP/TMP** (decision D). Already pending from gate 2; listed here because the recipe relies on it.
4. **`inspect` reports what a queued job waits behind**: the jobs ahead of it (IDs, groups, claims) in `inspect --json`. The SPEC only defines wait reason codes (L284), and the owner's WAIT requirement needs more.

## Acceptance checks (once the CLI exists)
Run through the real CLI, in the same spirit as the repo's e2e tests:
1. The generated `SKILL.md` and AGENTS.md snippet match `coord help agents` byte for byte (test fails on drift).
2. Every command in the recipe parses with the real CLI (`--help`, or a dry-run against a test daemon).
3. A scripted agent session on a fake bench (3 tiny crates, as in PR #11's e2e) with a capacity of 1:
   - the second and third `coord run` wait, then run, and their exit codes pass through;
   - with `coord` removed from PATH, or with the daemon made unavailable so `coord run` returns the ADR 2 code, commands run directly and the warning appears exactly once per session;
   - a build that fails through `coord run` is reported as failed and is **not** rerun directly;
   - a child that exits with exactly the ADR 2 code is treated as a job result, not as "unavailable", and is not rerun.
4. Lifetimes:
   - a job still queued when `--queue-timeout` is reached leaves the queue and never starts;
   - killing the `coord run` client while its job runs cancels the whole job tree (ADR 1), with no surviving processes.
5. Manual agent scenarios. Load the skill in Claude Code and the snippet in Codex, and check each against these observable pass criteria. The scripted test in check 3 tests the CLI path; these test whether a real agent follows the recipe:
   - "build and test this repo": uses `coord run` with `--result-file`, `--queue-timeout` and an explicit tool timeout; `git status` runs directly.
   - Capacity 1, with another long job holding it: the agent reports how long it waited, the reason code, the jobs ahead and its job ID; it doesn't resubmit, cancel the other job or poll with `sleep`.
   - `coord` missing: the agent warns once, and not again on the second and third heavy command.
   - A failing test through `coord run`: reported as failed, not rerun directly.

## Out of scope
Plugins and hooks that force commands through the coordinator, MCP, IDE integration, user-configurable heavy-command lists, agent identity labels, Windows/WSL shared budgets, and containers (SPEC V2).
