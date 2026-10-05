#!/usr/bin/env python3
"""Checks that a refactor only moves Rust functions (docs/fork/PORT_PLAN.md, "Code only moves").
Tests: check-moves-test.py"""
import os
import re
import subprocess
import sys

USAGE = """usage: check-moves.py <base> [<head>]
(from anywhere in the repository; <head> defaults to HEAD)

Scope: only top-level functions can be verified as moved; anything else is
reported as NOT A MOVE for a human to explain.

Every function in a changed .rs file at <base> must pair with its own
byte-identical copy at <head>, each copy used once: first in the same file, then
in another file (a move). The item is the function, from its visibility and
qualifiers on whatever line they start, with the attributes and comments directly
above it; a function inside any { } block (mod, impl, trait, fn) must keep the
same enclosing headers, each read whole across lines, and may not move. For a
move, only the call path may differ: a leading crate::, super::, self::, sys::,
os:: or os::<name>:: on a path in the body's code (not in the signature, a
string, a comment or a use statement, not after another ::). Prints each move,
then every changed line that is not part of a move. Exits 1 when a function has
no copy, 2 on a usage error."""
CALL_PATH = re.compile(r"(?<![\w:])(?:(?:crate|super|self|sys)::|os::(?:\w+::)?)+")
FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
QUALIFIERS = re.compile(
    r"(?<!\w)(?:(?:pub(?:\s*\([^()]*\))?|const|async|unsafe|safe|extern|default)\s+)*\Z")
USE = re.compile(r"\buse\b")
HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", re.M)


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True)
    if r.returncode != 0:
        sys.exit(f"check-moves: `git {' '.join(args)}` failed: "
                 f"{r.stderr.decode('utf-8', 'replace').strip()}")
    return r.stdout.decode("utf-8")


def show(rev, path):
    r = subprocess.run(["git", "cat-file", "blob", f"{rev}:{path}"], capture_output=True)
    return r.stdout.decode("utf-8") if r.returncode == 0 else None


def mask(text, strings=True):
    """Comments, strings and char literals blanked (newlines kept), so a brace or `fn`
    inside them doesn't count; strings=False keeps literals, for text shown or compared."""
    out, i, n = list(text), 0, len(text)

    def blank(a, b, literal=True):
        if literal and not strings:
            return
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j, literal=False)
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
            blank(i, j, literal=False)
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
    masked, plain, found, lines = mask(text), mask(text, strings=False), [], text.split("\n")
    mlines, plines = masked.split("\n"), plain.split("\n")
    starts = [0] + [k + 1 for k, ch in enumerate(text) if ch == "\n"]

    def line(pos):
        lo, hi = 0, len(starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            lo, hi = (mid, hi) if starts[mid] <= pos else (lo, mid - 1)
        return lo + 1

    def header_top(i):
        while i > 0 and lines[i - 1].strip():
            if not plines[i - 1].strip():
                lead = starts[i - 1] + len(lines[i - 1]) - len(lines[i - 1].lstrip())
                if text.startswith(("//", "/*"), lead):
                    i -= 1
                    continue
                opened = text.rfind("/*", 0, lead)
                if opened < 0 or plain[starts[line(opened) - 1]:opened].strip():
                    break
                i = line(opened) - 1
                continue
            code = mlines[i - 1].rstrip()
            if not code.endswith("]"):
                break
            p = starts[i - 1] + len(code) - 1
            while True:
                depth = 0
                while p >= 0:
                    depth += (masked[p] == "]") - (masked[p] == "[")
                    if depth == 0:
                        break
                    p -= 1
                if p <= 0 or masked[p - 1] != "#":
                    return i
                row = starts[line(p - 1) - 1]
                before = masked[row:p - 1].rstrip()
                if not before:
                    break
                if not before.endswith("]"):
                    return i
                p = row + len(before) - 1
            i = line(p - 1) - 1
        return i

    def header(b):
        """From the previous item's end, so a header split over lines (a `where` clause,
        `impl<T>` then `for A<T>`) counts whole."""
        start = max(masked.rfind(c, 0, b) for c in ";{}") + 1
        return " ".join(plain[start:b].split())

    open_at, stack, scan = {}, [], 0
    for m in FN.finditer(masked):
        while scan < m.start():
            if masked[scan] == "{":
                stack.append(scan)
            elif masked[scan] == "}" and stack:
                stack.pop()
            scan += 1
        open_at[m.start()] = list(stack)
    for m in FN.finditer(masked):
        scope = " / ".join(header(b) for b in open_at[m.start()])
        depth, j = 0, m.end()
        while j < len(masked) and not (depth == 0 and masked[j] in "{;"):
            depth += (masked[j] in "([") - (masked[j] in ")]")
            j += 1
        if j >= len(masked) or masked[j] == ";":
            continue
        depth, k = 0, j
        while k < len(masked):
            depth += (masked[k] == "{") - (masked[k] == "}")
            if depth == 0:
                break
            k += 1
        q = QUALIFIERS.search(masked, max(0, m.start() - 400), m.start()).start()
        first = line(q)
        top = header_top(first - 1)
        end = text.find("\n", k)
        found.append({
            "name": m[1], "cfg": " ".join(plain[starts[top]:q].split()), "line": first,
            "scope": scope, "nested": bool(open_at[m.start()]),
            "item": text[starts[top]:len(text) if end < 0 else end],
            "body_at": j - starts[top],
            "lines": range(top + 1, line(k) + 1),
        })
    return found


def without_call_paths(text):
    masked = mask(text)
    out, last = [], 0
    for m in CALL_PATH.finditer(masked):
        last_use = max((u.end() for u in USE.finditer(masked, 0, m.start())), default=-1)
        if last_use >= 0 and ";" not in masked[last_use:m.start()]:
            continue
        out.append(text[last:m.start()])
        last = m.end()
    return "".join(out) + text[last:]


def same(a, b, ignore_paths):
    if ignore_paths:
        head_a, body_a = a["item"][:a["body_at"]], a["item"][a["body_at"]:]
        head_b, body_b = b["item"][:b["body_at"]], b["item"][b["body_at"]:]
        return head_a == head_b and without_call_paths(body_a) == without_call_paths(body_b)
    return a["item"] == b["item"]


def diff_lines(base, head, path):
    removed, added, o, n, o_left, n_left = {}, {}, 0, 0, 0, 0
    rows = git("diff", "-U0", "--text", "--no-color", "--no-renames", "--no-ext-diff",
               "--no-textconv", base, head, "--", path).split("\n")
    for row in rows:
        if o_left == 0 and n_left == 0 and (h := HUNK.match(row)):
            o, n = int(h[1]), int(h[3])
            o_left, n_left = int(h[2] or 1), int(h[4] or 1)
        elif o_left and row.startswith("-"):
            removed[o], o, o_left = row[1:], o + 1, o_left - 1
        elif n_left and row.startswith("+"):
            added[n], n, n_left = row[1:], n + 1, n_left - 1
    return removed, added


def main():
    if len(sys.argv) not in (2, 3):
        print(USAGE, file=sys.stderr)
        sys.exit(2)
    base, head = sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else "HEAD"
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    files = [f for f in git("diff", "--name-only", "-z", "--no-renames", base, head, "--", "*.rs")
             .split("\0") if f]
    old = [(f, fn) for f in files if (t := show(base, f)) is not None for fn in functions(t)]
    new = [(f, g) for f in files if (t := show(head, f)) is not None for g in functions(t)]
    pairs, claimed = {}, set()
    for kind in ("in place", "identical", "identical apart from call paths"):
        for i, (f, fn) in enumerate(old):
            if i in pairs:
                continue
            for j, (g_file, g) in enumerate(new):
                if j in claimed or g["name"] != fn["name"] or g["scope"] != fn["scope"] \
                        or (kind == "in place") != (g_file == f) \
                        or (kind != "in place" and fn["nested"]):
                    continue
                if same(fn, g, kind == "identical apart from call paths"):
                    pairs[i] = (j, kind)
                    claimed.add(j)
                    break

    diffs = {f: diff_lines(base, head, f) for f in files}
    for i, (j, kind) in sorted(pairs.items()):
        (f, fn), (g_file, g) = old[i], new[j]
        if kind == "in place" and not any(n in diffs[f][0] for n in fn["lines"]):
            continue
        cfg = f" [{fn['cfg']}]" if fn["cfg"] else ""
        how = "identical (within the file)" if kind == "in place" else kind
        print(f"moved: {f}:{fn['line']} {fn['name']}{cfg} -> {g_file}:{g['line']}  {how}")
    failures = [old[i] for i in range(len(old)) if i not in pairs]
    for f, fn in failures:
        print(f"NOT A MOVE: {f}:{fn['line']} {fn['name']} changed or went, with no identical copy")

    moved_old = {(old[i][0], n) for i in pairs for n in old[i][1]["lines"]}
    moved_new = {(new[j][0], n) for j, _ in pairs.values() for n in new[j][1]["lines"]}
    rest = []
    for f in files:
        removed, added = diffs[f]
        rest += [f"  {f}:{o} -{t}" for o, t in removed.items() if (f, o) not in moved_old]
        rest += [f"  {f}:{n} +{t}" for n, t in added.items() if (f, n) not in moved_new]
    print(f"\n{len(rest)} changed lines are not part of a move" + (":" if rest else "."))
    if rest:
        print("\n".join(rest))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
