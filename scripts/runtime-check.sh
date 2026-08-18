#!/usr/bin/env bash
set -euo pipefail

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing, which is worse
# than a red one.
skip() {
  echo "runtime-check: $1" >&2
  if [ -n "${SLOPIUM_STRICT:-}" ]; then
    echo "runtime-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
    exit 1
  fi
}

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
(take std:float from-f64 println-f64 to-f64)
(take std:process arg args-len)
(take std:fs delete exists read write Error)
(take std:prelude Option Result)
(take std:list (map :as list-map) (sort-by :as list-sort-by))
(take std:option (map :as option-map) unwrap-or)
(take std:string (hash :as string-hash) (equals :as string-equals))
(take std:map Map map-new map-insert map-lookup map-delete map-size map-fold)
(take std:set Set set-of set-add set-discard set-count)

(struct Pair ((left String) (right String)))
(enum Message Empty (Text ((value String))))
; A function value beside an owning field: the struct has real drop glue, and
; the word holding the address must not be dropped as though it owned memory.
(struct Labelled ((name String) (render (Fn ((& String)) String))))

(fn shout ((text (& String))) -> String
  (let mark "!")
  (concat text (& mark)))

; Assignment to a field, which takes the value going in and drops the one that
; was there (`D-120`). Called twice over one `String` field, so a write that
; forgets the old value leaks it and a write that drops it twice is a double
; free — and neither is a type error.
(fn rename ((pair (&mut Pair)) (name String)) -> unit
  (match pair
    ((Pair :left left :right _)
      (set left name))))

; Owned here and dropped here, so the struct releases both its `String` and the
; function value in its other field. The field is read through a borrow because
; a `Fn` is owned since `D-101` and reading it out would move it out of a
; struct that is about to drop it.
(fn run-label ((item Labelled) (text (& String))) -> String
  (match (& item)
    ((Labelled :render render) (render text))))

(fn decorate ((text String)) -> String
  (let mark "?")
  (concat (& text) (& mark)))

(fn shorter ((left (& String)) (right (& String))) -> bool
  (< (len left) (len right)))

; Matching through a shared borrow (`D-099`) must free nothing and drop nothing:
; the payload is a `(& String)` the enum still owns. Getting that wrong is a
; double free on the second call and a use-after-free in the caller, neither of
; which is a type error, so this is the only thing in the suite that would say
; so. The `Labelled` case adds a field that is *not* pointer-shaped beside one
; that is, which is where the two payload-address forms differ.
(fn text-of ((message (& Message))) -> String
  (match message
    ((Message:Empty) "empty")
    ((Message:Text value) (clone value))))

(fn width-of ((item (& Labelled))) -> i64
  (match item
    ((Labelled :name name :render _) (len name))))

; A closure that owns a `String` and outlives the call that built it (`D-101`).
; Its environment is a heap block with generated glue, so every way of getting
; that wrong is invisible to the type checker and visible here: releasing the
; block without its captures leaks the `String`, releasing it twice is a double
; free, and forgetting the block itself leaks 32 bytes per function value.
(fn greeter ((who String)) -> (Fn ((& String)) String)
  (lambda (who) ((mark (& String))) -> String
    (concat (& who) mark)))

; A subnormal, spelled out, because there is no exponent literal to spell it
; with (`D-098`). It is the value whose conversion allocates the most.
; A map of owned keys to owned values, which is the whole of `D-104`'s drop
; glue: every entry owns two allocations, a rehash moves them between lists,
; and an entry replaced by a second insert of the same key is freed there.
(fn empty-labels () -> (Map String String)
  (map-new string-hash string-equals))

(fn empty-words () -> (Set String)
  (set-of string-hash string-equals))

(fn label-table ((upto i64)) -> (Map String String)
  (let mut labels (empty-labels))
  (let mut index 0)
  (while (< index upto)
    (let digits (from-i64 index))
    (let prefix "k")
    (let key (concat (& prefix) (& digits)))
    (map-insert (&mut labels) key (shout (& digits)))
    (set index (+ index 1)))
  labels)

(fn widest ((carried i64) (key (& String)) (value (& String))) -> i64
  (if (> (len value) carried)
    (len value)
    carried))

(fn tiny-text () -> String
  (let zero "0")
  (let mut text "0.")
  (let mut index 0)
  (while (< index 322)
    (let next (concat (& text) (& zero)))
    (set text next)
    (set index (+ index 1)))
  (let one "1")
  (concat (& text) (& one)))

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
  ; A clone that crosses a borrow (`D-091`) allocates a second buffer, and the
  ; value the borrow came from still drops its own. Getting that wrong is a
  ; double free rather than a type error, which is why it is checked here.
  (let borrowed-copy (clone (& joined)))
  (println-i64 (probe-strlen (& borrowed-copy)))
  ; A call through a function value, and one stored in an aggregate that also
  ; owns a `String`. A wrong drop decision for either is a double free here
  ; rather than a type error anywhere.
  (let labelled (Labelled :name (clone (& rendered)) :render shout))
  ; Looked at through a borrow before it is consumed, twice, so a payload that
  ; was freed or dropped by the first look is a double free at the second.
  (println-i64 (width-of (& labelled)))
  (println-i64 (width-of (& labelled)))
  (let shouted (run-label labelled (& suffix)))
  (println (& shouted))
  ; `core:list` and `core:option` over elements that own memory. Every one of
  ; these moves a `String` out of a list, hands it to a function value, and
  ; puts a different one back; `sort-by` additionally reorders them and drops
  ; the list they came from. A wrong ownership decision anywhere in that is a
  ; leak or a double free, and nothing else in the suite would see it.
  (let decorated (list-map (list "alpha" "bee" "c") decorate))
  (let sorted (list-sort-by decorated shorter))
  (println (get-ref (& sorted) 0))
  (println-i64 (len (& sorted)))
  (let maybe-name (option-map (Option:Some (clone (& rendered))) decorate))
  (let name (unwrap-or maybe-name "none"))
  (println (& name))
  ; `core:float`. One conversion builds and discards several hundred
  ; intermediate digit lists and as many strings, and the subnormal path is
  ; where that count is highest — a formatter that kept one of them would leak
  ; here and nowhere else in the suite.
  (println-f64 (/ 1.0 3.0))
  (let tiny (tiny-text))
  (let subnormal (unwrap-or (to-f64 (& tiny)) 0.0))
  (println-f64 subnormal)
  (let written (from-f64 subnormal))
  (println-i64 (len (& written)))
  ; Called twice, so a capture consumed by the first call is a use-after-free
  ; at the second; cloned, so the copy's captures are a second allocation the
  ; copy owns; and one of them is handed to the library, which drops it there.
  (let hello (greeter "hello"))
  (let mark "!")
  (let loud (hello (& mark)))
  (println (& loud))
  (let louder (hello (& mark)))
  (println (& louder))
  (let echo (clone hello))
  (let echoed (echo (& mark)))
  (println (& echoed))
  (let by-length
    (lambda (mark) ((left (& String)) (right (& String))) -> bool
      (< (+ (len left) (len (& mark))) (+ (len right) (len (& mark))))))
  (let ordered (list-sort-by (list "ccc" "a" "bb") by-length))
  (println (get-ref (& ordered) 0))
  ; Twenty entries into a table that starts at four buckets: three rehashes,
  ; each moving every entry between lists. Then the map is cloned, which clones
  ; every key, every value and both function values, and both copies are
  ; dropped — releasing a bucket without its entries leaks, releasing an entry
  ; twice is a double free, and neither is a type error.
  (let labels (label-table 20))
  (println-i64 (map-size (& labels)))
  (println-i64 (map-fold (& labels) 0 widest))
  (let copied (clone labels))
  (let wanted (from-i64 7))
  (let held (unwrap-or (map-lookup (& copied) (& wanted)) "none"))
  (println (& held))
  (let mut shrunk (label-table 6))
  (let doomed (from-i64 3))
  (map-delete (&mut shrunk) (& doomed))
  (println-i64 (map-size (& shrunk)))
  (let mut words (empty-words))
  (set-add (&mut words) "one")
  (set-add (&mut words) "one")
  (let gone "one")
  (set-discard (&mut words) (& gone))
  (println-i64 (set-count (& words)))
  ; Field assignment over an owning field, twice, with what the first write put
  ; there dropped by the second.
  (let mut renamed (Pair :left "before" :right "kept"))
  (rename (&mut renamed) (shout (& gone)))
  (rename (&mut renamed) "after")
  (match (& renamed)
    ((Pair :left left :right right)
      (do
        (println left)
        (println right))))
  ; `replace` on its own: the value that comes out is owned by the caller and
  ; the one that goes in is owned by the list (`D-103`).
  (let mut slots (list "first" "second"))
  (let displaced (replace (&mut slots) 0 (shout (& gone))))
  (println (& displaced))
  (println (get-ref (& slots) 0))
  (let message (Message:Text "payload"))
  (let looked (text-of (& message)))
  (println (& looked))
  (let again (text-of (& message)))
  (println (& again))
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
echo "runtime-check: the library and its runtime under ASan ... ok"

if command -v valgrind >/dev/null 2>&1; then
  cc -g -o "$check_dir/runtime-valgrind" \
    "$check_dir/runtime.s" "$check_dir/probe.c" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c"
  printf '42\n' | valgrind \
    --quiet --leak-check=full --show-leak-kinds=all --error-exitcode=99 \
    "$check_dir/runtime-valgrind" argument >/dev/null
  echo "runtime-check: the same program under valgrind ... ok"
else
  skip "valgrind not found; skipped"
fi

echo "runtime-check: all runtime checks passed"
