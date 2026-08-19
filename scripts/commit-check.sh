#!/usr/bin/env bash
# The commit contract of `AGENTS.md`, as a check.
#
# Everything in §6 and §7 a machine can decide is decided here: the shape of the
# subject, its mood and its width, the blank second line, a body of prose
# wrapped at eighty columns in no more than two paragraphs, the trailers, the
# forms of address a message may not use, and — for a commit rather than a bare
# message — the rule that the workspace version moves in a release commit and
# nowhere else.
#
# What is left over is judgement: whether the first paragraph says what is now
# true that was not true before, and whether the second is worth a reader's
# time. No script decides that, and this one does not pretend to. It exists so
# that the mechanical half of the contract holds for a contributor who has never
# read `git log` and has no `.notes/` to read either.
#
#   scripts/commit-check.sh                  # origin/main..HEAD, else HEAD alone
#   scripts/commit-check.sh <rev-range>      # every commit in the range
#   scripts/commit-check.sh --message FILE   # a message before it is a commit
set -euo pipefail

types="feat fix refactor perf test docs build ci chore revert"
scopes="slopium slopic manifest docs release"
needs_body="feat fix refactor perf test docs"
subject_max=95
body_wrap=80

failures=0
label="message"
message_type=""
is_merge=0

problem() {
  printf 'commit-check: %s: %s\n' "$label" "$1" >&2
  failures=$((failures + 1))
}

# Characters, not bytes. This history is full of em dashes and they are three
# bytes each, so a byte count would refuse a line that is exactly right.
width() {
  local stripped
  stripped="$(printf '%s' "$1" | LC_ALL=C tr -d '\200-\277')"
  printf '%s' "${#stripped}"
}

listed() { # listed <word> <space-separated list>
  local needle="$1" word
  for word in $2; do
    [ "$word" = "$needle" ] && return 0
  done
  return 1
}

# What git itself would drop before making a commit out of the file.
strip_comments() {
  awk '/^# *-+ >8 -+/ { exit } /^#/ { next } { print }'
}

check_subject() {
  local subject="$1"
  message_type=""

  # A merge takes the pull request's title and the forge appends the number to
  # it, so `(#41)` is part of every subject that reaches `main` this way.
  if [[ $subject =~ ^(.*)\ \(#[0-9]+\)$ ]]; then
    subject="${BASH_REMATCH[1]}"
  fi

  if [[ ! $subject =~ ^([a-z]+)(\(([a-z][a-z-]*)\))?(!)?:\ (.+)$ ]]; then
    problem "the subject is not \`type(scope): description\` — $subject"
    return
  fi

  local kind="${BASH_REMATCH[1]}"
  local scope="${BASH_REMATCH[3]}"
  local breaking="${BASH_REMATCH[4]}"
  local description="${BASH_REMATCH[5]}"
  message_type="$kind"

  listed "$kind" "$types" ||
    problem "unknown type \`$kind\`; allowed: $types"

  if [ -n "$scope" ]; then
    listed "$scope" "$scopes" ||
      problem "unknown scope \`$scope\`; this history uses $scopes, and a new one is agreed before it is used"
  fi

  case "${description:0:1}" in
    [a-z] | '`') ;;
    *) problem "the description starts with \`${description:0:1}\`; it is lowercase and imperative" ;;
  esac

  [ "${description: -1}" != "." ] ||
    problem "the subject ends with a period"

  local columns
  columns="$(width "$subject")"
  [ "$columns" -le "$subject_max" ] ||
    problem "the subject is $columns columns; the limit is $subject_max and the target is 72"

  [ -z "$breaking" ] || message_type="$kind!"
}

check_message() {
  local -a lines=()
  mapfile -t lines <<<"$1"

  # Git writes this itself when a branch is brought up to date with another,
  # and the `commit-msg` hook sees it before there is a commit to ask how many
  # parents it has. It is housekeeping either way, and `check_commit` skips the
  # same message once the merge exists.
  if [[ ${lines[0]-} =~ ^Merge\ (branch|remote-tracking\ branch|commit|tag)\  ]]; then
    return 0
  fi
  while [ "${#lines[@]}" -gt 0 ] && [ -z "${lines[-1]}" ]; do
    unset 'lines[-1]'
  done

  if [ "${#lines[@]}" -eq 0 ]; then
    problem "the message is empty"
    return
  fi

  check_subject "${lines[0]}"
  local breaking_subject="" kind="$message_type"
  case "$kind" in
    *'!') breaking_subject="yes" kind="${kind%!}" message_type="$kind" ;;
  esac

  if [ "${#lines[@]}" -ge 2 ] && [ -n "${lines[1]}" ]; then
    problem "the second line must be blank"
  fi

  local -a body=()
  [ "${#lines[@]}" -gt 2 ] && body=("${lines[@]:2}")
  if [ "${#body[@]}" -eq 0 ]; then
    if [ "$is_merge" -eq 0 ] && listed "$kind" "$needs_body"; then
      problem "a \`$kind\` commit needs a body: one or two paragraphs saying what is now true and what the work uncovered"
    fi
    return 0
  fi

  local paragraphs=0 inside=0 footer=0 line number=0 columns lowered
  for line in "${body[@]}"; do
    number=$((number + 1))

    if [ -z "$line" ]; then
      inside=0
      continue
    fi

    if [ "$inside" -eq 0 ]; then
      inside=1
      paragraphs=$((paragraphs + 1))
      case "$line" in
        'BREAKING CHANGE: '* | 'Fixes #'* | 'Refs #'*) footer=1 ;;
        *) footer=0 ;;
      esac
    fi

    columns="$(width "$line")"
    if [ "$columns" -gt "$body_wrap" ] && [[ $line == *" "* ]]; then
      problem "body line $number is $columns columns; wrap the body at $body_wrap"
    fi

    if [[ $line =~ ^[[:space:]]*([-*+]|[0-9]+[.\)])[[:space:]] ]]; then
      problem "body line $number is a list item; the body is prose, and a list belongs in \`docs/\` or \`.notes/\`"
    fi

    if [[ $line =~ ^[[:space:]]*\`\`\` ]]; then
      problem "body line $number opens a code block; that detail belongs in \`docs/\`"
    fi

    lowered="${line,,}"
    case "$lowered" in
      'signed-off-by: '* | 'co-authored-by: '* | 'reviewed-by: '* | \
        'acked-by: '* | 'tested-by: '* | 'reported-by: '* | \
        'generated-by: '* | 'assisted-by: '* | 'change-id: '*)
        problem "body line $number is a trailer; this history has none"
        ;;
    esac
  done

  local prose="$paragraphs"
  [ "$footer" -eq 1 ] && prose=$((paragraphs - 1))
  if [ "$prose" -gt 2 ]; then
    problem "the body has $prose paragraphs; two is the maximum, and the rest belongs in \`docs/\` or \`.notes/\`"
  fi

  # Attribution, not every mention of a name: `CLAUDE.md` is a tracked file and
  # a body is allowed to say so, while "by claude" in a commit message is the
  # thing this history has never carried.
  local whole="${1,,}"
  case "$whole" in
    *co-authored-by* | *assisted-by* | *generated-by* | \
      *'generated with'* | *'generated by'* | \
      *'by claude'* | *'with claude'* | *'claude code'* | *claude.ai* | \
      *chatgpt* | *copilot* | *anthropic.com* | *openai.com* | *'🤖'*)
      problem "the message carries AI or agent attribution; it never does"
      ;;
  esac

  case "$whole" in
    *'this commit'*)
      problem "the body says \"this commit\"; write what the code now does instead"
      ;;
  esac

  # A message is read by somebody who was in no conversation. Where the reason
  # for a change came out of one, the reason is what belongs here; its source is
  # a thing the reader cannot look up.
  case "$whole" in
    *'as discussed'* | *'as requested'* | *'as agreed'* | *'as suggested'* | \
      *'you asked'* | *'your suggestion'* | *'your request'* | *'per your'* | \
      *'as you '* | *'at your '* | *'the user asked'* | *'the user said'* | \
      *'the user chose'* | *'the user wanted'* | *'i decided'* | *'i chose'* | \
      *'thank you'* | *'thanks to'* | *'good catch'*)
      problem "the message addresses somebody; state the reason, never its source"
      ;;
  esac

  # `.notes/` is gitignored, so a path into it is a reference the reader cannot
  # follow. Naming the directory is fine; naming a document inside it is not.
  if [[ $1 =~ \.notes/[A-Za-z0-9_.@-]+ ]]; then
    problem "the message points at a file under \`.notes/\`, which a clone does not contain"
  fi

  if [ -n "$breaking_subject" ] && [[ $whole != *'breaking change:'* ]]; then
    problem "the subject is marked \`!\` but the body has no \`BREAKING CHANGE:\` footer"
  fi
  if [ -z "$breaking_subject" ] && [[ $whole == *'breaking change:'* ]]; then
    problem "the body has a \`BREAKING CHANGE:\` footer but the subject is not marked \`!\`"
  fi

  return 0
}

check_commit() {
  local sha="$1"
  label="$(git log -1 --format=%h "$sha")"

  # A merge carries the pull request's title and nothing else, and its diff
  # against its first parent is not what it introduced, so the version rules
  # below see an empty file list and stay quiet.
  local parents
  parents="$(git rev-list --parents -n 1 "$sha" | wc -w)"
  is_merge=0
  [ "$parents" -gt 2 ] && is_merge=1

  local subject release=""
  subject="$(git log -1 --format=%s "$sha")"

  # Git's own message for a branch brought up to date with `main`. It is
  # housekeeping inside a pull request rather than a claim about the software,
  # and the merge that lands on `main` carries the pull request's title instead.
  if [ "$is_merge" -eq 1 ] &&
    [[ $subject =~ ^Merge\ (branch|remote-tracking\ branch|commit|tag)\  ]]; then
    return 0
  fi

  check_message "$(git log -1 --format=%B "$sha")"
  if [[ $subject =~ ^chore\(release\):\ v([0-9]+\.[0-9]+\.[0-9]+)( \(#[0-9]+\))?$ ]]; then
    release="${BASH_REMATCH[1]}"
  fi

  local touched bumped=0 old="" new=""
  touched="$(git show --format= --name-only "$sha")"

  if printf '%s\n' "$touched" | grep -q '^\.notes/'; then
    problem "the commit contains \`.notes/\`, which is never committed"
  fi

  old="$(git show --format= -U0 "$sha" -- Cargo.toml | sed -n 's/^-version = "\(.*\)"/\1/p' | head -1)"
  new="$(git show --format= -U0 "$sha" -- Cargo.toml | sed -n 's/^+version = "\(.*\)"/\1/p' | head -1)"
  [ -n "$new" ] && bumped=1

  if [ -n "$release" ]; then
    if [ "$bumped" -eq 0 ]; then
      [ "$is_merge" -eq 1 ] ||
        problem "\`chore(release): v$release\` moves no version; set \`workspace.package.version\`"
    elif [ "$new" != "$release" ]; then
      problem "the subject says v$release and the manifest says $new"
    fi
    if [ "$bumped" -eq 1 ]; then
      local companion
      for companion in Cargo.lock CHANGELOG.md tests/consumer/Slopium.lock; do
        printf '%s\n' "$touched" | grep -qx "$companion" ||
          problem "the release does not touch \`$companion\`; §7 lists it"
      done
    fi
  elif [ "$bumped" -eq 1 ]; then
    problem "the version moved to $new outside a release; only \`chore(release): vX.Y.Z\` does that"
  fi

  if [ "$bumped" -eq 1 ] && [ -n "$old" ]; then
    if [ "$old" = "$new" ] ||
      [ "$(printf '%s\n%s\n' "$old" "$new" | sort -V | tail -1)" != "$new" ]; then
      problem "the version went from $old to $new, which is not forward"
    fi
  fi

  return 0
}

main() {
  local root
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  cd "$root"

  if [ "${1-}" = "--message" ]; then
    local file="${2-}"
    if [ -z "$file" ] || [ ! -r "$file" ]; then
      echo "commit-check: --message needs a readable file" >&2
      exit 2
    fi
    label="$(basename "$file")"
    check_message "$(strip_comments <"$file")"
  else
    local range="${1-}"
    if [ -z "$range" ]; then
      if git rev-parse --verify -q origin/main >/dev/null &&
        [ -n "$(git rev-list origin/main..HEAD)" ]; then
        range="origin/main..HEAD"
      elif git rev-parse --verify -q HEAD~1 >/dev/null; then
        range="HEAD~1..HEAD"
      else
        range="HEAD"
      fi
    fi

    local shas sha
    shas="$(git rev-list "$range")"
    if [ -z "$shas" ]; then
      echo "commit-check: no commits in $range"
      return
    fi
    for sha in $shas; do
      check_commit "$sha"
    done
  fi

  if [ "$failures" -gt 0 ]; then
    printf 'commit-check: %d problem(s). The contract is AGENTS.md §6 and §7.\n' \
      "$failures" >&2
    exit 1
  fi

  echo "commit-check: ok"
}

main "$@"
