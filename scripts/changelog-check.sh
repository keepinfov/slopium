#!/usr/bin/env bash
# The shape of a changelog fragment, as a check (`D-137`).
#
# A change writes its entry into its own file under `changelog.d/` rather than
# into `CHANGELOG.md`, so that two changes in flight collide over nothing. That
# only holds while the files are uniform: a collator that had to guess at a
# fragment's kind, or reflow its prose, would be editing what somebody wrote.
#
#   scripts/changelog-check.sh
#
# It runs from `scripts/verify.sh` and on every pull request.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

directory="changelog.d"
# Keep a Changelog's six, in the order a section prints them.
kinds="added changed deprecated removed fixed security"
width=80

failures=0

problem() {
  printf 'changelog-check: %s\n' "$1" >&2
  failures=$((failures + 1))
}

# `[Unreleased]` is a heading and a promise now, and nothing is written under
# it: an entry there is one that two branches would have fought over, which is
# the whole reason fragments exist. `release-check.sh` enforced this at the
# release and it is the invariant from here on.
check_unreleased_is_empty() {
  local pending
  pending="$(awk '
    /^## \[Unreleased\]/ { inside = 1; next }
    /^## / { inside = 0 }
    inside && NF { print }
  ' CHANGELOG.md)"
  [ -z "$pending" ] && return 0
  problem "\`[Unreleased]\` has entries; a change writes one file under \`$directory/\` instead"
  printf '%s\n' "$pending" | sed 's/^/  /' >&2
}

check_name() {
  local name="$1" kind="$2"
  case " $kinds " in
    *" $kind "*) ;;
    *)
      problem "\`$name\` is a \`$kind\` entry; the kinds are $kinds"
      return
      ;;
  esac
  # `<issue>[-<discriminator>].<kind>.md`. The issue number is what makes two
  # fragments unable to collide, and the discriminator is for the change that
  # owes two entries of one kind.
  [[ $name =~ ^[0-9]+(-[a-z0-9-]+)?\.[a-z]+\.md$ ]] ||
    problem "\`$name\` is not \`<issue>[-<discriminator>].<kind>.md\`"
}

check_body() {
  local path="$1" name="$2" line number=0 first=1
  if [ ! -s "$path" ]; then
    problem "\`$name\` is empty"
    return
  fi
  [ -n "$(tail -c 1 "$path")" ] &&
    problem "\`$name\` does not end with a newline"
  while IFS= read -r line; do
    number=$((number + 1))
    if [ "$first" -eq 1 ]; then
      first=0
      # The fragment holds the bullet as it will be published. A collator that
      # added the marker would own the wrapping too, and then the width a
      # writer sees would not be the width that ships.
      [[ $line == "- "* ]] ||
        problem "\`$name\` line 1 does not begin \`- \`; a fragment is one bullet, written as it will read"
    elif [ -n "$line" ]; then
      [[ $line == "  "* ]] ||
        problem "\`$name\` line $number is not indented two spaces; a bullet's continuation is"
    fi
    [ "${#line}" -le "$width" ] ||
      problem "\`$name\` line $number is ${#line} columns; wrap a fragment at $width"
    [[ $line != *[[:space:]] ]] ||
      problem "\`$name\` line $number has trailing whitespace"
  done <"$path"
  # One bullet per file: a second one is a second entry, and it has a name of
  # its own to be written under.
  local bullets
  bullets="$(grep -c '^- ' "$path" || true)"
  [ "$bullets" -eq 1 ] ||
    problem "\`$name\` holds $bullets bullets; a fragment is one entry"
}

main() {
  check_unreleased_is_empty

  if [ ! -d "$directory" ]; then
    printf 'changelog-check: no `%s/`, nothing to check ... ok\n' "$directory"
    return 0
  fi

  local count=0 path name kind
  for path in "$directory"/*; do
    [ -e "$path" ] || continue
    name="$(basename "$path")"
    # The directory explains itself to whoever opens it first.
    [ "$name" = "README.md" ] && continue
    if [ -d "$path" ]; then
      problem "\`$name\` is a directory; \`$directory/\` holds fragments and nothing else"
      continue
    fi
    kind="${name%.md}"
    kind="${kind##*.}"
    check_name "$name" "$kind"
    check_body "$path" "$name"
    count=$((count + 1))
  done

  if [ "$failures" -ne 0 ]; then
    printf 'changelog-check: %d problem(s). The contract is AGENTS.md §7.\n' "$failures" >&2
    return 1
  fi
  printf 'changelog-check: %d fragment(s) ... ok\n' "$count"
}

main "$@"
