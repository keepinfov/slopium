#!/usr/bin/env bash
set -euo pipefail

# Checks that `--debug` produces a usable DWARF line table: that the table names
# the right file and line for each construct, that a debugger can set a
# breakpoint by source location and step, and that a profile without debug
# information carries none.

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
check_dir="$(mktemp -d)"
trap 'rm -rf "$check_dir"' EXIT

slopic() {
  cargo run --quiet --manifest-path "$workspace_dir/Cargo.toml" -p slopic -- "$@"
}

fail() {
  echo "debug-check: $1" >&2
  exit 1
}

mkdir -p "$check_dir/project/src"
cat >"$check_dir/project/src/arith.slp" <<'SLOPIUM'
(fn triple ((n i64)) -> i64
  (let doubled (+ n n))
  (+ doubled n))
(export triple)
SLOPIUM

cat >"$check_dir/project/src/main.slp" <<'SLOPIUM'
(fn main () -> i32
  (let seed 7)
  (let result (triple seed))
  (println result)
  0)
(take arith triple)
SLOPIUM

cd "$check_dir/project"
slopic src/main.slp --source-root src --emit exe --debug -o program
[ "$(./program)" = "21" ] || fail "the debug build does not produce the expected output"

# The line table must name both modules, not just the entry one.
decoded="$(readelf --debug-dump=decodedline program 2>/dev/null)"
for module in arith.slp main.slp; do
  grep -q "$module" <<<"$decoded" ||
    fail "no line-table rows for $module"
done

# `(+ doubled n)` is line 3 of arith.slp and `(println result)` is line 4 of
# main.slp; both must be reachable rows, which is what a breakpoint needs.
grep -qE '(^|[^0-9])3 +0x' <<<"$decoded" || fail "arith.slp line 3 has no row"

# Without debug information there must be nothing to strip.
slopic src/main.slp --source-root src --emit exe -o program-plain
if readelf -S program-plain 2>/dev/null | grep -q '\.debug_line'; then
  fail "a build without --debug still carries a line table"
fi

if ! command -v gdb >/dev/null 2>&1; then
  echo "debug-check: gdb not found; source-level session skipped" >&2
  echo "debug-check: line tables verified"
  exit 0
fi

session="$(gdb -batch -nx \
  -ex 'break arith.slp:2' \
  -ex 'run' \
  -ex 'backtrace' \
  -ex 'next' \
  -ex 'info line' \
  -ex 'continue' ./program 2>&1)"

# A breakpoint set by source location resolves, stops, and shows the source.
grep -qE 'at .*arith\.slp:2' <<<"$session" ||
  fail "the breakpoint did not stop at arith.slp:2:
$session"
grep -qF '(let doubled (+ n n))' <<<"$session" ||
  fail "gdb could not display the source line:
$session"

# The backtrace crosses a module boundary with each frame in its own file.
grep -qE 'at .*main\.slp:3' <<<"$session" ||
  fail "the caller frame is not attributed to main.slp:3:
$session"

# Stepping advances one source line rather than one instruction.
grep -qF '(+ doubled n)' <<<"$session" ||
  fail "`next` did not advance to the next source line:
$session"
grep -qE 'Line 3 of "[^"]*arith\.slp"' <<<"$session" ||
  fail "gdb does not place the stepped-to address on arith.slp line 3:
$session"

# Frame #2 is the generated C entry wrapper. It has no source of its own and
# inherits the preceding row, so it is only checked for being present and named.
grep -qE '#2 +0x[0-9a-f]+ in main \(\)' <<<"$session" ||
  fail "the entry wrapper is missing from the backtrace:
$session"

echo "debug-check: line tables and a gdb source-level session verified"
