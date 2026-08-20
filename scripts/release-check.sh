#!/usr/bin/env bash
# The version half of `AGENTS.md` §7, as a check.
#
# `Cargo.toml` is the only place the version is written down: `flake.nix` reads
# it from there, `Cargo.lock` repeats it for six crates, and
# `tests/consumer/Slopium.lock` repeats it again because the bundled library's
# digest moves with it. Those are the copies that drift, and a release is the
# worst moment to find out that they have.
#
#   scripts/release-check.sh                        # print the version
#   scripts/release-check.sh --check                # the always-true invariants
#   scripts/release-check.sh --check-release vX.Y.Z # and the release-only ones
#
# `--check` runs on every pull request and inside `scripts/verify.sh`;
# `--check-release` runs when a tag is pushed, before anything is built.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

failures=0

problem() {
  printf 'release-check: %s\n' "$1" >&2
  failures=$((failures + 1))
}

version() {
  sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml |
    head -1
}

# The crates the workspace publishes a version for. `Cargo.lock` names them
# alongside every dependency, so the version is read from the entry rather than
# counted globally.
members="slopic slopic-core slopium slopium-lsp slopium-manifest slopium-std"

check_invariants() {
  local v="$1"

  if [[ ! $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    problem "the workspace version is \`$v\`, which is not X.Y.Z"
    return
  fi

  local member locked
  for member in $members; do
    locked="$(awk -v name="$member" '
      $1 == "name" && $3 == "\"" name "\"" { found = 1; next }
      found && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
    ' Cargo.lock)"
    if [ -z "$locked" ]; then
      problem "\`Cargo.lock\` has no entry for \`$member\`"
    elif [ "$locked" != "$v" ]; then
      problem "\`Cargo.lock\` has \`$member\` at $locked, and the manifest says $v"
    fi
  done

  # The Nix package derives the version instead of repeating it. A literal in
  # `flake.nix` is how `slopium --version` and the package drift apart.
  if grep -q "\"$v\"" flake.nix; then
    problem "\`flake.nix\` names $v literally; it reads the version from \`Cargo.toml\`"
  fi

  grep -q '^## \[Unreleased\]' CHANGELOG.md ||
    problem "\`CHANGELOG.md\` has no \`## [Unreleased]\` heading to write under"

  # The bundled library is archived and hashed per version, so the committed
  # consumer's lock carries whatever the toolchain was when it was regenerated.
  local package toolchain
  for package in core std; do
    toolchain="$(awk -v name="$package" '
      $1 == "name" && $3 == "\"" name "\"" { found = 1; next }
      found && $1 == "version" { gsub(/"/, "", $3); print $3; exit }
    ' tests/consumer/Slopium.lock)"
    if [ "$toolchain" != "$v" ]; then
      problem "\`tests/consumer/Slopium.lock\` has \`$package\` at ${toolchain:-nothing}, and the manifest says $v; regenerate with SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh"
    fi
  done
}

check_release() {
  local v="$1" tag="$2"

  [ "$tag" = "v$v" ] ||
    problem "the tag is \`$tag\` and the manifest says $v"

  local heading
  heading="$(grep -m1 "^## \[$v\]" CHANGELOG.md || true)"
  if [ -z "$heading" ]; then
    problem "\`CHANGELOG.md\` has no \`## [$v]\` section; a release says what it changed"
  elif [[ ! $heading =~ ^\#\#\ \[$v\]\ -\ [0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    problem "the \`## [$v]\` heading is \`$heading\`; it is \`## [$v] - YYYY-MM-DD\`"
  fi

  # Everything the release carries has been moved into its own section by now,
  # so what is left under `[Unreleased]` is either an entry that missed the
  # release or a line nobody moved.
  local pending
  pending="$(awk '
    /^## \[Unreleased\]/ { inside = 1; next }
    /^## / { inside = 0 }
    inside && NF { print }
  ' CHANGELOG.md)"
  [ -z "$pending" ] ||
    problem "\`[Unreleased]\` still has entries; move them into \`[$v]\` or leave them out deliberately"

  grep -q "^\[$v\]: " CHANGELOG.md ||
    problem "\`CHANGELOG.md\` has no link reference for \`[$v]\`"

  # A fragment still sitting in `changelog.d/` is an entry that did not make the
  # section above, which is the one way collation can be forgotten (`D-137`).
  local stray
  stray="$(find changelog.d -maxdepth 1 -name '*.md' ! -name 'README.md' -printf '%f\n' 2>/dev/null | sort)"
  [ -z "$stray" ] ||
    problem "\`changelog.d/\` still holds $(printf '%s\n' "$stray" | wc -l | tr -d ' ') fragment(s); collate with \`scripts/changelog-collate.sh v$v\`"
}

main() {
  local v
  v="$(version)"
  if [ -z "$v" ]; then
    echo "release-check: no \`version\` under \`[workspace.package]\` in Cargo.toml" >&2
    exit 2
  fi

  case "${1-}" in
    "")
      printf '%s\n' "$v"
      return
      ;;
    --check)
      check_invariants "$v"
      ;;
    --check-release)
      local tag="${2-}"
      if [ -z "$tag" ]; then
        echo "release-check: --check-release needs a tag" >&2
        exit 2
      fi
      check_invariants "$v"
      check_release "$v" "$tag"
      ;;
    *)
      echo "release-check: unknown argument \`$1\`" >&2
      exit 2
      ;;
  esac

  if [ "$failures" -gt 0 ]; then
    printf 'release-check: %d problem(s). The contract is AGENTS.md §7.\n' "$failures" >&2
    exit 1
  fi

  echo "release-check: version $v ... ok"
}

main "$@"
