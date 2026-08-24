#!/usr/bin/env bash
set -euo pipefail

# `SLOPIUM_STRICT=1` turns a skip into a failure. A machine that quietly lacks a
# tool otherwise reports a green check that verified nothing.
skip() {
  echo "project-tests: $1" >&2
  if [ -n "${SLOPIUM_STRICT:-}" ]; then
    echo "project-tests: SLOPIUM_STRICT is set; a skipped check is a failed one" >&2
    exit 1
  fi
}

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
projects_dir="$workspace_dir/tests/projects"
result_dir="$(mktemp -d)"
trap 'rm -rf "$result_dir"' EXIT

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"

compiler="$workspace_dir/target/debug/slopic"
manager="$workspace_dir/target/debug/slopium"
host_target="x86_64-unknown-linux-gnu"
cross_target="aarch64-unknown-linux-gnu"
cross_cc="${SLOPIUM_CC_AARCH64_UNKNOWN_LINUX_GNU:-aarch64-unknown-linux-gnu-cc}"
qemu="${SLOPIUM_QEMU_AARCH64:-qemu-aarch64}"

# Every target the manager can build for here. The cross target joins the list
# only when its toolchain and emulator are present, which is what keeps this
# suite runnable outside `nix develop`.
targets=("$host_target")
if command -v "$cross_cc" >/dev/null 2>&1 && command -v "$qemu" >/dev/null 2>&1; then
  targets+=("$cross_target")
else
  skip "no aarch64 toolchain; cross-target checks skipped"
fi

run_manager() {
  local manifest="$1"
  shift
  env SLOPIC="$compiler" "$manager" --manifest-path "$manifest" "$@"
}

run_manager_logged() {
  local label="$1"
  local stdout="$2"
  local stderr="$3"
  local manifest="$4"
  shift 4
  if ! run_manager "$manifest" "$@" >"$stdout" 2>"$stderr"; then
    echo "project-tests: $label failed" >&2
    sed -n '1,160p' "$stdout" >&2
    sed -n '1,240p' "$stderr" >&2
    return 1
  fi
}

assert_patterns() {
  local expected="$1"
  local actual="$2"
  while IFS= read -r pattern || [[ -n "$pattern" ]]; do
    if [[ -z "$pattern" ]]; then
      continue
    fi
    if ! grep -F --quiet -- "$pattern" "$actual"; then
      echo "project-tests: missing expected text: $pattern" >&2
      echo "project-tests: actual output:" >&2
      sed -n '1,240p' "$actual" >&2
      return 1
    fi
  done <"$expected"
}

environment_command() {
  local project="$1"
  shift
  local -a command=(env)
  local value

  if [[ -f "$project/run.unset-env" ]]; then
    while IFS= read -r value || [[ -n "$value" ]]; do
      if [[ -n "$value" ]]; then
        command+=(-u "$value")
      fi
    done <"$project/run.unset-env"
  fi

  command+=("SLOPIC=$compiler")
  if [[ -f "$project/run.env" ]]; then
    while IFS= read -r value || [[ -n "$value" ]]; do
      if [[ -n "$value" ]]; then
        command+=("$value")
      fi
    done <"$project/run.env"
  fi

  command+=("$@")
  if [[ -f "$project/run.stdin" ]]; then
    "${command[@]}" <"$project/run.stdin"
  else
    "${command[@]}" </dev/null
  fi
}

project_arguments() {
  local project="$1"
  local -n output="$2"
  if [[ -f "$project/run.args" ]]; then
    mapfile -t output <"$project/run.args"
  else
    output=()
  fi
}

env SLOPIC="$compiler" "$manager" targets >"$result_dir/targets.stdout"
assert_patterns \
  <(printf '%s\n' "$host_target (installed, default)" "$cross_target (installed)") \
  "$result_dir/targets.stdout"
env SLOPIC="$compiler" "$manager" compiler >"$result_dir/compiler.stdout"
assert_patterns <(printf '%s\n' '"protocol": 8') "$result_dir/compiler.stdout"

generated_project="$result_dir/generated-project"
env SLOPIC="$compiler" "$manager" new generated-project --path "$generated_project" \
  >"$result_dir/new.stdout"
run_manager "$generated_project/Slopium.toml" check >"$result_dir/new-check.stdout"
run_manager "$generated_project/Slopium.toml" run >"$result_dir/new-run.stdout"
run_manager "$generated_project/Slopium.toml" clean >"$result_dir/new-clean.stdout"
if [[ -e "$generated_project/target" ]]; then
  echo "project-tests: clean left the generated target directory behind" >&2
  exit 1
fi

format_project="$result_dir/format-project"
cp -R "$projects_dir/pass/basics" "$format_project"
run_manager "$format_project/Slopium.toml" clean >/dev/null
sed -i 's/(+ 20 22)/(+   20   22)/' "$format_project/src/main.slp"
format_before="$(sha256sum "$format_project/src/main.slp" | cut -d ' ' -f 1)"
run_manager "$format_project/Slopium.toml" fmt >"$result_dir/format.stdout"
format_after="$(sha256sum "$format_project/src/main.slp" | cut -d ' ' -f 1)"
if [[ "$format_before" == "$format_after" ]]; then
  echo "project-tests: fmt did not rewrite the unformatted project" >&2
  exit 1
fi
run_manager "$format_project/Slopium.toml" fmt --check >/dev/null

# `slopium fix` and the v0.5.1 move: a program that called `println` and
# `args-len` as builtins gains the `take` declarations that make it mean the
# same thing today. The fixture is committed in today's canonical format,
# missing only its imports, and `expected.slp` is the file `fix` must produce
# byte for byte — which is what keeps the comments in evidence. The mended
# program then satisfies `fmt --check`, builds and runs, and a second `fix`
# writes nothing, which is the idempotence half of the contract.
fix_project="$result_dir/fix-project"
cp -R "$projects_dir/fix/pre-move" "$fix_project"
if run_manager "$fix_project/Slopium.toml" fix --check \
  >"$result_dir/fix-check.stdout" 2>"$result_dir/fix-check.stderr"; then
  echo "project-tests: fix --check reported nothing for the pre-move program" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' 'spelling differs') "$result_dir/fix-check.stderr"
run_manager "$fix_project/Slopium.toml" fix >"$result_dir/fix.stdout"
assert_patterns <(printf '%s\n' 'Fixed ') "$result_dir/fix.stdout"
if ! cmp --silent "$projects_dir/fix/pre-move/expected.slp" "$fix_project/src/main.slp"; then
  echo "project-tests: fix did not produce the expected rewrite" >&2
  diff -u "$projects_dir/fix/pre-move/expected.slp" "$fix_project/src/main.slp" >&2 || true
  exit 1
fi
run_manager "$fix_project/Slopium.toml" fmt --check >/dev/null
run_manager "$fix_project/Slopium.toml" run >"$result_dir/fix-run.stdout" 2>/dev/null
sed '/^\(Compiling\|Fresh\|Finished\) /d' "$result_dir/fix-run.stdout" \
  >"$result_dir/fix-program.stdout"
if ! cmp --silent "$projects_dir/fix/pre-move/expected.stdout" "$result_dir/fix-program.stdout"; then
  echo "project-tests: stdout mismatch for the fixed program" >&2
  diff -u "$projects_dir/fix/pre-move/expected.stdout" "$result_dir/fix-program.stdout" >&2 || true
  exit 1
fi
run_manager "$fix_project/Slopium.toml" fix --check >/dev/null
fix_before="$(sha256sum "$fix_project/src/main.slp" | cut -d ' ' -f 1)"
run_manager "$fix_project/Slopium.toml" fix >/dev/null
fix_after="$(sha256sum "$fix_project/src/main.slp" | cut -d ' ' -f 1)"
if [[ "$fix_before" != "$fix_after" ]]; then
  echo "project-tests: a second fix rewrote a mended program" >&2
  exit 1
fi
echo "project-tests: fix rewrites the v0.5.1 move ... ok"

# Lockfile and dependency graph. The diamond fixture is the interesting one:
# `foundation` is reached through both `mathlib` and `geometry` and must appear
# once in the lock and once in the build.
# Run in place: the fixture's dependencies are relative paths, so a copy
# elsewhere would not resolve. The lock is a build product and gitignored.
lock_project="$projects_dir/pass/diamond-dependencies"
run_manager "$lock_project/Slopium.toml" clean >/dev/null
rm -f "$lock_project/Slopium.lock"
run_manager "$lock_project/Slopium.toml" check >/dev/null
if [[ ! -f "$lock_project/Slopium.lock" ]]; then
  echo "project-tests: check did not write Slopium.lock" >&2
  exit 1
fi
lock_first="$(sha256sum "$lock_project/Slopium.lock" | cut -d ' ' -f 1)"
assert_patterns <(printf '%s\n' 'name = "foundation"' 'source = "path+../../dependencies/foundation"') \
  "$lock_project/Slopium.lock"
if [[ "$(grep -c 'name = "foundation"' "$lock_project/Slopium.lock")" != "1" ]]; then
  echo "project-tests: the shared dependency appears more than once in the lock" >&2
  exit 1
fi
# Re-resolving is a no-op, and --locked accepts an up-to-date lock.
run_manager "$lock_project/Slopium.toml" check --locked >/dev/null
lock_second="$(sha256sum "$lock_project/Slopium.lock" | cut -d ' ' -f 1)"
if [[ "$lock_first" != "$lock_second" ]]; then
  echo "project-tests: resolution rewrote an up-to-date lock" >&2
  exit 1
fi
# --locked refuses to update a stale lock. Renaming a package keeps the file
# readable and makes it disagree with what resolution found, which is the
# situation --locked exists for.
sed -i 's/name = "foundation"/name = "phantom"/' "$lock_project/Slopium.lock"
if run_manager "$lock_project/Slopium.toml" check --locked \
  >"$result_dir/locked.stdout" 2>"$result_dir/locked.stderr"; then
  echo "project-tests: --locked accepted a stale lock" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' 'out of date') "$result_dir/locked.stderr"
rm -f "$lock_project/Slopium.lock"
run_manager_logged "tree" "$result_dir/tree.stdout" "$result_dir/tree.stderr" \
  "$lock_project/Slopium.toml" tree
assert_patterns <(printf '%s\n' 'diamond-dependencies v0.2.4' 'geometry v0.2.4' 'mathlib v0.2.4' 'foundation v0.2.4') \
  "$result_dir/tree.stdout"
echo "project-tests: lockfile and tree ... ok"

# Workspaces: one lock and one target directory at the root, package selection,
# and a member reached as a path dependency resolved as that member.
workspace_project="$projects_dir/pass/workspace"
run_manager "$workspace_project/Slopium.toml" clean >/dev/null
rm -f "$workspace_project/Slopium.lock" "$workspace_project/helper/Slopium.lock"
run_manager_logged "workspace fmt" "$result_dir/ws-fmt.stdout" "$result_dir/ws-fmt.stderr" \
  "$workspace_project/Slopium.toml" fmt --check --workspace
run_manager_logged "workspace check" "$result_dir/ws-check.stdout" "$result_dir/ws-check.stderr" \
  "$workspace_project/Slopium.toml" check --workspace
assert_patterns <(printf '%s\n' 'Checked helper v0.2.4' 'Checked workspace v0.2.4') \
  "$result_dir/ws-check.stdout"
if [[ ! -f "$workspace_project/Slopium.lock" ]]; then
  echo "project-tests: the workspace root has no lock" >&2
  exit 1
fi
if [[ -f "$workspace_project/helper/Slopium.lock" ]]; then
  echo "project-tests: a workspace member has its own lock" >&2
  exit 1
fi
if [[ "$(grep -c 'name = "foundation"' "$workspace_project/Slopium.lock")" != "1" ]]; then
  echo "project-tests: the shared dependency appears more than once in the workspace lock" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' 'name = "helper"' 'source = "path+helper"') \
  "$workspace_project/Slopium.lock"
# `-p` selects one member, and a member's version came from the workspace.
run_manager_logged "workspace -p" "$result_dir/ws-package.stdout" \
  "$result_dir/ws-package.stderr" "$workspace_project/Slopium.toml" check -p helper
assert_patterns <(printf '%s\n' 'Checked helper v0.2.4') "$result_dir/ws-package.stdout"
if grep --quiet 'workspace v0.2.4' "$result_dir/ws-package.stdout"; then
  echo "project-tests: --package checked more than the named member" >&2
  exit 1
fi
# Each member runs its own tests, and only its own.
run_manager_logged "workspace test" "$result_dir/ws-test.stdout" "$result_dir/ws-test.stderr" \
  "$workspace_project/Slopium.toml" test --workspace
assert_patterns <(printf '%s\n' 'test lib:a library member has its own tests' \
  'test main:a member of the same workspace is an ordinary dependency') \
  "$result_dir/ws-test.stdout"
if [[ "$(grep -c 'test lib:a library member has its own tests' "$result_dir/ws-test.stdout")" != "1" ]]; then
  echo "project-tests: a member's tests ran from another member's binary" >&2
  exit 1
fi
# One target directory, at the root.
if [[ -d "$workspace_project/helper/target" ]]; then
  echo "project-tests: a workspace member has its own target directory" >&2
  exit 1
fi
if [[ ! -x "$workspace_project/target/$host_target/dev/helper-tests" ]]; then
  echo "project-tests: the member's test binary is not under the workspace target directory" >&2
  exit 1
fi
run_manager "$workspace_project/Slopium.toml" clean >/dev/null
rm -f "$workspace_project/Slopium.lock"

# A root that defines no package of its own: selection is mandatory, `exclude`
# keeps a directory out, and `members` accepts a trailing `*`.
virtual_project="$projects_dir/workspaces/virtual-root"
run_manager "$virtual_project/Slopium.toml" clean >/dev/null
rm -f "$virtual_project/Slopium.lock"
if run_manager "$virtual_project/Slopium.toml" check \
  >"$result_dir/virtual.stdout" 2>"$result_dir/virtual.stderr"; then
  echo "project-tests: a virtual workspace root built without a package selection" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' '--package' '--workspace') "$result_dir/virtual.stderr"
run_manager_logged "virtual workspace" "$result_dir/virtual-check.stdout" \
  "$result_dir/virtual-check.stderr" "$virtual_project/Slopium.toml" check --workspace
assert_patterns <(printf '%s\n' 'Checked alpha v0.2.4' 'Checked beta v0.2.4') \
  "$result_dir/virtual-check.stdout"
if grep --quiet 'scratch' "$virtual_project/Slopium.lock"; then
  echo "project-tests: an excluded directory is in the workspace lock" >&2
  exit 1
fi
run_manager_logged "virtual run" "$result_dir/virtual-run.stdout" \
  "$result_dir/virtual-run.stderr" "$virtual_project/Slopium.toml" run -p alpha
assert_patterns <(printf '%s\n' '120') "$result_dir/virtual-run.stdout"
run_manager "$virtual_project/Slopium.toml" clean >/dev/null
rm -f "$virtual_project/Slopium.lock"

# `new --lib` produces a package that checks and tests but has nothing to run.
library_project="$result_dir/new-library"
run_manager "$projects_dir/pass/basics/Slopium.toml" new library-package \
  --lib --path "$library_project" >/dev/null
run_manager_logged "new --lib" "$result_dir/new-lib.stdout" "$result_dir/new-lib.stderr" \
  "$library_project/Slopium.toml" test
assert_patterns <(printf '%s\n' 'test lib:addition ... ok') "$result_dir/new-lib.stdout"
if run_manager "$library_project/Slopium.toml" run \
  >"$result_dir/new-lib-run.stdout" 2>"$result_dir/new-lib-run.stderr"; then
  echo "project-tests: a library package produced something to run" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' 'is a library') "$result_dir/new-lib-run.stderr"

# `D-072`: a package that declares no `entry` at all is entered through
# `<source>/lib.slp`. `D-046` always allowed omitting it, but only resolution
# exercised that — checking and building asked for an entry and refused.
implicit_project="$result_dir/implicit-library"
mkdir -p "$implicit_project/src"
cat >"$implicit_project/Slopium.toml" <<'EOF'
[package]
name = "implicit-library"
version = "1.0.0"
source = "src"
EOF
cat >"$implicit_project/src/lib.slp" <<'EOF'
(export add)

(fn add ((left i64) (right i64)) -> i64
  (+ left right))

(test "an implicit entry"
  (= (add 20 22) 42))
EOF
run_manager_logged "implicit entry" "$result_dir/implicit.stdout" \
  "$result_dir/implicit.stderr" "$implicit_project/Slopium.toml" test
assert_patterns <(printf '%s\n' 'test lib:an implicit entry ... ok') \
  "$result_dir/implicit.stdout"

# And when there is no such file, the message names the file it looked for
# rather than the field that was not written.
rm "$implicit_project/src/lib.slp"
printf '(export other)\n\n(fn other () -> i64 1)\n' >"$implicit_project/src/other.slp"
if run_manager "$implicit_project/Slopium.toml" check \
  >"$result_dir/implicit-missing.stdout" 2>"$result_dir/implicit-missing.stderr"; then
  echo "project-tests: a package with no entry and no lib.slp was accepted" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' 'SL1053' 'lib.slp') "$result_dir/implicit-missing.stderr"
echo "project-tests: workspaces ... ok"

basic_project="$projects_dir/pass/basics"
basic_source="$basic_project/src/main.slp"
emit_dir="$result_dir/emits"
mkdir -p "$emit_dir"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std --emit check
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit hir --output "$emit_dir/basics.hir.json"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit mir --output "$emit_dir/basics.mir.json"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit mir-text --output "$emit_dir/basics.mir.txt"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit asm --output "$emit_dir/basics-assembly.s"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit obj --output "$emit_dir/basics-object.o"
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit exe --optimize --strip --output "$emit_dir/basics"
"$emit_dir/basics" >"$emit_dir/basics.stdout"
if ! cmp --silent "$basic_project/expected.stdout" "$emit_dir/basics.stdout"; then
  echo "project-tests: direct slopic executable output mismatch" >&2
  diff -u "$basic_project/expected.stdout" "$emit_dir/basics.stdout" >&2 || true
  exit 1
fi
"$compiler" "$basic_source" --source-root "$basic_project/src" --toolchain-dependency std \
  --emit exe --test --output "$emit_dir/basics-tests"
"$emit_dir/basics-tests" >"$emit_dir/basics-tests.stdout"
assert_patterns <(printf '%s\n' ' ... ok') "$emit_dir/basics-tests.stdout"
for output in "$emit_dir/basics.hir.json" "$emit_dir/basics.mir.json" \
  "$emit_dir/basics.mir.txt" "$emit_dir/basics-assembly.s" \
  "$emit_dir/basics-object.o"; do
  if [[ ! -s "$output" ]]; then
    echo "project-tests: slopic produced an empty $(basename "$output")" >&2
    exit 1
  fi
done
# The readable dump must actually contain blocks and source locations, not
# just be non-empty.
assert_patterns <(printf '%s\n' 'bb0:' 'return' '// ') "$emit_dir/basics.mir.txt"

# An `inline` annotation has to change what `--emit mir` shows, or it is a word
# with no effect (`D-122`). `blend` is over the size the optimizer copies on its
# own, so the call in `mixed` is there without the hint and gone with it.
annotations_project="$projects_dir/pass/annotations"
"$compiler" "$annotations_project/src/main.slp" \
  --source-root "$annotations_project/src" --toolchain-dependency std \
  --optimize --emit mir-text --output "$emit_dir/annotations.mir.txt" \
  2>"$emit_dir/annotations.mir.stderr"
assert_patterns <(printf '%s\n' '#inline fn main:blend') "$emit_dir/annotations.mir.txt"
if sed -n '/^fn main:mixed/,/^}/p' "$emit_dir/annotations.mir.txt" | grep --quiet 'blend('; then
  echo "project-tests: the inline annotation did not reach the optimizer" >&2
  sed -n '/^fn main:mixed/,/^}/p' "$emit_dir/annotations.mir.txt" >&2
  exit 1
fi
# The same run is where a warning reaches a person: `slopic` prints it and
# still exits 0.
assert_patterns "$annotations_project/expected.stderr" "$emit_dir/annotations.mir.stderr"

# The manager turns debug information on for `dev` and off for `release`, which
# is what makes a plain `slopium build` debuggable without asking for anything.
# The linked section is the check rather than a `.loc` directive, because that
# is what survives assembly and linking.
if command -v readelf >/dev/null 2>&1; then
  run_manager_logged "dev debug build" "$emit_dir/debug-dev.stdout" \
    "$emit_dir/debug-dev.stderr" "$basic_project/Slopium.toml" build
  run_manager_logged "release debug build" "$emit_dir/debug-release.stdout" \
    "$emit_dir/debug-release.stderr" "$basic_project/Slopium.toml" build --release
  if ! readelf -S "$basic_project/target/$host_target/dev/basics" |
    grep -q '\.debug_line'; then
    echo "project-tests: a dev build carries no line table" >&2
    exit 1
  fi
  if readelf -S "$basic_project/target/$host_target/release/basics" |
    grep -q '\.debug_line'; then
    echo "project-tests: a release build carries a line table" >&2
    exit 1
  fi
  echo "project-tests: dev builds are debuggable and release builds are not ... ok"

  # A release build keeps no symbol table: the mangled `sl_fn_*` and runtime
  # names are stripped, both for size and so the binary is not trivially read.
  # A dev build keeps them, because a debugger needs them and stripping would
  # take the line table with it.
  if readelf -sW "$basic_project/target/$host_target/release/basics" 2>/dev/null |
    grep -qE 'sl_(fn|rt|test|drop|clone)_'; then
    echo "project-tests: a release build still carries its internal symbols" >&2
    exit 1
  fi
  if ! readelf -sW "$basic_project/target/$host_target/dev/basics" 2>/dev/null |
    grep -qE 'sl_fn_'; then
    echo "project-tests: a dev build lost the symbols a debugger needs" >&2
    exit 1
  fi
  # And nothing a program never calls is dragged along: `basics` makes no
  # slice, so the linker must have dropped the slice runtime.
  if readelf -sW "$basic_project/target/$host_target/dev/basics" 2>/dev/null |
    grep -q 'sl_rt_slice_'; then
    echo "project-tests: an unused runtime helper survived --gc-sections" >&2
    exit 1
  fi
  echo "project-tests: release binaries are stripped and unused helpers dropped ... ok"

  # The same is now true of a Slopium function, which is what a function owning
  # its own section bought (`D-030`). `basics` prints no float, so nothing it
  # calls reaches `core:float:assemble`.
  #
  # The object is checked first and on purpose. A module is emitted whole —
  # `emit` is per module, not per function — so the symbol really is there to
  # be dropped, and without that half this check would keep passing the day
  # something starts pruning earlier, for a reason that has nothing to do with
  # the linker. The mangling is spelled out rather than pasted, so a change to
  # it fails here instead of quietly looking for a symbol nobody emits.
  unused_symbol="sl_fn_$(printf 'core:float:assemble' | od -An -tx1 | tr -d ' \n')"
  unused_object="$basic_project/target/$host_target/dev/objects/basics/$(printf 'core:float' | od -An -tx1 | tr -d ' \n').o"
  if ! nm "$unused_object" 2>/dev/null | grep -q "$unused_symbol"; then
    echo "project-tests: $unused_symbol is not in its object, so dropping it proves nothing" >&2
    exit 1
  fi
  if readelf -sW "$basic_project/target/$host_target/dev/basics" 2>/dev/null |
    grep -q "$unused_symbol"; then
    echo "project-tests: an uncalled Slopium function survived --gc-sections" >&2
    exit 1
  fi
  echo "project-tests: an uncalled Slopium function is dropped at link time ... ok"
else
  skip "readelf not found; debug-section check skipped"
fi

# The manager has to place, name, and drive a build for every target it lists,
# not only the host: the artifact directory carries the triple, and the `cc` for
# a cross target comes from a different environment variable than the host's.
# `readelf` is what proves the object really is for the architecture asked for —
# a host `cc` handed aarch64 assembly fails loudly, but a misrouted target would
# otherwise produce a working host binary and look like a pass.
if command -v readelf >/dev/null 2>&1; then
  for target in "${targets[@]}"; do
    target_project="$result_dir/target-$target"
    cp -R "$projects_dir/pass/basics" "$target_project"
    run_manager_logged "build for $target" "$result_dir/target-$target.stdout" \
      "$result_dir/target-$target.stderr" "$target_project/Slopium.toml" \
      build --target "$target"
    artifact="$target_project/target/$target/dev/basics"
    if [[ ! -x "$artifact" ]]; then
      echo "project-tests: $target build produced no artifact at $artifact" >&2
      exit 1
    fi
    case "$target" in
      "$host_target") machine="X86-64" ;;
      "$cross_target") machine="AArch64" ;;
    esac
    if ! readelf -h "$artifact" | grep -q "Machine:.*$machine"; then
      echo "project-tests: $target build is not a $machine binary" >&2
      readelf -h "$artifact" >&2
      exit 1
    fi
    if [[ "$target" == "$host_target" ]]; then
      "$artifact" >"$result_dir/target-$target.run"
    else
      "$qemu" "$artifact" >"$result_dir/target-$target.run"
    fi
    if ! cmp --silent "$projects_dir/pass/basics/expected.stdout" \
      "$result_dir/target-$target.run"; then
      echo "project-tests: $target build produced the wrong output" >&2
      diff -u "$projects_dir/pass/basics/expected.stdout" \
        "$result_dir/target-$target.run" >&2 || true
      exit 1
    fi
    echo "project-tests: manager builds and runs for $target ... ok"
  done

  # The same `(take arch ...)` has to reach a different file for each target,
  # which is the whole claim of `[target."<triple>"] modules` (`D-135`). A
  # fixture whose output did not depend on the target could not prove it.
  for target in "${targets[@]}"; do
    modules_project="$result_dir/modules-$target"
    cp -R "$projects_dir/pass/target-modules" "$modules_project"
    run_manager_logged "target modules for $target" \
      "$result_dir/modules-$target.stdout" "$result_dir/modules-$target.stderr" \
      "$modules_project/Slopium.toml" build --target "$target"
    artifact="$modules_project/target/$target/dev/target-modules"
    if [[ "$target" == "$host_target" ]]; then
      "$artifact" >"$result_dir/modules-$target.run"
    else
      "$qemu" "$artifact" >"$result_dir/modules-$target.run"
    fi
    case "$target" in
      "$host_target") expected_architecture="x86-64" ;;
      "$cross_target") expected_architecture="aarch64" ;;
    esac
    if [[ "$(head -n 1 "$result_dir/modules-$target.run")" != "$expected_architecture" ]]; then
      echo "project-tests: $target selected the wrong module for \`arch\`" >&2
      cat "$result_dir/modules-$target.run" >&2
      exit 1
    fi
    echo "project-tests: a module named per target, built for $target ... ok"
  done

  # A module named for a target this toolchain cannot build for is never
  # compiled, so a file that would not typecheck anywhere else costs nothing.
  if grep -q 'riscv32-unknown-none' "$projects_dir/pass/target-modules/Slopium.toml"; then
    echo "project-tests: a module for an unbuildable target is not compiled ... ok"
  fi

  # The same source, built for each target, has to answer differently — which is
  # the whole claim of `(target "...")` (`D-136`). `basics` above proves the
  # manager can drive every target; this proves the *program* changed with it,
  # and a fixture whose output did not depend on the target could not.
  for target in "${targets[@]}"; do
    selection_project="$result_dir/selection-$target"
    cp -R "$projects_dir/pass/target-selection" "$selection_project"
    run_manager_logged "target selection for $target" \
      "$result_dir/selection-$target.stdout" "$result_dir/selection-$target.stderr" \
      "$selection_project/Slopium.toml" build --target "$target"
    artifact="$selection_project/target/$target/dev/target-selection"
    if [[ "$target" == "$host_target" ]]; then
      "$artifact" >"$result_dir/selection-$target.run"
    else
      "$qemu" "$artifact" >"$result_dir/selection-$target.run"
    fi
    case "$target" in
      "$host_target") expected_architecture="x86-64" ;;
      "$cross_target") expected_architecture="aarch64" ;;
    esac
    if [[ "$(head -n 1 "$result_dir/selection-$target.run")" != "$expected_architecture" ]]; then
      echo "project-tests: $target selected the wrong declaration" >&2
      cat "$result_dir/selection-$target.run" >&2
      exit 1
    fi
    echo "project-tests: a declaration selected by target, built for $target ... ok"
  done
else
  skip "readelf not found; per-target build checks skipped"
fi

set +e
"$compiler" "$projects_dir/compile-fail/ownership-move/src/main.slp" \
  --source-root "$projects_dir/compile-fail/ownership-move/src" \
  --toolchain-dependency std --emit check --diagnostic-format json \
  >"$emit_dir/diagnostic.stdout" 2>"$emit_dir/diagnostic.jsonl"
diagnostic_status=$?
set -e
if [[ "$diagnostic_status" -eq 0 ]]; then
  echo "project-tests: JSON diagnostic fixture unexpectedly passed" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' '"code":"SL0300"') "$emit_dir/diagnostic.jsonl"
echo "project-tests: manager and slopic command surfaces ... ok"

# A fixture directory with no manifest is almost always a *stranded* one: the
# fixture was added on another branch, and switching away left its gitignored
# `target/` and `Slopium.lock` behind. The walk then finds a directory and no
# package, and `slopium` reports a missing file, which says nothing about why.
require_manifest() {
  [ -f "$2" ] && return 0
  echo "project-tests: \`$1\` has no Slopium.toml" >&2
  echo "project-tests: this is usually a build directory left behind by a branch that added the fixture; \`rm -rf\` it, it is build output" >&2
  exit 1
}

pass_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
  require_manifest "$project" "$manifest"
  prefix="$result_dir/pass-$name"
  args=()
  project_arguments "$project" args

  run_manager_logged "pass/$name clean" "$prefix.clean.stdout" "$prefix.clean.stderr" \
    "$manifest" clean
  run_manager_logged "pass/$name fmt" "$prefix.fmt.stdout" "$prefix.fmt.stderr" \
    "$manifest" fmt --check
  run_manager_logged "pass/$name check" "$prefix.check.stdout" "$prefix.check.stderr" \
    "$manifest" check
  # A fixture whose point is what the compiler *says* about a program that
  # compiles: `clean` ran above, so nothing is cached and every warning is
  # printed exactly once.
  if [[ -f "$project/expected.stderr" ]]; then
    assert_patterns "$project/expected.stderr" "$prefix.check.stderr"
  fi

  run_command=("$manager" --manifest-path "$manifest" run)
  if ((${#args[@]} > 0)); then
    run_command+=(-- "${args[@]}")
  fi
  environment_command "$project" "${run_command[@]}" \
    >"$prefix.run.stdout" 2>"$prefix.run.stderr"
  sed '/^\(Compiling\|Fresh\|Finished\) /d' \
    "$prefix.run.stdout" >"$prefix.program.stdout"
  if ! cmp --silent "$project/expected.stdout" "$prefix.program.stdout"; then
    echo "project-tests: stdout mismatch for pass/$name" >&2
    diff -u "$project/expected.stdout" "$prefix.program.stdout" >&2 || true
    exit 1
  fi

  run_manager_logged "pass/$name test" "$prefix.test.stdout" "$prefix.test.stderr" \
    "$manifest" test

  if [[ -f "$project/release" ]]; then
    run_manager_logged "pass/$name release build" "$prefix.release-build.stdout" \
      "$prefix.release-build.stderr" "$manifest" build --release
    artifact="$project/target/$host_target/release/$name"
    environment_command "$project" "$artifact" "${args[@]}" \
      >"$prefix.release.stdout" 2>"$prefix.release.stderr"
    if ! cmp --silent "$project/expected.stdout" "$prefix.release.stdout"; then
      echo "project-tests: release stdout mismatch for pass/$name" >&2
      diff -u "$project/expected.stdout" "$prefix.release.stdout" >&2 || true
      exit 1
    fi
  fi

  echo "project-tests: pass/$name ... ok"
  pass_count=$((pass_count + 1))
done < <(find "$projects_dir/pass" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z)

cache_project="$result_dir/cache-project"
cp -R "$projects_dir/pass/modules" "$cache_project"
cache_manifest="$cache_project/Slopium.toml"
run_manager "$cache_manifest" clean >/dev/null
run_manager "$cache_manifest" build >/dev/null
cache_objects="$cache_project/target/$host_target/dev/objects/modules"
main_stamp="$cache_objects/6d61696e.slop-cache"
core_stamp="$cache_objects/6d6174683a636f7265.slop-cache"
main_before="$(sha256sum "$main_stamp" | cut -d ' ' -f 1)"
core_before="$(sha256sum "$core_stamp" | cut -d ' ' -f 1)"
sed -i 's/(+ left right))/(+ (+ left right) 0))/' "$cache_project/src/math/core.slp"
run_manager "$cache_manifest" build >/dev/null
main_after_body="$(sha256sum "$main_stamp" | cut -d ' ' -f 1)"
core_after_body="$(sha256sum "$core_stamp" | cut -d ' ' -f 1)"
if [[ "$main_before" != "$main_after_body" || "$core_before" == "$core_after_body" ]]; then
  echo "project-tests: body-only cache invalidation is incorrect" >&2
  exit 1
fi
sed -i 's/left/lhs/g' "$cache_project/src/math/core.slp"
run_manager "$cache_manifest" build >/dev/null
main_after_interface="$(sha256sum "$main_stamp" | cut -d ' ' -f 1)"
if [[ "$main_after_body" == "$main_after_interface" ]]; then
  echo "project-tests: interface change did not invalidate the consumer object" >&2
  exit 1
fi
echo "project-tests: module object cache ... ok"

# A `const` is inlined wherever it is used, so its value is interface: a
# dependent that is not rebuilt keeps the old number compiled into it. Until
# v0.9.2 the module interface did not mention constants at all, and a program
# went on printing the old value forever (`D-122`).
const_project="$result_dir/const-cache-project"
cp -R "$projects_dir/pass/annotations" "$const_project"
const_manifest="$const_project/Slopium.toml"
run_manager "$const_manifest" clean >/dev/null
run_manager "$const_manifest" run >"$result_dir/const-before.stdout" 2>/dev/null
if grep --quiet --line-regexp 55 "$result_dir/const-before.stdout"; then
  echo "project-tests: the const cache fixture already prints the new value" >&2
  exit 1
fi
sed -i 's/retry-limit 3)/retry-limit 55)/' "$const_project/src/legacy.slp"
run_manager "$const_manifest" run >"$result_dir/const-after.stdout" 2>/dev/null
if ! grep --quiet --line-regexp 55 "$result_dir/const-after.stdout"; then
  echo "project-tests: a changed const did not reach the module that uses it" >&2
  diff -u "$result_dir/const-before.stdout" "$result_dir/const-after.stdout" >&2 || true
  exit 1
fi
echo "project-tests: a changed const rebuilds its consumers ... ok"

dependency_count=0
while IFS= read -r -d '' manifest; do
  run_manager "$manifest" fmt --check >/dev/null
  dependency_count=$((dependency_count + 1))
done < <(find "$projects_dir/dependencies" -name Slopium.toml -print0 | sort -z)

compile_fail_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
  require_manifest "$project" "$manifest"
  prefix="$result_dir/compile-fail-$name"

  run_manager_logged "compile-fail/$name clean" "$prefix.clean.stdout" \
    "$prefix.clean.stderr" "$manifest" clean
  run_manager_logged "compile-fail/$name fmt" "$prefix.fmt.stdout" \
    "$prefix.fmt.stderr" "$manifest" fmt --check
  if run_manager "$manifest" check >"$prefix.check.stdout" 2>"$prefix.check.stderr"; then
    echo "project-tests: compile-fail/$name unexpectedly passed" >&2
    exit 1
  fi
  assert_patterns "$project/expected.stderr" "$prefix.check.stderr"

  # A build asks `slopic` for one object per module and each invocation checks
  # the whole program first, so a program that does not compile fails all of
  # them. What the summary line says about that is a fixture's business too, and
  # a project carrying `expected.build.stderr` asserts it (`D-154`).
  if [ -f "$project/expected.build.stderr" ]; then
    if run_manager "$manifest" build >"$prefix.build.stdout" 2>"$prefix.build.stderr"; then
      echo "project-tests: compile-fail/$name unexpectedly built" >&2
      exit 1
    fi
    assert_patterns "$project/expected.build.stderr" "$prefix.build.stderr"
  fi

  echo "project-tests: compile-fail/$name ... ok"
  compile_fail_count=$((compile_fail_count + 1))
done < <(
  find "$projects_dir/compile-fail" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z
)

runtime_fail_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
  require_manifest "$project" "$manifest"
  prefix="$result_dir/runtime-fail-$name"
  args=()
  project_arguments "$project" args

  run_manager_logged "runtime-fail/$name clean" "$prefix.clean.stdout" \
    "$prefix.clean.stderr" "$manifest" clean
  run_manager_logged "runtime-fail/$name fmt" "$prefix.fmt.stdout" \
    "$prefix.fmt.stderr" "$manifest" fmt --check
  run_manager_logged "runtime-fail/$name check" "$prefix.check.stdout" \
    "$prefix.check.stderr" "$manifest" check
  run_manager_logged "runtime-fail/$name build" "$prefix.build.stdout" \
    "$prefix.build.stderr" "$manifest" build
  run_manager_logged "runtime-fail/$name test" "$prefix.test.stdout" \
    "$prefix.test.stderr" "$manifest" test

  artifact="$project/target/$host_target/dev/$name"
  set +e
  environment_command "$project" "$artifact" "${args[@]}" \
    >"$prefix.run.stdout" 2>"$prefix.run.stderr"
  status=$?
  set -e
  if [[ "$status" -ne 101 ]]; then
    echo "project-tests: runtime-fail/$name exited $status instead of 101" >&2
    sed -n '1,240p' "$prefix.run.stderr" >&2
    exit 1
  fi
  assert_patterns "$project/expected.stderr" "$prefix.run.stderr"

  echo "project-tests: runtime-fail/$name ... ok"
  runtime_fail_count=$((runtime_fail_count + 1))
done < <(
  find "$projects_dir/runtime-fail" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z
)

# A test suite with a failure in it. Every other fixture asserts that the tests
# pass, which is the one case where the harness prints nothing beyond the
# verdict, so what a *failing* test says was held to nothing until `D-130`.
test_fail_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
  require_manifest "$project" "$manifest"
  prefix="$result_dir/test-fail-$name"

  run_manager_logged "test-fail/$name clean" "$prefix.clean.stdout" \
    "$prefix.clean.stderr" "$manifest" clean
  run_manager_logged "test-fail/$name fmt" "$prefix.fmt.stdout" \
    "$prefix.fmt.stderr" "$manifest" fmt --check
  if run_manager "$manifest" test >"$prefix.test.stdout" 2>"$prefix.test.stderr"; then
    echo "project-tests: test-fail/$name unexpectedly passed" >&2
    sed -n '1,160p' "$prefix.test.stdout" >&2
    exit 1
  fi
  assert_patterns "$project/expected.stdout" "$prefix.test.stdout"

  echo "project-tests: test-fail/$name ... ok"
  test_fail_count=$((test_fail_count + 1))
done < <(
  find "$projects_dir/test-fail" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z
)

# ---------------------------------------------------------------------------
# Freestanding: the manager links a program with no C library under it.
#
# These cannot live in `pass/`. There is no `std:io` in a freestanding program
# and so nothing to compare stdout against — the answer leaves through the exit
# status, the way `core-check.sh` argues it must, or through a serial port when
# there is no host to exit to — and `run` and `test` are not things this shape
# supports at all.
# ---------------------------------------------------------------------------

freestanding_target="x86_64-unknown-none"
freestanding_count=0

if ! command -v nm >/dev/null 2>&1 || ! command -v readelf >/dev/null 2>&1; then
  skip "no nm/readelf; freestanding checks skipped"
else
  while IFS= read -r -d '' project; do
    name="$(basename "$project")"
    manifest="$project/Slopium.toml"
    prefix="$result_dir/freestanding-$name"

    run_manager_logged "freestanding/$name clean" "$prefix.clean.stdout" \
      "$prefix.clean.stderr" "$manifest" clean
    run_manager_logged "freestanding/$name fmt" "$prefix.fmt.stdout" \
      "$prefix.fmt.stderr" "$manifest" fmt --check
    run_manager_logged "freestanding/$name check" "$prefix.check.stdout" \
      "$prefix.check.stderr" "$manifest" check
    run_manager_logged "freestanding/$name build" "$prefix.build.stdout" \
      "$prefix.build.stderr" "$manifest" build

    artifact="$project/target/$freestanding_target/dev/$name"
    if [[ ! -f "$artifact" ]]; then
      echo "project-tests: freestanding/$name built no $artifact" >&2
      exit 1
    fi

    # The claim `core-check.sh` makes about an object, made here about the
    # linked program: it owes the world nothing. The fixture keeps its symbol
    # table on purpose, so this is a statement about undefined symbols rather
    # than about an empty table.
    undefined="$(nm -u "$artifact")"
    if [[ -n "$undefined" ]]; then
      echo "project-tests: freestanding/$name left symbols undefined:" >&2
      echo "$undefined" >&2
      exit 1
    fi

    # That `-T` reached `cc`, observably: the fixture's script discards
    # `.comment`, which every default link keeps. Without the flag the section
    # is present and this fails, which is the point of asserting it rather than
    # trusting the command line.
    if readelf -S "$artifact" | grep -q '\.comment'; then
      echo "project-tests: freestanding/$name kept .comment; the linker script did not apply" >&2
      exit 1
    fi

    # A freestanding program that the host can still execute says its answer in
    # its exit status. A kernel cannot be executed here at all — it is entered
    # by a loader in 32-bit protected mode, and `scripts/kernel-check.sh` boots
    # it. Building and inspecting it here anyway is the point: the fixture stays
    # linked, `nm -u` clean and layout-checked on every run, including on a
    # machine with no emulator, rather than rotting behind a skip.
    if [[ -f "$project/expected.status" ]]; then
      set +e
      "$artifact"
      status=$?
      set -e
      expected="$(cat "$project/expected.status")"
      if [[ "$status" -ne "$expected" ]]; then
        echo "project-tests: freestanding/$name exited $status instead of $expected" >&2
        exit 1
      fi
    fi

    # A freestanding target has no harness, and saying so is the difference
    # between a refusal and a binary that runs no test in silence.
    if run_manager "$manifest" test >"$prefix.test.stdout" 2>"$prefix.test.stderr"; then
      echo "project-tests: freestanding/$name accepted a test it cannot run" >&2
      exit 1
    fi
    if ! grep -qF 'no test harness' "$prefix.test.stderr"; then
      echo "project-tests: freestanding/$name refused a test without saying why" >&2
      sed -n '1,40p' "$prefix.test.stderr" >&2
      exit 1
    fi

    run_manager_logged "freestanding/$name clean" "$prefix.clean2.stdout" \
      "$prefix.clean2.stderr" "$manifest" clean

    echo "project-tests: freestanding/$name ... ok"
    freestanding_count=$((freestanding_count + 1))
  done < <(
    find "$projects_dir/freestanding" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z
  )
fi

echo "project-tests: $pass_count pass, $compile_fail_count compile-fail, $runtime_fail_count runtime-fail, $test_fail_count test-fail, $freestanding_count freestanding, $dependency_count dependency fixtures"
