#!/usr/bin/env bash
# Cross-backend differential and ABI conformance.
#
# Two backends are only worth having if they agree. Every program in the corpus
# is compiled twice — once natively, once for aarch64 — and both are run, the
# second under qemu; stdout and exit status must match exactly. Nothing here
# inspects the generated code, because agreement on behaviour is the property
# that matters and it is the one an inspection cannot establish.
#
# The ABI half is separate and checks something the differential cannot: that
# Slopium's idea of the calling convention matches the platform's. It links
# Slopium-generated functions against a C caller compiled by the real toolchain,
# so the two only agree if both got AAPCS64 right.
#
# Skipped with a message when the cross toolchain or qemu is absent, which is
# how it behaves outside `nix develop`.
set -euo pipefail

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
  echo "cross-check: $1" >&2
  if [ -n "${SLOPIUM_STRICT:-}" ]; then
    echo "cross-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
    exit 1
  fi
}

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
projects_dir="$workspace_dir/tests/projects"
result_dir="$(mktemp -d)"
trap 'rm -rf "$result_dir"' EXIT

cross_target="aarch64-unknown-linux-gnu"
cross_cc="${SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU:-aarch64-unknown-linux-gnu-cc}"
qemu="${SLOPIUM_QEMU_AARCH64:-qemu-aarch64}"

if ! command -v "$cross_cc" >/dev/null 2>&1; then
  echo "cross-check: run inside 'nix develop' for the aarch64 toolchain" >&2
  skip "$cross_cc not found; cross-backend checks skipped"
  exit 0
fi
if ! command -v "$qemu" >/dev/null 2>&1; then
  skip "$qemu not found; cross-backend checks skipped"
  exit 0
fi

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"
compiler="$workspace_dir/target/debug/slopic"

fail() {
  echo "cross-check: $*" >&2
  exit 1
}

# Runs a program and writes "<stdout>" plus a trailing status line, so a
# difference in either is one comparison.
capture() {
  local output="$1"
  shift
  set +e
  "$@" >"$output" 2>"$output.stderr"
  local status=$?
  set -e
  echo "exit status: $status" >>"$output"
}

# ---------------------------------------------------------------------------
# Differential: the same source, both backends, both profiles.
# ---------------------------------------------------------------------------

differential_count=0

compare_program() {
  local label="$1"
  local prefix="$2"
  local profile="$3"
  shift 3
  local -a compile=("$@")

  # `slopic` knows no profiles; the manager's release policy is just
  # optimization here, and neither backend's output depends on stripping.
  local -a opt=()
  [ "$profile" = release ] && opt=(--optimize)

  "${compile[@]}" "${opt[@]}" --output "$prefix.native" \
    >"$prefix.native.build" 2>&1 ||
    fail "$label ($profile) failed to build for the host"
  "${compile[@]}" "${opt[@]}" --target "$cross_target" --cc "$cross_cc" \
    --output "$prefix.cross" >"$prefix.cross.build" 2>&1 ||
    fail "$label ($profile) failed to build for $cross_target"

  capture "$prefix.native.out" "$prefix.native"
  capture "$prefix.cross.out" "$qemu" "$prefix.cross"

  if ! cmp --silent "$prefix.native.out" "$prefix.cross.out"; then
    echo "cross-check: backends disagree on $label ($profile)" >&2
    diff -u "$prefix.native.out" "$prefix.cross.out" >&2 || true
    exit 1
  fi
  differential_count=$((differential_count + 1))
}

for profile in dev release; do
  for source in "$workspace_dir"/examples/*.slp; do
    name="$(basename "$source" .slp)"
    # Compiles only if the file is a complete program; the corpus holds one
    # example that is kept for its syntax rather than for building.
    if ! "$compiler" "$source" --emit check >/dev/null 2>&1; then
      continue
    fi
    compare_program "example $name" "$result_dir/example-$name-$profile" "$profile" \
      "$compiler" "$source" --emit exe
  done

  for project in "$projects_dir"/pass/*/; do
    name="$(basename "$project")"
    entry="$project/src/main.slp"
    [[ -f "$entry" ]] || continue
    # Fixtures whose behaviour depends on arguments, stdin, or the environment
    # are covered by the project suite; here they would compare two runs of
    # different inputs.
    [[ -f "$project/run.args" || -f "$project/run.stdin" || -f "$project/run.env" ]] &&
      continue
    # A fixture with `c-sources` needs objects `slopic` alone does not link;
    # the FFI section below builds that shape itself, on both targets.
    grep -q '^c-sources' "$project/Slopium.toml" && continue
    if ! "$compiler" "$entry" --source-root "$project/src" --toolchain-dependency std \
      --emit check >/dev/null 2>&1; then
      continue
    fi
    compare_program "fixture $name" "$result_dir/pass-$name-$profile" "$profile" \
      "$compiler" "$entry" --source-root "$project/src" --toolchain-dependency std --emit exe
  done
done

echo "cross-check: $differential_count differential comparisons ... ok"

# ---------------------------------------------------------------------------
# Trapping arithmetic: a panic must be a panic on both backends.
#
# The differential corpus above is all programs that succeed, so nothing in it
# exercises the overflow and division checks the second backend had to build
# out of `b.vs` and an explicit comparison rather than out of a hardware fault.
# ---------------------------------------------------------------------------

trap_count=0
trap_dir="$result_dir/traps"
mkdir -p "$trap_dir"

cat >"$trap_dir/overflow.slp" <<'EOF'
(take std:io println-i64)
(fn main () -> i32
  (let big 9223372036854775807)
  (let step (+ big 1))
  (println-i64 step)
  0)
EOF
cat >"$trap_dir/narrow-overflow.slp" <<'EOF'
(fn wide () -> i32 2000000000)
(fn grow ((n i32)) -> i32 (* n 3))
(fn main () -> i32 (grow (wide)))
EOF
cat >"$trap_dir/div-zero.slp" <<'EOF'
(take std:io println-i64)
(fn divide ((a i64) (b i64)) -> i64 (/ a b))
(fn main () -> i32
  (println-i64 (divide 7 0))
  0)
EOF
cat >"$trap_dir/div-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn divide ((a i64) (b i64)) -> i64 (/ a b))
(fn main () -> i32
  (println-i64 (divide -9223372036854775808 -1))
  0)
EOF
# `%` reaches both checks `/` does, and on AArch64 it reaches them through a
# different instruction pair — `sdiv` and `msub` rather than `sdiv` alone.
cat >"$trap_dir/rem-zero.slp" <<'EOF'
(take std:io println-i64)
(fn rest ((a i64) (b i64)) -> i64 (% a b))
(fn main () -> i32
  (println-i64 (rest 7 0))
  0)
EOF
cat >"$trap_dir/rem-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn rest ((a i64) (b i64)) -> i64 (% a b))
(fn main () -> i32
  (println-i64 (rest -9223372036854775808 -1))
  0)
EOF
# The shift checks, and the reason they exist: x86-64 masks a count to six bits
# in hardware and AArch64 reduces it modulo the width, so an unchecked shift by
# 64 faults on neither machine and answers differently on each. The count comes
# through a function so the constant folder cannot decide it early.
cat >"$trap_dir/shift-wide.slp" <<'EOF'
(take std:io println-i64)
(fn amount () -> i64 64)
(fn shift ((value i64) (count i64)) -> i64 (shl value count))
(fn main () -> i32
  (println-i64 (shift 1 (amount)))
  0)
EOF
cat >"$trap_dir/shift-narrow.slp" <<'EOF'
(fn amount () -> i32 32)
(fn shift ((value i32) (count i32)) -> i32 (shr value count))
(fn main () -> i32 (shift 1 (amount)))
EOF
cat >"$trap_dir/shift-negative.slp" <<'EOF'
(take std:io println-i64)
(fn amount () -> i64 -1)
(fn shift ((value i64) (count i64)) -> i64 (shl value count))
(fn main () -> i32
  (println-i64 (shift 1 (amount)))
  0)
EOF
# `(- x)` is `0 - x`, so the smallest integer has no negation and says so.
cat >"$trap_dir/negate-min.slp" <<'EOF'
(take std:io println-i64)
(fn negate ((value i64)) -> i64 (- value))
(fn main () -> i32
  (println-i64 (negate -9223372036854775808))
  0)
EOF
# The eight-type axis (`D-107`). A narrow type overflows at its own width and
# not the word's, and it is checked by canonicalising the 64-bit result and
# comparing — a shape neither backend had before, and one that a backend
# reaching for the wide bound would silently pass rather than fail.
cat >"$trap_dir/u8-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn near () -> u8 200)
(fn add ((a u8) (b u8)) -> u8 (+ a b))
(fn main () -> i32
  (println-i64 (as i64 (add (near) 56)))
  0)
EOF
cat >"$trap_dir/u8-underflow.slp" <<'EOF'
(take std:io println-i64)
(fn zero () -> u8 0)
(fn take-one ((a u8) (b u8)) -> u8 (- a b))
(fn main () -> i32
  (println-i64 (as i64 (take-one (zero) 1)))
  0)
EOF
cat >"$trap_dir/i8-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn near () -> i8 127)
(fn add ((a i8) (b i8)) -> i8 (+ a b))
(fn main () -> i32
  (println-i64 (as i64 (add (near) 1)))
  0)
EOF
# A `u32` square reaches 2^64, so this is the one narrow product whose 64-bit
# result has already wrapped when the range check would look at it.
cat >"$trap_dir/u32-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn root () -> u32 0x1_0000)
(fn square ((a u32) (b u32)) -> u32 (* a b))
(fn main () -> i32
  (println-i64 (as i64 (square (root) (root))))
  0)
EOF
# `u64` is the one width with no room above it, so its overflow is the carry
# flag rather than a range check.
cat >"$trap_dir/u64-overflow.slp" <<'EOF'
(take std:io println-i64)
(fn big () -> u64 0xFFFF_FFFF_FFFF_FFFF)
(fn add ((a u64) (b u64)) -> u64 (+ a b))
(fn main () -> i32
  (println-i64 (as i64 (add (big) 1)))
  0)
EOF
# Unsigned division reaches `div` on x86-64 and `udiv` on AArch64, neither of
# which the compiler emitted before this patch.
cat >"$trap_dir/u64-div-zero.slp" <<'EOF'
(take std:io println-i64)
(fn zero () -> u64 0)
(fn divide ((a u64) (b u64)) -> u64 (/ a b))
(fn main () -> i32
  (println-i64 (as i64 (divide 7 (zero))))
  0)
EOF
# The shift bound is the type's width, so eight is out of range for a `u8` the
# way 64 is for an `i64`.
cat >"$trap_dir/shift-byte.slp" <<'EOF'
(take std:io println-i64)
(fn amount () -> u8 8)
(fn shift ((value u8) (count u8)) -> u8 (shl value count))
(fn main () -> i32
  (println-i64 (as i64 (shift 1 (amount))))
  0)
EOF

for profile in dev release; do
  for source in "$trap_dir"/*.slp; do
    name="$(basename "$source" .slp)"
    compare_program "trap $name" "$trap_dir/$name-$profile" "$profile" \
      "$compiler" "$source" --emit exe
    # A trap that stopped trapping would compare equal but succeed, so the
    # status is checked as well as the agreement.
    if ! grep -q 'exit status: 101' "$trap_dir/$name-$profile.native.out"; then
      fail "trap $name ($profile) did not panic"
    fi
    trap_count=$((trap_count + 1))
  done
done

echo "cross-check: $trap_count trapping-arithmetic comparisons ... ok"

# ---------------------------------------------------------------------------
# Float comparison: both backends, and the constant folder, must say the same
# thing about a NaN.
#
# This is the one place the two backends were found to disagree, and it is also
# where agreement alone would not be enough: two backends can agree and both be
# wrong, so the expected answers are written down rather than compared. IEEE 754
# says a NaN is neither less than, greater than, nor equal to anything, itself
# included, which is what the constant folder already computed and what the
# x86-64 backend did not emit.
# ---------------------------------------------------------------------------

float_dir="$result_dir/floats"
mkdir -p "$float_dir"

cat >"$float_dir/nan.slp" <<'EOF'
(take std:io println-i64)
(fn zero () -> f64 0.0)
(fn one () -> f64 1.0)
(fn flag ((c bool)) -> i64 (if c 1 0))
(fn main () -> i32
  (let nan (/ (zero) (zero)))
  (let unit (one))
  (println-i64 (flag (< nan unit)))
  (println-i64 (flag (> nan unit)))
  (println-i64 (flag (= nan nan)))
  (println-i64 (flag (< (zero) unit)))
  (println-i64 (flag (> unit (zero))))
  (println-i64 (flag (= unit unit)))
  0)
EOF

cat >"$float_dir/expected.stdout" <<'EOF'
0
0
0
1
1
1
exit status: 0
EOF

float_count=0
for profile in dev release; do
  compare_program "float comparison" "$float_dir/nan-$profile" "$profile" \
    "$compiler" "$float_dir/nan.slp" --emit exe
  if ! cmp --silent "$float_dir/expected.stdout" "$float_dir/nan-$profile.native.out"; then
    echo "cross-check: float comparison answers changed ($profile)" >&2
    diff -u "$float_dir/expected.stdout" "$float_dir/nan-$profile.native.out" >&2 || true
    exit 1
  fi
  float_count=$((float_count + 1))
done

echo "cross-check: $float_count float-comparison comparisons ... ok"

# ---------------------------------------------------------------------------
# The integer axis: unsigned arithmetic above 2^63, where a backend that
# reached for the signed instruction gives a plausible wrong answer rather than
# a crash (`D-107`).
#
# Agreement between two backends is not enough here for the same reason it was
# not enough for a NaN: both could reach for `idiv` and both be wrong. So the
# answers are written down. Every value goes through a function, so the
# constant folder cannot decide them early — and the folder is checked against
# the same expectations by the release profile, which does fold them.
# ---------------------------------------------------------------------------

unsigned_dir="$result_dir/unsigned"
mkdir -p "$unsigned_dir"

cat >"$unsigned_dir/wide.slp" <<'EOF'
(take std:io println-i64 println-u64)
(fn big () -> u64 0xFFFF_FFFF_FFFF_FFFF)
(fn half () -> u64 0x8000_0000_0000_0000)
(fn three () -> u64 3)
(fn flag ((c bool)) -> i64 (if c 1 0))
(fn main () -> i32
  ; A signed divide would answer 0 here, and a signed compare would call the
  ; big one negative.
  (println-u64 (/ (big) (three)))
  (println-u64 (% (big) 10))
  (println-i64 (flag (> (big) (three))))
  (println-i64 (flag (< (half) (big))))
  (println-i64 (flag (>= (half) (three))))
  ; A logical shift, where an arithmetic one would carry the sign down.
  (println-u64 (shr (big) 60))
  (println-u64 (shr (half) 63))
  ; And the widths below it, which travel through `as` and nothing else.
  (println-i64 (as i64 (as i8 0xFF)))
  (println-i64 (as i64 (as u8 0xFF)))
  (println-i64 (as i64 (bit-not (as u16 0))))
  (println-u64 (as u64 (as i8 -1)))
  0)
EOF

cat >"$unsigned_dir/expected.stdout" <<'EOF'
6148914691236517205
5
1
1
1
15
1
-1
255
65535
18446744073709551615
exit status: 0
EOF

unsigned_count=0
for profile in dev release; do
  compare_program "unsigned arithmetic" "$unsigned_dir/wide-$profile" "$profile" \
    "$compiler" "$unsigned_dir/wide.slp" --emit exe
  if ! cmp --silent "$unsigned_dir/expected.stdout" "$unsigned_dir/wide-$profile.native.out"; then
    echo "cross-check: unsigned answers changed ($profile)" >&2
    diff -u "$unsigned_dir/expected.stdout" "$unsigned_dir/wide-$profile.native.out" >&2 || true
    exit 1
  fi
  unsigned_count=$((unsigned_count + 1))
done

echo "cross-check: $unsigned_count unsigned-arithmetic comparisons ... ok"

# ---------------------------------------------------------------------------
# ABI conformance, outward: Slopium calls C through `extern`.
#
# The section after this one proves the toolchain can call us. This one proves
# we can call it, which is a different set of code — the argument classification
# and the stack spill in each backend's `ordinary_call`, not the prologue. Same
# reason for ten of each kind: two of each land on the stack.
#
# It also pins the two runtime layout facts `extern_arguments` encodes and
# nothing else tests — a `String`'s pointer and a `Slice`'s pointer and length
# are read at fixed byte offsets, and C here reads them as ordinary arguments.
# ---------------------------------------------------------------------------

ffi_dir="$result_dir/ffi"
mkdir -p "$ffi_dir"

cat >"$ffi_dir/callee.c" <<'EOF'
/* Unmangled names, because that is the whole point: an `extern` asks the
   linker for the name C gave the function, not for `sl_fn_<hex>`. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct { uint64_t len; uint64_t cap; char *ptr; } SlString;
SlString *sl_rt_string_new(const char *bytes, uint64_t len);

/* The library has no `println-i32`: an `i32` cannot reach `from-i64`, because
   the language has no widening conversion (`D-086`). The probe prints its own
   narrow return, which is the value under test anyway. */
void probe_println_i32(int32_t value) { printf("%d\n", value); }

int64_t probe_ten(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e,
                  int64_t f, int64_t g, int64_t h, int64_t i, int64_t j) {
    return a + b + c + d + e + f + g + h + i + j;
}

double probe_ten_doubles(double a, double b, double c, double d, double e,
                         double f, double g, double h, double i, double j) {
    return a + b + c + d + e + f + g + h + i + j;
}

/* Ten of each, interleaved: the two sequences fill independently, so reading
   them as one is invisible to every other case here. */
int64_t probe_mixed(int64_t a1, double d1, int64_t a2, double d2,
                    int64_t a3, double d3, int64_t a4, double d4,
                    int64_t a5, double d5, int64_t a6, double d6,
                    int64_t a7, double d7, int64_t a8, double d8,
                    int64_t a9, double d9, int64_t a10, double d10) {
    double floats = d1 + d2 + d3 + d4 + d5 + d6 + d7 + d8 + d9 + d10;
    int64_t integers = a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10;
    return integers * 100 + (int64_t)floats;
}

/* A narrow return leaves the upper half of the result register undefined. */
int32_t probe_narrow(int32_t value) { return -value; }

int64_t probe_strlen(const char *text) { return (int64_t)strlen(text); }

int64_t probe_slice(const int64_t *values, int64_t len) {
    int64_t total = 0;
    for (int64_t index = 0; index < len; index++) {
        total += values[index] * (index + 1);
    }
    return total;
}

SlString *probe_string(void) { return sl_rt_string_new("from C", 6); }
EOF

cat >"$ffi_dir/ffi.slp" <<'EOF'
(take std:io println println-bool println-i64)

(extern "probe_println_i32" (println-i32 (value i32)) -> unit)

(extern "probe_ten" (probe-ten (a i64) (b i64) (c i64) (d i64) (e i64) (f i64) (g i64) (h i64) (i i64) (j i64)) -> i64)

(extern "probe_ten_doubles" (probe-ten-doubles (a f64) (b f64) (c f64) (d f64) (e f64) (f f64) (g f64) (h f64) (i f64) (j f64)) -> f64)

(extern "probe_mixed" (probe-mixed (a1 i64) (d1 f64) (a2 i64) (d2 f64) (a3 i64) (d3 f64) (a4 i64) (d4 f64) (a5 i64) (d5 f64) (a6 i64) (d6 f64) (a7 i64) (d7 f64) (a8 i64) (d8 f64) (a9 i64) (d9 f64) (a10 i64) (d10 f64)) -> i64)

(extern "probe_narrow" (probe-narrow (value i32)) -> i32)

(extern "probe_strlen" (probe-strlen (text (& String))) -> i64)

(extern "probe_slice" (probe-slice (values (& (Slice i64)))) -> i64)

(extern "probe_string" (probe-string) -> String)

(fn main () -> i32
  (println-i64 (probe-ten 1 2 3 4 5 6 7 8 9 10))
  (let total (probe-ten-doubles 1.5 2.5 3.5 4.5 5.5 6.5 7.5 8.5 9.5 10.5))
  (println-bool (= total 60.0))
  (println-i64 (probe-mixed 1 1.5 2 2.5 3 3.5 4 4.5 5 5.5 6 6.5 7 7.5 8 8.5 9 9.5 10 10.5))
  (println-i32 (probe-narrow 2000000000))
  (let text "borrowed")
  (println-i64 (probe-strlen (& text)))
  (let values (array 10 20 30 40))
  (let view (slice (& values) 1 4))
  (println-i64 (probe-slice (& view)))
  (let greeting (probe-string))
  (println (& greeting))
  0)
EOF

cat >"$ffi_dir/expected.stdout" <<'EOF'
55
1
5560
-2000000000
8
200
from C
exit status: 0
EOF

ffi_count=0
for target in host "$cross_target"; do
  for profile in dev release; do
    opt=()
    [ "$profile" = release ] && opt=(--optimize)
    prefix="$ffi_dir/$target-$profile"
    if [[ "$target" == host ]]; then
      "$compiler" "$ffi_dir/ffi.slp" --emit obj "${opt[@]}" --output "$prefix.o" >/dev/null
      cc "$prefix.o" "$ffi_dir/callee.c" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c" \
        -o "$prefix" >"$prefix.link" 2>&1 ||
        fail "the host FFI program did not link ($profile)"
      capture "$prefix.out" "$prefix"
    else
      "$compiler" "$ffi_dir/ffi.slp" --emit obj "${opt[@]}" --target "$target" \
        --cc "$cross_cc" --output "$prefix.o" >/dev/null
      "$cross_cc" "$prefix.o" "$ffi_dir/callee.c" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c" \
        -o "$prefix" >"$prefix.link" 2>&1 ||
        fail "the $target FFI program did not link ($profile)"
      capture "$prefix.out" "$qemu" "$prefix"
    fi
    if ! cmp --silent "$ffi_dir/expected.stdout" "$prefix.out"; then
      echo "cross-check: FFI mismatch on $target ($profile)" >&2
      diff -u "$ffi_dir/expected.stdout" "$prefix.out" >&2 || true
      exit 1
    fi
    ffi_count=$((ffi_count + 1))
  done
done

echo "cross-check: $ffi_count FFI conformance programs ... ok"

# ---------------------------------------------------------------------------
# ABI conformance, inward: the platform toolchain calls Slopium code.
#
# Everything here is deliberately past a register boundary. Ten integers so two
# arrive on the stack, ten doubles so two do, and a mixture, because AAPCS64
# fills the integer and floating-point sequences independently and getting that
# wrong is invisible until a call has both kinds.
# ---------------------------------------------------------------------------

abi_dir="$result_dir/abi"
mkdir -p "$abi_dir"

cat >"$abi_dir/abi.slp" <<'EOF'
(take std:io println-i64)

(fn sum-ten ((a i64) (b i64) (c i64) (d i64) (e i64)
             (f i64) (g i64) (h i64) (i i64) (j i64)) -> i64
  (+ (+ (+ (+ a b) (+ c d)) (+ (+ e f) (+ g h))) (+ i j)))

(fn sum-ten-floats ((a f64) (b f64) (c f64) (d f64) (e f64)
                    (f f64) (g f64) (h f64) (i f64) (j f64)) -> f64
  (+ (+ (+ (+ a b) (+ c d)) (+ (+ e f) (+ g h))) (+ i j)))

(fn mixed-integers (
  (a1 i64) (d1 f64) (a2 i64) (d2 f64) (a3 i64) (d3 f64) (a4 i64) (d4 f64)
  (a5 i64) (d5 f64) (a6 i64) (d6 f64) (a7 i64) (d7 f64) (a8 i64) (d8 f64)
  (a9 i64) (d9 f64) (a10 i64) (d10 f64)
  ) -> i64
  (+ (+ (+ (+ a1 a2) (+ a3 a4)) (+ (+ a5 a6) (+ a7 a8))) (+ a9 a10)))

(fn mixed-floats (
  (a1 i64) (d1 f64) (a2 i64) (d2 f64) (a3 i64) (d3 f64) (a4 i64) (d4 f64)
  (a5 i64) (d5 f64) (a6 i64) (d6 f64) (a7 i64) (d7 f64) (a8 i64) (d8 f64)
  (a9 i64) (d9 f64) (a10 i64) (d10 f64)
  ) -> f64
  (+ (+ (+ (+ d1 d2) (+ d3 d4)) (+ (+ d5 d6) (+ d7 d8))) (+ d9 d10)))

(fn narrowed ((a i32) (b i32)) -> i32 (- a b))

(fn main () -> i32
  (println-i64 (sum-ten 1 2 3 4 5 6 7 8 9 10))
  0)
EOF

cat >"$abi_dir/caller.c" <<'EOF'
/* The platform toolchain's own view of the calling convention. If Slopium's
   differs — an argument in the wrong register, a stack slot at the wrong
   offset, an i32 that was not sign-extended — these totals come out wrong.

   `mixed` is the interesting one. AAPCS64 fills the integer and the
   floating-point sequences independently, so with ten of each interleaved the
   first eight of each kind go in registers and the last two of each kind land
   on the stack, interleaved there in declaration order. Reading them as one
   sequence, or in the wrong order, is invisible to every other case here. */
#include <stdio.h>
#include <stdint.h>

#define MIXED_PARAMS                                                   \
    int64_t, double, int64_t, double, int64_t, double, int64_t, double, \
    int64_t, double, int64_t, double, int64_t, double, int64_t, double, \
    int64_t, double, int64_t, double

#define MIXED_ARGS                                              \
    1, 1.5, 2, 2.5, 3, 3.5, 4, 4.5, 5, 5.5,                     \
    6, 6.5, 7, 7.5, 8, 8.5, 9, 9.5, 10, 10.5

/* `abi.slp` is a package of one module named after the file, so every symbol
 * carries the `abi:` prefix the hex spells out (`D-077`). */
int64_t sl_fn_6162693a73756d2d74656e(int64_t, int64_t, int64_t, int64_t, int64_t,
                             int64_t, int64_t, int64_t, int64_t, int64_t);
double sl_fn_6162693a73756d2d74656e2d666c6f617473(double, double, double, double, double,
                                          double, double, double, double, double);
int64_t sl_fn_6162693a6d697865642d696e746567657273(MIXED_PARAMS);
double sl_fn_6162693a6d697865642d666c6f617473(MIXED_PARAMS);
int32_t sl_fn_6162693a6e6172726f776564(int32_t, int32_t);

int main(void) {
    printf("integers %lld\n",
           (long long)sl_fn_6162693a73756d2d74656e(1, 2, 3, 4, 5, 6, 7, 8, 9, 10));
    printf("floats %.1f\n",
           sl_fn_6162693a73756d2d74656e2d666c6f617473(1.5, 2.5, 3.5, 4.5, 5.5,
                                              6.5, 7.5, 8.5, 9.5, 10.5));
    printf("mixed integers %lld\n",
           (long long)sl_fn_6162693a6d697865642d696e746567657273(MIXED_ARGS));
    printf("mixed floats %.1f\n", sl_fn_6162693a6d697865642d666c6f617473(MIXED_ARGS));
    printf("narrowed %d\n", (int)sl_fn_6162693a6e6172726f776564(-2000000000, 147483647));
    return 0;
}
EOF

cat >"$abi_dir/expected.stdout" <<'EOF'
integers 55
floats 60.0
mixed integers 55
mixed floats 60.0
narrowed -2147483647
EOF

# `slopic` emits a program entry point beside the functions, and here the C
# file supplies `main` instead. Renaming Slopium's out of the way is what lets
# the two link, and leaves the functions under test untouched.
host_objcopy="objcopy"
cross_objcopy="${cross_cc%cc}objcopy"
for tool in "$host_objcopy" "$cross_objcopy"; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    skip "$tool not found; ABI conformance skipped"
    echo "cross-check: all cross-backend checks passed"
    exit 0
  fi
done

abi_count=0
for target in host "$cross_target"; do
  if [[ "$target" == host ]]; then
    "$compiler" "$abi_dir/abi.slp" --emit obj --output "$abi_dir/host.o" >/dev/null
    "$host_objcopy" --redefine-sym main=sl_abi_unused_entry "$abi_dir/host.o"
    cc "$abi_dir/caller.c" "$abi_dir/host.o" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c" \
      -o "$abi_dir/host" >"$abi_dir/host.link" 2>&1 ||
      fail "the host ABI program did not link"
    capture "$abi_dir/host.out" "$abi_dir/host"
    actual="$abi_dir/host.out"
  else
    "$compiler" "$abi_dir/abi.slp" --emit obj --target "$target" --cc "$cross_cc" \
      --output "$abi_dir/cross.o" >/dev/null
    "$cross_objcopy" --redefine-sym main=sl_abi_unused_entry "$abi_dir/cross.o"
    "$cross_cc" "$abi_dir/caller.c" "$abi_dir/cross.o" "$workspace_dir/runtime/slop_rt_core.c" \
    "$workspace_dir/runtime/slop_rt_hosted.c" \
      -o "$abi_dir/cross" >"$abi_dir/cross.link" 2>&1 ||
      fail "the $target ABI program did not link"
    capture "$abi_dir/cross.out" "$qemu" "$abi_dir/cross"
    actual="$abi_dir/cross.out"
  fi
  if ! diff -u <(cat "$abi_dir/expected.stdout"; echo "exit status: 0") "$actual" >/dev/null; then
    echo "cross-check: ABI mismatch on $target" >&2
    diff -u <(cat "$abi_dir/expected.stdout"; echo "exit status: 0") "$actual" >&2 || true
    exit 1
  fi
  abi_count=$((abi_count + 1))
done

echo "cross-check: $abi_count ABI conformance programs ... ok"

echo "cross-check: all cross-backend checks passed"
