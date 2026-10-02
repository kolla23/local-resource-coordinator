#!/usr/bin/env python3
"""Rejects raw control characters in the repository's text files.

An edit script that writes an escape such as "\\0" through one quoting layer
too many (a Python script inside a shell heredoc) leaves the raw byte in the
file. A raw NUL got into files that way three times while editing this
project's files; a NUL also makes git treat a file as binary, so diffs and
reviews stop showing it.

Every tracked file must hold no control character other than tab, LF, and CR
as part of a CRLF line end; a bare CR (not followed by LF) is rejected too.
Files whose extension marks them as binary (BINARY below) are skipped. Prints
each bad file with the characters and the first line, and exits 1; exits 0
when all are clean.

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


def main():
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True,
                         text=True, check=True).stdout.strip()
    names = subprocess.run(["git", "ls-files", "-z"], cwd=top, capture_output=True,
                           check=True).stdout.split(b"\0")
    bad = 0
    checked = 0
    for name in filter(None, names):
        path = name.decode("utf-8", "replace")
        # The extension only: a file named `a` or `bin` has none.
        if os.path.splitext(path)[1][1:].lower() in BINARY:
            continue
        try:
            with open(f"{top}/{path}", "rb") as f:
                data = f.read()
        except (FileNotFoundError, IsADirectoryError):
            continue  # deleted in the working tree, or a submodule
        checked += 1
        found = sorted(set(CONTROL.findall(data)))
        if found:
            first = CONTROL.search(data).start()
            line = data.count(b"\n", 0, first) + 1
            kinds = ", ".join(f"0x{c[0]:02x}" for c in found)
            print(f"{path}: control character(s) {kinds}, first on line {line}")
            bad += 1
    if bad:
        print(f"\n{bad} file(s) hold raw control characters. Write escapes such as \\0 "
              "with the Write tool, not through a heredoc, and remove the raw bytes.")
        return 1
    print(f"{checked} text files checked; no control characters.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
