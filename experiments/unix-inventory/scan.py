#!/usr/bin/env python3
"""Count the Unix-only code in src/ (decision gate, step 3).

For each .rs file in src/ this prints the production lines (every line except
`#[cfg(test)]` items and the modules that main.rs declares only for tests), and,
for each category of Unix API, the number of production lines that use it and
the functions they are in. Test code is not
counted: a port has to rewrite the tests too, but the product is the measure.

The categories are text patterns, so a hit is a place to read, not a proof of
a dependency. The compile in compile-windows.sh is the other half: it shows
what actually breaks.

Usage: python experiments/unix-inventory/scan.py [--json] (from the repo root)
"""
import json
import re
import sys
from pathlib import Path

CATEGORIES = {
    # Process control: what qex calls "the process control of Unix".
    "process-group/session": r"\bsetsid\b|\bsetpgid\b|\bgetpgid\b|\bkillpg\b|\bprocess_group\b|\bpgid\b|\bpre_exec\b",
    "wait/reap": r"\bwaitpid\b|\bwait4\b|\bWNOHANG\b|\bWIFEXITED\b|\bWEXITSTATUS\b|\bWIFSIGNALED\b|\bWTERMSIG\b|\bExitStatusExt\b",
    "signals": r"\bSIG[A-Z]{2,}\b|\blibc::kill\b|\bsigaction\b|\bsignal\(|\bsigprocmask\b|\bsighandler",
    "fork/exec/fds": r"\bfork\(|\bexecv|\bCommandExt\b|\bdup2\b|\bpipe\(|\bO_CLOEXEC\b|\bFD_CLOEXEC\b|\bRawFd\b|\bAsRawFd\b|\bFromRawFd\b|\bOwnedFd\b|\bclose_range\b",
    "unix-socket IPC": r"\bUnixListener\b|\bUnixStream\b|\bUnixDatagram\b|\bSO_PEERCRED\b|\bpeer_cred\b|\bgetpeereid\b|\bsun_path\b",
    "users/permissions": r"\bgetuid\b|\bgeteuid\b|\bgetgid\b|\buid\(\)|\bPermissionsExt\b|\bfrom_mode\b|\bmode\(\)|\bchmod\b|\bumask\b|\bS_ISVTX\b|0o[0-7]{3,4}",
    "file identity/metadata": r"\bMetadataExt\b|\.ino\(\)|\.dev\(\)|\.nlink\(\)|\bO_NOFOLLOW\b|\bO_NONBLOCK\b|\bflock\b|\bfcntl\b|\bmkfifo\b|\bsymlink\b",
    "/proc, /sys, PSI, cgroups": r"\"/proc|/proc/|\"/sys/|/sys/fs|\bpressure\b.*\bproc\b|\bcgroup",
    "macOS kernel APIs": r"\bsysctl\b|\bmach_|\bproc_pidinfo\b|\bvm_stat\b|\bhost_statistics\b|\blibproc\b",
    "terminal/tty": r"\btermios\b|\btcgetattr\b|\btcsetattr\b|\bisatty\b|\bTIOC|\bioctl\b|\bwinsize\b",
    "resource usage/limits": r"\bgetrusage\b|\bRUSAGE_\b|\bsetrlimit\b|\bgetrlimit\b|\bRLIMIT_\b|\bsysconf\b|\b_SC_\b",
    "other libc": r"\blibc::",
    "std::os::unix": r"std::os::unix",
    "cfg(unix)/target_os": r"#\[cfg\((not\()?(unix|target_os|any\(target_os)",
}

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:unsafe\s+)?(?:extern\s+\"C\"\s+)?fn\s+(\w+)")


def production(lines):
    """Drop every `#[cfg(test)]` item: a `mod` block, a function, or a `use`.

    A bare cut at the first `#[cfg(test)]` is wrong: some files mark one helper
    or one `use` for tests near the top and keep production code below it.
    """
    out = []
    i = 0
    while i < len(lines):
        if lines[i].strip() != "#[cfg(test)]":
            out.append(lines[i])
            i += 1
            continue
        # Skip the attribute, any further attributes, and the item it marks.
        i += 1
        while i < len(lines) and lines[i].lstrip().startswith("#["):
            i += 1
        depth = 0
        opened = False
        while i < len(lines):
            depth += lines[i].count("{") - lines[i].count("}")
            opened = opened or "{" in lines[i]
            i += 1
            if (opened and depth <= 0) or (not opened and lines[i - 1].rstrip().endswith(";")):
                break
    return out


def scan(path):
    lines = production(path.read_text(encoding="utf-8").splitlines())
    current = "(module level)"
    out = {"production_lines": len(lines), "categories": {}}
    for line in lines:
        m = FN.match(line)
        if m:
            current = m.group(1)
        code = line.split("//", 1)[0] if "\"" not in line else line
        if line.lstrip().startswith("//"):
            continue
        for cat, pat in CATEGORIES.items():
            if re.search(pat, code):
                c = out["categories"].setdefault(cat, {"lines": 0, "functions": []})
                c["lines"] += 1
                if current not in c["functions"]:
                    c["functions"].append(current)
    return out


def test_only_modules(root):
    """Modules that main.rs declares under `#[cfg(test)]`: all of them is test code."""
    lines = (root / "main.rs").read_text(encoding="utf-8").splitlines()
    names = set()
    for i, line in enumerate(lines):
        m = re.match(r"\s*mod (\w+);", line)
        if not m:
            continue
        k = i - 1
        while k >= 0 and lines[k].lstrip().startswith("#["):
            if lines[k].strip() == "#[cfg(test)]":
                names.add(m.group(1) + ".rs")
            k -= 1
    return names


def main():
    root = Path("src")
    skip = test_only_modules(root)
    result = {p.name: scan(p) for p in sorted(root.glob("*.rs")) if p.name not in skip}
    if "--json" in sys.argv:
        json.dump(result, sys.stdout, indent=1)
        return
    total = sum(r["production_lines"] for r in result.values())
    touched = {k: r for k, r in result.items()
               if any(c != "cfg(unix)/target_os" for c in r["categories"])}
    print(f"production lines in src/: {total}")
    print(f"files with a Unix API hit: {len(touched)} of {len(result)} "
          f"({sum(r['production_lines'] for r in touched.values())} production lines)")
    print()
    print(f"{'file':16} {'prod':>6}  hits by category (lines/functions)")
    for name, r in sorted(result.items(), key=lambda kv: -kv[1]["production_lines"]):
        cats = ", ".join(f"{c} {v['lines']}/{len(v['functions'])}"
                         for c, v in r["categories"].items())
        print(f"{name:16} {r['production_lines']:6}  {cats or '-'}")
    print()
    print("totals by category (production lines, files):")
    for cat in CATEGORIES:
        lines = sum(r["categories"].get(cat, {}).get("lines", 0) for r in result.values())
        files = sum(1 for r in result.values() if cat in r["categories"])
        print(f"  {cat:28} {lines:5} lines in {files:2} files")


if __name__ == "__main__":
    main()
