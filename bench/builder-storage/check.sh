#!/usr/bin/env bash
# Research checks only; does not install a runtime or update any fixture.
set -euo pipefail
ulimit -c 0
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
compiler="${1:?pass the compiler executable}"
here="$root/bench/builder-storage"
mkdir -p "$root/bench/results/builder-storage"
work="$(mktemp -d "$root/bench/results/builder-storage/check.XXXXXX")"
mkdir -p "$work/package"
cp "$here/packed.slp" "$work/package/packed.slp"
cp "$here/check.slp" "$work/package/main.slp"
base=(--source-root "$work/package" --toolchain-dependency std "$work/package/main.slp"
  --runtime "$here/packed.c" --runtime "$root/runtime/slop_rt_core.c"
  --runtime "$root/runtime/slop_rt_hosted.c")
for target in host aarch64; do
  for profile in dev release; do
    opt=(); [[ "$profile" == dev ]] || opt=(--optimize)
    target_flags=(); execute=()
    if [[ "$target" == aarch64 ]]; then
      target_flags=(--target aarch64-unknown-linux-gnu --cc aarch64-unknown-linux-gnu-cc)
      execute=(qemu-aarch64)
    fi
    binary="$work/$target-$profile"
    "$compiler" "${base[@]}" "${opt[@]}" "${target_flags[@]}" -o "$binary" >"$binary.build" 2>&1
    "${execute[@]}" "$binary"
    if [[ "$target" == host ]]; then
      valgrind --leak-check=full --error-exitcode=99 --log-file="$binary.valgrind" "$binary"
    fi
    echo "storage-study: packed ownership/read checks $target/$profile ... ok"
  done
done
cat >"$work/asan-cc" <<'CC'
#!/usr/bin/env bash
exec cc -fsanitize=address,undefined -fno-omit-frame-pointer "$@"
CC
chmod +x "$work/asan-cc"
"$compiler" "${base[@]}" --optimize --cc "$work/asan-cc" -o "$work/packed-asan" >"$work/packed-asan.build" 2>&1
ASAN_OPTIONS=detect_leaks=1 "$work/packed-asan"
for mode in 0 1 2; do
  sources=("$here/models.c" "$here/list-model.c" "$here/model-probe.c")
  cc -std=c11 -O2 -Wall -Wextra -Werror -DSTORAGE="$mode" "${sources[@]}" -o "$work/model-$mode"
  valgrind --leak-check=full --error-exitcode=99 --log-file="$work/model-$mode.valgrind" "$work/model-$mode" --check
  cc -std=c11 -O1 -g -fsanitize=address,undefined -fno-omit-frame-pointer \
    -Wall -Wextra -Werror -DSTORAGE="$mode" "${sources[@]}" -o "$work/model-$mode-asan"
  ASAN_OPTIONS=detect_leaks=1 "$work/model-$mode-asan" --check
  echo "storage-study: C model $mode ownership under Memcheck/ASan/UBSan ... ok"
done
cc -std=c11 -D_POSIX_C_SOURCE=200809L -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined \
  "$here/bridge-check.c" "$here/packed.c" "$root/runtime/slop_rt_core.c" \
  "$root/runtime/slop_rt_hosted.c" -o "$work/bridge"
ASAN_OPTIONS=detect_leaks=1 "$work/bridge"
for failure in short negative overflow tail null finish; do
  if ASAN_OPTIONS=detect_leaks=0 "$work/bridge" "$failure" >"$work/$failure.out" 2>&1; then
    echo "storage-study: bridge accepted invalid $failure" >&2; exit 1
  else
    status=$?
  fi
  [[ "$status" == 101 ]] && grep -Fq 'storage study: packed' "$work/$failure.out"
done
# The prototype's public read also rejects its lower and upper bounds.
for index in -1 0 1; do
  cat >"$work/package/main.slp" <<SLP
(take packed new read-at)
(fn main () -> i32 (let out (new)) (read-at &out $index) 0)
SLP
  "$compiler" "${base[@]}" --optimize -o "$work/bounds-$index" >"$work/bounds-$index.build" 2>&1
  if "$work/bounds-$index" >"$work/bounds-$index.out" 2>&1; then
    echo "storage-study: packed read accepted invalid index $index" >&2; exit 1
  else status=$?; fi
  [[ "$status" == 101 ]] && grep -Fq 'storage study: read bounds' "$work/bounds-$index.out"
done
echo "storage-study: bridge and read bounds ... ok"
cat >"$work/package/main.slp" <<'SLP'
(take packed new write-str write-byte write-u64 build)
(take core:string equals)
(export answer)
(fn answer () -> i64
  (let mut out (new))
  (write-str &mut out &"core\0\xff:")
  (write-u64 &mut out 18446744073709551615)
  (write-byte &mut out 33)
  (if (equals &(build out) &"core\0\xff:18446744073709551615!") 42 0))
SLP
cat >"$work/driver.c" <<'C'
#include <stdint.h>
#include <stdlib.h>
extern int64_t answer(void) __asm__("sl_fn_6d61696e3a616e73776572");
void *sl_rt_alloc(uint64_t size) { return malloc((size_t)size); }
void sl_rt_free(void *memory) { free(memory); }
_Noreturn void sl_rt_abort(void) { exit(101); }
_Noreturn void sl_rt_panic(const char *message) { (void)message; exit(101); }
int main(void) { return answer()==42 ? 0 : 1; }
C
for target in host aarch64; do
  cross=(); execute=(); cc=cc
  if [[ "$target" == aarch64 ]]; then
    cross=(--target aarch64-unknown-linux-gnu --cc aarch64-unknown-linux-gnu-cc)
    execute=(qemu-aarch64); cc=aarch64-unknown-linux-gnu-cc
  fi
  for profile in dev release; do
    opt=(); [[ "$profile" == dev ]] || opt=(--optimize)
    prefix="$work/core-$target-$profile"
    "$compiler" --freestanding --library --emit obj "${opt[@]}" "${cross[@]}" \
      --source-root "$work/package" --toolchain-dependency core \
      "$work/package/main.slp" -o "$prefix.o" >"$prefix.build" 2>&1
    "$cc" -O2 -ffreestanding -fno-builtin -fno-stack-protector -c "$here/packed.c" -o "$prefix-helper.o"
    undefined="$(nm -u "$prefix-helper.o" | awk '{print $NF}' | sort)"
    [[ "$undefined" == $'sl_rt_alloc\nsl_rt_panic' ]]
    "$cc" -O2 -ffreestanding -fno-builtin -fno-stack-protector \
      "$prefix.o" "$prefix-helper.o" "$root/runtime/slop_rt_core.c" "$work/driver.c" -o "$prefix"
    "${execute[@]}" "$prefix"
    echo "storage-study: core-only linkage $target/$profile ... ok"
  done
done
echo "storage-study: artifacts $work"
