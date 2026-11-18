---
title: fairq inside a sandbox
description: What fairq needs from a sandbox, how to test it, and what to change when an agent reports that fairq cannot start.
---
<!-- Modified by the local-resource-coordinator fork, 2026-11-02: the state directory of a sandbox goes under /tmp, since qex refuses one below a folder that shows an unmapped owner (issue #88). -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-02: sandbox reports go to this repository's issues, not the upstream tips issue (issue #100). -->
<!-- Modified by the local-resource-coordinator fork, 2026-11-18: the program is named fairq (issue #28). -->

*[Home](index.md) · [Agents](agents.md) · [Reference](reference.md) · [Design](design.md) · [Security](security.md) · [Sandbox](sandbox.md)*

# fairq inside a sandbox

**This page is for a person.** An agent that meets this fault cannot correct it
by itself: the permissions belong to whoever starts the agent.

Some agent harnesses run every command inside a sandbox. Codex uses
[bubblewrap](https://github.com/containers/bubblewrap) (`bwrap`) on Linux, and
other harnesses use a container, `seccomp` or macOS Seatbelt. A sandbox of that
kind can stop fairq completely, and the message that an agent sees is:

```
fairq cannot make a Unix socket in /home/you/.local/state/fairq/run:
Operation not permitted (os error 1).
```

That is not a fault in fairq or in the job. It says that the sandbox refuses the
one thing that fairq needs.

## What fairq needs

fairq keeps **one coordinator process** for each user, and every fairq command talks
to it over a **Unix-domain socket**. The queue exists so that commands from
different agents and different sessions see one another; a design with no shared
process cannot do that. Four things must hold:

| | |
| --- | --- |
| A writable state directory | `$XDG_STATE_HOME/fairq`, or `~/.local/state/fairq`. It holds the records, the logs and the socket. |
| Permission to `bind` and `connect` an `AF_UNIX` socket in it | The coordinator binds; every command connects. A sandbox that refuses either one stops fairq. |
| The same directory in every fairq command | A sandbox that gives each command its own private `/tmp` or its own state directory gives each command its own empty queue. |
| A process that lives after the command ends | The coordinator and the jobs outlive the command that started them. That is the property that makes a job survive a person who stops the agent. |

## Test it in one command

Run this **inside** the sandbox, as the agent runs:

```sh
fairq info
```

A queue that answers prints the coordinator, the budget and the load. If it
reports that it cannot make a Unix socket, the sandbox is the cause. To test
the socket alone, with no fairq:

```sh
python3 -c 'import socket, os
d = os.environ.get("XDG_STATE_HOME", os.path.expanduser("~/.local/state")) + "/fairq/run"
os.makedirs(d, exist_ok=True)
p = os.path.join(d, "probe")
os.path.exists(p) and os.unlink(p)
s = socket.socket(socket.AF_UNIX); s.bind(p); s.close(); os.unlink(p)
print("a Unix socket is allowed in", d)'
```

Test the directory that **fairq** uses, and not `/tmp`: a sandbox frequently gives
`/tmp` its own rules. `fairq info --no-start` names the directory when fairq can
read it.

## What to change

**Take one of these. The first is the simplest.**

### 1. Let the fairq commands run outside the sandbox

Every harness has a way to approve a command or a program. In Codex, approve the
`fairq` commands, or add `fairq` to the programs that run with elevated permission.
fairq is a queue for long work, so it is a reasonable thing to allow: it starts
the same programs that the agent already runs, and it starts fewer of them at
one time.

### 2. Give the sandbox the state directory and the socket

With `bwrap`, the state directory must be a real, shared, writable mount, and it
must be the same one for every command. Use a fixed folder under `/tmp`, and
give every command, inside the sandbox and outside it, the same
`XDG_STATE_HOME`:

```sh
mkdir -p "/tmp/fairq-state-$USER"
bwrap \
  --bind / / \
  --dev /dev \
  --proc /proc \
  --bind "/tmp/fairq-state-$USER" "/tmp/fairq-state-$USER" \
  --setenv XDG_STATE_HOME "/tmp/fairq-state-$USER" \
  -- your-agent
```

The important part is that the directory is **bound**, and not a fresh `tmpfs`
for each command. A `--tmpfs /tmp` without that bind, or a private state
directory, gives each command its own queue, and the queue is then useless: no
command finds the jobs of any other.

Many systems empty `/tmp` at a restart, so the queue and the records of past
jobs do not survive a restart of the machine.

If the sandbox filters system calls, `socket`, `bind`, `connect` and `listen`
for `AF_UNIX` must be allowed. fairq needs no network socket, and it opens none.

### 3. Put the queue somewhere the sandbox already allows

fairq follows the XDG variables, so you can move the whole queue into a directory
that the sandbox shares:

```sh
export XDG_STATE_HOME="/tmp/fairq-state-$USER"
export XDG_CONFIG_HOME=/workspace/.fairq-config
```

Give the **same** values to every command, and to the agent itself. The state
goes under `/tmp` because inside a user namespace a folder that root owns on the
host, such as `/home` or `/workspace`, shows as an unmapped user, and fairq refuses
a state directory below it.

This corrects the state directory only. It does **not** help when the sandbox
refuses the socket itself; the message names that case exactly.

## What does not work

- **A per-command sandbox with no shared directory.** Each command then starts
  its own coordinator, sees no other job, and the queue cannot do the one thing
  it exists for.
- **A sandbox that stops every process at the end of the command.** The
  coordinator and the jobs live longer than the command on purpose. A harness
  that kills them stops the work, and `fairq list` then shows a job that no
  process serves.

## If nothing above is possible

Use `fairq run` for the work that you can wait for, and accept that the queue is
one command deep, or run the agent with no sandbox on a machine that you own.
fairq has no mode that removes the coordinator: the coordinator IS the queue, and
a design with none cannot stop two agents from filling the machine, which is the
reason fairq exists.

Tell us what your sandbox does, and what you had to change, in an
[issue](https://github.com/kolla23/local-resource-coordinator/issues).
