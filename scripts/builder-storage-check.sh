#!/usr/bin/env bash
# Exercise the private owner in its real module without exporting a test API.
set -euo pipefail
ulimit -c 0
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
slopic="${SLOPIC:-$root/target/debug/slopic}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
source="$root/std/core/builder.slp"
mkdir -p "$work/package"
main="$work/package/main.slp"
expected="$(awk '/^\(test "packed / { n++ } END { print n+0 }' "$source")"
[[ "$expected" -gt 0 ]]
skip() {
    echo "builder-storage-check: $1" >&2
    if [[ -n "${SLOPIUM_STRICT:-}" ]]; then return 1; fi
}
cat > "$work/asan-cc" <<'CC'
#!/usr/bin/env bash
exec cc -fsanitize=address,undefined -fno-omit-frame-pointer "$@"
CC
chmod +x "$work/asan-cc"
cat > "$work/driver.c" <<'C'
#include <stdint.h>
#include <stdlib.h>
extern int64_t answer(void) __asm__("sl_fn_6d61696e3a616e73776572");
void *sl_rt_alloc(uint64_t n) { return malloc((size_t)n); }
void sl_rt_free(void *p) { free(p); }
_Noreturn void sl_rt_abort(void) { exit(101); }
_Noreturn void sl_rt_panic(const char *p) { (void)p; exit(101); }
int main(void) { return answer()==42 ? 0 : 1; }
C
for target in host aarch64; do
    cc=cc; flags=(); execute=()
    if [[ "$target" == aarch64 ]]; then
        cc=aarch64-unknown-linux-gnu-cc
        if ! command -v "$cc" >/dev/null || ! command -v qemu-aarch64 >/dev/null; then
            skip 'missing AArch64 compiler or QEMU'; continue
        fi
        flags=(--target aarch64-unknown-linux-gnu --cc "$cc")
        execute=(qemu-aarch64)
    fi
    for profile in dev release; do
        opt=(); [[ "$profile" == dev ]] || opt=(--optimize)
        cp "$source" "$main"
        base=(--source-root "$work/package" --toolchain-dependency core "$main")
        out="$work/$target-$profile"
        "$slopic" "${base[@]}" "${flags[@]}" "${opt[@]}" --test -o "$out"
        "${execute[@]}" "$out" > "$out.tests"
        passed="$(awk '/^test .* \.\.\. ok$/ { n++ } END { print n+0 }' "$out.tests")"
        [[ "$passed" == "$expected" ]]
        if [[ "$target" == host ]]; then
            if command -v valgrind >/dev/null; then
                valgrind --quiet --leak-check=full --error-exitcode=99 "$out" >/dev/null
            else skip 'missing Valgrind'; fi
        fi
        # Failures must be deliberate panics with the expected message.
        for failure in negative-used short long tail full negative-extra overflow \
            integer-zero-overflow integer-width-overflow integer-sign-overflow; do
            cp "$source" "$main"
            message='invalid state'
            case "$failure" in
                negative-used) expression='(storage-grow-zero &mut (PackedStorage :words (list) :tail 0 :used -1) 0)' ;;
                short) expression='(storage-grow-zero &mut (PackedStorage :words (list) :tail 0 :used 8) 0)' ;;
                long) expression='(storage-grow-zero &mut (PackedStorage :words (list (as u64 0)) :tail 0 :used 0) 0)' ;;
                tail) expression='(storage-grow-zero &mut (PackedStorage :words (list) :tail 256 :used 1) 0)' ;;
                full) expression='(storage-grow-zero &mut (PackedStorage :words (list (as u64 0)) :tail 1 :used 8) 0)' ;;
                negative-extra) expression='(storage-grow-zero &mut (storage-new) -1)'; message='negative length' ;;
                overflow) expression='(storage-length 9223372036854775807 1)'; message='length overflow' ;;
                integer-zero-overflow) expression='(storage-integer-room 9223372036854775807 0 0)'; message='length overflow' ;;
                integer-width-overflow) expression='(storage-integer-room 9223372036854775788 18446744073709551615 0)'; message='length overflow' ;;
                integer-sign-overflow) expression='(storage-integer-room 9223372036854775788 9223372036854775808 1)'; message='length overflow' ;;
            esac
            printf '\n(fn main () -> i32 %s 0)\n' "$expression" >> "$main"
            "$slopic" "${base[@]}" "${flags[@]}" "${opt[@]}" -o "$out-fail"
            if "${execute[@]}" "$out-fail" > "$out-fail.out" 2>&1; then
                echo "builder-storage-check: accepted $failure" >&2; exit 1
            else status=$?; fi
            [[ "$status" == 101 ]]
            grep -Fq "builder storage: $message" "$out-fail.out"
        done
        # Every future exposed writer checks forged states, including empty
        # pieces and zero integers, before any word access or allocation.
        for state in \
            '(PackedStorage :words (list) :tail 0 :used -1)' \
            '(PackedStorage :words (list) :tail 0 :used 8)' \
            '(PackedStorage :words (list (as u64 0)) :tail 0 :used 0)' \
            '(PackedStorage :words (list) :tail 256 :used 1)' \
            '(PackedStorage :words (list (as u64 0)) :tail 1 :used 8)'; do
            for operation in 'storage-write-byte 0' 'storage-write-str &""' \
                'storage-write-str &"a"' 'storage-write-i64 0' 'storage-write-u64 0'; do
                cp "$source" "$main"
                read -r writer argument <<< "$operation"
                printf '\n(fn main () -> i32 (let mut state %s) (%s &mut state %s) 0)\n' \
                    "$state" "$writer" "$argument" >> "$main"
                "$slopic" "${base[@]}" "${flags[@]}" "${opt[@]}" -o "$out-fail"
                if "${execute[@]}" "$out-fail" > "$out-fail.out" 2>&1; then
                    echo "builder-storage-check: accepted $operation on $state" >&2; exit 1
                else result=$?; fi
                [[ "$result" == 101 ]]
                grep -Fq 'builder storage: invalid state' "$out-fail.out"
            done
        done
        # Link only core and the four allocator/failure hooks on both targets.
        cp "$source" "$main"
        cat >> "$main" <<'SLP'
(export answer)
(fn answer () -> i64
  (let mut state (PackedStorage :words (list (as u64 65)) :tail 66 :used 9))
  (storage-grow-zero &mut state 71)
  (storage-write-str &mut state &"core\0\xffé")
  (storage-write-byte &mut state 255)
  (storage-write-i64 &mut state -9223372036854775808)
  (storage-write-u64 &mut state 18446744073709551615)
  (if (storage-valid &state) 42 1))
SLP
        "$slopic" "${base[@]}" "${flags[@]}" "${opt[@]}" --freestanding --library --emit obj -o "$out.o"
        "$cc" -O2 -ffreestanding -fno-builtin -fno-stack-protector -c "$root/runtime/slop_rt_core.c" -o "$out-runtime.o"
        "$cc" -r "$out.o" "$out-runtime.o" -o "$out-core.o"
        nm -u "$out-core.o" | awk '{print $NF}' > "$out-symbols"
        while read -r symbol; do
            case "$symbol" in sl_rt_alloc|sl_rt_free|sl_rt_abort|sl_rt_panic) ;;
                *) echo "builder-storage-check: non-core dependency $symbol" >&2; exit 1 ;;
            esac
        done < "$out-symbols"
        "$cc" "$out-core.o" "$work/driver.c" -o "$out-core"
        "${execute[@]}" "$out-core"
        # Observe actual runtime calls, without changing the production owner.
        cp "$source" "$main"
        python3 - "$main" <<'PY'
import sys
with open(sys.argv[1], 'a') as f:
    f.write('''
(export answer)
(fn answer () -> i64
  (let mut state (storage-new))
  (let piece "''' + 'a' * 4096 + '''")
  (let mut repeat 0)
  (while (< repeat 256)
    (storage-write-str &mut state &piece)
    (set repeat (+ repeat 1)))
  (storage-write-str &mut state &"")
  (storage-write-byte &mut state 255)
  (storage-check &state)
  (match &state ((PackedStorage :words _ :tail _ :used used) (clone used))))
''')
PY
        "$slopic" "${base[@]}" "${flags[@]}" "${opt[@]}" --freestanding --library --emit obj -o "$out-count.o"
        "$cc" -D_POSIX_C_SOURCE=200809L -Wl,--wrap=sl_rt_list_push \
            -Wl,--wrap=sl_rt_builder_pack_words "$out-count.o" \
            "$root/scripts/builder-pack-count.c" "$root/runtime/slop_rt_core.c" \
            "$root/runtime/slop_rt_hosted.c" -o "$out-count"
        "${execute[@]}" "$out-count"
        echo "builder-storage-check: $expected owner tests, panics and core linkage $target/$profile ... ok"
    done
done
cp "$source" "$main"
"$slopic" --source-root "$work/package" --toolchain-dependency core "$main" \
    --test --optimize --cc "$work/asan-cc" -o "$work/asan"
ASAN_OPTIONS=detect_leaks=1 "$work/asan" >/dev/null
cat >> "$main" <<'SLP'
(fn main () -> i32
  (let mut state (storage-new))
  (let held &state)
  (storage-grow-zero &mut state 1)
  (storage-valid held)
  0)
SLP
if "$slopic" --source-root "$work/package" --toolchain-dependency core "$main" \
    --diagnostic-format json -o "$work/loan" >"$work/loan.out" 2>"$work/loan.json"; then
    echo 'builder-storage-check: accepted live shared owner loan' >&2; exit 1
fi
python3 - "$work/loan.json" <<'PY'
import json, sys
errors=[json.loads(line) for line in open(sys.argv[1]) if line.strip()]
assert len(errors)==1 and errors[0]['code']=='SL0300', errors
assert errors[0]['span']['line']>0 and errors[0]['span']['column']>0, errors
PY
echo 'builder-storage-check: ASan/UBSan and owner loan ... ok'
