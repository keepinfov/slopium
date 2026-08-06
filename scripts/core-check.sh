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
cat > "$work/program.slp" <<'SLP'
(take core:option Option)
(take core:string from-i64 to-i64)

(export answer)

(fn answer () -> i64
  (let mut values (list 3 4))
  (push (&mut values) 35)
  (let total (+ (get (& values) 0) (+ (get (& values) 1) (get (& values) 2))))
  (let text (from-i64 total))
  (match (to-i64 (& text))
    ((Option:Some parsed) parsed)
    ((Option:None) 0)))
SLP

# `sl_fn_` + the hex of `program:answer`, which is how a module-qualified name
# is mangled. Spelled out rather than computed, so a change to the mangling
# fails here instead of silently linking something else.
answer_symbol="sl_fn_$(printf 'program:answer' | od -An -tx1 | tr -d ' \n')"

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
        echo "core-check: skipping $triple; no $cc"
        return 0
    fi

    "$slopic" "$work/program.slp" \
        --target "$triple" --freestanding --library --emit obj \
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
            echo "core-check: $triple linked; not run (no ${run%% *})"
            echo "core-check: $triple ok"
            return 0
        fi
        $run "$out/program" || return 1
    else
        "$out/program" || return 1
    fi
    echo "core-check: $triple ok"
}

status=0
check_target x86_64-unknown-linux-gnu "cc" "nm" "" || status=1
check_target aarch64-unknown-linux-gnu \
    "aarch64-unknown-linux-gnu-cc" \
    "aarch64-unknown-linux-gnu-nm" \
    "qemu-aarch64" || status=1
exit "$status"
