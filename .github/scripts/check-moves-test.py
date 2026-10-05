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
HEAD_SYS = "pub fn total() -> u64 {\n    crate::os::total()\n}\n\npub fn keep() {}\n"
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
     {"src/sys.rs": HEAD_SYS, "src/os/linux.rs": "/// Total memory.\n" + LINUX + "\n" + FIELD,
      "src/os/macos.rs": MACOS},
     0, ['moved: src/sys.rs:3 total [#[cfg(target_os = "linux")]] -> src/os/linux.rs:3  identical',
         'moved: src/sys.rs:8 total [#[cfg(target_os = "macos")]] -> src/os/macos.rs:2  identical',
         "moved: src/sys.rs:13 field", "+    crate::os::total()"]),
    ("a cfg swapped during the move",
     {"src/a.rs": LINUX}, {"src/a.rs": None, "src/b.rs": LINUX.replace("linux", "macos")},
     1, ["NOT A MOVE: src/a.rs:2 total"]),
    ("an attribute added during the move",
     {"src/a.rs": LINUX}, {"src/a.rs": None, "src/b.rs": "#[cfg(test)]\n" + LINUX},
     1, ["NOT A MOVE: src/a.rs:2 total"]),
    ("a comment above dropped during the move",
     {"src/a.rs": "// SAFETY: one caller.\n" + LINUX},
     {"src/a.rs": None, "src/b.rs": LINUX},
     1, ["NOT A MOVE: src/a.rs:3 total"]),
    ("a path changed inside a string",
     {"src/a.rs": 'pub fn f() -> &\'static str {\n    "crate::x"\n}\n'},
     {"src/a.rs": None, "src/b.rs": 'pub fn f() -> &\'static str {\n    "x"\n}\n'},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("a path changed after std::",
     {"src/a.rs": "pub fn f() {\n    use std::os::unix::fs::MetadataExt;\n}\n"},
     {"src/a.rs": None, "src/b.rs": "pub fn f() {\n    use std::os::linux::fs::MetadataExt;\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("a moved body whose calls gain a path",
     {"src/sys.rs": FIELD + "pub fn total() -> u64 {\n    field(\"MemTotal:\")\n}\n"},
     {"src/sys.rs": FIELD, "src/os/unix.rs": TOTAL % ("super::field", "MemTotal:")},
     0, ["moved: src/sys.rs:5 total -> src/os/unix.rs:1  identical apart from call paths"]),
    ("a call that loses os::linux:: is still a move",
     {"src/sys.rs": "pub fn g() -> u8 {\n    os::linux::h()\n}\n"},
     {"src/sys.rs": None, "src/os/linux.rs": "pub fn g() -> u8 {\n    h()\n}\n"},
     0, ["identical apart from call paths"]),
    ("a call that loses a bare linux:: is not a move",
     {"src/a.rs": "pub fn g() -> u8 {\n    linux::h()\n}\n"},
     {"src/a.rs": None, "src/b.rs": "pub fn g() -> u8 {\n    h()\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 g"]),
    ("a path changed in a use group is not a move",
     {"src/a.rs": "pub fn f() {\n    use std::os::{unix::fs::MetadataExt};\n}\n"},
     {"src/a.rs": None, "src/b.rs": "pub fn f() {\n    use std::os::{linux::fs::MetadataExt};\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("a use path that loses crate:: is not a move",
     {"src/a.rs": "pub fn f() {\n    use crate::sys::x;\n}\n"},
     {"src/a.rs": None, "src/b.rs": "pub fn f() {\n    use x;\n}\n"},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("a deleted function with an unchanged twin in another changed file",
     {"src/a.rs": "fn helper() -> u8 {\n    1\n}\n", "src/b.rs": "fn helper() -> u8 {\n    1\n}\n"},
     {"src/a.rs": None, "src/b.rs": "fn helper() -> u8 {\n    1\n}\n\nfn other() {}\n"},
     1, ["NOT A MOVE: src/a.rs:1 helper"]),
    ("a twin in the same file changed in place",
     {"src/a.rs": "impl A {\n    fn f() -> u8 {\n        1\n    }\n}\n"
                  "impl B {\n    fn f() -> u8 {\n        1\n    }\n}\n"},
     {"src/a.rs": "impl A {\n    fn f() -> u8 {\n        1\n    }\n}\n"
                  "impl B {\n    fn f() -> u8 {\n        2\n    }\n}\n"},
     1, ["NOT A MOVE: src/a.rs:7 f"]),
    ("a twin in the same file deleted",
     {"src/a.rs": "impl A {\n    fn f() -> u8 {\n        1\n    }\n}\n"
                  "impl B {\n    fn f() -> u8 {\n        1\n    }\n}\n"},
     {"src/a.rs": "impl A {\n    fn f() -> u8 {\n        1\n    }\n}\nimpl B {}\n"},
     1, ["NOT A MOVE: src/a.rs:7 f"]),
    ("a changed line that starts with -- is listed",
     {"src/a.rs": 'pub const HELP: &str = r"\n--signal\n--other\n";\n'},
     {"src/a.rs": 'pub const HELP: &str = r"\n--other\n";\n'},
     0, ["1 changed lines are not part of a move", "src/a.rs:2 ---signal"]),
    ("a visibility change is not a move",
     {"src/a.rs": "pub fn f() -> u8 {\n    1\n}\n"},
     {"src/b.rs": "pub(crate) fn f() -> u8 {\n    1\n}\n", "src/a.rs": None},
     1, ["NOT A MOVE: src/a.rs:1 f"]),
    ("braces in comments, strings, raw strings and chars",
     {"src/sys.rs": TRICKY + "\npub fn keep() {}\n"},
     {"src/sys.rs": "pub fn keep() {}\n", "src/os/unix.rs": TRICKY},
     0, ["moved: src/sys.rs:1 tricky -> src/os/unix.rs:1  identical", "0 changed lines"]),
    ("a re-indented function is not a move",
     {"src/a.rs": "mod inner {\n    pub fn f() -> u8 {\n        let x = 1;\n        x\n    }\n}\n"},
     {"src/a.rs": "mod inner {}\n", "src/b.rs": "pub fn f() -> u8 {\n    let x = 1;\n    x\n}\n"},
     1, ["NOT A MOVE: src/a.rs:2 f"]),
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
