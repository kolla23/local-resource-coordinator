#!/usr/bin/env bash
#
# Compile the coordinator for native Windows with its Unix gates removed, and
# keep every compiler error (decision gate, step 3).
#
# `src/main.rs` stops a Windows build with `compile_error!` and puts
# `#[cfg(unix)]` on almost every module, so a normal build reports one line and
# says nothing about WHERE the code depends on Unix. This script copies the
# repository to a scratch folder, removes the stop and the module gates there
# (never in the repository), and runs `cargo check` for the host target with
# JSON messages. `summarize.py` turns the JSON into a list by file and function.
#
# rustc reports name-resolution errors first; some type errors behind them only
# appear once those are fixed. The count is therefore a lower bound on the
# breaks, and the list is the first layer of them.
#
# Usage, from the repo root, on Windows with the MSVC Rust toolchain:
#   experiments/unix-inventory/compile-windows.sh <scratch dir> <output .jsonl>

set -euo pipefail

scratch="${1:?give a scratch folder outside the repository}"
out="${2:?give the output file for the JSON messages}"
root="$(cd "$(dirname "$0")/../.." && pwd)"

case "$(cd "$(dirname "$scratch")" && pwd)/$(basename "$scratch")" in
    "$root"|"$root"/*) echo "the scratch folder must be outside the repository" >&2; exit 2 ;;
esac

rm -rf "$scratch"
mkdir -p "$scratch"
git -C "$root" archive HEAD | tar -x -C "$scratch"

python - "$scratch/src/main.rs" <<'EOF'
import re, sys
p = sys.argv[1]
s = open(p, encoding="utf-8").read()
# Remove the stop for non-Unix builds.
s, n = re.subn(r'#\[cfg\(not\(unix\)\)\]\ncompile_error!\((?:.|\n)*?\);\n', '', s, count=1)
assert n == 1, "compile_error! block not found"
# Remove the empty `main` for non-Unix builds: with the gates gone, it would be
# a second `main` and add an error that a port would never meet.
s, k = re.subn(r'#\[cfg\(not\(unix\)\)\]\nfn main\(\) \{\}\n', '', s, count=1)
assert k == 1, "the non-Unix main stub was not found"
# Remove the module gates (`#[cfg(unix)]` directly above `mod x;`).
s, m = re.subn(r'#\[cfg\(unix\)\]\n(mod \w+;)', r'\1', s)
# Remove the gates on main's own items too.
s = s.replace('#[cfg(unix)]\n', '')
open(p, "w", encoding="utf-8").write(s)
print(f"removed compile_error!, {m} module gates", file=sys.stderr)
EOF

rustc -Vv | sed -n '1p;/^host/p' >&2
(cd "$scratch" && cargo check --message-format=json --bins 2>/dev/null) > "$out" || true
echo "messages written to $out" >&2
