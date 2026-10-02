#!/usr/bin/env python3
"""Tests check-control-chars.py on throwaway git repositories.

Each case commits files to a new repository and runs the real script there.
Usage: check-control-chars-test.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

CHECK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check-control-chars.py")


def run(files, untracked=None):
    repo = tempfile.mkdtemp()
    try:
        subprocess.run(["git", "init", "-q", repo], check=True)
        subprocess.run(["git", "-C", repo, "config", "core.autocrlf", "false"], check=True)
        for name, data in files.items():
            os.makedirs(os.path.dirname(os.path.join(repo, name)) or repo, exist_ok=True)
            with open(os.path.join(repo, name), "wb") as f:
                f.write(data)
        subprocess.run(["git", "-C", repo, "add", "-A"], check=True)
        for name, data in (untracked or {}).items():
            with open(os.path.join(repo, name), "wb") as f:
                f.write(data)
        p = subprocess.run([sys.executable, CHECK], cwd=repo, capture_output=True, text=True)
        return p.returncode, p.stdout
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def main():
    failed = 0

    def expect(name, want_rc, words, files, untracked=None):
        nonlocal failed
        rc, out = run(files, untracked)
        ok = rc == want_rc and words in out
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
        if not ok:
            print(f"     rc {rc}, want {want_rc}; output: {out.strip()}")

    clean = {"src/a.rs": b"fn main() {}\n", "README.md": b"text\r\nmore\n"}
    expect("clean text files pass", 0, "no control characters", clean)
    expect("a NUL in a text file fails, with its line", 1,
           "t.sh: control character(s) 0x00, first on line 2",
           {**clean, "t.sh": b"#!/bin/sh\nprintf '\x00'\n"})
    expect("an ESC fails", 1, "0x1b", {**clean, "c.py": b"RED = '\x1b[31m'\n"})
    expect("several kinds are all named", 1, "0x00, 0x07, 0x7f",
           {**clean, "m.txt": b"a\x07b\x00c\x7f\n"})
    expect("tab, CR and LF pass", 0, "no control characters",
           {**clean, "t.md": b"a\tb\r\nc\n"})
    expect("a written escape (backslash zero) passes", 0, "no control characters",
           {**clean, "t.sh": b"printf '\\0'\n"})
    expect("a NUL in a binary file passes", 0, "no control characters",
           {**clean, "i.png": b"\x89PNG\r\n\x1a\n\x00\x00"})
    expect("an untracked file is not checked", 0, "no control characters",
           clean, untracked={"scratch.txt": b"\x00"})
    expect("a binary extension is matched without case", 0, "no control characters",
           {**clean, "I.PNG": b"\x00"})

    print(f"\n{failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
