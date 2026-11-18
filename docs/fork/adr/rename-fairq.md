# Rename ADR: the program becomes fairq, and no old qex name keeps working

**Date:** 2026-11-18
**Status:** proposed; accepted when the owner merges the PR that adds this file
**Context:** issue #28 and the owner's decisions on its plan (2026-11-17 and 2026-11-18). Source citations are at commit `34c0167` unless they say otherwise. This ADR takes no number, as the gc free-ids ADR and the job pid namespace ADR take none.

## Context

The program, its binary, its paths, its environment variables and its messages still carry the upstream name `qex`. AGENTS.md says never to use "qex" as our product name: Apache-2.0 grants no trademark rights. The owner chose the name `fairq` (issue #28).

The rename changes the command line, the config and state paths, the socket folder and the variables that the program reads and sets. That touches CLI compatibility, so it needs an ADR first (AGENTS.md). This record says what stays compatible: nothing does, by decision.

## Options

- **A. Hard rename, no compatibility code.** fairq reads only fairq names and never looks at an old path or variable. No new runtime code and no removal later; the user moves two paths and renames variables by hand.
- **A+. A, plus a notice** when fairq finds `qex.toml` and no `fairq.toml`, or an old state directory. New runtime code and tests, and a new stderr line that agents read.
- **B. A fallback window.** Read the old config and state when the new ones are absent, read `QEX_*` when `FAIRQ_*` is unset, and set both sets of variables for jobs and hooks. No user action, but two names for the state directory sit on the path that decides how many coordinators exist, every job gets a doubled environment, the old name stays in shipped text, and a removal change with a date must follow.
- **C. An alias binary or link named `qex`.** It keeps command lines working and nothing else, and it ships a program under the name AGENTS.md forbids.

## Decision

**A.** The repository has made no release, and its Release workflow is switched off, so nobody is known to run a build of this fork under the old name. Every other option adds code on the path where a mistake runs two coordinators with two full budgets.

The new names are the plain token mapping qex to fairq and QEX to FAIRQ. The table holds one row per interface class; each search is the command that finds every instance at `34c0167`, with its count.

| Class | What it is | Search at 34c0167 (count) | After the rename |
|---|---|---|---|
| S3 | Variables the program reads: `QEX_IDLE_EXIT_SECS`, `QEX_SAY_WAITING_AFTER_SECS`, `QEX_COORDINATOR_CEILING_SECS`, `QEX_EVENTS_RETAINED`, `QEX_PEERS_DIR`, and 8 `QEX_TEST_*` switches | `git grep -n -E 'const [A-Z_]+: &str = "QEX_' -- src` (13) | Read only as `FAIRQ_*`; a `QEX_*` variable is ignored |
| S4 | Every variable name the program holds, among them those it sets for jobs (`QEX_CPU`, `QEX_MEM`, `QEX_MEM_MB`, `QEX_CLAIM_<POOL>`, `QEX_<POOL>_DEVICES`, `QEX_<POOL>_<QTY>`) and for hooks (`QEX_JOB_ID`, `QEX_STATE`, `QEX_EXIT_CODE` and the rest) | `git grep -h -o -E 'QEX_[A-Z0-9_]+' -- src build.rs \| sort -u` (38) | Set only as `FAIRQ_*`; a job or hook script that reads `$QEX_CPU` gets an empty value |
| S5 | Config, state and socket paths: `~/.config/qex.toml`, `~/.local/state/qex`, socket folder `qex-<uid>-<hash>` in TMPDIR | `git grep -n -E 'join\("qex(\.toml)?"\)\|format!\("qex-\{uid\}-' -- src` (42) | `~/.config/fairq.toml`, `~/.local/state/fairq`, `fairq-<uid>-<hash>`; the old paths are never read, and old `qex-<uid>-` socket folders are no longer swept |
| S5a | The shared peers folder, default `/tmp/qex` | `git grep -n -E '[/]tmp/qex([^-]\|$)' -- <the rename's scope>` (25) | `/tmp/fairq`; a fairq user and an upstream qex user on one machine no longer see each other's claims |
| S6 | Behaviour tied to the process name: the supervisor's name ends a context (src/context.rs:119), a peer's name identifies the command (src/daemon.rs:2295), completions, the dashboard pane title | `git grep -n -E '== "qex"\|^ +"qex",$\|b"qex"\|generate\(shell, &mut command, "qex"' -- src` (7) | The name `fairq`; a process named `qex` is no longer recognised as ours by name (the same-file check at src/daemon.rs:2305 still applies) |
| schema | The 4 JSON Schema identifiers `https://github.com/qex/schema/*.json` (src/schema.rs:22, 172, 287, 478) | `git grep -n 'github.com/qex/schema' -- src/schema.rs` (4) | Unchanged in this task: neither that GitHub name nor a `fairq` one is ours; the titles and descriptions around them are renamed. Open point for a later decision |
| binary | The crate, the binary and its command name | `Cargo.toml` `name = "qex"` | `fairq`, with no `qex` alias |

## Upgrade steps for a machine that runs an older build

1. Make sure no old coordinator operates. There is no stop command. `qex info --no-start` prints `no coordinator operates` and exits 1 when none runs (src/commands.rs:4516-4529); otherwise it prints the coordinator's process id. Let its jobs finish: the coordinator exits after an hour with no job and no command (`IDLE_EXIT`, src/daemon.rs:41). Or, once no job of it operates, end it with `kill <that pid>` (reasoned, not run).
2. Move `~/.config/qex.toml` to `~/.config/fairq.toml`, and `~/.local/state/qex` to `~/.local/state/fairq` (or the same names under `XDG_CONFIG_HOME` and `XDG_STATE_HOME`).
3. Rename `QEX_*` to `FAIRQ_*` in your shell setup, job scripts and hooks.
4. Install the fairq build. Moving the two paths back and reinstalling the older build undoes the upgrade.

**The two-coordinator risk.** The state directory decides the socket (src/paths.rs:44, 80, 101). An old coordinator left running beside a new one holds its own full budget, so the machine admits up to twice the work until the old one goes idle. Approach A documents this and does not prevent it; step 1 is the answer. This is reasoned from the code, not run.

## Recorded decisions

- **The peers folder** default becomes `/tmp/fairq` (row S5a). The rename goes before any other change planned on src/peers.rs, unless that change has already started.
- **The schema identifiers** stay unchanged in this task (row schema).
- **crates.io:** the release workflow's crate name and `cargo install` line are renamed with the rest. Nothing is published or reserved by this change; reserving the name is the owner's step.

## Porting a branch started before the rename

The rename script is added by its own change and deleted by the rename itself. To port an older branch afterwards, fetch the script from the commit that added it:

```
git show "$(git log --diff-filter=A --format=%H -- .github/scripts/rename-fairq.py)":.github/scripts/rename-fairq.py
```

## Consequences

- Every user-visible name changes at once; nothing keeps working under the old name.
- No runtime code is added, so no runtime path can read an old name by mistake.
- The attribution to qex, the LICENSE, the upstream links and the fork's history documents stay as they are.
