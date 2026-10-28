#!/usr/bin/env bash
# Check the public bridge independently of Slopium's validated callers.
set -euo pipefail
ulimit -c 0
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
sources=("$root/scripts/builder-pack-check.c" "$root/runtime/slop_rt_core.c"
    "$root/runtime/slop_rt_hosted.c")
failures=(negative-used negative-length overflow negative-count short-count
    long-count null-words null-text tail full-tail)
check() {
    local binary="$1"; shift
    "$@" "$binary"
    for failure in "${failures[@]}"; do
        if "$@" "$binary" "$failure" > "$work/failure.out" 2>&1; then
            echo "builder-pack-check: accepted $failure" >&2; exit 1
        else result=$?; fi
        [[ "$result" == 101 ]]
        grep -Fxq 'slopium runtime error: builder packed append bounds' "$work/failure.out"
    done
}
for target in host aarch64; do
    cc=cc; execute=()
    if [[ "$target" == aarch64 ]]; then
        cc=aarch64-unknown-linux-gnu-cc
        if ! command -v "$cc" >/dev/null || ! command -v qemu-aarch64 >/dev/null; then
            echo 'builder-pack-check: missing AArch64 compiler or QEMU' >&2
            [[ -z "${SLOPIUM_STRICT:-}" ]] || exit 1
            continue
        fi
        execute=(qemu-aarch64)
    fi
    for opt in 0 2; do
        out="$work/$target-O$opt"
        # Fortify requires optimization; undefine it in the preprocessor after
        # any compiler-wrapper flags for this deliberately unoptimized row.
        hardening=()
        [[ "$opt" != 0 ]] || hardening=(-Wp,-U_FORTIFY_SOURCE)
        "$cc" -std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wextra -Werror \
            "-O$opt" "${hardening[@]}" "${sources[@]}" -o "$out"
        check "$out" "${execute[@]}"
        if [[ "$target" == host && "$opt" == 2 ]]; then
            if command -v valgrind >/dev/null; then
                valgrind --quiet --leak-check=full --error-exitcode=99 "$out"
            else
                echo 'builder-pack-check: missing Valgrind' >&2
                [[ -z "${SLOPIUM_STRICT:-}" ]] || exit 1
            fi
        fi
        echo "builder-pack-check: bytes, guards and ten failures $target/O$opt ... ok"
    done
done
cc -std=c11 -D_POSIX_C_SOURCE=200809L -O1 -g -Wall -Wextra -Werror \
    -fsanitize=address,undefined -fno-omit-frame-pointer "${sources[@]}" -o "$work/asan"
ASAN_OPTIONS=detect_leaks=1 check "$work/asan"
echo 'builder-pack-check: ASan/UBSan ... ok'
