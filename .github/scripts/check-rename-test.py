#!/usr/bin/env python3
"""Tests check-rename.py and rename-fairq.py on throwaway git repositories, running the real
scripts in them. Usage: check-rename-test.py"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CHECK = os.path.join(HERE, "check-rename.py")
RENAME = os.path.join(HERE, "rename-fairq.py")
SCOPE = [".", ":!docs/fork", ":!docs/baseline", ":!docs/decision-gate", ":!experiments",
         ":!docs/specs", ":!LICENSE", ":!AGENTS.md", ":!Cargo.lock",
         ":!.github/scripts/rename-fairq.py", ":!.github/scripts/check-rename.py",
         ":!.github/scripts/check-rename-test.py"]
LINK = "link"
EXEC = "exec"
BODY = b"".join(b"line %d of the skill page\n" % n for n in range(12))
TOOLS = (".github/scripts/rename-fairq.py", ".github/scripts/check-rename.py",
         ".github/scripts/check-rename-test.py")
KEPT_DIRS = ("docs/fork", "docs/baseline", "docs/decision-gate", "docs/specs", "experiments")


def g(repo, *args, data=None):
    done = subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", *args],
                          input=data, capture_output=True, check=True)
    return done.stdout.decode("utf-8", "replace").strip()


def apply(repo, spec):
    for path, value in spec.items():
        full = os.path.join(repo, path)
        if value is None:
            g(repo, "rm", "-q", "--", path)
            continue
        kind, data = value if isinstance(value, tuple) else ("file", value)
        if kind == LINK:
            sha = g(repo, "hash-object", "-w", "--stdin", data=data)
            g(repo, "update-index", "--add", "--cacheinfo", f"120000,{sha},{path}")
            g(repo, "checkout-index", "-f", "--", path)
            continue
        os.makedirs(os.path.dirname(full) or repo, exist_ok=True)
        with open(full, "wb") as f:
            f.write(data)
        g(repo, "add", "--", path)
        if kind == EXEC:
            os.chmod(full, 0o755)
            g(repo, "update-index", "--chmod=+x", "--", path)


def make(spec):
    repo = tempfile.mkdtemp()
    g(repo, "init", "-q")
    g(repo, "config", "core.autocrlf", "false")
    g(repo, "config", "core.symlinks", "true")
    apply(repo, spec)
    g(repo, "commit", "-q", "--allow-empty", "-m", "base")
    return repo


def run(script, repo, *args):
    p = subprocess.run([sys.executable, script, *args], cwd=repo, capture_output=True)
    return p.returncode, (p.stdout + p.stderr).decode("utf-8", "replace")


def check(base, head, args=("HEAD~1", "HEAD")):
    repo = make(base)
    try:
        apply(repo, head)
        g(repo, "commit", "-q", "--allow-empty", "-m", "head")
        return run(CHECK, repo, *args)
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def tree(repo):
    return g(repo, "write-tree")


def blob_id(repo, path):
    return g(repo, "rev-parse", f":{path}")


def repo_lines(*greps):
    root = g(HERE, "rev-parse", "--show-toplevel")
    out = []
    for args in greps:
        p = subprocess.run(["git", "-C", root, "grep", "-h", *args, "--", *SCOPE],
                           capture_output=True)
        out.append(p.stdout)
    return out


CHECKER_ROWS = [
    # (name, base, head, want exit code, texts the output must hold)
    ("control-pure", {"a.txt": b"run qex now\nQEX_CPU=2\nother\n"},
     {"a.txt": b"run fairq now\nFAIRQ_CPU=2\nother\n"}, 0, ["listed lines: 0", "pure renamed lines: 2"]),
    ("moved-file-pure", {"skills/qex/SKILL.md": b"name: qex\n" + BODY},
     {"skills/qex/SKILL.md": None, "skills/fairq/SKILL.md": b"name: fairq\n" + BODY},
     0, ["listed lines: 0", "pure renamed lines: 1"]),
    ("extra-edit", {"a.txt": b"run qex now\n"}, {"a.txt": b"run fairq later\n"},
     1, ["a.txt:1: not the base line renamed: run fairq later"]),
    ("kept-token", {"a.txt": b"see stephenc/qex and qex\n"}, {"a.txt": b"see stephenc/fairq and fairq\n"},
     1, ["a.txt:1: a kept token that was renamed: see stephenc/fairq and fairq"]),
    ("added-line", {"a.txt": b"qex\n"}, {"a.txt": b"fairq\nnew line\n"},
     1, ["a.txt:2: a line with no base line to pair with: new line"]),
    ("removed-line", {"a.txt": b"qex\nend\ngone\n"}, {"a.txt": b"fairq\nend\n"},
     1, ["a.txt:2: 1 base line(s) removed after this line"]),
    ("wrong-case", {"a.txt": b"qex\n"}, {"a.txt": b"Fairq\n"},
     1, ["a.txt:1: not the base line renamed: Fairq"]),
    ("plain-edit", {"a.txt": b"abc\n"}, {"a.txt": b"abd\n"},
     1, ["a.txt:1: not the base line renamed: abd"]),
    ("wrong-path", {"d/qex.txt": b"qex\n" + BODY},
     {"d/qex.txt": None, "d/other.txt": b"fairq\n" + BODY},
     1, ["d/other.txt:0: a path that is not the mapped form of d/qex.txt"]),
    ("rewrapped-line", {"a.txt": b"one qex two\n"}, {"a.txt": b"one fairq\ntwo\n"},
     1, ["a line with no base line to pair with: one fairq"]),
    ("new-file", {"a.txt": b"x\n"}, {"b.txt": b"fairq\n"},
     1, ["b.txt:1: a line of a new file: fairq"]),
    ("deleted-file", {"a.txt": b"x\n", "b.txt": b"qex\n"}, {"b.txt": None},
     1, ["b.txt:0: a deleted file"]),
    ("link-target", {"skills/qex/SKILL.md": BODY, ".claude/skills/qex": (LINK, b"../../skills/qex")},
     {".claude/skills/qex": None, ".claude/skills/fairq": (LINK, b"../../skills/fairq")},
     1, [".claude/skills/fairq:1: a line of a new file: ../../skills/fairq",
         ".claude/skills/qex:0: a deleted file"]),
    ("mode-change", {"run.sh": b"echo\n"}, {"run.sh": (EXEC, b"echo\n")},
     1, ["run.sh:0: a mode change 100644 -> 100755"]),
    ("base-name-changed-file", {"a.txt": b"fairq and qex\n"}, {"a.txt": b"fairq and fairq\n"},
     3, ["a changed file holds the new name at the base", "a.txt"]),
    ("base-name-unchanged-file", {"a.txt": b"qex\n", "adr.md": b"FAIRQ_ and fairq.toml\n"},
     {"a.txt": b"fairq\n"}, 0, ["listed lines: 0", "pure renamed lines: 1"]),
    ("not-utf8", {"a.bin": bytes([255]) + b" qex\n"}, {"a.bin": bytes([255]) + b" fairq\n"},
     0, ["listed lines: 0", "pure renamed lines: 1"]),
    ("crlf", {"a.txt": b"qex" + bytes([13, 10])}, {"a.txt": b"fairq" + bytes([13, 10])},
     0, ["listed lines: 0", "pure renamed lines: 1"]),
]


def checker_rows():
    results = []
    for name, base, head, want, texts in CHECKER_ROWS:
        rc, out = check(base, head)
        results.append((name, rc == want and all(t in out for t in texts), out))
    rc, out = check({"a.txt": b"x\n"}, {}, args=())
    results.append(("usage", rc == 2 and "usage: check-rename.py" in out, out))
    return results


SOURCE = {
    "a.txt": b"qex\nQEX_CPU\nmyqexword\nnone\n",
    "plain.txt": b"no name here\n",
    "skills/qex/SKILL.md": b"name: qex\n" + BODY,
    "tools/qex-run.sh": (EXEC, b"exec qex run\n"),
    "docs/fork/QEX_NOTES.md": b"qex history\n",
}
LINKED = {
    "skills/qex/SKILL.md": b"name: qex\n" + BODY,
    ".claude/skills/qex": (LINK, b"../../skills/qex"),
    "b.txt": b"qex\n",
}


def renamed(spec, rerun=False):
    repo = make(spec)
    rc, out = run(RENAME, repo)
    if rerun:
        rc, out = run(RENAME, repo)
    return repo, rc, out


def link_ok(repo):
    ls = g(repo, "ls-files", "-s", "--", ".claude/skills")
    want = g(repo, "hash-object", "--stdin", data=b"../../skills/fairq")
    return (ls == f"120000 {want} 0\t.claude/skills/fairq"
            and os.path.isdir(os.path.join(repo, ".claude/skills/fairq"))
            and not os.path.lexists(os.path.join(repo, ".claude/skills/qex")))


def clean_tree(spec):
    repo, _rc, _out = renamed(spec)
    try:
        return tree(repo)
    finally:
        shutil.rmtree(repo, ignore_errors=True)


def script_rows():
    results = []

    def row(name, fn):
        repos = []
        try:
            ok, out = fn(repos)
        except subprocess.CalledProcessError as e:
            ok, out = False, f"git failed: {e.cmd} {e.stderr!r}"
        finally:
            for r in repos:
                shutil.rmtree(r, ignore_errors=True)
        results.append((name, ok, out))

    def content_mapped(repos):
        repo, rc, out = renamed(SOURCE)
        repos.append(repo)
        mapped = subprocess.run(["git", "-C", repo, "show", ":a.txt"], capture_output=True).stdout
        quiet = subprocess.run(["git", "-C", repo, "diff", "--quiet"]).returncode == 0
        plain = blob_id(repo, "plain.txt") == g(repo, "rev-parse", "HEAD:plain.txt")
        return rc == 0 and mapped == b"fairq\nFAIRQ_CPU\nmyfairqword\nnone\n" and quiet and plain, out

    def kept_tokens(repos):
        tokens, = repo_lines(["-E", r"stephenc/qex|base/qex-v0\.33\.0|QEX_ASSESSMENT"
                                    r"|QEX_AUDIT_EVIDENCE|github\.com/qex/schema"])
        every = (b"x stephenc/qex base/qex-v0.33.0 QEX_ASSESSMENT QEX_AUDIT_EVIDENCE "
                 b"github.com/qex/schema qex\n")
        repo, rc, out = renamed({"tokens.txt": tokens, "every.txt": every})
        repos.append(repo)
        kept = blob_id(repo, "tokens.txt") == g(repo, "rev-parse", "HEAD:tokens.txt")
        got = subprocess.run(["git", "-C", repo, "show", ":every.txt"], capture_output=True).stdout
        return rc == 0 and bool(tokens) and kept and got == every[:-4] + b"fairq\n", out

    def kept_lines(repos):
        parts = repo_lines(["-F", "Queued EXecutor"],
                           ["-i", "-E", "Modified by the local-resource-coordinator fork.*qex"],
                           ["-F", "upstream's or to `cargo install qex`"],
                           ["-F", "upstream qex's ci.yml"])
        repo, rc, out = renamed({"lines.txt": b"".join(parts)})
        repos.append(repo)
        kept = blob_id(repo, "lines.txt") == g(repo, "rev-parse", "HEAD:lines.txt")
        return rc == 0 and all(parts) and kept, out

    def regular_path_moved(repos):
        repo, rc, out = renamed(SOURCE)
        repos.append(repo)
        ls = g(repo, "ls-files", "-s")
        moved = ("100644" in g(repo, "ls-files", "-s", "--", "skills/fairq/SKILL.md")
                 and "100755" in g(repo, "ls-files", "-s", "--", "tools/fairq-run.sh")
                 and "skills/qex" not in ls and "tools/qex-run.sh" not in ls
                 and not os.path.exists(os.path.join(repo, "skills/qex/SKILL.md"))
                 and not os.path.exists(os.path.join(repo, "tools/qex-run.sh")))
        run_sh = subprocess.run(["git", "-C", repo, "show", ":tools/fairq-run.sh"],
                                capture_output=True).stdout
        kept = blob_id(repo, "docs/fork/QEX_NOTES.md") == g(repo, "rev-parse", "HEAD:docs/fork/QEX_NOTES.md")
        return rc == 0 and moved and run_sh == b"exec fairq run\n" and kept, out

    def second_run(repos):
        repo, _rc, _out = renamed(SOURCE)
        repos.append(repo)
        first = tree(repo)
        rc, out = run(RENAME, repo)
        return (rc == 0 and tree(repo) == first and "files with changed content: 0" in out
                and "moved paths: 0" in out), out

    def kept_paths(repos):
        spec = {f"{d}/n.md": b"qex " + d.encode() + b"\n" for d in KEPT_DIRS}
        spec.update({"LICENSE": b"qex\n", "AGENTS.md": b"qex\n", "Cargo.lock": b'name = "qex"\n',
                     "a.txt": b"qex\n"})
        repo = make(spec)
        repos.append(repo)
        before = [g(repo, "rev-parse", f"HEAD:{p}") for p in (*KEPT_DIRS, "LICENSE", "AGENTS.md", "Cargo.lock")]
        rc, out = run(RENAME, repo)
        after = [g(repo, "write-tree", f"--prefix={p}/") for p in KEPT_DIRS]
        after += [blob_id(repo, p) for p in ("LICENSE", "AGENTS.md", "Cargo.lock")]
        return rc == 0 and before == after and "files with changed content: 1" in out, out

    def tools_kept(repos):
        spec = {path: b"qex QEX fairq\n" for path in TOOLS}
        spec["a.txt"] = b"qex\n"
        repo = make(spec)
        repos.append(repo)
        before = [g(repo, "rev-parse", f"HEAD:{p}") for p in TOOLS]
        rc, out = run(RENAME, repo)
        after = [blob_id(repo, p) for p in TOOLS]
        return rc == 0 and before == after and "files with changed content: 1" in out, out

    def list_writes_nothing(repos):
        repo = make(SOURCE)
        repos.append(repo)
        before = (tree(repo), g(repo, "status", "--porcelain"))
        rc, listed = run(RENAME, repo, "--list")
        after = (tree(repo), g(repo, "status", "--porcelain"))
        rc2, written = run(RENAME, repo)
        figures = [ln for ln in listed.splitlines() if not ln.startswith(("content ", "move    ", "link    "))]
        return (rc == 0 and rc2 == 0 and before == after and "move    skills/qex/SKILL.md" in listed
                and figures == written.splitlines()), listed + written

    def marker_byte(repos):
        repo = make({"a.txt": b"qex\n", "m.bin": b"qex" + bytes([1]) + b"\n"})
        repos.append(repo)
        before = tree(repo)
        rc, out = run(RENAME, repo)
        return rc == 3 and "m.bin: holds the byte 0x01" in out and tree(repo) == before, out

    def real_directory_symlink(repos):
        repo, rc, out = renamed(LINKED)
        repos.append(repo)
        return rc == 0 and link_ok(repo), out

    def second_run_with_link(repos):
        repo, _rc, _out = renamed(LINKED)
        repos.append(repo)
        first = tree(repo)
        rc, out = run(RENAME, repo)
        return rc == 0 and tree(repo) == first and link_ok(repo) and "moved paths: 0" in out, out

    def crash(state):
        def fn(repos):
            want = clean_tree(LINKED)
            repo = make(LINKED)
            repos.append(repo)
            state(repo)
            rc, out = run(RENAME, repo)
            return rc == 0 and tree(repo) == want and link_ok(repo), out
        return fn

    def after_write_before_add(repo):
        with open(os.path.join(repo, "b.txt"), "wb") as f:
            f.write(b"fairq\n")

    def after_new_link(repo):
        run(RENAME, repo)
        sha = g(repo, "rev-parse", "HEAD:.claude/skills/qex")
        g(repo, "update-index", "--add", "--cacheinfo", f"120000,{sha},.claude/skills/qex")
        g(repo, "checkout-index", "-f", "--", ".claude/skills/qex")

    def after_old_link_gone(repo):
        run(RENAME, repo)
        sha = g(repo, "rev-parse", "HEAD:.claude/skills/qex")
        g(repo, "update-index", "--add", "--cacheinfo", f"120000,{sha},.claude/skills/qex")

    def unstaged_edit_refuses(repos):
        repo = make(SOURCE)
        repos.append(repo)
        with open(os.path.join(repo, "a.txt"), "wb") as f:
            f.write(b"someone's edit\n")
        before = tree(repo)
        rc, out = run(RENAME, repo)
        with open(os.path.join(repo, "a.txt"), "rb") as f:
            same = f.read() == b"someone's edit\n"
        return rc == 3 and "the work tree holds an edit" in out and tree(repo) == before and same, out

    row("content-mapped", content_mapped)
    row("kept-tokens", kept_tokens)
    row("kept-lines", kept_lines)
    row("regular-path-moved", regular_path_moved)
    row("second-run", second_run)
    row("kept-paths", kept_paths)
    row("tools-kept", tools_kept)
    row("list-writes-nothing", list_writes_nothing)
    row("marker-byte", marker_byte)
    row("real-directory-symlink", real_directory_symlink)
    row("second-run-with-link", second_run_with_link)
    row("crash-after-write-before-add", crash(after_write_before_add))
    row("crash-after-new-link", crash(after_new_link))
    row("crash-after-old-link-gone", crash(after_old_link_gone))
    row("unstaged-edit-refuses", unstaged_edit_refuses)
    return results


def main():
    failed = 0
    for name, ok, out in checker_rows() + script_rows():
        failed += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
        if not ok:
            print("\n".join("     " + line for line in out.splitlines()))
    print(f"\n{failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
