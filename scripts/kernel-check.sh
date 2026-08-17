#!/usr/bin/env bash
# The exit condition of v0.8: a kernel boots on a machine with no operating
# system under it and prints what it was asked to.
#
# Everything else in this suite runs a program under something that has already
# solved the hard part. This one does not: `tests/projects/freestanding/kernel`
# is entered by a multiboot loader in 32-bit protected mode, sets up its own
# page tables and its own stack, and only then is there anywhere for compiled
# Slopium to run. What it then does is the claim — a VGA write through a
# volatile `u16` at `0xB8000`, read back through the same pointer, and sent out
# of a `u8` serial port by a UART driver written in Slopium.
#
# Reading the framebuffer back is the point of the design. A kernel that only
# wrote would be asserting its own assumption; this one reports what the
# hardware actually holds, so a volatile access that was optimized away, widened
# or misaddressed changes the serial output.
#
# The image handed to QEMU is a 32-bit re-wrap of the linked kernel, because
# QEMU's multiboot loader refuses a 64-bit one outright — `hw/i386/multiboot.c`
# answers "Cannot load x86-64 image, give a 32bit one." Nothing about the
# program changes: `objcopy` rewrites the container, the contents and the entry
# stay where the linker script put them, and every address fits in 32 bits
# because the whole image lives at 1 MiB. The alternative was multiboot2 through
# `grub-mkrescue`, which reads a 64-bit image and costs `grub2` and `xorriso` in
# the dev shell and an ISO build on every run.
set -euo pipefail

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
    echo "kernel-check: $1" >&2
    if [ -n "${SLOPIUM_STRICT:-}" ]; then
        echo "kernel-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
        exit 1
    fi
}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

project="$root/tests/projects/freestanding/kernel"
manifest="$project/Slopium.toml"

qemu="${SLOPIUM_QEMU_SYSTEM_X86_64:-qemu-system-x86_64}"
if ! command -v "$qemu" >/dev/null 2>&1; then
    skip "no $qemu; the kernel is built but not booted"
    exit 0
fi
if ! command -v objcopy >/dev/null 2>&1; then
    skip "no objcopy; the kernel cannot be re-wrapped for the loader"
    exit 0
fi

manager="${SLOPIUM:-$root/target/debug/slopium}"
compiler="${SLOPIC:-$root/target/debug/slopic}"
if [ ! -x "$manager" ] || [ ! -x "$compiler" ]; then
    echo "kernel-check: building the toolchain"
    cargo build -q --workspace --manifest-path "$root/Cargo.toml"
fi

fail() {
    echo "kernel-check: $1" >&2
    exit 1
}

# ---------------------------------------------------------------------------
# Build.
# ---------------------------------------------------------------------------

env SLOPIC="$compiler" "$manager" --manifest-path "$manifest" clean >/dev/null
env SLOPIC="$compiler" "$manager" --manifest-path "$manifest" build >"$work/build.stdout" 2>"$work/build.stderr" ||
    { sed -n '1,40p' "$work/build.stderr" >&2; fail "the kernel did not build"; }

image="$project/target/x86_64-unknown-none/dev/kernel"
[ -f "$image" ] || fail "the kernel built no $image"

objcopy -I elf64-x86-64 -O elf32-i386 "$image" "$work/kernel.elf32" ||
    fail "objcopy could not re-wrap the kernel as a 32-bit image"

# ---------------------------------------------------------------------------
# Boot.
#
# `-display none` rather than `-nographic`: the latter claims the serial line
# for stdio, and the serial line is the answer. `isa-debug-exit` turns a write
# to port 0xf4 into an exit status of `(value << 1) | 1`, which is how a machine
# with no operating system reports one — and why the status is never zero.
# ---------------------------------------------------------------------------

serial="$work/serial.txt"
status=0
timeout --foreground 30 "$qemu" \
    -display none \
    -no-reboot \
    -m 64 \
    -kernel "$work/kernel.elf32" \
    -serial "file:$serial" \
    -device isa-debug-exit,iobase=0xf4,iosize=0x04 \
    >"$work/qemu.stdout" 2>"$work/qemu.stderr" || status=$?

if [ "$status" -eq 124 ]; then
    fail "the kernel did not halt within 30s; it hung rather than answered"
fi

# 0x10 is what `main` returns, and `sl_rt_panic` leaves with 0x11, so a panicked
# kernel cannot be read as a finished one.
expected_status=$(((0x10 << 1) | 1))
if [ "$status" -ne "$expected_status" ]; then
    echo "kernel-check: QEMU exited $status, expected $expected_status" >&2
    sed -n '1,20p' "$work/qemu.stderr" >&2
    [ -f "$serial" ] && { echo "kernel-check: serial held:" >&2; sed -n '1,20p' "$serial" >&2; }
    exit 1
fi

# ---------------------------------------------------------------------------
# What it said.
# ---------------------------------------------------------------------------

[ -f "$serial" ] || fail "the kernel wrote nothing to the serial port"

if ! diff -u "$project/expected.serial" "$serial" >"$work/serial.diff"; then
    echo "kernel-check: the kernel said something other than it was asked to" >&2
    sed -n '1,40p' "$work/serial.diff" >&2
    exit 1
fi

env SLOPIC="$compiler" "$manager" --manifest-path "$manifest" clean >/dev/null

echo "kernel-check: x86_64-unknown-none ok (booted under $(basename "$qemu"), answered over COM1)"
