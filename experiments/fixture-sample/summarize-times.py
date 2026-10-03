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
        cells = [test]
        for runs in (before[test], after.get(test, [])):
            passed = sum(r == "pass" for r, _ in runs)
            mean = sum(s for _, s in runs) / max(len(runs), 1)
            cells += [f"{passed}/{len(runs)}", f"{mean:.1f}"]
        print("\t".join(cells))


if __name__ == "__main__":
    main()
