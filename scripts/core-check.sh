#!/usr/bin/env bash
# Prove that the core half of the runtime is what `D-066` says it is: a program
# built against it alone links with `-nostdlib`, leaves no undefined symbol but
# the four hooks of `D-080`, and runs.
#
# This exists before any freestanding target does, on purpose. The runtime ABI
# freezes at v0.8, and freezing a core half nothing had ever linked would be
# freezing a guess — `D-026` and `D-029` are the precedent.
#
# Two checks per target:
#
#   1. `nm -u` over a relocatable link of the program object and
#      `slop_rt_core.o`. Nothing may be left undefined except `sl_rt_alloc`,
#      `sl_rt_free`, `sl_rt_abort` and `sl_rt_panic`. It is a subset and not an
#      equality: which of the four a given program reaches is the program's
#      business, and a build with `panic = "abort"` swaps `sl_rt_panic` for
#      `sl_rt_abort`. What must never appear is a fifth.
#   2. A real `-nostdlib` link against a `freestanding.c` that supplies those
#      four and an `_start`, then a run. The exit status carries the answer,
#      because a program with no libc has nothing to print with.
set -euo pipefail

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
    echo "core-check: $1" >&2
    if [ -n "${SLOPIUM_STRICT:-}" ]; then
        echo "core-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
        exit 1
    fi
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

slopic="${SLOPIC:-$root/target/debug/slopic}"
if [ ! -x "$slopic" ]; then
    echo "core-check: building slopic"
    cargo build -q -p slopic --manifest-path "$root/Cargo.toml"
fi

# The hooks a freestanding program owes the core runtime, and nothing else.
allowed_undefined="sl_rt_abort sl_rt_alloc sl_rt_free sl_rt_panic"

# The answer goes out through `core:string` and comes back, so the string half
# of the library is linked with `-nostdlib` on every run (`D-083`). A primitive
# that reached for libc fails here.
#
# Since v0.7.2 it goes out through `core:float` as well. That is the exit
# condition of the milestone in one line — a program with no C library prints a
# float — and `-1/3` is the value that makes it mean something: negative,
# non-terminating in binary, and needing all seventeen digits to come back as
# itself (`D-097`, `D-098`).
cat > "$work/program.slp" <<'SLP'
(take core:option Option)
(take core:string from-i64 to-i64 from-u64 to-u64 hash equals)
(take core:float from-f64 to-f64)
(take core:map Map new insert lookup)

; Named `main` and exported, because the link below is asked to validate an
; entry point rather than to skip the question with `--library`.
(export main)

; The allocator a freestanding program supplies, reached as a raw address so
; that the volatile accesses below have somewhere real to point (`D-067`).
(extern "sl_rt_alloc" (rt-alloc (size u64)) -> (Ptr u8))

; An empty map takes its type from a function that says what it returns, which
; is the only place one can be written (`D-104`).
(fn empty-table () -> (Map String i64)
  (new hash equals))

; The eight integer types reach no further than `core` does (`D-107`). A narrow
; type computes at 64 bits and is put back into its own width by a shift or a
; mask, and an unsigned one divides with an instruction rather than a call, so
; none of it can want libc — which is what `nm -u` below is asked to confirm.
(fn narrow-answer () -> i64
  (let masked (bit-and (as u8 0xFF) 0x2A))
  (let widened (as u64 masked))
  (let quotient (/ (* widened 3) 3))
  (let shifted (shr (as u64 0xFFFF_FFFF_FFFF_FFFF) 58))
  (if (and (= quotient (as u64 42)) (= shifted (as u64 63)))
    (as i64 quotient)
    0))

; A volatile access is instructions and never a call (`D-067`), which is what
; makes a raw pointer usable in a program with no operating system under it. A
; byte, a half and a signed narrow read, because those are the ones that need a
; `movzx` or an `ldrb` and could plausibly have reached for a helper.
(fn pointer-answer () -> i64
  (unsafe
    (let cell (rt-alloc 8))
    (volatile-write cell 0x2A)
    (volatile-write (as (Ptr u16) (ptr-offset cell 2)) 0xFFFF)
    (if (and (= (volatile-read cell) 0x2A)
        (= (volatile-read (as (Ptr i8) (ptr-offset cell 2))) -1))
      (as i64 (volatile-read cell))
      0)))

(fn main () -> i64
  (let mut values (list 3 4))
  (push (&mut values) 35)
  (let total (+ (get (& values) 0) (+ (get (& values) 1) (get (& values) 2))))
  (let text (from-i64 total))
  (match (to-i64 (& text))
    ((Option:Some parsed)
      (do
        (let third (/ (- 0.0 1.0) 3.0))
        (let written (from-f64 third))
        (match (to-f64 (& written))
          ((Option:Some restored)
            (if (= restored third)
              (do
                ; And through `core:map`, which is `core` because a bucket is a
                ; list and a list needs an allocator and not an operating
                ; system (`D-104`).
                (let mut table (empty-table))
                (insert (&mut table) "one" 1)
                (insert (&mut table) "two" 2)
                (let key "two")
                (match (lookup (& table) (& key))
                  ((Option:Some held)
                    (if (and (= held 2) (= (narrow-answer) 42) (= (pointer-answer) 42))
                      (do
                        ; And unsigned text, which is the one thing `D-107`
                        ; added to the library.
                        (let wide (from-u64 0xFFFF_FFFF_FFFF_FFFF))
                        (match (to-u64 (& wide))
                          ((Option:Some back)
                            (if (= back 0xFFFF_FFFF_FFFF_FFFF) parsed 0))
                          ((Option:None) 0)))
                      0))
                  ((Option:None) 0)))
              0))
          ((Option:None) 0))))
    ((Option:None) 0)))
SLP

# `sl_fn_` + the hex of `main`. A program's entry keeps its bare name where any
# other function is qualified by its module, so this is the name a freestanding
# program's own `_start` has to call — there is no `main(argc, argv)` wrapper
# standing in front of it. Spelled out rather than computed, so a change to the
# mangling fails here instead of silently linking something else.
answer_symbol="sl_fn_$(printf 'main' | od -An -tx1 | tr -d ' \n')"

# The other half of a freestanding program: an allocator over a static arena, a
# way to die, and an entry point. No libc, so the exit syscall is written out.
cat > "$work/freestanding.c" <<'FREESTANDING'
#include <stddef.h>
#include <stdint.h>

extern int64_t ANSWER_SYMBOL(void);

static unsigned char arena[1 << 16];
static uint64_t used = 0;

/* A bump allocator. `sl_rt_free` is a hook a freestanding program has to
 * define and is under no obligation to make do anything. */
void *sl_rt_alloc(uint64_t size) {
    uint64_t aligned = (size + 15) & ~(uint64_t)15;
    if (used + aligned > sizeof arena) {
        return NULL;
    }
    void *memory = &arena[used];
    used += aligned;
    return memory;
}

void sl_rt_free(void *memory) {
    (void)memory;
}

_Noreturn static void sys_exit(int64_t code) {
#if defined(__x86_64__)
    __asm__ volatile("syscall" ::"a"(60L), "D"(code) : "memory");
#elif defined(__aarch64__)
    register long number __asm__("x8") = 93;
    register long status __asm__("x0") = (long)code;
    __asm__ volatile("svc #0" ::"r"(number), "r"(status) : "memory");
#else
#error "no exit for this architecture"
#endif
    __builtin_unreachable();
}

_Noreturn void sl_rt_abort(void) {
    sys_exit(101);
}

_Noreturn void sl_rt_panic(const char *message) {
    (void)message;
    sys_exit(101);
}

_Noreturn void _start(void) {
    sys_exit(ANSWER_SYMBOL() == 42 ? 0 : 1);
}
FREESTANDING
sed -i "s/ANSWER_SYMBOL/$answer_symbol/g" "$work/freestanding.c"

# Called as `check_target ... || status=1`, which switches `set -e` off for
# everything inside it. Every command that can fail says so itself.
check_target() {
    local triple="$1" cc="$2" nm="$3" run="$4"
    local out="$work/$triple"
    mkdir -p "$out"

    if ! command -v "${cc%% *}" > /dev/null 2>&1; then
        skip "skipping $triple; no $cc"
        return 0
    fi

    # No `--library`: the entry point is validated, which is what makes the
    # `main` this program defines the thing the stub below has to reach.
    "$slopic" "$work/program.slp" \
        --target "$triple" --freestanding --emit obj \
        -o "$out/program.o" || return 1
    $cc -c -O2 -ffreestanding -fno-builtin -fno-stack-protector -ffunction-sections -fdata-sections \
        -o "$out/slop_rt_core.o" "$root/runtime/slop_rt_core.c" || return 1

    # 1. The claim. `-r` resolves what the two objects owe each other, so what
    # is left is what the program owes the world. `-no-pie` because a
    # relocatable link and a position-independent executable are alternatives.
    $cc -nostdlib -no-pie -Wl,-r -o "$out/combined.o" \
        "$out/program.o" "$out/slop_rt_core.o" || return 1
    local undefined unexpected=""
    undefined="$("$nm" -u "$out/combined.o" | awk '{ print $NF }' | sort -u)" || return 1
    for symbol in $undefined; do
        case " $allowed_undefined " in
            *" $symbol "*) ;;
            *) unexpected="$unexpected $symbol" ;;
        esac
    done
    if [ -n "$unexpected" ]; then
        echo "core-check: $triple owes the world more than the hooks:$unexpected" >&2
        echo "allowed: $allowed_undefined" >&2
        return 1
    fi

    # 2. It links and runs with no C library at all.
    $cc -c -O2 -ffreestanding -fno-builtin -fno-stack-protector -o "$out/freestanding.o" \
        "$work/freestanding.c" || return 1
    $cc -nostdlib -static -no-pie -o "$out/program" \
        "$out/program.o" "$out/slop_rt_core.o" "$out/freestanding.o" || return 1

    local left
    left="$("$nm" -u "$out/program" || true)"
    if [ -n "$left" ]; then
        echo "core-check: $triple linked but left symbols undefined:" >&2
        echo "$left" >&2
        return 1
    fi

    if [ -n "$run" ]; then
        if ! command -v "${run%% *}" > /dev/null 2>&1; then
            skip "$triple linked; not run (no ${run%% *})"
            echo "core-check: $triple ok"
            return 0
        fi
        $run "$out/program" || return 1
    else
        "$out/program" || return 1
    fi
    echo "core-check: $triple ok"
}

# The same program again, linked by the compiler instead of by the two `cc`
# lines above.
#
# Everything the stages above spell out — `-nostdlib`, `-static`, `-no-pie`, the
# freestanding compile flags, which half of the runtime to take — is a decision
# the toolchain now makes from the target alone, so this is the check that the
# hand-written command line and the shipped one agree. It is what `slopium build`
# does for a package, minus the manifest, and what v0.8.5's kernel is built with.
#
# `--freestanding` is absent on purpose: the environment comes from the `-none`
# row (`D-081`). So is `--library`, which is what leaves the entry point to be
# validated.
check_through_the_compiler() {
    local triple="$1" cc="$2" nm="$3"
    local out="$work/$triple-via-slopic"
    mkdir -p "$out"

    if ! command -v "${cc%% *}" > /dev/null 2>&1; then
        skip "skipping $triple; no $cc"
        return 0
    fi

    "$slopic" "$work/program.slp" \
        --target "$triple" --emit exe \
        --runtime "$root/runtime/slop_rt_core.c" \
        --runtime "$work/freestanding.c" \
        --cc "$cc" \
        -o "$out/program" || return 1

    local left
    left="$("$nm" -u "$out/program" || true)"
    if [ -n "$left" ]; then
        echo "core-check: $triple left symbols undefined after a compiler-driven link:" >&2
        echo "$left" >&2
        return 1
    fi

    "$out/program" || return 1
    echo "core-check: $triple ok (linked by slopic)"
}

status=0
check_target x86_64-unknown-linux-gnu "cc" "nm" "" || status=1
check_target aarch64-unknown-linux-gnu \
    "aarch64-unknown-linux-gnu-cc" \
    "aarch64-unknown-linux-gnu-nm" \
    "qemu-aarch64" || status=1
# Only x86-64 has a `-none` row: freestanding AArch64 waits until freestanding
# x86-64 is proven, which is what this line does.
check_through_the_compiler x86_64-unknown-none "cc" "nm" || status=1
exit "$status"
