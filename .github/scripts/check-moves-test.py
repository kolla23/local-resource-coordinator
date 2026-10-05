#!/usr/bin/env python3
"""Tests check-moves.py on throwaway git repositories: a base commit, a head commit, and the
real script run between them. Usage: check-moves-test.py"""
import os
import shutil
import subprocess
import sys
import tempfile

CHECK = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check-moves.py")

LINUX = '''#[cfg(target_os = "linux")]
pub fn total() -> u64 {
    field("MemTotal:")
}
'''
MACOS = '''#[cfg(target_os = "macos")]
pub fn total() -> u64 {
    sysctl("hw.memsize")
}
'''
FIELD = '''#[cfg(target_os = "linux")]
fn field(key: &str) -> u64 {
    key.len() as u64
}
'''
TRICKY = '''pub fn tricky() -> usize {
    // a } in a comment
    /* and { in /* a nested */ block */
    let s = "}{";
    let r = r#"}"#;
    let c = '}';
    let l: &'static str = "{";
    s.len() + r.len() + c.len_utf8() + l.len()
}
'''
BASE_SYS = "/// Total memory.\n" + LINUX + "\n" + MACOS + "\n" + FIELD + "\npub fn keep() {}\n"
HEAD_SYS = ("/// Total memory.\npub fn total() -> u64 {\n    crate::os::total()\n}\n"
            "\npub fn keep() {}\n")
TOTAL = "pub fn total() -> u64 {\n    %s(\"%s\")\n}\n"


def run(base, head, args=None):
    repo = tempfile.mkdtemp()
    try:
        def g(*a):
            subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", *a],
                           check=True, capture_output=True)
        g("init", "-q")
        g("config", "core.autocrlf", "false")
        for files, msg in ((base, "base"), (head, "head")):
            for name in list(files):
                path = os.path.join(repo, name)
                if files[name] is None:
                    os.remove(path)
                    continue
                os.makedirs(os.path.dirname(path), exist_ok=True)
                with open(path, "w", newline="\n") as f:
                    f.write(files[name])
            g("add", "-A")
            g("commit", "-q", "--allow-empty", "-m", msg)
        args = ["HEAD~1", "HEAD"] if args is None else args
        p = subprocess.run([sys.executable, CHECK, *args], cwd=repo, capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr
    finally:
        shutil.rmtree(repo, ignore_errors=True)


CASES = [
    # (name, base files, head files, want exit code, texts the output must hold)
    ("cfg twins move to linux.rs and macos.rs; sys.rs forwards",
     {"src/sys.rs": BASE_SYS},
     {"src/sys.rs": HEAD_SYS,
      "src/os/linux.rs": LINUX.split("\n", 1)[1] + "\n" + FIELD.split("\n", 1)[1],
      "src/os/macos.rs": MACOS.split("\n", 1)[1]},
     0, ['moved: src/sys.rs:3 total [#[cfg(target_os = "linux")]] -> src/os/linux.rs:1  identical',
         'moved: src/sys.rs:8 total [#[cfg(target_os = "macos")]] -> src/os/macos.rs:1  identical',
         "moved: src/sys.rs:13 field", "src/sys.rs:3 +    crate::os::total()",
         "1 changed lines are not part of a move"]),
    ("a moved body whose calls gain a path",
     {"src/sys.rs": FIELD + "pub fn total() -> u64 {\n    field(\"MemTotal:\")\n}\n"},
     {"src/sys.rs": FIELD, "src/os/unix.rs": TOTAL % ("super::field", "MemTotal:")},
     0, ["moved: src/sys.rs:5 total -> src/os/unix.rs:1  identical apart from call paths"]),
    ("a visibility change only",
     {"src/a.rs": "pub fn f() -> u8 {\n    1\n}\n"},
     {"src/b.rs": "pub(crate) fn f() -> u8 {\n    1\n}\n", "src/a.rs": None},
     0, ["moved: src/a.rs:1 f -> src/b.rs:1  identical"]),
    ("braces in comments, strings, raw strings and chars",
     {"src/sys.rs": TRICKY + "\npub fn keep() {}\n"},
     {"src/sys.rs": "pub fn keep() {}\n", "src/os/unix.rs": TRICKY},
     0, ["moved: src/sys.rs:1 tricky -> src/os/unix.rs:1  identical", "0 changed lines"]),
    ("a function moved out of a nested module",
     {"src/a.rs": "mod inner {\n    pub fn f() -> u8 {\n        let x = 1;\n        x\n    }\n}\n"},
     {"src/a.rs": "mod inner {}\n", "src/b.rs": "pub fn f() -> u8 {\n    let x = 1;\n    x\n}\n"},
     0, ["moved: src/a.rs:2 f -> src/b.rs:1  identical"]),
    ("a body changed during the move",
     {"src/sys.rs": FIELD + "pub fn total() -> u64 {\n    field(\"MemTotal:\")\n}\n"},
     {"src/sys.rs": FIELD, "src/os/linux.rs": TOTAL % ("field", "MemFree:")},
     1, ["NOT A MOVE: src/sys.rs:5 total"]),
    ("a change inside a moved string",
     {"src/sys.rs": TRICKY},
     {"src/sys.rs": None, "src/os/unix.rs": TRICKY.replace('"}{"', '"}{}"')},
     1, ["NOT A MOVE: src/sys.rs:1 tricky"]),
    ("a signature changed during the move",
     {"src/a.rs": "pub fn f(x: u8) -> u8 {\n    x\n}\n"},
     {"src/a.rs": None, "src/b.rs": "pub fn f(x: u16) -> u8 {\n    x\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("a body changed in place",
     {"src/a.rs": "pub fn f() -> u8 {\n    1\n}\n"},
     {"src/a.rs": "pub fn f() -> u8 {\n    2\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 f", "src/a.rs:2 +    2"]),
    ("unchanged functions are not listed",
     {"src/a.rs": "pub fn f() {}\n\npub fn g() {}\n"},
     {"src/a.rs": "pub fn f() {}\n\npub fn g() {}\n\npub fn h() {}\n"},
     0, ["1 changed lines are not part of a move", "src/a.rs:5 +pub fn h() {}"]),
]


def main():
    failed = 0
    for name, base, head, want, texts in CASES:
        rc, out = run(base, head)
        ok = rc == want and all(t in out for t in texts)
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
        if not ok:
            print(f"     exit {rc}, want {want}; output:")
            print("\n".join("     " + r for r in out.splitlines()))
    rc, out = run({"a.rs": "fn f() {}\n"}, {"a.rs": "fn f() {}\n"}, args=[])
    ok = rc == 2 and "usage" in out
    failed += not ok
    print(f"{'ok  ' if ok else 'FAIL'} no arguments is a usage error")
    print(f"\n{failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
