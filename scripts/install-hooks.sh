#!/usr/bin/env bash
# Point this clone's hooks at the tracked ones in `scripts/hooks`.
#
# One hook lives there today: `commit-msg`, which runs the commit contract over
# the message you just wrote. Installing is a local configuration change and
# nothing else, so it is opt-in and reversible:
#
#   scripts/install-hooks.sh
#   git config --unset core.hooksPath   # to undo
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
current="$(git -C "$root" config --get core.hooksPath || true)"

if [ -n "$current" ] && [ "$current" != "scripts/hooks" ]; then
  echo "install-hooks: core.hooksPath is already '$current'; leaving it alone" >&2
  exit 1
fi

git -C "$root" config core.hooksPath scripts/hooks
echo "install-hooks: core.hooksPath = scripts/hooks"
echo "install-hooks: commit messages are now checked by scripts/commit-check.sh"
