# fairq — shared admission control for independent local agents
<!-- Modified by the local-resource-coordinator fork, 2026-10-02: the test command needs --features test-fixtures. -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-02: fault reports go to this repository's issues; the testimonials and tips links are gone (issue #100). -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-03: no upstream CI, release or crates.io badges, the install section builds from the source, and the documentation link names docs/ (issue #100). -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-03: the install section gives the clone address, and says that building needs Rust 1.85 or later (issue #118). -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-18: the program is named fairq (issue #28). -->

fairq is an independently maintained fork of qex (https://github.com/stephenc/qex), Apache-2.0.

fairq gives independent agents and harnesses one shared resource scheduler for one
machine.

A coding agent sees how much CPU and memory the machine has free now. It does
not see the jobs that another agent, another subagent or another harness is
about to start. Ten agents can therefore each decide that the same free memory
is theirs.

fairq makes those decisions in one place. Claude Code, Codex, subagents, harnesses
such as CI, and ordinary shells all submit work independently. They do not know
each other, and they do not have to. fairq admits only the set of jobs whose
claims fit the machine together.

```sh
fairq submit --wait --cpu guess --mem guess --id-file train.id -- uv run train.py
fairq logs "$(cat train.id)" --grep 'ERROR|FAIL'
```

`--wait` holds the command until the job stops, gives you the exit code of the
job, and ends with the record: the state, the code and the last lines of both
streams. The output of the job stays in the log file. `guess` claims one half of
the budget, which is 75% of the machine by default; `fairq info` gives the number.

**Agents: run `fairq help agents` first.** It is one page and it covers everything.

## Three faults that one scheduler removes

**1. The machine runs out of memory.** Each agent measures correctly and each
agent decides alone. The out-of-memory killer then selects a victim. One queue
holds every claim. A job starts when the machine has capacity for its claim, and
it waits when the machine does not.

**2. Every hand-rolled watcher waits on a proxy, and a proxy can go permanently
false.** An agent with no way to wait writes a monitor. That monitor watches a
*proxy* for the work — a pattern in the process list, a line in a log, a file
that should appear — and a proxy can stop being reachable without anything
noticing. Four real watchers on one machine, in one day, slept for **95 hours
combined** on conditions that could never become true. Three of them:

```sh
while pgrep -f "solve.py"; do sleep 60; done      # matches its own command line
until grep -q "DONE" run.log; do sleep 60; done   # the writer was killed
until grep -q "READY" ~/other.log; do sleep 60; done  # that file never existed
```

A different user later found a fourth kind, on a machine two agents shared. It
had slept for 63 hours waiting for `ps -Ao args | grep -c solver` to reach zero
on that machine and on one reached over `ssh`. Its own work had finished two days
earlier; the *other* agent's work held the count above zero. On a shared machine,
"wait until nothing matches" is unsatisfiable by construction.

Only the first is the classic `pgrep -f` self-match. The others contain no
pattern bug at all — they are careful commands whose evidence simply stopped
arriving, or whose condition depended on somebody else. That is the general
failure, and it is why the fix is not "write a better pattern".

fairq waits on the **process**, not on a proxy for it. fairq is the parent of your
task and calls `waitpid` on that exact process: it exits or it does not, and
there is no third outcome. `fairq wait` therefore always returns — including 125
the moment somebody kills the job, where a log-watcher would still be sleeping.

**3. There is no handle on a running task.** fairq gives each job a UUID. Use that
id to read the state of the job, read its output, stop it, or remove it from the
queue.

## Install

This project has no releases yet, so build it from the source. Building needs
Rust 1.85 or later. Clone this repository and build in the clone:

```sh
git clone https://github.com/kolla23/local-resource-coordinator.git
cd local-resource-coordinator
cargo build --release --locked
```

The binary is `target/release/fairq`. Run it by that path, or copy it to a
directory on your path:

```sh
mkdir -p ~/.local/bin &&
install -m 755 target/release/fairq ~/.local/bin/fairq
command -v fairq
```

`mkdir` makes `~/.local/bin` when that directory is not there. `install` puts
`fairq` in that directory, and it REPLACES a `fairq` that is already there.

**`install` does not put `~/.local/bin` on your path.** `command -v fairq` must
show a file in `~/.local/bin`. If it shows nothing, add `~/.local/bin` to the
path of this shell. If it shows a different file, that file is the one the
shell will run.

fairq needs Linux or macOS. Building it needs Rust 1.85 or later; running it
needs nothing else. The first fairq command starts the background coordinator
for you, and you configure no service.

**On Windows, use WSL2** — fairq builds and runs there unchanged, and the jobs an
agent starts (`make`, `cargo`, `uv`) usually live in WSL2 anyway. A native
Windows build is a port, not a flag: fairq holds a job with `waitpid` on one
process id and stops a job tree with a process group, and Windows has neither.
Building for Windows fails with one message that says this, rather than a page
of errors.

## Five minutes

```sh
fairq submit --wait --cpu guess --mem guess --id-file t.id -- make test
```

That command blocks until the job stops, and it ends with the record. From
another shell, while it runs:

```sh
ID=$(cat t.id)
fairq list                    # what operates now, and why anything waits
fairq logs $ID --follow       # the output as it arrives
fairq kill $ID                # stop it
fairq status $ID --wait       # attach again, from any session
```

Put `fairq run -- make test` in front of a command when the work is **short and
heavy** and you wait for it now: it takes its turn in the queue, so everyone
else on the machine keeps the capacity they claimed, the output arrives as it
happens, and the exit code is the exit code of the job. When something stops the
job, `fairq run` gives 125 in place of it.

`fairq run` ties the job to that command, but only for the stops that it can
catch. Ctrl-C stops the job, and a SIGTERM on `fairq run` stops the job. A
SIGKILL, and the hangup that a terminal sends when it closes, do not reach the
job: it continues, and `fairq list` finds it. Use `fairq submit` for anything you
might come back to.

Driving many jobs at one time? Read one stream in place of a loop that asks
about each job:

```sh
fairq events --json     # one JSON object on one line for each change of state
```

A pipeline gives each stage its own log, its own exit code and its own claim:

```sh
BUILD=$(fairq submit --name build -- make)
TEST=$(fairq submit --name test --needs $BUILD -- make test)
fairq wait $TEST      # the code of the test, or 126 if the build failed
```

## Your session can stop, and the work continues

The job is not a child of your shell, and it is not a child of your agent. A
supervisor holds it in the session of the supervisor, and the record of the job is on the disk.

Somebody can therefore stop your agent, close the terminal, or replace the fairq
binary. The job continues and it still writes its result. Your wait is the only
thing that stops, and any later session attaches to the same job with the id.
This is about `fairq submit`; a job of `fairq run` also stops when Ctrl-C or a
SIGTERM stops the command that waits for it.

```sh
fairq submit --wait --id-file build.id -- make    # session 1
# a person stops the agent here. `make` continues.
fairq status "$(cat build.id)" --wait      # session 2, and the result is there
```

**This is why an agent that uses fairq is safe to interrupt.** A monitor script
holds the answer in its own memory: stop the monitor, and the answer is gone.

Put the id file where it lasts longer than the session — the project directory
or the home directory, and not a scratch directory that the harness owns or
`/tmp`. fairq gives a warning when the file goes to such a directory, because the
file would go away at the moment that the handle becomes necessary.

## A script that runs a second time starts no second job

An agent that loses its context runs its script again. Give the submission a
key, and the second run starts nothing:

```sh
fairq submit --wait --dedupe-key train:$(pwd) -- uv run train.py
```

While a job with that key waits or operates, a second submission with the same
key gives **that job's id** and exits with the code 0. `ID=$(fairq submit ...)` is
thus correct in both cases, and the script needs no test of its own.

The coordinator makes the test and the submission one step, so two agents that
run the same script in the same moment get one job and one id. A test that a
script makes itself — read `fairq list`, decide, submit — has a gap between the
read and the submission, and both agents start a job in that gap.

## The budget, and what a claim is

A claim is a number of cores and a quantity of memory. fairq starts a job when the
claims of the jobs that operate, plus this claim, stay inside the budget.

**The budget is 75% of the cores and 75% of the memory of the machine.** The
missing quarter is room for work that fairq does not control. Set another value in
`~/.config/fairq.toml`, as a percentage or as an exact size:

```toml
[budget]
cpu = "50%"
mem = "20GB"
```

**Tell fairq if it has the machine to itself.** `[enforce] mode` says who else
uses it, and neither value limits a job:

```toml
[enforce]
mode = "single-user"   # or "cooperative", the default
```

`single-user` says that fairq decides what runs here. fairq then uses 90% of the
machine instead of 75%, keeps 512MB free instead of 2GB, and looks for no other
coordinator. A value that you write always wins in either mode.

`fairq info` gives the budget and what the jobs hold now:

```console
$ fairq info
cores:           2 of 12 in use
memory:          4GB of 24GB in use
```

**`guess` and `half` claim one half of the budget**, so two such jobs operate
together. `full` and `max` claim all of it, so the job operates alone. fairq
calculates the word at the submission, and the record then holds an exact value.

**A job with no claim is not free.** It gets 1 core and the memory of the machine
divided by the number of cores, or the values in `[defaults]`. fairq also learns:
a command that ran before gets a claim from its own measurements. No claim is
ever zero, because a claim of zero would let fairq start jobs without end.

**A claim larger than the budget is accepted, and the job then operates alone.**
fairq gives a warning at the submission. It starts the job when no other job
operates and the queue was quiet for 3 seconds. The field `forced` is true for
such a job, and the job can swap or stop for memory. Set `[queue] oversized` to
`reject` to refuse it at the submission, or to `queue` to hold it. A claim above
a pool, such as a GPU, is always refused: an empty machine makes no fifth device.

**Jobs start in the order of submission, and one large job does not hold the
queue.** A smaller job behind it starts while the capacity is free. After 2 such
jobs, fairq keeps the capacity for the job at the front and starts nothing else, so
the large job always gets its turn. `[queue] max_bypass` sets that number, and
`0` gives a strict order. `--priority N` puts a job before the jobs with a
smaller number.

fairq keeps no capacity when another user, or a program that fairq never started,
holds it. Such a wait has no known end, so the jobs behind continue to start.

## fairq tells the job what it claimed

Give both `--cpu` and `--mem`, and fairq writes the claim into the environment of
the job:

```
FAIRQ_CPU=2  GOMAXPROCS=2  OMP_NUM_THREADS=2  FAIRQ_MEM=4294967296
```

A runtime then sizes its THREAD POOL to the claim, and not to the machine. This
is the one place where fairq changes how your program runs.

**No value carries memory.** fairq does not limit the memory of a job, so it does
not tell a runtime to limit its own. A hint that named a memory value would hold
a Go job and a node job to the claim and leave every other program free, and it
would decide whether a job succeeds. `FAIRQ_MEM` NAMES the claim for a script that
reads it, and no runtime acts on it.

**The value is the claim, and not a measurement.** fairq writes the number that
you asked for.

**Both halves must come from you.** `--cpu 2` with no `--mem` writes nothing,
and a claim that fairq chose or learned writes nothing. A claim of 1 core that fairq
invented would otherwise make a build of 16 cores 16 times slower, with no
message.

**fairq never replaces a value that you set.** A program that asks the operating
system directly still sees the whole machine.

`--no-limit-env-hints` turns this off for one job, and `[claims] export_env =
false` turns it off for every job.

## What fairq does not do

**fairq does not measure your program before it starts.** A claim is a promise,
and fairq trusts it. fairq measures a job while the job runs, and it uses that
measurement as the claim for the next job of the same command.

**fairq applies no limit**, so nothing stops a job that goes above its
claim. A job that claims 2GB and uses 20GB can still fill the machine. fairq tests
the free memory before each start, which limits the damage, but an accurate
claim is better.

**fairq does not limit the CPU of a job.** The `cpu` controller of cgroup v2 is
not available to a user on a usual Linux system, and macOS has no equivalent.
The queue controls the number of cores instead.

**fairq does not divide the machine between agents.** The budget controls the
total load, and the queue is the order of submission. A fan-out of 11 jobs from
one agent takes the capacity, and the 2 jobs of a second agent wait for it.
There is no share for each agent.

**fairq controls one machine.** Two machines that farm work to each other are not
coordinated.

**One coordinator serves one user.** Every agent that runs as you shares one
queue. A second user on the machine gets a second coordinator, and each
coordinator reads the claims of the other from `/tmp/fairq` before it starts a
job. That accounting is cooperative. It needs no administrator rights, and it
trusts what the other coordinator writes.

## When the kernel stops a job for memory

A training run that the kernel stops at hour four gets the state `oom` and the
exit code 99.

**fairq reports it, and it acts on it in no way.** No new attempt starts, no claim
changes, and the learner keeps nothing. The record says what you can do:

```sh
fairq submit --wait --mem 16GB --id-file train.id -- uv run train.py
```

**fairq cannot say that your job was the victim.** It reads a count of
out-of-memory kills that Linux keeps for the cgroup of its own process, and
every program of your user raises that count. A machine that is short of memory
is also the machine on which a person uses `kill -9`, so the two arrive
together. Your claim can be correct, and a larger claim is then the wrong
answer: read the `usage` field of the record and compare it with the claim.

**macOS keeps no such count**, so a kill for memory there gives the state
`killed` and the code 125. Do not wait for 99 on a Mac.

## The documentation

The full documentation is in the [`docs`](docs/index.md) directory.

| Page | What it holds |
| ---- | ------------- |
| [Agents](docs/agents.md) | The page for an agent. Start here. |
| [Sandbox](docs/sandbox.md) | What fairq needs when a harness runs each command in a sandbox. |
| [Reference](docs/reference.md) | Each command, option and configuration field. |
| [Design](docs/design.md) | The coordinator, the supervisor and the files. |
| [Security](docs/security.md) | What fairq writes, and who can read it. |

Every page is also in the binary, so an agent needs no network: `fairq help
agents`, `fairq help resources`, `fairq help config`, `fairq help events`,
`fairq schema status`.

## Completions for your shell

```sh
fairq completions bash > ~/.local/share/bash-completion/completions/fairq
fairq completions zsh  > ~/.zfunc/_fairq
fairq completions fish > ~/.config/fish/completions/fairq.fish
```

The shell then offers each job by its id and by its name, and it offers the set
that each command accepts: the jobs that operate after `fairq kill`, and the jobs
that wait after `fairq cancel`.

## A skill for your agent

`skills/fairq/SKILL.md` teaches an agent to use fairq instead of a background
command and a polling loop:

```sh
mkdir -p ~/.claude/skills && cp -r skills/fairq ~/.claude/skills/fairq
```

See [skills/README.md](skills/README.md). The same material is in the binary as
`fairq help agents`, for an agent with no network.

## Tell us what happened

**Something did not work?** Search first — `gh issue list --repo
kolla23/local-resource-coordinator --state all --search "your words"` — and
comment on the issue that exists rather than opening a second one for the same
fault. If nothing matches, [open an
issue](https://github.com/kolla23/local-resource-coordinator/issues). A small
fault counts: a word that sent you the wrong way is worth a report, and most of
what fairq does today came from one.

**If you are an agent, end every issue and every comment with the standard
footer**: your model and its exact id, your harness and its version, the output
of `fairq version`, and the system, with `unknown` for a value that you do not
know. `fairq help agents` gives the footer, and so does [the agent
page](docs/agents.md#tell-the-people-who-make-fairq). A fault that appears in one
harness and not in another is half diagnosed by those lines.

Never paste the output of `--show-env` into a public page.

## Development

```sh
cargo test --features test-fixtures -- --test-threads=2
cargo build --release
```

Each end-to-end test makes its own config, state and peer directory, starts
its own coordinator, and stops it at the end. The tests do not touch the
coordinator of the user, and they do not write into `/tmp/fairq`.

Use two test threads. Each end-to-end test starts real processes and waits for
them. With more threads, the machine becomes busy, a job starts late, and a test
reports a failure that the program does not have.

The documentation, the code comments, the help text and the error messages use
Simplified Technical English (ASD-STE100).

See [CONTRIBUTING.md](CONTRIBUTING.md). The title of a pull request gives the
next version number, so its form is part of the build.

Do not change the version in `Cargo.toml`. `main` holds `0.0.0-dev` for ever;
the number of a release lives on the tag, and `build.rs` puts the commit into
the version of your build, such as `0.0.0-dev+g98513e2`.

## License

Apache License 2.0. See [LICENSE](LICENSE).
