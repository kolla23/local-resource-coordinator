#!/usr/bin/env python3
"""Rejects raw control characters in the repository's text files.

An edit script that writes an escape such as "\\0" through one quoting layer
too many (a Python script inside a shell heredoc) leaves the raw byte in the
file. A raw NUL got into files that way three times while editing this
project's files; a NUL also makes git treat a file as binary, so diffs and
reviews stop showing it.

Every regular file in git's index must hold no control character other than
tab, LF, and CR as part of a CRLF line end; a bare CR (not followed by LF) is
rejected too. Files whose extension marks them as binary (BINARY below) are
skipped.

The content comes from git (`git ls-files -s`, `git cat-file --batch`), not
from the working tree: what is checked is what is tracked, symlinks
(mode 120000) and submodules (160000) are skipped and counted, and file names
stay raw bytes, so a name that is not UTF-8 is still checked. Prints each bad
file with the characters and the first line, and exits 1; exits 0 when all are
clean.

Usage: check-control-chars.py   (from anywhere inside the repository)
"""
import os
import re
import subprocess
import sys

CONTROL = re.compile(rb"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]|\r(?!\n)")
BINARY = {"png", "jpg", "jpeg", "gif", "ico", "webp", "pdf", "zip", "gz", "tgz", "xz",
          "bz2", "7z", "exe", "dll", "so", "dylib", "a", "o", "class", "jar", "woff",
          "woff2", "ttf", "otf", "eot", "mp3", "mp4", "wav", "sqlite", "db", "bin"}
REGULAR = (b"100644", b"100755")


def tracked_files(top):
    """(path bytes, blob id) of every regular file in the index, and the
    number of other entries (symlinks, submodules) skipped."""
    out = subprocess.run(["git", "ls-files", "-s", "-z"], cwd=top, capture_output=True,
                         check=True).stdout
    files, skipped = [], 0
    for entry in filter(None, out.split(b"\0")):
        meta, path = entry.split(b"\t", 1)
        mode, blob, _stage = meta.split(b" ")
        if mode in REGULAR:
            files.append((path, blob))
        else:
            skipped += 1
    return files, skipped


def blobs(top, ids):
    """The content of each blob, in order, from one `git cat-file --batch`."""
    out = subprocess.run(["git", "cat-file", "--batch"], cwd=top, capture_output=True,
                         input=b"".join(i + b"\n" for i in ids), check=True).stdout
    pos = 0
    for _ in ids:
        header_end = out.index(b"\n", pos)
        size = int(out[pos:header_end].split(b" ")[2])
        start = header_end + 1
        yield out[start:start + size]
        pos = start + size + 1


def main():
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True,
                         text=True, check=True).stdout.strip()
    files, skipped = tracked_files(top)
    # The extension only: a file named `a` or `bin` has none.
    text = [(p, b) for p, b in files
            if os.path.splitext(p)[1][1:].decode("ascii", "replace").lower() not in BINARY]
    bad = 0
    for (path, _), data in zip(text, blobs(top, [b for _, b in text])):
        found = sorted(set(CONTROL.findall(data)))
        if found:
            first = CONTROL.search(data).start()
            line = data.count(b"\n", 0, first) + 1
            kinds = ", ".join(f"0x{c[0]:02x}" for c in found)
            name = path.decode("utf-8", "backslashreplace")
            print(f"{name}: control character(s) {kinds}, first on line {line}")
            bad += 1
    if bad:
        print(f"\n{bad} file(s) hold raw control characters. Write escapes such as \\0 "
              "with the Write tool, not through a heredoc, and remove the raw bytes.")
        return 1
    print(f"{len(text)} text files checked; no control characters "
          f"({skipped} symlinks and submodules skipped).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
