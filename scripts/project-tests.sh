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
assert_patterns <(printf '%s\n' '"protocol": 3') "$result_dir/compiler.stdout"

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
  --emit exe --profile release --output "$emit_dir/basics"
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
cache_objects="$cache_project/target/$host_target/dev/objects"
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
