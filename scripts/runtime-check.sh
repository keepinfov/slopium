#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
check_dir="$(mktemp -d)"
trap 'rm -rf "$check_dir"' EXIT

# The C an `extern` reaches. Under ASan this is where a borrow handed across
# the boundary is checked for real: the pointer must be live, and a `String`'s
# must be NUL-terminated, or `strlen` walks off the allocation.
cat >"$check_dir/probe.c" <<'PROBE'
#include <stdint.h>
#include <string.h>

typedef struct { uint64_t len; uint64_t cap; char *ptr; } SlString;
SlString *sl_rt_string_new(const char *bytes, uint64_t len);

int64_t probe_strlen(const char *text) { return (int64_t)strlen(text); }

int64_t probe_slice(const int64_t *values, int64_t len) {
    int64_t total = 0;
    for (int64_t index = 0; index < len; index++) {
        total += values[index];
    }
    return total;
}

SlString *probe_string(void) { return sl_rt_string_new("from C", 6); }
PROBE

cat >"$check_dir/runtime.slp" <<'SLOPIUM'
(take std:io println println-i64 read-i64)
(take std:string concat from-i64 split substring to-i64 trim)
(take std:process arg args-len)
(take std:fs delete exists read write Error)
(take std:prelude Option Result)

(struct Pair ((left String) (right String)))
(enum Message Empty (Text ((value String))))

(extern "probe_strlen" (probe-strlen (text (& String))) -> i64)
(extern "probe_slice" (probe-slice (values (& (Slice i64)))) -> i64)
(extern "probe_string" (probe-string) -> String)

(fn main () -> i32
  (let number
    (match (read-i64)
      ((Option:Some value) value)
      ((Option:None) 0)))
  ; The string and file halves of the library allocate, so they belong under a
  ; leak checker too.
  (let rendered (from-i64 number))
  (let suffix ",7,x")
  (let joined (concat (& rendered) (& suffix)))
  (let parts (split (& joined) 44))
  (println-i64 (len (& parts)))
  (let piece (get-ref (& parts) 1))
  (let trimmed (trim piece))
  (println-i64
    (match (to-i64 (& trimmed))
      ((Option:Some value) value)
      ((Option:None) 0)))
  (let path "/tmp/slopium-runtime-check.txt")
  (match (write (& path) (& joined))
    ((Result:Ok written) (println-i64 written))
    ((Result:Err (Error :code code)) (println-i64 code)))
  (match (read (& path))
    ((Result:Ok text) (println (& text)))
    ((Result:Err (Error :code code)) (println-i64 code)))
  (match (delete (& path))
    ((Result:Ok status) (println-i64 status))
    ((Result:Err (Error :code code)) (println-i64 code)))
  (match (read (& path))
    ((Result:Ok text) (println (& text)))
    ((Result:Err (Error :code code)) (println-i64 code)))
  (println-i64 (if (exists (& path)) 1 0))
  (let pair (Pair :left "left" :right "right"))
  (let pair-copy (clone pair))
  (let message (Message:Text "payload"))
  (let mut values (list number 2 3))
  (do (push (&mut values) 4))
  (println-i64 (get (& values) 0))
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
  (println-i64 (args-len))
  (let first
    (match (arg 0)
      ((Option:Some value) value)
      ((Option:None) (substring (& suffix) 0 0))))
  (println (& first))
  (println-i64 (probe-strlen (& first)))
  (let numbers (array 10 20 30 40))
  (let number-view (slice (& numbers) 1 4))
  (println-i64 (probe-slice (& number-view)))
  (let greeting (probe-string))
  (println (& greeting))
  (match message
    ((Message:Empty) 1)
    ((Message:Text value) (do (println (& value)) 0))))
SLOPIUM

cargo run --quiet --manifest-path "$workspace_dir/Cargo.toml" -p slopic -- \
  "$check_dir/runtime.slp" --emit asm --output "$check_dir/runtime.s"

cc -g -fsanitize=address -fno-omit-frame-pointer \
  -o "$check_dir/runtime-asan" \
  "$check_dir/runtime.s" "$check_dir/probe.c" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c"
printf '42\n' | ASAN_OPTIONS=detect_leaks="${SLOPIUM_ASAN_DETECT_LEAKS:-0}":halt_on_error=1 \
  "$check_dir/runtime-asan" argument >/dev/null

if command -v valgrind >/dev/null 2>&1; then
  cc -g -o "$check_dir/runtime-valgrind" \
    "$check_dir/runtime.s" "$check_dir/probe.c" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c"
  printf '42\n' | valgrind \
    --quiet --leak-check=full --show-leak-kinds=all --error-exitcode=99 \
    "$check_dir/runtime-valgrind" argument >/dev/null
else
  echo "runtime-check: valgrind not found; skipped" >&2
fi
