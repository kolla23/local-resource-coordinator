# gc free-ids ADR: qex gc asks the coordinator which ids are free before it removes a directory

**Date:** 2026-11-17
**Status:** proposed; accepted when the owner merges the PR that adds this file
**Context:** issue #200 and its fix; the owner's decisions on the plan of that fix (2026-11-17, quoted under "The owner's decisions"). Source citations are at commit `d31999a` unless they say otherwise. This ADR takes no number, as the per-job TEMP ADR and the job pid namespace ADR take none.

## Context

`qex gc` removes each directory under `jobs/` that has no readable record and whose id is not in the coordinator's job list (src/commands.rs:6768, the scan at 6818-6837, the removal at 6879). The coordinator admits a job in three steps (`handle_submit`, src/daemon.rs:2941): under its state lock it tests the id and the directory and holds the id in `State::submitting` (3061-3088, 3166); with the lock given back it makes `jobs/<id>` and writes the record (3198-3200); under the lock again it puts the job in its map, which ends the hold (3233).

Between the mkdir and the record, the directory has no record, and its id is in no List answer: the List holds jobs only (src/daemon.rs:2172-2195). A gc that runs then removes the directory of a Submit in progress, and the Submit fails with `qex could not write the job record`. Reading the List again after the scan does not help, since no List names an id that is only held by a submission.

This changes the IPC (a new request and its answer), so it needs an ADR first (AGENTS.md).

## Decisions

### D1 The request `free_ids`

The CLI sends `{"op":"free_ids","ids":[...]}`. The coordinator answers `{"result":"free_ids","ids":[...]}`: the ids of the request, in its order, for which `State::id_in_use` (src/daemon.rs:1014) gives none, read under one hold of the state lock. That is the lock a Submit tests and holds its id under, so "free" means exactly "a Submit could take this id": no job that waits, runs or has stopped and is still held, and no submission in progress. The request reads state and writes nothing. The answer lists what is free, not what is held, so a lost, cut or wrong answer leaves a directory in place and never removes one.

### D2 The capability `free-ids`

The coordinator names `free-ids` in its Capabilities answer. It joins the one table `capabilities::ALL` (src/capabilities.rs:183), so `qex version` names it under `can do:` and, against an older coordinator, prints `cannot do:   free-ids (this coordinator is older)` (src/commands.rs:5636, 5644-5652).

### D3 When gc asks

gc asks after its scan and before it prints or deletes anything, in the dry run too, so the dry run says what a real run would do. It asks only about directories whose name reads as a job id, through one function that the scan's `known` test also uses; any other directory with no record can be no Submit's and is removed as before. The ids go in parts of 500, as `qex cancel` sends its ids (src/commands.rs:3323). gc removes only the directories whose id an answer names. A directory that a submission holds is not counted and gets no line.

### D4 A coordinator without the capability

gc leaves each directory with no record that is named like a job id, prints one stderr line, `qex left <n> directories with no record in place: this coordinator is older and cannot say whether a submission holds them. Run qex gc again after that coordinator stops; it stops when no job operates.` (singular for one), and keeps the code 0. Other directories with no record are removed as before.

### D5 Any other answer

An error answer stops gc with the coordinator's own text and the code 1; any other kind of answer stops it with `the coordinator did not say which ids are free`. Both come before gc deletes any record or directory, so nothing is half done.

### D6 What the answer guarantees, and for how long

A directory that a submission holds when gc asks is not removed by that gc run. A directory that the answer calls free is safe to remove while it exists without a break from the question to the removal, because a Submit refuses an id whose directory exists (src/daemon.rs:3071-3088).

The removal comes later than the answer: gc first asks the coordinator to delete each old record (src/commands.rs:6874-6878), and one such deletion can wait for a stop hook, 30 s by default (src/lifecycle.rs:485-491). If the directory goes away in that time and a Submit under the same id makes it again, gc removes the new directory. Two routes lead there, both through a resend, a Submit sent again under an id already used:
1. gc scans `jobs/X` while a Submit holds X. That Submit's record write fails; the coordinator removes `jobs/X` and frees X (src/daemon.rs:3205, 3216). gc asks: X is free. A resend under X makes `jobs/X` again; gc then removes it.
2. Two gc runs and one stale `jobs/X`: both are told X is free; the first removes it, a resend under X makes it again, and the second removes that.

`qex submit` never sends an id twice (src/spec.rs:919), but the protocol allows a resend and the coordinator expects one (the comment at src/daemon.rs:3203-3204). This stays open; the upgrade path below closes it. Before this fix, both routes existed and the window was wider.

## Options

- **A. A read-only request (chosen).** gc keeps the scan and the removal and asks the coordinator which ids are free. Small, additive on the wire, and the one rule for "free" stays in the coordinator.
- **B. The coordinator sweeps.** The coordinator's lock alone does not cover the window, since the mkdir and the record writes run with it given back (src/daemon.rs:3168-3200); B needs the same test of `submitting`, and either removes whole directories under the lock, which stops the scheduler (the reason `finish_removal` runs outside it, src/lifecycle.rs:361-368), or needs a second hold. More code for the same guarantee.
- **C. A temporary name and a rename.** The coordinator makes the directory under a name that is no id and renames it to `jobs/<id>` after the record. No IPC change, and it also covers an older `qex gc`, but it changes the on-disk layout, needs a rule for stale temporary directories, moves the #187 rules, and re-points seven tests that pause the mkdir in `jobs/`.
- **D. An age limit.** gc skips a directory younger than some age. Not sound: it reads a directory's mtime against the wall clock, which a clock step, a suspend or a stalled disk defeats, and no test can pin "never".

Reading the List again after the scan, with no new request, cannot work: no List names an id that only a submission holds.

## Upgrade path

A request under which the coordinator, with its state lock held, renames each free `jobs/<id>` to a name that is no id and answers with the new paths; gc then removes those. A Submit that comes later finds no directory under its id and none can be renamed away from it, so D6's routes close. It costs a request that writes, a form for the dry run that renames nothing, and more text here.

## Consequences

- The wire stays additive (src/proto.rs:6-32): an older CLI never sends `free_ids`, and this CLI meets an older coordinator through D4.
- One request of 500 ids is 19,526 bytes, far under ADR 1's request limit of 16 MiB per line (docs/fork/adr/0001-ipc-transport-and-launch.md:56).
- An older `qex gc`, a CLI from before this fix, still removes without asking, whatever the coordinator. Only option C would cover it.
- A hold that never ends (#192, a request thread that dies between the two locks) keeps its directory out of gc until the coordinator stops.
- A directory with no readable record whose id the coordinator still holds as a stopped job stays until that coordinator stops; before this fix gc removed it.
- `qex du` still counts the directory of a Submit in progress as one with no record for that moment; it reads the disk and asks no coordinator.

## The owner's decisions

The picked options, word for word (2026-11-17):
- The approach:
  A: new free_ids request (Recommended)
- Where this ADR lands:
  First commit of this PR (Recommended)
- A coordinator without the capability:
  Leave them, warn, code 0 (Recommended)
- What "held" means:
  Any id a Submit couldn't take (Recommended)
- The window after the answer (D6): "No extra code. Plan and ADR say the guarantee holds while the directory exists from question to removal; they name the resend route; the window becomes a found-in-pr issue; the ADR names a lock-held rename as the upgrade path."
  Correct the claim only (Recommended)
- The names `free_ids`, `free-ids` and this file's name:
  Keep all three (Recommended)
- The stderr line of D4:
  Add how it stops (Recommended)
