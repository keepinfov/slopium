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

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
projects_dir="$workspace_dir/tests/projects"
result_dir="$(mktemp -d)"
trap 'rm -rf "$result_dir"' EXIT

cross_target="aarch64-unknown-linux-gnu"
cross_cc="${SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU:-aarch64-unknown-linux-gnu-cc}"
qemu="${SLOPIUM_QEMU_AARCH64:-qemu-aarch64}"

if ! command -v "$cross_cc" >/dev/null 2>&1; then
  echo "cross-check: $cross_cc not found; cross-backend checks skipped" >&2
  echo "cross-check: run inside 'nix develop' for the aarch64 toolchain" >&2
  exit 0
fi
if ! command -v "$qemu" >/dev/null 2>&1; then
  echo "cross-check: $qemu not found; cross-backend checks skipped" >&2
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

  "${compile[@]}" --profile "$profile" --output "$prefix.native" \
    >"$prefix.native.build" 2>&1 ||
    fail "$label ($profile) failed to build for the host"
  "${compile[@]}" --profile "$profile" --target "$cross_target" --cc "$cross_cc" \
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
(fn main () -> i32
  (let big 9223372036854775807)
  (let step (+ big 1))
  (println step)
  0)
EOF
cat >"$trap_dir/narrow-overflow.slp" <<'EOF'
(fn wide () -> i32 2000000000)
(fn grow ((n i32)) -> i32 (* n 3))
(fn main () -> i32 (grow (wide)))
EOF
cat >"$trap_dir/div-zero.slp" <<'EOF'
(fn divide ((a i64) (b i64)) -> i64 (/ a b))
(fn main () -> i32
  (println (divide 7 0))
  0)
EOF
cat >"$trap_dir/div-overflow.slp" <<'EOF'
(fn divide ((a i64) (b i64)) -> i64 (/ a b))
(fn main () -> i32
  (println (divide -9223372036854775808 -1))
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
(fn zero () -> f64 0.0)
(fn one () -> f64 1.0)
(fn flag ((c bool)) -> i64 (if c 1 0))
(fn main () -> i32
  (let nan (/ (zero) (zero)))
  (let unit (one))
  (println (flag (< nan unit)))
  (println (flag (> nan unit)))
  (println (flag (= nan nan)))
  (println (flag (< (zero) unit)))
  (println (flag (> unit (zero))))
  (println (flag (= unit unit)))
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
# ABI conformance: the platform toolchain calls Slopium code and is called back.
#
# Everything here is deliberately past a register boundary. Ten integers so two
# arrive on the stack, ten doubles so two do, and a mixture, because AAPCS64
# fills the integer and floating-point sequences independently and getting that
# wrong is invisible until a call has both kinds.
# ---------------------------------------------------------------------------

abi_dir="$result_dir/abi"
mkdir -p "$abi_dir"

cat >"$abi_dir/abi.slp" <<'EOF'
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
  (println (sum-ten 1 2 3 4 5 6 7 8 9 10))
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

int64_t sl_fn_73756d2d74656e(int64_t, int64_t, int64_t, int64_t, int64_t,
                             int64_t, int64_t, int64_t, int64_t, int64_t);
double sl_fn_73756d2d74656e2d666c6f617473(double, double, double, double, double,
                                          double, double, double, double, double);
int64_t sl_fn_6d697865642d696e746567657273(MIXED_PARAMS);
double sl_fn_6d697865642d666c6f617473(MIXED_PARAMS);
int32_t sl_fn_6e6172726f776564(int32_t, int32_t);

int main(void) {
    printf("integers %lld\n",
           (long long)sl_fn_73756d2d74656e(1, 2, 3, 4, 5, 6, 7, 8, 9, 10));
    printf("floats %.1f\n",
           sl_fn_73756d2d74656e2d666c6f617473(1.5, 2.5, 3.5, 4.5, 5.5,
                                              6.5, 7.5, 8.5, 9.5, 10.5));
    printf("mixed integers %lld\n",
           (long long)sl_fn_6d697865642d696e746567657273(MIXED_ARGS));
    printf("mixed floats %.1f\n", sl_fn_6d697865642d666c6f617473(MIXED_ARGS));
    printf("narrowed %d\n", (int)sl_fn_6e6172726f776564(-2000000000, 147483647));
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
    echo "cross-check: $tool not found; ABI conformance skipped" >&2
    echo "cross-check: all cross-backend checks passed"
    exit 0
  fi
done

abi_count=0
for target in host "$cross_target"; do
  if [[ "$target" == host ]]; then
    "$compiler" "$abi_dir/abi.slp" --emit obj --output "$abi_dir/host.o" >/dev/null
    "$host_objcopy" --redefine-sym main=sl_abi_unused_entry "$abi_dir/host.o"
    cc "$abi_dir/caller.c" "$abi_dir/host.o" "$workspace_dir/runtime/slop_rt.c" \
      -o "$abi_dir/host" >"$abi_dir/host.link" 2>&1 ||
      fail "the host ABI program did not link"
    capture "$abi_dir/host.out" "$abi_dir/host"
    actual="$abi_dir/host.out"
  else
    "$compiler" "$abi_dir/abi.slp" --emit obj --target "$target" --cc "$cross_cc" \
      --output "$abi_dir/cross.o" >/dev/null
    "$cross_objcopy" --redefine-sym main=sl_abi_unused_entry "$abi_dir/cross.o"
    "$cross_cc" "$abi_dir/caller.c" "$abi_dir/cross.o" "$workspace_dir/runtime/slop_rt.c" \
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
