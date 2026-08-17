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

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
  echo "object-check: $1" >&2
  if [ -n "${SLOPIUM_STRICT:-}" ]; then
    echo "object-check: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
    exit 1
  fi
}

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
  # A field address and a load through a borrow (`D-099`, `D-100`): the same
  # `lea` and `add` the backends already emitted, now off a heap pointer rather
  # than the frame, which is an operand combination neither had assembled.
  "tests/projects/pass/borrow-reads/src/main.slp"
  # Closures add no instruction (`D-101`), and that is the claim: what this
  # compares is the glue a lifted `lambda` and its environment generate, which
  # is a clone and a drop helper per capture shape and one more function than
  # the source has.
  "tests/projects/pass/closures/src/main.slp"
  # A map is a generic struct holding two function values and a list of lists
  # (`D-104`), so its clone and drop helpers nest one level deeper than
  # anything else in this corpus, and `replace` is a runtime call neither
  # backend had emitted.
  "tests/projects/pass/maps/src/main.slp"
  # The largest single addition to this corpus so far (`D-106`): a remainder,
  # six bitwise operations, two shifts and four comparisons, on both widths.
  # Nine of those encodings neither backend had ever emitted, and the AArch64
  # half includes `msub` — the only four-register instruction the compiler
  # selects, and therefore the one most worth holding against `as`.
  "tests/projects/pass/vocabulary/src/main.slp"
  # The integer axis (`D-107`), which is the second-largest addition: `shr`,
  # `div`, `mul` and the constant-count shift on x86-64, and `udiv`, `lsr`,
  # `umulh` and the four sub-word extensions on AArch64. None of the eight had
  # ever been emitted, and the canonicalising tail that follows every narrow
  # operation is a shape neither backend had assembled at all.
  "tests/projects/pass/integer-axis/src/main.slp"
  # The narrow memory a raw pointer reaches through (`D-067`), which is the
  # only sub-word load or store this compiler emits: `movzx` from a byte and a
  # half and the `0x66`-prefixed store on x86-64, and `ldrb`, `ldrh`, `strb`,
  # `strh` and the four-byte pair on AArch64. Ten encodings neither backend
  # had, and the ones most worth holding against `as` — a width that is one
  # size wrong does not fault, it writes over the register next door.
  "tests/projects/pass/raw-pointers/src/main.slp"
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
#
# Deliberately not tolerant of a missing section: a section that is absent
# dumps nothing, and comparing nothing against nothing is how this check would
# pass without having looked at anything.
section_hex() {
  local objdump="$1" object="$2" section="$3"
  "$objdump" -s -j "$section" "$object" | tail -n +4
}

# Every section that holds bytes, by name. A function owns the `.text` its code
# sits in, so the set is per program and cannot be written down here.
section_names() {
  local readelf="$1" object="$2"
  # The `[ 4]` index is stripped before the columns are read: it is one field
  # when the number reaches two digits and two fields below that, which would
  # otherwise shift every column depending on how many sections there are.
  #
  # Only sections that hold bytes, which is what the comparison is about: `as`
  # emits an empty `.data` whatever it was given, and `.note.GNU-stack` is
  # empty by design. A section non-empty on one side and absent on the other
  # still differs, so nothing is hidden by this.
  "$readelf" -SW "$object" \
    | sed -e 's/^ *\[ *[0-9]\{1,\}\] *//' \
    | awk '$1 ~ /^\./ && $2 == "PROGBITS" && $5 ~ /[1-9a-f]/ { print $1 }' \
    | sort
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

# Which section each defined symbol sits in, by name rather than by index —
# the indices are each writer's own, the names are the claim. Without this a
# function attached to the wrong section still compares equal, which is
# precisely the property per-function sections add (`D-030`).
symbol_sections() {
  local objdump="$1" object="$2"
  "$objdump" -t "$object" \
    | awk '$NF ~ /^sl_/ { print $NF, $(NF-2) }' \
    | sort
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

      # The set of sections first: a section present in one object and absent
      # from the other has to fail here, rather than have each of its contents
      # compared against nothing.
      if ! diff -u \
        <(section_names "$readelf" "$stem.ours.o") \
        <(section_names "$readelf" "$stem.gas.o") >"$stem.sections.diff"; then
        head -40 "$stem.sections.diff" >&2
        fail "$source ($target, $profile): the section set differs from the assembler"
      fi

      if [ "$byte_exact" = "yes" ]; then
        for section in $(section_names "$readelf" "$stem.ours.o"); do
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

      if ! diff -u \
        <(symbol_sections "$objdump" "$stem.ours.o") \
        <(symbol_sections "$objdump" "$stem.gas.o") >"$stem.symsec.diff"; then
        head -40 "$stem.symsec.diff" >&2
        fail "$source ($target, $profile): a symbol sits in a different section"
      fi

      checked=$((checked + 1))
    done
  done
  echo "$checked"
}

# ----- x86-64, natively ------------------------------------------------------

if ! command -v as >/dev/null || ! command -v objdump >/dev/null; then
  skip "no host binutils; skipped"
  exit 0
fi

host_checked="$(check_target "$host_target" objdump readelf as no)"
echo "object-check: $host_checked x86-64 objects match the assembler ... ok"

# Behaviour, which is the only claim that ultimately matters.
host_run=0
for source in "${programs[@]}"; do
  "$slopic" --emit exe -o "$scratch/ours.out" "$source" >/dev/null
  SLOPIUM_OBJECT_WRITER=external "$slopic" --emit exe -o "$scratch/gas.out" "$source" >/dev/null
  # Through files rather than `$(...)`: a program is allowed to print a NUL
  # (`D-079`), and command substitution drops one — quietly weakening the
  # comparison for exactly the byte a payload is most likely to carry.
  "$scratch/ours.out" </dev/null >"$scratch/ours.run" 2>&1
  echo "exit=$?" >>"$scratch/ours.run"
  "$scratch/gas.out" </dev/null >"$scratch/gas.run" 2>&1
  echo "exit=$?" >>"$scratch/gas.run"
  if ! cmp --silent "$scratch/ours.run" "$scratch/gas.run"; then
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
  skip "no aarch64 toolchain; cross checks skipped"
  exit 0
fi

cross_checked="$(check_target "$cross_target" "$cross_objdump" "$cross_readelf" "$cross_as" yes)"
echo "object-check: $cross_checked aarch64 objects are byte-identical to the assembler ... ok"

if ! command -v "$qemu" >/dev/null; then
  skip "no emulator; aarch64 programs not run"
  exit 0
fi

cross_run=0
for source in "${programs[@]}"; do
  "$slopic" --emit exe --cc "$cross_cc" --target "$cross_target" \
    -o "$scratch/ours.out" "$source" >/dev/null
  SLOPIUM_OBJECT_WRITER=external "$slopic" --emit exe --cc "$cross_cc" \
    --target "$cross_target" -o "$scratch/gas.out" "$source" >/dev/null
  "$qemu" "$scratch/ours.out" </dev/null >"$scratch/ours.run" 2>&1
  echo "exit=$?" >>"$scratch/ours.run"
  "$qemu" "$scratch/gas.out" </dev/null >"$scratch/gas.run" 2>&1
  echo "exit=$?" >>"$scratch/gas.run"
  if ! cmp --silent "$scratch/ours.run" "$scratch/gas.run"; then
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
