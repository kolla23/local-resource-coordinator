# Skills
<!-- Modified by the local-resource-coordinator fork, 2026-11-03: no install from upstream's repository; copy the skill from a clone (issue #100). -->

A skill is a page of instructions that an agent loads when a task matches it.
The skill in `fairq/` teaches an agent to use fairq instead of a background command
and a polling loop.

## Install it

For one user, in every project:

```sh
mkdir -p ~/.claude/skills
cp -r skills/fairq ~/.claude/skills/fairq
```

For one project, so that each agent in it gets the skill:

```sh
mkdir -p .claude/skills
cp -r /path/to/fairq/skills/fairq .claude/skills/fairq
```

This repository holds `.claude/skills/fairq` as a link to `skills/fairq`, so an
agent that works on fairq itself gets the same page.

## The form

`SKILL.md` opens with a name and a description. The DESCRIPTION is the part that
decides whether an agent loads the skill, so it says WHEN to use fairq and not
what fairq is.

Other agent tools read a different file. The page is ordinary Markdown, so it
can be copied into `AGENTS.md`, a rule file, or a prompt with no change.

`fairq help agents` holds the same material inside the binary, for an agent with
no network.
