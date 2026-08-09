#!/usr/bin/env bash
# Checks the compiler's own object writer against the platform assembler.
#
# The claim being tested is that `--emit obj` produces the same program `as`
# would have produced from the same `--emit asm` output. On AArch64 that is
# checked byte for byte, which fixed-width instructions make possible. On
# x86-64 it is checked instruction by instruction, because this encoder does
# not always pick the same one of several correct encodings — see the note in
# `x86_64_inst.rs`. Both are then linked and run.
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_dir"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

cargo build --release --workspace >/dev/null

slopic="$workspace_dir/target/release/slopic"
host_target="x86_64-unknown-linux-gnu"
cross_target="aarch64-unknown-linux-gnu"

programs=(
  "tests/projects/pass/basics/src/main.slp"
  "tests/projects/pass/loops/src/main.slp"
  "tests/projects/pass/aggregates-patterns/src/main.slp"
  "tests/projects/pass/process-io/src/main.slp"
  "tests/projects/pass/ownership-borrows/src/main.slp"
  "tests/projects/pass/function-values/src/main.slp"
  "examples/fibonacci.slp"
  "examples/lists.slp"
  "examples/match.slp"
  "examples/structs.slp"
  "examples/ownership.slp"
)

fail() {
  echo "object-check: $*" >&2
  exit 1
}

# Section contents as a hex dump, so a difference is reported as one.
section_hex() {
  local objdump="$1" object="$2" section="$3"
  "$objdump" -s -j "$section" "$object" 2>/dev/null | tail -n +4 || true
}

# The instruction stream with every address and symbol reference removed, so
# that two correct encodings of different lengths still compare equal.
instruction_stream() {
  local objdump="$1" object="$2"
  "$objdump" -d -M intel --no-addresses --no-show-raw-insn "$object" \
    | sed -e 's/<[^>]*>//g' -e 's/0x[0-9a-f]*//g' -e '/file format/d' \
    | grep -v '^[[:space:]]*$'
}

# Relocations and symbols, sorted and stripped of the columns that only say
# where in the file a thing happens to live.
relocations() {
  local readelf="$1" object="$2"
  "$readelf" -r "$object" \
    | awk '/^[0-9a-f]{12}/ { print $3, $5, $6, $7 }' \
    | sort
}

# `$3` is the size, which only matches when the encodings do: this compiler
# always uses a 32-bit jump displacement, so its x86-64 functions are a few
# bytes longer than the assembler's. Names, bindings and types have to agree
# either way, and on AArch64 the sizes are compared too.
symbols() {
  local readelf="$1" object="$2" with_sizes="$3"
  if [ "$with_sizes" = "yes" ]; then
    "$readelf" -s "$object" \
      | awk '$4 == "FUNC" || $4 == "NOTYPE" { print $4, $5, $3, $8 }' \
      | grep -v '^NOTYPE LOCAL' | sort
  else
    "$readelf" -s "$object" \
      | awk '$4 == "FUNC" || $4 == "NOTYPE" { print $4, $5, $8 }' \
      | grep -v '^NOTYPE LOCAL' | sort
  fi
}

check_target() {
  local target="$1" objdump="$2" readelf="$3" assembler="$4" byte_exact="$5"
  local checked=0

  for source in "${programs[@]}"; do
    for profile in dev release; do
      local stem opt
      stem="$scratch/$(echo "$source" | tr '/' '_').$target.$profile"
      opt=()
      [ "$profile" = release ] && opt=(--optimize)

      "$slopic" --emit asm --target "$target" "${opt[@]}" \
        -o "$stem.s" "$source" >/dev/null
      "$slopic" --emit obj --target "$target" "${opt[@]}" \
        -o "$stem.ours.o" "$source" >/dev/null
      "$assembler" -o "$stem.gas.o" "$stem.s"

      if [ "$byte_exact" = "yes" ]; then
        for section in .text .rodata; do
          if ! diff -u \
            <(section_hex "$objdump" "$stem.ours.o" "$section") \
            <(section_hex "$objdump" "$stem.gas.o" "$section") >"$stem.$section.diff"; then
            head -20 "$stem.$section.diff" >&2
            fail "$source ($target, $profile): $section differs from the assembler"
          fi
        done
      else
        if ! diff -u \
          <(instruction_stream "$objdump" "$stem.ours.o") \
          <(instruction_stream "$objdump" "$stem.gas.o") >"$stem.dis.diff"; then
          head -20 "$stem.dis.diff" >&2
          fail "$source ($target, $profile): the instruction stream differs"
        fi
        if ! diff -u \
          <(section_hex "$objdump" "$stem.ours.o" .rodata) \
          <(section_hex "$objdump" "$stem.gas.o" .rodata) >"$stem.rodata.diff"; then
          head -20 "$stem.rodata.diff" >&2
          fail "$source ($target, $profile): .rodata differs from the assembler"
        fi
      fi

      if ! diff -u \
        <(relocations "$readelf" "$stem.ours.o") \
        <(relocations "$readelf" "$stem.gas.o") >"$stem.rel.diff"; then
        head -20 "$stem.rel.diff" >&2
        fail "$source ($target, $profile): the relocations differ"
      fi

      if ! diff -u \
        <(symbols "$readelf" "$stem.ours.o" "$byte_exact") \
        <(symbols "$readelf" "$stem.gas.o" "$byte_exact") >"$stem.sym.diff"; then
        head -20 "$stem.sym.diff" >&2
        fail "$source ($target, $profile): the symbol table differs"
      fi

      checked=$((checked + 1))
    done
  done
  echo "$checked"
}

# ----- x86-64, natively ------------------------------------------------------

if ! command -v as >/dev/null || ! command -v objdump >/dev/null; then
  echo "object-check: no host binutils; skipped"
  exit 0
fi

host_checked="$(check_target "$host_target" objdump readelf as no)"
echo "object-check: $host_checked x86-64 objects match the assembler ... ok"

# Behaviour, which is the only claim that ultimately matters.
host_run=0
for source in "${programs[@]}"; do
  "$slopic" --emit exe -o "$scratch/ours.out" "$source" >/dev/null
  SLOPIUM_OBJECT_WRITER=external "$slopic" --emit exe -o "$scratch/gas.out" "$source" >/dev/null
  ours="$("$scratch/ours.out" </dev/null 2>&1; echo "exit=$?")"
  theirs="$("$scratch/gas.out" </dev/null 2>&1; echo "exit=$?")"
  if [ "$ours" != "$theirs" ]; then
    fail "$source: the two object paths disagree at run time"
  fi
  host_run=$((host_run + 1))
done
echo "object-check: $host_run x86-64 programs run identically either way ... ok"

# ----- AArch64, cross --------------------------------------------------------

cross_cc="${SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU:-}"
cross_as="${cross_cc%cc}as"
cross_objdump="${cross_cc%cc}objdump"
cross_readelf="${cross_cc%cc}readelf"
qemu="${SLOPIUM_QEMU_AARCH64:-qemu-aarch64}"

if [ -z "$cross_cc" ] || ! command -v "$cross_as" >/dev/null; then
  echo "object-check: no aarch64 toolchain; cross checks skipped"
  exit 0
fi

cross_checked="$(check_target "$cross_target" "$cross_objdump" "$cross_readelf" "$cross_as" yes)"
echo "object-check: $cross_checked aarch64 objects are byte-identical to the assembler ... ok"

if ! command -v "$qemu" >/dev/null; then
  echo "object-check: no emulator; aarch64 programs not run"
  exit 0
fi

cross_run=0
for source in "${programs[@]}"; do
  "$slopic" --emit exe --cc "$cross_cc" --target "$cross_target" \
    -o "$scratch/ours.out" "$source" >/dev/null
  SLOPIUM_OBJECT_WRITER=external "$slopic" --emit exe --cc "$cross_cc" \
    --target "$cross_target" -o "$scratch/gas.out" "$source" >/dev/null
  ours="$("$qemu" "$scratch/ours.out" </dev/null 2>&1; echo "exit=$?")"
  theirs="$("$qemu" "$scratch/gas.out" </dev/null 2>&1; echo "exit=$?")"
  if [ "$ours" != "$theirs" ]; then
    fail "$source: the two object paths disagree at run time on aarch64"
  fi
  cross_run=$((cross_run + 1))
done
echo "object-check: $cross_run aarch64 programs run identically either way ... ok"

# ----- the fallback the debug path depends on --------------------------------

# `--debug` must not reach the object writer, which emits no DWARF (`D-028`).
"$slopic" --emit obj --debug -o "$scratch/debug.o" "examples/fibonacci.slp" >/dev/null
if ! readelf -S "$scratch/debug.o" | grep -q '\.debug_line'; then
  fail "a --debug object lost its line table"
fi
echo "object-check: a debug build still goes through the assembler ... ok"

echo "object-check: all object-writer checks passed"
