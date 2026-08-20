#!/usr/bin/env bash
# Step 4 of `AGENTS.md` §7: turn the fragments into a release's section.
#
# A change writes its entry into its own file under `changelog.d/` so that two
# changes in flight collide over nothing (`D-137`). This is where those files
# become the one thing a reader opens — run in the release pull request, once,
# after the version has moved.
#
#   scripts/changelog-collate.sh vX.Y.Z [YYYY-MM-DD]
#
# The date defaults to today. Ordering is by kind, in Keep a Changelog's order,
# then by issue number, so the same fragments always produce the same section
# and a re-run is a way of checking rather than a way of changing.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

directory="changelog.d"
kinds="added changed deprecated removed fixed security"

usage() {
  printf 'usage: scripts/changelog-collate.sh vX.Y.Z [YYYY-MM-DD]\n' >&2
  exit 2
}

[ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
tag="$1"
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || usage
version="${tag#v}"
date="${2:-$(date +%F)}"
[[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || usage

"$root/scripts/changelog-check.sh" >/dev/null

if grep -q "^## \[$version\]" CHANGELOG.md; then
  printf 'changelog-collate: `CHANGELOG.md` already has a `[%s]` section\n' "$version" >&2
  exit 1
fi

# The issue number a fragment is named for, which is what it sorts by. `sort -n`
# on the name would put 10 before 6.
number() {
  local name="${1##*/}"
  name="${name%%.*}"
  printf '%s' "${name%%-*}"
}

section="$(
  for kind in $kinds; do
    fragments=()
    for path in "$directory"/*."$kind".md; do
      [ -e "$path" ] || continue
      fragments+=("$(number "$path")	$path")
    done
    [ "${#fragments[@]}" -eq 0 ] && continue
    heading="$(printf '%s' "${kind:0:1}" | tr '[:lower:]' '[:upper:]')${kind:1}"
    printf '\n### %s\n' "$heading"
    printf '%s\n' "${fragments[@]}" | sort -n -k1,1 -k2,2 | cut -f2- |
      while IFS= read -r path; do cat "$path"; done
  done
)"

if [ -z "$section" ]; then
  printf 'changelog-collate: no fragments in `%s/`; a release says what it changed\n' "$directory" >&2
  exit 1
fi

# `[Unreleased]` stays where it is and stays empty: it is the heading the next
# change's fragments will be collated under, not a place anything is written.
python3 - "$version" "$date" "$section" <<'PY'
import re
import sys

version, date, section = sys.argv[1], sys.argv[2], sys.argv[3]
COMPARE = "https://github.com/keepinfov/slopium"
path = "CHANGELOG.md"
text = open(path, encoding="utf-8").read()

anchor = "## [Unreleased]\n"
if anchor not in text:
    sys.exit("changelog-collate: `CHANGELOG.md` has no `## [Unreleased]` heading")

released = f"## [{version}] - {date}\n{section}\n"
text = text.replace(anchor, f"{anchor}\n{released}\n", 1)

# The link reference `release-check.sh --check-release` asks for, against the
# version this one follows — which is the next heading down, now that ours is
# the first.
headings = re.findall(r"^## \[([0-9]+\.[0-9]+\.[0-9]+)\]", text, flags=re.M)
if len(headings) > 1:
    previous = headings[1]
    reference = f"[{version}]: {COMPARE}/compare/v{previous}...v{version}\n"
    anchor_reference = f"[{previous}]: "
    if anchor_reference not in text:
        sys.exit(f"changelog-collate: `CHANGELOG.md` has no link reference for `[{previous}]`")
    text = text.replace(anchor_reference, reference + anchor_reference, 1)

open(path, "w", encoding="utf-8").write(text)
PY

# The fragments have become the section; the directory keeps only the note that
# explains what it is for.
for path in "$directory"/*.md; do
  [ -e "$path" ] || continue
  [ "$(basename "$path")" = "README.md" ] && continue
  rm -f "$path"
done

printf 'changelog-collate: wrote `[%s] - %s` and cleared `%s/`\n' "$version" "$date" "$directory"
