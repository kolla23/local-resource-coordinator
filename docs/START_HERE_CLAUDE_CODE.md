# Start prompt for Claude Code (session 1)

Continue this project from the handoff package in docs/.
Read in this order: docs/CLAUDE_HANDOFF.md, docs/FOUNDATION_DECISION.md, docs/QEX_ASSESSMENT.md, docs/SPEC.md. Read docs/protocol.schema.json only when an interface needs it. docs/PACKAGE_MANIFEST.json has file hashes.

Decisions already made:
- We maintain our own independent fork of qex (https://github.com/stephenc/qex), starting from release v0.33.0 (commit 78b4e86c5ff8be0c7f4bc26bea809aa75fe230f0). Rust.
- We do not contribute upstream or contact upstream maintainers.
- Preserve the Apache-2.0 license, NOTICE files and attribution; mark files we modify.
- The project name is a placeholder. The working folder is local-resource-coordinator.
- The public schemas describe a target contract, not existing qex compatibility.
- Keep open V1 engineering work separate from deferred V2 scope.

Git and publishing:
- The final repository should keep upstream history: base it on a clone of qex at the pinned release, with these docs added. Keep the upstream remote fetch-only (disable pushing to it).
- Pushing branches and opening PRs to MY PRIVATE GitHub repo is allowed. Nothing public, no merging without my OK.
- Note: my user folder C:\Users\kolla may itself be a git repo; make sure this project has its own .git and never runs git commands against the parent.

Step 1 (plan mode, no changes yet):
1. Summarize the goal, V1 scope and deferred V2 scope in plain language.
2. List conflicts, gaps or unverified assumptions between the docs.
3. Summarize what Apache-2.0 requires of our fork.
4. Inspect this Windows machine: Rust toolchain (rustup, cargo, MSVC build tools), Git, WSL availability. List what is missing and ask before installing anything.
5. Propose a plan for milestone 1 only: set up the repository from the pinned release, build the unchanged baseline, and run its existing tests where they can run. Note that the audit says upstream requires Unix, so on native Windows the build may fail; record that honestly as a finding. Linux tests only if WSL is available; macOS is not available here and must be marked "not evaluated".
Wait for my OK.

Step 2 (after I approve):
- Do milestone 1 on its own branch. Record the pinned commit, tool versions, exact commands and results (including failures) in docs/baseline/.
- Then run /new-project.
- Open one small PR to my private GitHub repo, then stop and report in plain language.

Decision gate (report after milestone 1, before any further work):
- Try unchanged qex in WSL2 (if available) on a realistic workload of mine and report whether it already helps.
- Run a small native Windows process-ownership probe (Job Objects: start a process tree, measure its memory, cancel it cleanly) as a standalone experiment.
- Measure how much of qex is Unix-specific (modules, lines, what the Windows port would have to replace).
- Then give me a plain-language recommendation with evidence: (a) continue the qex-based fork, (b) use unchanged qex in WSL2 for now, or (c) a new core. Do not continue past this gate without my decision.
- Legal reminder: keep LICENSE and copyright notices, mark modified files, and do not use "qex" as our product name (Apache-2.0 grants no trademark rights).

Later milestones (not now): native Windows process ownership (Job Objects), whole-workload memory measurement, cancellation and crash recovery.

Never claim a build, test or benchmark passed unless it ran in this session and you show the output.
