#!/usr/bin/env python3
"""Checks that a change only renames qex to fairq (issue #28; the proof for rename-fairq.py).
Tests: check-rename-test.py"""
import difflib
import re
import subprocess
import sys

USAGE = """usage: check-rename.py <base> <head>
(from anywhere in the repository)

Reads the two commits only, and of them only the files the change touches. A
changed line is PURE when the head line, with every fairq put back to qex and
every FAIRQ to QEX, is byte for byte its base line and holds no renamed kept
token. Every other changed line is LISTED, and so is a moved path that is not the
mapped base path, a mode or type change, a new file and a deleted file. Lines pair
by position inside a changed block, so a rewrapped line is listed, not proven.
The checker proves nothing about a listed line: a person reads each one.

Exit 0: nothing listed. Exit 1: it listed lines. Exit 2: usage. Exit 3: it could
not do its work (the base blob of a changed file holds the new name, so a line of
it cannot be put back; git failed). Files the change leaves alone may hold the name.
"""

NL = bytes([10])
NUL = bytes([0])
INVERSE = ((b"fairq", b"qex"), (b"FAIRQ", b"QEX"))
DAMAGED = re.compile(
    rb"stephenc/fairq|base/fairq-v|FAIRQ_ASSESSMENT|FAIRQ_AUDIT_EVIDENCE|github[.]com/fairq/schema"
)


def git(*args):
    done = subprocess.run(["git", *args], capture_output=True)
    if done.returncode != 0:
        sys.stderr.write(done.stderr.decode("utf-8", "replace"))
        sys.exit(3)
    return done.stdout


def inverse(data):
    for new, old in INVERSE:
        data = data.replace(new, old)
    return data


def blob(rev, path):
    return git("cat-file", "blob", f"{rev}:{path}")


def entries(base, head):
    fields = git("diff", "--raw", "-M", "-z", "--no-ext-diff", base, head).split(NUL)
    i = 0
    while i < len(fields) and fields[i]:
        meta = fields[i].decode().lstrip(":").split()
        old_mode, new_mode, status = meta[0], meta[1], meta[4]
        if status[0] in "RC":
            old_path, new_path = fields[i + 1].decode(), fields[i + 2].decode()
            i += 3
        else:
            old_path = new_path = fields[i + 1].decode()
            i += 2
        yield status[0], old_mode, new_mode, old_path, new_path


def main():
    if len(sys.argv) != 3:
        sys.stderr.write(USAGE)
        return 2
    base, head = sys.argv[1], sys.argv[2]
    changes = list(entries(base, head))
    holds = [old for st, _om, _nm, old, _new in changes
             if st not in "AD" and b"fairq" in blob(base, old).lower()]
    if holds:
        sys.stderr.write("a changed file holds the new name at the base, so a line cannot be put back:\n")
        sys.stderr.write("".join(f"{p}\n" for p in holds))
        return 3

    listed = []
    pure = 0
    for status, old_mode, new_mode, old_path, new_path in changes:
        if status == "A":
            for n, line in enumerate(blob(head, new_path).split(NL), 1):
                listed.append((new_path, n, "a line of a new file", line))
            continue
        if status == "D":
            listed.append((old_path, 0, "a deleted file", b""))
            continue
        if old_mode != new_mode:
            listed.append((new_path, 0, f"a mode change {old_mode} -> {new_mode}", b""))
        if inverse(new_path.encode()) != old_path.encode():
            listed.append((new_path, 0, f"a path that is not the mapped form of {old_path}", b""))
        old = blob(base, old_path).split(NL)
        new = blob(head, new_path).split(NL)
        matcher = difflib.SequenceMatcher(None, old, new, autojunk=False)
        for tag, a1, a2, b1, b2 in matcher.get_opcodes():
            if tag == "equal":
                continue
            if tag == "replace" and a2 - a1 == b2 - b1:
                for k in range(a2 - a1):
                    line = new[b1 + k]
                    if inverse(line) != old[a1 + k]:
                        listed.append((new_path, b1 + k + 1, "not the base line renamed", line))
                    elif DAMAGED.search(line):
                        listed.append((new_path, b1 + k + 1, "a kept token that was renamed", line))
                    else:
                        pure += 1
                continue
            for k in range(b1, b2):
                listed.append((new_path, k + 1, "a line with no base line to pair with", new[k]))
            if b1 == b2:
                listed.append((new_path, b1, f"{a2 - a1} base line(s) removed after this line", b""))

    for path, n, why, line in listed:
        text = line.decode("utf-8", "replace")
        print(f"{path}:{n}: {why}: {text}"[:300])
    print(f"pure renamed lines: {pure}")
    print(f"listed lines: {len(listed)}")
    return 1 if listed else 0


if __name__ == "__main__":
    sys.exit(main())
