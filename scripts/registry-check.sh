#!/usr/bin/env bash
# The registry gate: selection with backtracking, checksums, yanking, offline,
# `update -p`, and each refusal that keeps one name coming from one place.
#
# The registry is built here, at the moment this runs, because a registry is a
# directory and nothing else — `index/` holds one line per published version and
# `packages/` holds the archives those lines describe. Nothing reaches the
# network: the transport that does is exercised over loopback by
# `crates/slopium-manifest/tests/http_registry.rs`, which is where a test server
# can be written in Rust rather than in shell.
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'chmod -R u+w "$scratch" 2>/dev/null || true; rm -rf "$scratch"' EXIT

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"

compiler="$workspace_dir/target/debug/slopic"
manager="$workspace_dir/target/debug/slopium"
export SLOPIUM_HOME="$scratch/home"
registry="$scratch/registry"

slopium() {
  env SLOPIC="$compiler" "$manager" "$@"
}

fail() {
  echo "registry-check: $1" >&2
  exit 1
}

# --- publishing ---------------------------------------------------------------

# The index fans out by name length, exactly as `docs/packaging.md` specifies.
index_file() {
  local name="$1"
  case ${#name} in
  1) echo "$registry/index/1/$name.json" ;;
  2) echo "$registry/index/2/$name.json" ;;
  3) echo "$registry/index/3/${name:0:1}/$name.json" ;;
  *) echo "$registry/index/${name:0:2}/${name:2:2}/$name.json" ;;
  esac
}

configure() {
  mkdir -p "$1/.slopium"
  cat >"$1/.slopium/config.toml" <<EOF
[registry.default]
index = "$registry"
EOF
}

# publish <name> <version> <dependencies-toml> <body>
#
# Writes the archive and appends the line, which is all putting a package into a
# static index takes — and is what `slopium publish` will do in v0.4.5.
publish() {
  local name="$1" version="$2" dependencies="$3" body="$4"
  local root="$scratch/published/$name-$version"
  mkdir -p "$root/src"
  cat >"$root/Slopium.toml" <<EOF
[package]
name = "$name"
version = "$version"
source = "src"

[dependencies]
std = { toolchain = true }
$dependencies
EOF
  printf '%s\n' "$body" >"$root/src/lib.slp"
  configure "$root"

  local entry
  entry="$(cd "$root" && slopium package --index-entry)" ||
    fail "cannot package $name v$version"

  local index
  index="$(index_file "$name")"
  mkdir -p "$(dirname "$index")"
  printf '%s\n' "$entry" >>"$index"
  mkdir -p "$registry/packages/$name"
  cp "$root/target/package/$name-$version.sl.tar" \
    "$registry/packages/$name/$name-$version.sl.tar"
}

publish units 1.0.0 "" '(export factor)

(fn factor () -> i64 10)'

publish units 2.0.0 "" '(export factor)

(fn factor () -> i64 20)'

# `geometry` at its newest needs the newer `units`, and `shapes` cannot have it.
# Selecting maximally and stopping would take geometry 1.1.0 and then fail.
publish geometry 1.0.0 'units = "^1"' '(take units:lib factor)
(export area)

(fn area () -> i64 (+ 90 (factor)))'

publish geometry 1.1.0 'units = "^2"' '(take units:lib factor)
(export area)

(fn area () -> i64 (+ 180 (factor)))'

publish shapes 1.0.0 'units = "^1"' '(take units:lib factor)
(export perimeter)

(fn perimeter () -> i64 (factor))'

# --- a consumer ---------------------------------------------------------------

consumer() {
  local name="$1" dependencies="$2" root="$scratch/$1"
  mkdir -p "$root/src"
  cat >"$root/Slopium.toml" <<EOF
[package]
name = "$name"
version = "1.0.0"
source = "src"
entry = "src/main.slp"

[dependencies]
std = { toolchain = true }
$dependencies

[build]
target = "x86_64-unknown-linux-gnu"
EOF
  cat >"$root/src/main.slp" <<'EOF'
(take std:io println-i64)
(take geometry:lib area)
(take shapes:lib perimeter)

(fn main () -> i32
  (println-i64 (+ (area) (perimeter)))
  0)
EOF
  configure "$root"
}

consumer application 'geometry = "^1"
shapes = "^1"'

# --- backtracking is what makes this resolvable at all ------------------------

cd "$scratch/application"
slopium build >"$scratch/build.out" 2>&1 ||
  fail "a registry diamond does not build: $(cat "$scratch/build.out")"
[[ "$(./target/x86_64-unknown-linux-gnu/dev/application)" == "110" ]] ||
  fail "the built program did not run the selected versions' code"

grep --quiet 'version = "1.0.0"' <(grep --after-context 1 'name = "geometry"' Slopium.lock) ||
  fail "the newest geometry was taken even though nothing could satisfy it"
grep --quiet 'version = "1.0.0"' <(grep --after-context 1 'name = "units"' Slopium.lock) ||
  fail "units did not backtrack to the version both dependents accept"

# --- the lock records the index and the digest --------------------------------

grep --quiet "source = \"registry+$registry\"" Slopium.lock ||
  fail "the lock does not name the index: $(grep source Slopium.lock)"
checksum="$(grep --after-context 3 'name = "geometry"' Slopium.lock |
  awk -F'"' '/checksum/ {print $2}')"
[[ -n "$checksum" ]] || fail "the lock records no checksum for a registry package"
if [[ "$(sha256sum "$SLOPIUM_HOME/archives/$checksum.sl.tar" | cut -d ' ' -f 1)" != "$checksum" ]]; then
  fail "the stored archive is not what the lock says it is"
fi
if ! cmp --quiet "$SLOPIUM_HOME/archives/$checksum.sl.tar" \
  "$registry/packages/geometry/geometry-1.0.0.sl.tar"; then
  fail "what was stored is not what the registry published"
fi

# --- a pin is not consulted again, whatever the index grows -------------------

cp Slopium.lock "$scratch/lock.first"
publish shapes 1.1.0 'units = "^1"' '(take units:lib factor)
(export perimeter)

(fn perimeter () -> i64 (+ 1 (factor)))'

slopium check >/dev/null || fail "a second resolve failed"
cmp Slopium.lock "$scratch/lock.first" ||
  fail "a newly published version moved a pinned dependency"
slopium check --locked >/dev/null || fail "--locked rejected an unchanged lock"

# --- `update -p` moves one package, and the lock diff proves it ---------------

slopium update -p shapes >"$scratch/update.out" 2>&1 ||
  fail "update -p failed: $(cat "$scratch/update.out")"
grep --quiet "Updated shapes v1.0.0 -> v1.1.0" "$scratch/update.out" ||
  fail "update -p did not report what it moved: $(cat "$scratch/update.out")"
changed="$(diff "$scratch/lock.first" Slopium.lock | grep --count '^[<>] version\|^[<>] checksum' || true)"
[[ "$changed" == "4" ]] ||
  fail "update -p changed more than one package's version and checksum ($changed lines)"

slopium update -p shapes --precise 1.0.0 >/dev/null ||
  fail "update --precise failed"
cmp Slopium.lock "$scratch/lock.first" ||
  fail "--precise did not put the lock back where it was"

slopium update -p nonexistent >"$scratch/update.out" 2>&1 &&
  fail "update -p accepted a package the lock does not pin"
grep --quiet "nothing to update" "$scratch/update.out" ||
  fail "update -p on an unknown package said the wrong thing"

# --- offline resolution against a registry that is a directory ----------------

# A directory registry is local, so `--offline` has nothing to forbid about
# reading it: a dependency nothing has pinned resolves without a network.
consumer unpinned 'geometry = "^1"
shapes = "^1"'
(cd "$scratch/unpinned" && slopium check --offline) >"$scratch/offline.out" 2>&1 ||
  fail "an unpinned dependency did not resolve offline from a directory registry: $(cat "$scratch/offline.out")"
grep --quiet '^name = "geometry"' "$scratch/unpinned/Slopium.lock" ||
  fail "the offline resolution did not write a lock naming geometry"

# --- offline builds from the store, with no registry at all -------------------

mv "$registry" "$scratch/registry.away"
rm -rf target
slopium build --offline --locked >"$scratch/offline.out" 2>&1 ||
  fail "an offline build from the store failed: $(cat "$scratch/offline.out")"
[[ "$(./target/x86_64-unknown-linux-gnu/dev/application)" == "110" ]] ||
  fail "the offline build did not produce the same program"

# A registry directory that is not there is not a registry that publishes
# nothing: the message has to send somebody looking for a path, not a package.
consumer moved 'geometry = "^1"'
(cd "$scratch/moved" && slopium check) >"$scratch/moved.out" 2>&1 &&
  fail "a registry whose directory is gone was accepted"
grep --quiet "SL1030" "$scratch/moved.out" ||
  fail "a missing registry directory did not report SL1030: $(cat "$scratch/moved.out")"
mv "$scratch/registry.away" "$registry"

# --- the registry serving other bytes than it published -----------------------

rm -f "$SLOPIUM_HOME/archives/$checksum.sl.tar"
rm -rf "$SLOPIUM_HOME/store"
printf 'tampered' >>"$registry/packages/geometry/geometry-1.0.0.sl.tar"
slopium check >"$scratch/tampered.out" 2>&1 &&
  fail "a registry serving other bytes than it published was accepted"
grep --quiet "SL1034" "$scratch/tampered.out" ||
  fail "a served-bytes mismatch did not report SL1034: $(cat "$scratch/tampered.out")"
[[ ! -f "$SLOPIUM_HOME/archives/$checksum.sl.tar" ]] ||
  fail "bytes that failed their digest were filed under it anyway"

cp "$scratch/published/geometry-1.0.0/target/package/geometry-1.0.0.sl.tar" \
  "$registry/packages/geometry/geometry-1.0.0.sl.tar"
slopium check >/dev/null || fail "the registry did not recover once it served the right bytes"

# --- yanking ------------------------------------------------------------------

publish shapes 2.0.0 'units = "^1"' '(take units:lib factor)
(export perimeter)

(fn perimeter () -> i64 (factor))'
index="$(index_file shapes)"
sed --in-place 's/"name":"shapes","version":"2.0.0"\(.*\)"yanked":false/"name":"shapes","version":"2.0.0"\1"yanked":true/' "$index"
grep --quiet '"version":"2.0.0".*"yanked":true' "$index" ||
  fail "the fixture did not manage to yank a version"

consumer yanked 'geometry = "^1"
shapes = "^2"'
(cd "$scratch/yanked" && slopium check) >"$scratch/yanked.out" 2>&1 &&
  fail "a yanked version was selected"
grep --quiet "SL1035" "$scratch/yanked.out" ||
  fail "a yanked-only requirement did not report SL1035: $(cat "$scratch/yanked.out")"

# --- one name, one source -----------------------------------------------------

# A local directory calling itself `units` alongside the registry's `units`,
# reached through `geometry`. Picking one silently is the whole attack.
mkdir -p "$scratch/local-units/src"
cat >"$scratch/local-units/Slopium.toml" <<'EOF'
[package]
name = "units"
version = "1.0.0"
source = "src"
EOF
cat >"$scratch/local-units/src/lib.slp" <<'EOF'
(export factor)

(fn factor () -> i64 999)
EOF
consumer confused 'geometry = "^1"
shapes = "^1"
units = { path = "../local-units" }'
(cd "$scratch/confused" && slopium check) >"$scratch/confused.out" 2>&1 &&
  fail "one name resolved from two sources"
grep --quiet "SL1031" "$scratch/confused.out" ||
  fail "two sources for one name did not report SL1031: $(cat "$scratch/confused.out")"

# --- a registry nobody configured ---------------------------------------------

consumer unconfigured 'geometry = { version = "^1", registry = "somewhere" }'
(cd "$scratch/unconfigured" && slopium check) >"$scratch/unconfigured.out" 2>&1 &&
  fail "an unconfigured registry was resolved"
grep --quiet "SL1030" "$scratch/unconfigured.out" ||
  fail "an unconfigured registry did not report SL1030: $(cat "$scratch/unconfigured.out")"

rm -rf "$scratch/unconfigured/.slopium"
(cd "$scratch/unconfigured" && slopium check) >"$scratch/unconfigured.out" 2>&1 &&
  fail "the default registry resolved without being configured"
grep --quiet "ships no registry URL" "$scratch/unconfigured.out" ||
  fail "an unconfigured default registry did not say it is a choice"

# --- what may be published ----------------------------------------------------

mkdir -p "$scratch/unpublishable/src"
cat >"$scratch/unpublishable/Slopium.toml" <<'EOF'
[package]
name = "unpublishable"
version = "1.0.0"
source = "src"

[dependencies]
units = { path = "../local-units" }
EOF
printf '(export nothing)\n\n(fn nothing () -> i64 0)\n' >"$scratch/unpublishable/src/lib.slp"
(cd "$scratch/unpublishable" && slopium package --index-entry) >"$scratch/publish.out" 2>&1 &&
  fail "a package depending on a directory was given an index entry"
grep --quiet "SL1032" "$scratch/publish.out" ||
  fail "an unpublishable dependency did not report SL1032: $(cat "$scratch/publish.out")"

# --- add, remove, and the source column ---------------------------------------

cd "$scratch/application"
slopium tree >"$scratch/tree.out" 2>&1 || fail "tree failed"
grep --quiet "registry $registry" "$scratch/tree.out" ||
  fail "tree does not say where a package came from: $(cat "$scratch/tree.out")"

slopium add units@^1 >"$scratch/add.out" 2>&1 ||
  fail "add failed: $(cat "$scratch/add.out")"
grep --quiet 'units = "\^1"' Slopium.toml ||
  fail "add did not write the bare requirement form: $(cat Slopium.toml)"
grep --quiet "Added units v1.0.0 (registry" "$scratch/add.out" ||
  fail "add did not report what it resolved to: $(cat "$scratch/add.out")"
grep --quiet 'target = "x86_64-unknown-linux-gnu"' Slopium.toml ||
  fail "add rewrote the rest of the manifest"

slopium remove units >/dev/null || fail "remove failed"
grep --quiet "^units" Slopium.toml && fail "remove left the dependency behind"

# --- tree --depth and --duplicates --------------------------------------------

slopium tree --depth 1 >"$scratch/depth.out" 2>&1 || fail "tree --depth failed"
grep --quiet "geometry v1.0.0" "$scratch/depth.out" ||
  fail "tree --depth 1 dropped a direct dependency: $(cat "$scratch/depth.out")"
grep --quiet "(\.\.\.)" "$scratch/depth.out" ||
  fail "tree --depth 1 did not mark the subtree it cut: $(cat "$scratch/depth.out")"
grep --quiet "^|   \`-- units" "$scratch/depth.out" &&
  fail "tree --depth 1 printed a level it should have cut: $(cat "$scratch/depth.out")"

# `units` is reached through both `geometry` and `shapes`, which is the only
# kind of duplicate this resolver can produce: one version per name (`D-036`).
slopium tree --duplicates >"$scratch/duplicates.out" 2>&1 ||
  fail "tree --duplicates failed"
grep --quiet "^units v1.0.0" "$scratch/duplicates.out" ||
  fail "tree --duplicates did not report the shared package: $(cat "$scratch/duplicates.out")"
grep --quiet "required by geometry" "$scratch/duplicates.out" ||
  fail "tree --duplicates did not name a dependent: $(cat "$scratch/duplicates.out")"
grep --quiet "required by shapes" "$scratch/duplicates.out" ||
  fail "tree --duplicates did not name both dependents: $(cat "$scratch/duplicates.out")"

# --- vendor -p ----------------------------------------------------------------

# Two members with unequal needs: `full` pulls the registry graph, `bare` needs
# only the bundled library. Vendoring one of them must copy one of them.
members="$scratch/members"
mkdir -p "$members/full/src" "$members/bare/src"
configure "$members"
cat >"$members/Slopium.toml" <<EOF
[workspace]
members = ["full", "bare"]
EOF
cat >"$members/full/Slopium.toml" <<EOF
[package]
name = "full"
version = "1.0.0"
source = "src"
entry = "src/lib.slp"

[dependencies]
geometry = "^1.0.0"
EOF
printf '(take geometry:lib area)\n(export size)\n\n(fn size () -> i64 (area))\n' \
  >"$members/full/src/lib.slp"
cat >"$members/bare/Slopium.toml" <<EOF
[package]
name = "bare"
version = "1.0.0"
source = "src"
entry = "src/lib.slp"

[dependencies]
std = { toolchain = true }
EOF
printf '(export nothing)\n\n(fn nothing () -> i64 0)\n' >"$members/bare/src/lib.slp"

(cd "$members" && slopium vendor -p bare) >"$scratch/vendor.out" 2>&1 ||
  fail "vendor -p failed: $(cat "$scratch/vendor.out")"
[[ -d "$members/vendor/std" ]] ||
  fail "vendor -p did not copy what the selected member needs"
[[ ! -d "$members/vendor/geometry" ]] ||
  fail "vendor -p copied a package only the other member needs"
grep --quiet '`full` still needs packages that were not copied' "$scratch/vendor.out" ||
  fail "vendor -p did not say which member it left unbuildable: $(cat "$scratch/vendor.out")"

# The whole workspace leaves nothing out, and says nothing about it.
(cd "$members" && slopium vendor) >"$scratch/vendor.out" 2>&1 ||
  fail "vendor of the whole workspace failed: $(cat "$scratch/vendor.out")"
[[ -d "$members/vendor/geometry" ]] ||
  fail "vendor did not copy the whole workspace's packages"
grep --quiet "still needs packages" "$scratch/vendor.out" &&
  fail "a complete vendor warned about a member anyway: $(cat "$scratch/vendor.out")"

(cd "$members" && slopium check --workspace --offline --locked) >"$scratch/vendor.out" 2>&1 ||
  fail "a fully vendored workspace does not build offline: $(cat "$scratch/vendor.out")"

echo "registry-check: selection, checksums, yanking, offline and confusion ... ok"
