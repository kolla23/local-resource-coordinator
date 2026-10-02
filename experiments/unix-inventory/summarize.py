#!/usr/bin/env python3
"""Summarize the compiler errors from compile-windows.sh.

Reads cargo's JSON messages and prints the errors grouped by file, with the
error code and the item named in each message (the unresolved import, the
missing function, the unknown type), so a reader can see which Unix API each
file needs. Warnings are ignored.

Usage: python experiments/unix-inventory/summarize.py <messages .jsonl>
"""
import collections
import json
import re
import sys


def main():
    errors = []
    for line in open(sys.argv[1], encoding="utf-8"):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        msg = m.get("message")
        if m.get("reason") != "compiler-message" or not msg or msg.get("level") != "error":
            continue
        span = next((s for s in msg.get("spans", []) if s.get("is_primary")), None)
        if not span:
            continue
        code = (msg.get("code") or {}).get("code", "-")
        names = re.findall(r"`([^`]+)`", msg["message"])
        errors.append((span["file_name"].replace("\\", "/"), span["line_start"], code,
                       msg["message"].split("\n")[0], names))

    if not errors:
        print("no errors: the build passed, or the messages file is empty")
        return
    by_file = collections.defaultdict(list)
    for e in errors:
        by_file[e[0]].append(e)
    print(f"{len(errors)} errors in {len(by_file)} files")
    codes = collections.Counter(e[2] for e in errors)
    print("by code: " + ", ".join(f"{c} {n}" for c, n in codes.most_common()))
    names = collections.Counter(n for e in errors for n in e[4][:1])
    print("most named items: " + ", ".join(f"{n} {c}" for n, c in names.most_common(25)))
    print()
    for f, es in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
        print(f"{f}: {len(es)}")
        for _, line, code, text, _ in sorted(es, key=lambda e: e[1]):
            print(f"  {line:5} {code:6} {text}")


if __name__ == "__main__":
    main()
