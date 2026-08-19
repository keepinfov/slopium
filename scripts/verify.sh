#!/usr/bin/env bash
# Everything that has to hold before a change lands.
#
# `SLOPIUM_STRICT=1` refuses to skip: a check that cannot find valgrind, gdb,
# either qemu or the aarch64 toolchain fails instead of printing a line and
# passing.
# CI sets it, because a suite that goes green having verified nothing is the one
# failure a check suite cannot survive. A person on a laptop without the cross
# toolchain usually wants the skips, so it is off by default.
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_dir"

# The cheapest check in the suite, and the one whose failure invalidates a
# release rather than a build, so it runs before anything is compiled.
"$workspace_dir/scripts/release-check.sh" --check

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
"$workspace_dir/scripts/project-tests.sh"
"$workspace_dir/scripts/package-check.sh"
"$workspace_dir/scripts/git-check.sh"
"$workspace_dir/scripts/registry-check.sh"
"$workspace_dir/scripts/publish-check.sh"
"$workspace_dir/scripts/runtime-check.sh"
"$workspace_dir/scripts/core-check.sh"
"$workspace_dir/scripts/kernel-check.sh"
"$workspace_dir/scripts/debug-check.sh"
"$workspace_dir/scripts/cross-check.sh"
"$workspace_dir/scripts/object-check.sh"
nix flake check "path:$workspace_dir"
