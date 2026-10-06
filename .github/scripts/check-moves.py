#!/usr/bin/env python3
"""Checks that a refactor only moves Rust functions (docs/fork/PORT_PLAN.md, "Code only moves").
Tests: check-moves-test.py"""
import difflib
import os
import re
import subprocess
import sys
import traceback

USAGE = """usage: check-moves.py <base> [<head>]
(from anywhere in the repository; <head> defaults to HEAD)

Scope: only top-level functions can be verified as moved; anything else is
reported as NOT A MOVE for a human to explain.

Every function in a changed .rs file at <base> must pair with its own
byte-identical copy at <head>, each copy used once: first in the same file, then
in another file (a move). The item is the function, from its visibility and
qualifiers on whatever line they start, with the attributes and comments directly
above it. A function inside any { } block (mod, impl, trait, fn) may not move,
and counts as unchanged only if its whole top-level block, from the header to
the closing brace, is byte-identical in the same file at <head>.

Prints each move, then every line that is not part of a move, in every changed
file. A changed line is left out only when it belongs to a paired function on
its side (the <base> function for a removed line, the <head> function for an
added one). Every line of a <head> function that pairs with nothing is listed,
changed or not, and so are lines outside .rs files, file mode changes and
submodule changes. Changed lines come from comparing the stored blobs, not from
`git diff`. Exits 1 when a <base> function has no byte-identical copy, 2 on a
usage error, 3 when the checker itself fails. At 0 or 1, every listed line needs
an explanation.

Known blind spots (#48): an attribute separated from its function by a blank line
is not part of the item; a `{` inside generics in a signature is taken as the
body's opening brace, so only the signature line is compared (the body's
changed lines are still listed).

The authoritative result is the CI run, which uses a clean git config; local
runs are a convenience."""
FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
QUALIFIERS = re.compile(
    r"(?<!\w)(?:(?:pub(?:\s*\([^()]*\))?|const|async|unsafe|safe|extern|default)\s+)*\Z")


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True)
    if r.returncode != 0:
        print(f"check-moves: `git {' '.join(args)}` failed: "
              f"{r.stderr.decode('utf-8', 'replace').strip()}", file=sys.stderr)
        sys.exit(3)
    return r.stdout.decode("utf-8", "surrogateescape")


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
        elif (m := re.match(r'[bc]?r(#*)"', text[i:i + 260])) \
                and (i == 0 or not text[i - 1].isalnum()):
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

    item_end, brace, paren = [], 0, 0
    for row in mlines:
        for ch in row:
            brace += (ch == "{") - (ch == "}")
            paren += (ch in "([") - (ch in ")]")
        item_end.append(brace == 0 and paren == 0 and row.rstrip().endswith(("}", ";")))

    def block(b):
        """The whole top-level item around the brace at b, from the line after the previous
        item's end, so a header with `;` or `{ }` inside brackets can't be cut short."""
        k = line(b) - 2
        while k >= 0 and not item_end[k]:
            k -= 1
        k += 1
        while k < line(b) - 1 and not lines[k].strip():
            k += 1
        stop = text.find("\n", close.get(b, len(text)))
        return text[starts[k]:len(text) if stop < 0 else stop]

    open_at, close, stack = {}, {}, []
    fns = iter(FN.finditer(masked))
    upcoming = next(fns, None)
    for pos, ch in enumerate(masked):
        while upcoming and upcoming.start() == pos:
            open_at[pos] = list(stack)
            upcoming = next(fns, None)
        if ch == "{":
            stack.append(pos)
        elif ch == "}" and stack:
            close[stack.pop()] = pos
    for m in FN.finditer(masked):
        outer = block(open_at[m.start()][0]) if open_at[m.start()] else ""
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
            "outer": outer, "nested": bool(open_at[m.start()]),
            "item": text[starts[top]:len(text) if end < 0 else end],
            "lines": range(top + 1, line(k) + 1),
        })
    return found


def blob(sha, mode):
    return None if mode in ("000000", "160000") else git("cat-file", "blob", sha)


def diff_lines(a, b):
    """Computed here, not parsed from `git diff`, whose output git config and the environment
    can reshape."""
    x, y = rows(a), rows(b)
    removed, added = {}, {}
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, x, y, False).get_opcodes():
        if tag != "equal":
            removed.update((i + 1, shown(x[i])) for i in range(i1, i2))
            added.update((j + 1, shown(y[j])) for j in range(j1, j2))
    return removed, added


def rows(text):
    parts = [] if text is None else text.split("\n")
    return [p + "\n" for p in parts[:-1]] + [p for p in parts[-1:] if p]


def shown(row):
    return row[:-1] if row.endswith("\n") else row + "  (no newline at end of file)"


def clean(text):
    """Paths and lines come from the PR; any character outside printable ASCII (a newline, a
    line separator, a bidi override) could forge or reorder what a reviewer sees."""
    return "".join(c if " " <= c <= "~" else f"\\u{{{ord(c):04X}}}" for c in text)


def main():
    if len(sys.argv) not in (2, 3):
        print(USAGE, file=sys.stderr)
        sys.exit(2)
    base, head = sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else "HEAD"
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    sys.stdout.reconfigure(errors="backslashreplace")
    raw = git("diff", "--raw", "-z", "--no-renames", "--no-abbrev", "--ignore-submodules=none",
              base, head).split("\0")
    meta = {f: m.lstrip(":").split()[:4] for m, f in zip(raw[0::2], raw[1::2]) if f}
    texts = {f: (blob(sa, ma), blob(sb, mb)) for f, (ma, mb, sa, sb) in meta.items()}
    files, rs = list(meta), [f for f in meta if f.endswith(".rs")]
    old = [(f, fn) for f in rs if (t := texts[f][0]) is not None for fn in functions(t)]
    new = [(f, g) for f in rs if (t := texts[f][1]) is not None for g in functions(t)]
    pairs, claimed = {}, set()
    for kind in ("in place", "identical"):
        for i, (f, fn) in enumerate(old):
            if i in pairs:
                continue
            for j, (g_file, g) in enumerate(new):
                if j in claimed or g["name"] != fn["name"] or g["outer"] != fn["outer"] \
                        or (kind == "in place") != (g_file == f) \
                        or (kind != "in place" and fn["nested"]):
                    continue
                if fn["item"] == g["item"]:
                    pairs[i] = (j, kind)
                    claimed.add(j)
                    break

    diffs = {f: diff_lines(*texts[f]) for f in files}
    for i, (j, kind) in sorted(pairs.items()):
        (f, fn), (g_file, g) = old[i], new[j]
        if kind == "in place" and not any(n in diffs[f][0] for n in fn["lines"]):
            continue
        cfg = f" [{fn['cfg']}]" if fn["cfg"] else ""
        how = "identical (within the file)" if kind == "in place" else kind
        print(clean(f"moved: {f}:{fn['line']} {fn['name']}{cfg} -> {g_file}:{g['line']}  {how}"))
    failures = [old[i] for i in range(len(old)) if i not in pairs]
    for f, fn in failures:
        print(clean(f"NOT A MOVE: {f}:{fn['line']} {fn['name']} changed or went, "
                    "with no identical copy"))

    moved_old = {(old[i][0], n) for i in pairs for n in old[i][1]["lines"]}
    moved_new = {(new[j][0], n) for j, _ in pairs.values() for n in new[j][1]["lines"]}
    rest = []
    for f in files:
        removed, added = diffs[f]
        head_rows = rows(texts[f][1])
        for j, (g_file, g) in enumerate(new):
            if g_file == f and j not in claimed:
                added.update((n, shown(head_rows[n - 1])) for n in g["lines"])
        added = dict(sorted(added.items()))
        before, after, sa, sb = meta[f]
        if "000000" not in (before, after) and before != after:
            rest.append(f"  {f}: mode {before} -> {after}")
        if "160000" in (before, after) and sa != sb:
            rest.append(f"  {f}: submodule {sa} -> {sb}")
        rest += [f"  {f}:{o} -{t}" for o, t in removed.items() if (f, o) not in moved_old]
        rest += [f"  {f}:{n} +{t}" for n, t in added.items() if (f, n) not in moved_new]
    print(f"\n{len(rest)} lines are not part of a move" + (":" if rest else "."))
    if rest:
        print("\n".join(clean(r) for r in rest))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        traceback.print_exc()
        sys.exit(3)
