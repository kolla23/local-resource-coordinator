#!/usr/bin/env python3
"""Tests check-control-chars.py on throwaway git repositories.

Each case stages files in a new repository and runs the real script there.
Symlinks and file names that are not UTF-8 go straight into git's index
(`update-index --index-info`), so the cases run the same on every platform.
Usage: check-control-chars-test.py
"""
import os
import shutil
import subprocess
import sys
import tempfile

CHECK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check-control-chars.py")


def run(files, untracked=None, index=None):
    """`files` are written and staged; `index` entries (mode, path bytes, content
    bytes) are added to the index only; `untracked` files are written after
    staging (a new name is untracked, a staged name becomes an unstaged change)."""
    repo = tempfile.mkdtemp()
    try:
        subprocess.run(["git", "init", "-q", repo], check=True)
        subprocess.run(["git", "-C", repo, "config", "core.autocrlf", "false"], check=True)
        for name, data in files.items():
            os.makedirs(os.path.dirname(os.path.join(repo, name)) or repo, exist_ok=True)
            with open(os.path.join(repo, name), "wb") as f:
                f.write(data)
        subprocess.run(["git", "-C", repo, "add", "-A"], check=True)
        lines = b""
        for mode, path, data in index or []:
            blob = subprocess.run(["git", "-C", repo, "hash-object", "-w", "--stdin"],
                                  input=data, capture_output=True, check=True).stdout.strip()
            lines += mode + b" " + blob + b"\t" + path + b"\n"
        if lines:
            subprocess.run(["git", "-C", repo, "update-index", "--index-info"],
                           input=lines, check=True)
        for name, data in (untracked or {}).items():
            with open(os.path.join(repo, name), "wb") as f:
                f.write(data)
        p = subprocess.run([sys.executable, CHECK], cwd=repo, capture_output=True, text=True)
        return p.returncode, p.stdout
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def main():
    failed = 0

    def expect(name, want_rc, words, files, untracked=None, index=None, absent=None):
        nonlocal failed
        rc, out = run(files, untracked, index)
        ok = rc == want_rc and words in out and (absent is None or absent not in out)
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
        if not ok:
            print(f"     rc {rc}, want {want_rc}; output: {out.strip()}")

    clean = {"src/a.rs": b"fn main() {}\n", "README.md": b"text\r\nmore\n"}
    expect("clean text files pass", 0, "2 staged text files checked; no control characters",
           clean)
    expect("a NUL in a text file fails, with its line", 1,
           "t.sh: control character(s) 0x00, first on line 2",
           {**clean, "t.sh": b"#!/bin/sh\nprintf '\x00'\n"})
    expect("an ESC fails", 1, "0x1b", {**clean, "c.py": b"RED = '\x1b[31m'\n"})
    expect("several kinds are all named", 1, "0x00, 0x07, 0x7f",
           {**clean, "m.txt": b"a\x07b\x00c\x7f\n"})
    expect("tab, LF and CRLF pass", 0, "no control characters",
           {**clean, "t.md": b"a\tb\r\nc\n"})
    expect("a bare CR fails", 1, "x.py: control character(s) 0x0d, first on line 1",
           {**clean, "x.py": b"x = 'a\rb'\n"})
    expect("a CR at the end of the file fails", 1, "0x0d", {**clean, "e.txt": b"end\r"})
    expect("a file with no extension named like a binary one is checked", 1,
           "bin: control character(s) 0x00", {**clean, "bin": b"y\x00\n", "a": b"ok\n"})
    expect("a binary extension under a dotted folder is still found", 0,
           "no control characters", {**clean, "v1.2/i.png": b"\x00"})
    expect("a text file under a folder named like a binary extension is checked", 1,
           "so.d/t.txt", {**clean, "so.d/t.txt": b"\x00"})
    expect("a written escape (backslash zero) passes", 0, "no control characters",
           {**clean, "t.sh": b"printf '\\0'\n"})
    expect("a NUL in a binary file passes", 0, "no control characters",
           {**clean, "i.png": b"\x89PNG\r\n\x1a\n\x00\x00"})
    # Only the index is read; what it didn't see is named (review round 3 of PR #38).
    expect("an untracked file is not checked, and the output says so", 0,
           "1 unstaged or untracked file(s) were not checked (stage them with `git add` "
           "and run again): scratch.txt",
           clean, untracked={"scratch.txt": b"\x00"})
    expect("an unstaged change is not checked, and the output says so", 0,
           "were not checked (stage them with `git add` and run again): src/a.rs",
           clean, untracked={"src/a.rs": b"o\x00k\n"})
    expect("a clean tree gives no warning", 0, "2 staged text files checked", clean,
           absent="warning")
    expect("a binary extension is matched without case", 0, "no control characters",
           {**clean, "I.PNG": b"\x00"})
    # What git tracks is checked, not the working tree (review round 2 of PR #38).
    expect("symlinks are skipped and counted, never followed", 0,
           "2 staged text files checked; no control characters and no invisible "
           "characters (2 symlinks and submodules skipped)",
           clean, index=[(b"120000", b"tool", b"/usr/bin/true"),
                         (b"120000", b"zero", b"/dev/zero")])
    expect("a file name that is not UTF-8 is still checked", 1,
           "caf\\xe9.txt: control character(s) 0x00",
           clean, index=[(b"100644", b"caf\xe9.txt", b"a\x00b\n")])
    expect("an executable file is checked", 1, "run.sh: control character(s) 0x00",
           clean, index=[(b"100755", b"run.sh", b"\x00")])
    # Invisible characters that an editing tool can leave raw (issue #50).
    for name, raw, code in (("a line separator", b"\xe2\x80\xa8", "U+2028"),
                            ("a paragraph separator", b"\xe2\x80\xa9", "U+2029"),
                            ("a bidi override", b"\xe2\x80\xae", "U+202E"),
                            ("a zero-width space", b"\xe2\x80\x8b", "U+200B"),
                            ("a byte-order mark", b"\xef\xbb\xbf", "U+FEFF"),
                            ("a C1 next-line control", b"\xc2\x85", "U+0085")):
        expect(f"{name} fails, with its line", 1,
               f"u.md: invisible character(s) {code}, first on line 2",
               {**clean, "u.md": b"ok\nx" + raw + b"y\n"})
    expect("several invisible kinds are all named", 1, "U+0085, U+2028, U+202E",
           {**clean, "v.txt": b"\xe2\x80\xae\xc2\x85\xe2\x80\xa8\n"})
    expect("a file with both kinds names both and counts once", 1,
           "w.txt: control character(s) 0x00, first on line 1\n"
           "w.txt: invisible character(s) U+2028, first on line 1\n\n1 file(s) hold",
           {**clean, "w.txt": b"\x00\xe2\x80\xa8\n"})
    expect("ordinary non-ASCII text passes", 0, "no invisible characters",
           {**clean, "n.md": b"caf\xc3\xa9 \xe2\x86\x92 \xc3\xbcber \xe6\xbc\xa2\n"})
    expect("a file that is not UTF-8 keeps the byte check only", 0, "no invisible characters",
           {**clean, "l.txt": b"caf\xe9 \x85\n"})

    print(f"\n{failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
