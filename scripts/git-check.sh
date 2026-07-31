#!/usr/bin/env bash
# The git-dependency gate: pinning, `--locked`, `--offline`, and vendoring.
#
# Everything here happens in a temporary directory: the repository being
# depended on is created by this script, at the moment it runs. The suite has no
# network — `nix flake check` runs it in a sandbox — so a fixture that had to be
# cloned would be a test that cannot run where it matters. It also means the
# repository can be moved, rewritten and deleted mid-test, which is how the
# interesting assertions are made.
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'chmod -R u+w "$scratch" 2>/dev/null || true; rm -rf "$scratch"' EXIT

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"

compiler="$workspace_dir/target/debug/slopic"
manager="$workspace_dir/target/debug/slopium"
export SLOPIUM_HOME="$scratch/home"
repository="$scratch/geometry"

slopium() {
  env SLOPIC="$compiler" "$manager" "$@"
}

fail() {
  echo "git-check: $1" >&2
  exit 1
}

# Never this machine's git configuration: a `url.*.insteadOf` rule would send a
# fetch somewhere the lock does not name, and the toolchain holds it away for
# the same reason.
repo() {
  env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
    GIT_AUTHOR_NAME=Test GIT_AUTHOR_EMAIL=test@example.invalid \
    GIT_COMMITTER_NAME=Test GIT_COMMITTER_EMAIL=test@example.invalid \
    git -C "$repository" "$@"
}

# --- a repository with two branches and a tag ---------------------------------

write_library() {
  cat >"$repository/Slopium.toml" <<EOF
[package]
name = "geometry"
version = "$1"
source = "src"
EOF
  cat >"$repository/src/lib.slp" <<EOF
(export area)

(fn area () -> i64 $2)
EOF
}

mkdir -p "$repository/src"
repo init --quiet --initial-branch=main
write_library 1.0.0 100
repo add --all
repo commit --quiet --message "first"
repo tag v1.0.0
first="$(repo rev-parse HEAD)"

repo checkout --quiet -b next
write_library 1.1.0 200
repo add --all
repo commit --quiet --message "second"
second="$(repo rev-parse HEAD)"
repo checkout --quiet main

# --- a consumer, one per way of naming a commit -------------------------------

consumer() {
  local name="$1" spec="$2" root="$scratch/$1"
  mkdir -p "$root/src"
  cat >"$root/Slopium.toml" <<EOF
[package]
name = "$name"
version = "1.0.0"
source = "src"
entry = "src/main.slp"

[dependencies]
geometry = { $spec }

[build]
target = "x86_64-unknown-linux-gnu"
EOF
  cat >"$root/src/main.slp" <<'EOF'
(take geometry:lib area)

(fn main () -> i32
  (println (area))
  0)
EOF
}

consumer by-branch "git = \"$repository\", branch = \"main\""
consumer by-tag "git = \"$repository\", tag = \"v1.0.0\""
consumer by-rev "git = \"$repository\", rev = \"${second:0:8}\""
consumer by-default "git = \"$repository\""
consumer unpinned "git = \"$repository\""

# --- resolution pins a commit and the digest of its archive -------------------

cd "$scratch/by-branch"
slopium build >"$scratch/build.out" 2>&1 ||
  fail "a git dependency does not build: $(cat "$scratch/build.out")"
[[ "$(./target/x86_64-unknown-linux-gnu/dev/by-branch)" == "100" ]] ||
  fail "the built program did not run the dependency's code"

grep --quiet "source = \"git+$repository?branch=main#$first\"" Slopium.lock ||
  fail "the lock does not pin the commit: $(grep source Slopium.lock)"
checksum="$(grep --after-context 3 'name = "geometry"' Slopium.lock |
  awk -F'"' '/checksum/ {print $2}')"
[[ -n "$checksum" ]] || fail "the lock records no checksum for a git package"
archive="$SLOPIUM_HOME/archives/$checksum.sl.tar"
[[ -f "$archive" ]] ||
  fail "the archive is not in the store under the digest the lock records"
if [[ "$(sha256sum "$archive" | cut -d ' ' -f 1)" != "$checksum" ]]; then
  fail "the stored archive is not what the lock says it is"
fi

# --- each way of naming a commit finds the one it names -----------------------

(cd "$scratch/by-tag" && slopium check >/dev/null) || fail "a tag dependency does not resolve"
grep --quiet "?tag=v1.0.0#$first\"" "$scratch/by-tag/Slopium.lock" ||
  fail "a tag did not pin the commit it names"

(cd "$scratch/by-rev" && slopium check >/dev/null) || fail "a rev dependency does not resolve"
# The abbreviation is what was asked for; the commit is what it resolved to.
grep --quiet "?rev=${second:0:8}#$second\"" "$scratch/by-rev/Slopium.lock" ||
  fail "a short rev did not pin all forty digits: $(grep source "$scratch/by-rev/Slopium.lock")"
grep --quiet 'version = "1.1.0"' "$scratch/by-rev/Slopium.lock" ||
  fail "a rev on another branch resolved to the wrong tree"

(cd "$scratch/by-default" && slopium check >/dev/null) ||
  fail "a dependency naming no reference does not resolve"
grep --quiet "source = \"git+$repository#$first\"" "$scratch/by-default/Slopium.lock" ||
  fail "the default branch did not pin the head of main"

# --- a second resolve is a no-op, even when the branch has moved --------------

cd "$scratch/by-branch"
cp Slopium.lock "$scratch/lock.first"
slopium check --locked >/dev/null || fail "a second resolve was not a no-op"
cmp Slopium.lock "$scratch/lock.first" || fail "a second resolve rewrote the lock"

write_library 1.2.0 300
repo add --all
repo commit --quiet --message "third"
[[ "$(repo rev-parse HEAD)" != "$first" ]] || fail "the branch did not move"

slopium check >/dev/null || fail "a moved branch broke a pinned build"
cmp Slopium.lock "$scratch/lock.first" ||
  fail "a moved branch was picked up; a pinned commit is not resolved again"

# --- --offline uses the store and never runs git ------------------------------

# With the repository gone there is nothing to fetch, so a build that succeeds
# is a build that read the store.
mv "$repository" "$scratch/geometry.moved"
slopium build --offline --locked >/dev/null ||
  fail "a pinned project does not build offline from the store"

# A dependency nothing has pinned cannot be resolved offline, and says which.
if (cd "$scratch/unpinned" && slopium check --offline >"$scratch/offline.out" 2>&1); then
  fail "an unpinned git dependency resolved with --offline"
fi
grep --quiet 'SL1011' "$scratch/offline.out" ||
  fail "an unpinned dependency was refused without SL1011: $(cat "$scratch/offline.out")"
mv "$scratch/geometry.moved" "$repository"

# --- a tampered store entry is caught before it is unpacked -------------------

rm -rf "$SLOPIUM_HOME/store"
python3 - "$archive" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
data[1024] ^= 1
path.write_bytes(bytes(data))
PY
if slopium check --locked >"$scratch/tampered.out" 2>&1; then
  fail "a tampered store entry was accepted"
fi
grep --quiet 'SL1010' "$scratch/tampered.out" ||
  fail "a tampered store entry was refused without SL1010: $(cat "$scratch/tampered.out")"
if [[ -d "$SLOPIUM_HOME/store/$checksum" ]]; then
  fail "a tampered archive was unpacked before it was verified"
fi

# Deleting it is the documented repair, and the commit is still pinned, so the
# same bytes come back.
rm "$archive"
slopium check --locked >/dev/null || fail "a re-fetch after a tampered entry failed"
[[ "$(sha256sum "$archive" | cut -d ' ' -f 1)" == "$checksum" ]] ||
  fail "the re-fetched archive is not the one the lock pins"

# --- vendoring a git package --------------------------------------------------

slopium vendor >"$scratch/vendor.out" || fail "vendoring a git package failed"
grep --quiet '^Vendored geometry v1.0.0 ' "$scratch/vendor.out" ||
  fail "vendor did not copy the git package: $(cat "$scratch/vendor.out")"
[[ -f vendor/geometry/Slopium.toml ]] || fail "the vendored package has no manifest"
grep --quiet '\[source.git\]' .slopium/config.toml ||
  fail "vendor did not redirect the git source"

cmp Slopium.lock "$scratch/lock.first" || fail "vendoring rewrote the lock"
slopium check --locked >/dev/null || fail "vendoring changed what the project resolves to"

# A vendored copy is the whole answer: no repository, no store, no git at all.
rm -rf "$repository" "$SLOPIUM_HOME"
slopium build --offline --locked >/dev/null ||
  fail "a vendored git package does not build without the store or the repository"

printf '(export area)\n\n(fn area () -> i64 999)\n' >vendor/geometry/src/lib.slp
if slopium check --offline --locked >"$scratch/edited.out" 2>&1; then
  fail "an edited vendored git package was accepted"
fi
grep --quiet 'SL1012' "$scratch/edited.out" ||
  fail "an edited vendored copy was refused without SL1012: $(cat "$scratch/edited.out")"

echo "git-check: pinning, offline, vendoring and tampering ... ok"
