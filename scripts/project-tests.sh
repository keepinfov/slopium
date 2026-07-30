#!/usr/bin/env bash
set -euo pipefail

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
  echo "project-tests: no aarch64 toolchain; cross-target checks skipped" >&2
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
assert_patterns <(printf '%s\n' '"protocol": 6') "$result_dir/compiler.stdout"

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
echo "project-tests: workspaces ... ok"

basic_project="$projects_dir/pass/basics"
basic_source="$basic_project/src/main.slp"
emit_dir="$result_dir/emits"
mkdir -p "$emit_dir"
"$compiler" "$basic_source" --source-root "$basic_project/src" --emit check
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit hir --output "$emit_dir/basics.hir.json"
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit mir --output "$emit_dir/basics.mir.json"
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit mir-text --output "$emit_dir/basics.mir.txt"
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit asm --output "$emit_dir/basics-assembly.s"
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit obj --output "$emit_dir/basics-object.o"
"$compiler" "$basic_source" --source-root "$basic_project/src" \
  --emit exe --optimize --strip --output "$emit_dir/basics"
"$emit_dir/basics" >"$emit_dir/basics.stdout"
if ! cmp --silent "$basic_project/expected.stdout" "$emit_dir/basics.stdout"; then
  echo "project-tests: direct slopic executable output mismatch" >&2
  diff -u "$basic_project/expected.stdout" "$emit_dir/basics.stdout" >&2 || true
  exit 1
fi
"$compiler" "$basic_source" --source-root "$basic_project/src" \
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
  # And nothing a program never calls is dragged along: `basics` touches no
  # list, so the linker must have dropped the list runtime.
  if readelf -sW "$basic_project/target/$host_target/dev/basics" 2>/dev/null |
    grep -q 'sl_rt_list_'; then
    echo "project-tests: an unused runtime helper survived --gc-sections" >&2
    exit 1
  fi
  echo "project-tests: release binaries are stripped and unused helpers dropped ... ok"
else
  echo "project-tests: readelf not found; debug-section check skipped" >&2
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
else
  echo "project-tests: readelf not found; per-target build checks skipped" >&2
fi

set +e
"$compiler" "$projects_dir/compile-fail/ownership-move/src/main.slp" \
  --source-root "$projects_dir/compile-fail/ownership-move/src" \
  --emit check --diagnostic-format json \
  >"$emit_dir/diagnostic.stdout" 2>"$emit_dir/diagnostic.jsonl"
diagnostic_status=$?
set -e
if [[ "$diagnostic_status" -eq 0 ]]; then
  echo "project-tests: JSON diagnostic fixture unexpectedly passed" >&2
  exit 1
fi
assert_patterns <(printf '%s\n' '"code":"SL0300"') "$emit_dir/diagnostic.jsonl"
echo "project-tests: manager and slopic command surfaces ... ok"

pass_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
  prefix="$result_dir/pass-$name"
  args=()
  project_arguments "$project" args

  run_manager_logged "pass/$name clean" "$prefix.clean.stdout" "$prefix.clean.stderr" \
    "$manifest" clean
  run_manager_logged "pass/$name fmt" "$prefix.fmt.stdout" "$prefix.fmt.stderr" \
    "$manifest" fmt --check
  run_manager_logged "pass/$name check" "$prefix.check.stdout" "$prefix.check.stderr" \
    "$manifest" check

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

dependency_count=0
while IFS= read -r -d '' manifest; do
  run_manager "$manifest" fmt --check >/dev/null
  dependency_count=$((dependency_count + 1))
done < <(find "$projects_dir/dependencies" -name Slopium.toml -print0 | sort -z)

compile_fail_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
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

  echo "project-tests: compile-fail/$name ... ok"
  compile_fail_count=$((compile_fail_count + 1))
done < <(
  find "$projects_dir/compile-fail" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z
)

runtime_fail_count=0
while IFS= read -r -d '' project; do
  name="$(basename "$project")"
  manifest="$project/Slopium.toml"
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

echo "project-tests: $pass_count pass, $compile_fail_count compile-fail, $runtime_fail_count runtime-fail, $dependency_count dependency fixtures"
