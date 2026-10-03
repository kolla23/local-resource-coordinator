#!/usr/bin/env python3
"""Pass count and mean wall time per test, before and after (fixture sample).

Usage: summarize-times.py docs/decision-gate/logs/fixture-sample/before.tsv \
                          docs/decision-gate/logs/fixture-sample/after.tsv
"""
import sys


def read(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        if line.startswith("#") or not line.strip():
            continue
        _, test, _, result, secs = line.rstrip("\n").split("\t")
        runs = out.setdefault(test, [])
        runs.append((result, float(secs.split()[0])))
    return out


def main():
    before, after = read(sys.argv[1]), read(sys.argv[2])
    print("test\tbefore pass\tbefore mean s\tafter pass\tafter mean s")
    for test in before:
        b, a = before[test], after.get(test, [])
        print(f"{test}\t{sum(r == 'pass' for r, _ in b)}/{len(b)}\t{sum(s for _, s in b) / len(b):.1f}"
              f"\t{sum(r == 'pass' for r, _ in a)}/{len(a)}\t{sum(s for _, s in a) / max(len(a), 1):.1f}")


if __name__ == "__main__":
    main()
