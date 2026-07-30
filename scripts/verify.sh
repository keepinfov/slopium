#!/usr/bin/env bash
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$workspace_dir"

cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
"$workspace_dir/scripts/project-tests.sh"
"$workspace_dir/scripts/runtime-check.sh"
"$workspace_dir/scripts/debug-check.sh"
nix flake check "path:$workspace_dir"
