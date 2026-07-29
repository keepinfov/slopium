#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
check_dir="$(mktemp -d)"
trap 'rm -rf "$check_dir"' EXIT

cat >"$check_dir/runtime.slp" <<'SLOPIUM'
(struct Pair ((left String) (right String)))
(enum Message Empty (Text ((value String))))

(fn main () -> i32
  (let line (read-line))
  (let number (parse-i64 (& line)))
  (let pair (Pair :left "left" :right "right"))
  (let pair-copy (clone pair))
  (let message (Message:Text "payload"))
  (let mut values (list number 2 3))
  (do (push (&mut values) 4))
  (println (get (& values) 0))
  (let mut owned (list "one" "two"))
  (do (push (&mut owned) "three"))
  (let owned-first (get-ref (& owned) 0))
  (println owned-first)
  (let removed (remove (&mut owned) 1))
  (println (& removed))
  (let fixed (array "zero" "one" "two"))
  (let view (slice (& fixed) 1 3))
  (let viewed (get-ref (& view) 0))
  (println viewed)
  (println (args-len))
  (let first (arg 0))
  (println (& first))
  (match message
    ((Message:Empty) 1)
    ((Message:Text value) (do (println (& value)) 0))))
SLOPIUM

cargo run --quiet --manifest-path "$workspace_dir/Cargo.toml" -p slopic -- \
  "$check_dir/runtime.slp" --emit asm --output "$check_dir/runtime.s"

cc -g -fsanitize=address -fno-omit-frame-pointer \
  -o "$check_dir/runtime-asan" \
  "$check_dir/runtime.s" "$workspace_dir/runtime/slop_rt.c"
printf '42\n' | ASAN_OPTIONS=detect_leaks="${SLOPIUM_ASAN_DETECT_LEAKS:-0}":halt_on_error=1 \
  "$check_dir/runtime-asan" argument >/dev/null

if command -v valgrind >/dev/null 2>&1; then
  cc -g -o "$check_dir/runtime-valgrind" \
    "$check_dir/runtime.s" "$workspace_dir/runtime/slop_rt.c"
  printf '42\n' | valgrind \
    --quiet --leak-check=full --show-leak-kinds=all --error-exitcode=99 \
    "$check_dir/runtime-valgrind" argument >/dev/null
else
  echo "runtime-check: valgrind not found; skipped" >&2
fi
