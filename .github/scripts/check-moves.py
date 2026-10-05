#!/usr/bin/env python3
"""Checks that a refactor only moves Rust functions (docs/fork/PORT_PLAN.md, "Code only moves").
Tests: check-moves-test.py"""
import re
import subprocess
import sys

USAGE = """usage: check-moves.py <base> [<head>]
(from the repository root; <head> defaults to HEAD)

Every function in a changed .rs file whose body changed or disappeared between
<base> and <head> must have an identical copy at <head>: the same name, signature
and body. Only the visibility (pub, pub(crate), ...) and the call path (crate::,
super::, self::, sys::, os::...::) may differ. Prints each move, then every
changed line that is not part of a move. Exits 1 when a changed function has no
identical copy, 2 on a usage error."""
CALL_PATH = re.compile(r"\b(?:(?:crate|super|self|sys|os|unix|linux|macos|windows)::)+")
VISIBILITY = re.compile(r"^\s*pub(?:\s*\([^)]*\))?\s+")
FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", re.M)


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True)
    if r.returncode != 0:
        sys.exit(f"check-moves: `git {' '.join(args)}` failed: "
                 f"{r.stderr.decode('utf-8', 'replace').strip()}")
    return r.stdout.decode("utf-8")


def show(rev, path):
    r = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True)
    return r.stdout.decode("utf-8").replace("\r\n", "\n") if r.returncode == 0 else None


def mask(text):
    """Comments, strings and char literals blanked (newlines kept), so a brace or `fn`
    inside them doesn't count."""
    out, i, n = list(text), 0, len(text)

    def blank(a, b):
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif (m := re.match(r'b?r(#*)"', text[i:i + 260])) and (i == 0 or not text[i - 1].isalnum()):
            end = text.find('"' + m[1], i + len(m[0]))
            j = n if end < 0 else end + 1 + len(m[1])
            blank(i, j)
            i = j
        elif c == '"' or text.startswith('b"', i):
            j = i + (2 if c == "b" else 1)
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            blank(i, j + 1)
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'", text[i:i + 12])
            if m:
                blank(i, i + len(m[0]))
                i += len(m[0])
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


def functions(text):
    """Each function with a body; `lines` also covers the attributes and docs above it, which
    move with it."""
    masked, found, lines = mask(text), [], text.split("\n")
    starts = [0] + [k + 1 for k, ch in enumerate(text) if ch == "\n"]

    def line(pos):
        lo, hi = 0, len(starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            lo, hi = (mid, hi) if starts[mid] <= pos else (lo, mid - 1)
        return lo + 1

    for m in FN.finditer(masked):
        depth, j = 0, m.end()
        while j < len(masked) and not (depth == 0 and masked[j] in "{;"):
            depth += (masked[j] in "([") - (masked[j] in ")]")
            j += 1
        if j >= len(masked) or masked[j] == ";":
            continue
        open_at, depth, k = j, 0, j
        while k < len(masked):
            depth += (masked[k] == "{") - (masked[k] == "}")
            if depth == 0:
                break
            k += 1
        start = text.rfind("\n", 0, m.start()) + 1
        first, cfg, above = line(start), [], line(start) - 2
        while above >= 0 and lines[above].strip().startswith(("#[", "//")):
            if lines[above].strip().startswith("#[cfg"):
                cfg.insert(0, lines[above].strip())
            above -= 1
        found.append({
            "name": m[1], "cfg": " ".join(cfg), "line": first,
            "sig": " ".join(VISIBILITY.sub("", text[start:open_at]).split()),
            "body": undent(text[open_at:k + 1],
                           len(lines[first - 1]) - len(lines[first - 1].lstrip())),
            "lines": range(above + 2, line(k) + 1),
        })
    return found


def undent(body, indent):
    """The body as if its `fn` line started at column 0, so a move between nesting levels
    still compares equal."""
    first, *rest = body.split("\n")
    return "\n".join([first] + [r[indent:] if not r[:indent].strip() else r.lstrip() for r in rest])


def same(a, b, ignore_paths):
    norm = (lambda s: CALL_PATH.sub("", s)) if ignore_paths else (lambda s: s)
    return norm(a["sig"]) == norm(b["sig"]) and norm(a["body"]) == norm(b["body"])


def find_copy(fn, new, used):
    for exact in (True, False):
        for g_file, g_fns in new.items():
            for g in g_fns:
                if (g_file, g["line"]) not in used and g["name"] == fn["name"] \
                        and same(fn, g, not exact):
                    return g_file, g, exact
    return None


def main():
    if len(sys.argv) not in (2, 3):
        print(USAGE, file=sys.stderr)
        sys.exit(2)
    base, head = sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else "HEAD"
    files = [f for f in git("diff", "--name-only", "-z", "--no-renames", base, head, "--", "*.rs")
             .split("\0") if f]
    old = {f: functions(t) for f in files if (t := show(base, f)) is not None}
    new = {f: functions(t) for f in files if (t := show(head, f)) is not None}
    used, moves, failures = set(), [], []
    for f, fns in old.items():
        for fn in fns:
            if any(g["name"] == fn["name"] and same(fn, g, False) for g in new.get(f, [])):
                continue
            target = find_copy(fn, new, used)
            if target is None:
                failures.append((f, fn))
                continue
            used.add((target[0], target[1]["line"]))
            moves.append((f, fn, *target))

    for f, fn, g_file, g, exact in moves:
        how = "identical" if exact else "identical apart from call paths"
        cfg = f" [{fn['cfg']}]" if fn["cfg"] else ""
        print(f"moved: {f}:{fn['line']} {fn['name']}{cfg} -> {g_file}:{g['line']}  {how}")
    for f, fn in failures:
        print(f"NOT A MOVE: {f}:{fn['line']} {fn['name']} changed or went, with no identical copy")

    moved_old = {(f, n) for f, fn, *_ in moves for n in fn["lines"]}
    moved_new = {(g_file, n) for _, _, g_file, g, _ in moves for n in g["lines"]}
    rest = []
    for f in files:
        o = n = 0
        for row in git("diff", "-U0", "--no-color", "--no-renames", base, head, "--", f).split("\n"):
            if (h := HUNK.match(row)):
                o, n = int(h[1]), int(h[3])
            elif row.startswith("-") and not row.startswith("---"):
                if (f, o) not in moved_old and row[1:].strip():
                    rest.append(f"  {f}:{o} -{row[1:]}")
                o += 1
            elif row.startswith("+") and not row.startswith("+++"):
                if (f, n) not in moved_new and row[1:].strip():
                    rest.append(f"  {f}:{n} +{row[1:]}")
                n += 1
    print(f"\n{len(rest)} changed lines are not part of a move" + (":" if rest else "."))
    if rest:
        print("\n".join(rest))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
