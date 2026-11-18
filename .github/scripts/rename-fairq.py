#!/usr/bin/env python3
"""Renames the program from qex to fairq in the tracked files (issue #28; docs/fork/adr/rename-fairq.md).
Run once at the top of a git work tree. Proof: check-rename.py. Tests: check-rename-test.py"""
import os
import re
import subprocess
import sys

USAGE = """usage: rename-fairq.py [--list]

Maps qex to fairq and QEX to FAIRQ in every tracked file outside KEEP_PATHS, keeps
the kept tokens and kept lines as they are, and moves the tracked paths that hold
the name. Each file is read from the index, so a rerun after a crash finishes the
work. A symbolic link (mode 120000) is changed through the index and never opened,
after every move, so that a link to a directory finds its target.

Before any write it refuses (exit 3) a file whose work tree holds bytes that are
neither the index's nor the renamed ones, and a file that holds the byte 0x01,
which it uses as a marker. --list prints what a write run would do and the same
figures, and writes nothing. Exit 2 on a usage error, 3 when git fails.
"""

NL = bytes([10])
TAB = bytes([9])
NUL = bytes([0])
MARK = bytes([1])
# History, attribution, files edited by hand or by cargo, and this tool's own three files.
KEEP_PATHS = (
    "LICENSE",
    "AGENTS.md",
    "Cargo.lock",
    "docs/fork/",
    "docs/baseline/",
    "docs/decision-gate/",
    "docs/specs/",
    "experiments/",
    ".github/scripts/rename-fairq.py",
    ".github/scripts/check-rename.py",
    ".github/scripts/check-rename-test.py",
)
KEEP_LINES = [
    re.compile(rb"Modified by the local-resource-coordinator fork"),
    re.compile(rb"Queued EXecutor"),
    re.compile(rb"upstream's or to `cargo install qex`"),
    re.compile(rb"upstream qex's ci[.]yml"),
]
KEEP_TOKENS = re.compile(
    rb"stephenc/qex|base/qex-v0[.]33[.]0|QEX_ASSESSMENT|QEX_AUDIT_EVIDENCE"
    rb"|github[.]com/qex/schema"
)
TOKEN = re.compile(MARK + rb"([0-9]+)" + MARK)
MAP = ((b"qex", b"fairq"), (b"QEX", b"FAIRQ"))


def map_line(line):
    if any(p.search(line) for p in KEEP_LINES):
        return line
    kept = []

    def hide(m):
        kept.append(m.group(0))
        return MARK + str(len(kept) - 1).encode() + MARK

    out = KEEP_TOKENS.sub(hide, line)
    for old, new in MAP:
        out = out.replace(old, new)
    return TOKEN.sub(lambda m: kept[int(m.group(1))], out)


def map_path(path):
    return path.replace("qex", "fairq").replace("QEX", "FAIRQ")


def git(*args, data=None):
    done = subprocess.run(["git", *args], input=data, capture_output=True)
    if done.returncode != 0:
        sys.stderr.write(done.stderr.decode("utf-8", "replace"))
        sys.exit(3)
    return done.stdout


def on_disk(path, mode):
    if not os.path.lexists(path):
        return None
    if mode == "120000" and os.path.islink(path):
        return os.readlink(path).replace(os.sep, "/").encode("utf-8")
    with open(path, "rb") as f:
        return f.read()


def unlink(path):
    # Windows removes a directory link with rmdir; unlink refuses it there.
    try:
        os.unlink(path)
    except (IsADirectoryError, PermissionError):
        os.rmdir(path)


def main():
    args = sys.argv[1:]
    if args not in ([], ["--list"]):
        sys.stderr.write(USAGE)
        return 2
    listing = args == ["--list"]
    files, links = [], []
    changed_files = changed_lines = moved = 0
    for entry in git("ls-files", "-s", "-z").split(NUL):
        if not entry:
            continue
        meta, raw = entry.split(TAB, 1)
        mode, sha, _stage = meta.decode().split()
        path = raw.decode("utf-8")
        if path.startswith(KEEP_PATHS):
            continue
        old = git("cat-file", "blob", sha)
        if MARK in old:
            sys.stderr.write(f"{path}: holds the byte 0x01, which this script uses as a marker\n")
            return 3
        if mode == "120000":
            new = map_line(old)
        else:
            lines = old.split(NL)
            mapped = [map_line(line) for line in lines]
            changed_lines += sum(1 for a, b in zip(lines, mapped) if a != b)
            new = NL.join(mapped)
        new_path = map_path(path)
        disk = on_disk(path, mode)
        if mode != "120000" and disk is None:
            sys.stderr.write(f"{path}: missing from the work tree\n")
            return 3
        if disk is not None and disk not in (old, new):
            sys.stderr.write(f"{path}: the work tree holds an edit that is not the rename\n")
            return 3
        changed_files += new != old
        moved += new_path != path
        if mode == "120000":
            if new != old or new_path != path or disk is None:
                links.append((path, new_path, old, new))
        elif new != old or new_path != path:
            files.append((path, new_path, mode, old, new, disk))
    if listing:
        for path, new_path, _mode, old, new, _disk in files:
            if new != old:
                print(f"content {path}")
            if new_path != path:
                print(f"move    {path} -> {new_path}")
        for path, new_path, _old, _new in links:
            print(f"link    {path} -> {new_path}")
    else:
        for path, new_path, mode, old, new, disk in files:
            if new != old:
                if disk != new:
                    with open(path, "wb") as f:
                        f.write(new)
                git("add", "--", path)
            if new_path != path:
                os.makedirs(os.path.dirname(new_path) or ".", exist_ok=True)
                git("mv", "--", path, new_path)
        # Links go last: a link made before its target directory exists is no directory on Windows.
        for path, new_path, old, new in links:
            sha = git("hash-object", "-w", "--stdin", data=new).decode().strip()
            git("update-index", "--add", "--cacheinfo", f"120000,{sha},{new_path}")
            if on_disk(new_path, "120000") != new:
                if os.path.lexists(new_path):
                    unlink(new_path)
                git("checkout-index", "--", new_path)
            if new_path != path:
                if os.path.lexists(path):
                    unlink(path)
                git("rm", "--cached", "-q", "--", path)
    print(f"files with changed content: {changed_files}")
    print(f"changed lines (regular files): {changed_lines}")
    print(f"moved paths: {moved}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
