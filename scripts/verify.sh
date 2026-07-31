#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_dir"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
"$workspace_dir/scripts/project-tests.sh"
"$workspace_dir/scripts/package-check.sh"
"$workspace_dir/scripts/git-check.sh"
"$workspace_dir/scripts/registry-check.sh"
"$workspace_dir/scripts/publish-check.sh"
"$workspace_dir/scripts/runtime-check.sh"
"$workspace_dir/scripts/debug-check.sh"
"$workspace_dir/scripts/cross-check.sh"
"$workspace_dir/scripts/object-check.sh"
nix flake check "path:$workspace_dir"
