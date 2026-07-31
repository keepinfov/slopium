#!/usr/bin/env bash
# The signing gate: publishing, trusted keys, and every way a package can fail
# to be the one it claims to be.
#
# A registry is a directory (`D-052`), so publishing is writing three files into
# one and this script can build a whole registry, sign into it, and consume from
# it without a server existing anywhere. The last section regenerates
# `tests/registry` and `tests/consumer/Slopium.lock` and requires them to come
# out byte-identical to what is committed — which is what makes the archive
# format's reproducibility (`D-039`) and Ed25519's determinism assertions rather
# than claims, and is what the Nix bridge builds from.
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'chmod -R u+w "$scratch" 2>/dev/null || true; rm -rf "$scratch"' EXIT

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"

compiler="$workspace_dir/target/debug/slopic"
manager="$workspace_dir/target/debug/slopium"

slopium() {
  env SLOPIC="$compiler" "$manager" "$@"
}

fail() {
  echo "publish-check: $1" >&2
  exit 1
}

# The key the committed fixture registry is signed with. It is a constant in
# this file rather than a file in the repository, so that nobody has to decide
# whether a key checked into git is a secret: this one is a pattern, it signs
# test fixtures, and it is public by construction.
fixture_seed="00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"

# --- fixtures -----------------------------------------------------------------

# write_geometry <root> <version> <area>
#
# The package the whole gate publishes: one exported function returning a number
# the consumer prints, so "did the right version arrive" is answerable by
# looking at stdout.
write_geometry() {
  local root="$1" version="$2" area="$3"
  mkdir -p "$root/src"
  cat >"$root/Slopium.toml" <<EOF
[package]
name = "geometry"
version = "$version"
source = "src"
EOF
  cat >"$root/src/lib.slp" <<EOF
(export area)

(fn area () -> i32 $area)
EOF
}

# write_consumer <root> <registry-index> [trusted-key]
write_consumer() {
  local root="$1" index="$2" key="${3:-}"
  mkdir -p "$root/src" "$root/.slopium"
  cat >"$root/Slopium.toml" <<'EOF'
[package]
name = "consumer"
version = "0.1.0"
source = "src"
entry = "src/main.slp"

[dependencies]
geometry = "^1"
EOF
  cat >"$root/src/main.slp" <<'EOF'
(take geometry:lib area)

(fn main () -> i32
  (println (area))
  0)
EOF
  printf '[registry.default]\nindex = "%s"\n' "$index" >"$root/.slopium/config.toml"
  if [ -n "$key" ]; then
    printf 'trusted-keys = ["%s"]\n' "$key" >>"$root/.slopium/config.toml"
  fi
}

# --- publishing ---------------------------------------------------------------

export SLOPIUM_HOME="$scratch/home"
registry="$scratch/registry"
key="$scratch/signing-key"
other_key="$scratch/other-key"

mkdir -p "$registry"
slopium key new "$key" >"$scratch/key-output" ||
  fail "cannot make a signing key"
slopium key new "$other_key" >/dev/null
public="$(slopium key public "$key")"
other_public="$(slopium key public "$other_key")"

grep -q "trusted-keys" "$scratch/key-output" ||
  fail "\`key new\` does not say where the public key goes"
[ "$(stat -c '%a' "$key")" = "600" ] ||
  fail "a new signing key is not mode 0600"

publish() {
  local root="$1"
  shift
  mkdir -p "$root/.slopium"
  printf '[registry.default]\nindex = "%s"\n' "$registry" >"$root/.slopium/config.toml"
  (cd "$root" && slopium publish --key "$key" "$@")
}

write_geometry "$scratch/geometry-1.0.0" 1.0.0 100
publish "$scratch/geometry-1.0.0" >"$scratch/published" ||
  fail "cannot publish geometry v1.0.0"

grep -q "Signed by $public" "$scratch/published" ||
  fail "publishing does not say which key signed"
[ -f "$registry/packages/geometry/geometry-1.0.0.sl.tar.sig" ] ||
  fail "publishing writes no detached signature"
grep -q '"signature"' "$registry/index/ge/om/geometry.json" ||
  fail "the index line carries no signature"

# The archive and the index agree about the signature, because publishing writes
# the same line to both (`D-056`).
sig_file="$(cat "$registry/packages/geometry/geometry-1.0.0.sl.tar.sig")"
sig_index="$(sed -n 's/.*"signature":"\([^"]*\)".*/\1/p' "$registry/index/ge/om/geometry.json")"
[ "$sig_file" = "$sig_index" ] ||
  fail "the detached signature and the index line disagree"

# --- consuming ----------------------------------------------------------------

# A fresh store, so this is the whole path: resolve, download, check the digest,
# check the signature, unpack, build, run.
write_consumer "$scratch/consumer" "$registry" "$public"
output="$(cd "$scratch/consumer" && slopium run | tail -1)" ||
  fail "a signed package does not build"
[ "$output" = "100" ] ||
  fail "expected 100 from the published package, got: $output"

grep -q "registry+$registry" "$scratch/consumer/Slopium.lock" ||
  fail "the lock does not name the index"

(cd "$scratch/consumer" && slopium verify) >"$scratch/verified" ||
  fail "\`verify\` refuses a package it has just built"
grep -q "signed by $public" "$scratch/verified" ||
  fail "\`verify\` does not report who signed"

# --- what is refused ----------------------------------------------------------

refuses() {
  local what="$1" code="$2"
  shift 2
  local output
  if output="$("$@" 2>&1)"; then
    fail "$what was accepted: $output"
  fi
  grep -q "$code" <<<"$output" ||
    fail "$what should report $code, got: $output"
}

# A version already in the index is never rewritten (`D-059`).
refuses "republishing a version" SL1043 \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/geometry-1.0.0' && '$manager' publish --key '$key'"

# A key nobody listed. This is also what a publisher's key rotation looks like
# from here, which is why the message names the key to add.
write_consumer "$scratch/stranger" "$registry" "$other_public"
refuses "a package signed by an unlisted key" SL1042 \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/stranger' && '$manager' build"
(cd "$scratch/stranger" && slopium build 2>&1 || true) | grep -q "$public" ||
  fail "the refusal does not name the key that signed"

# A trusted key's signature over something else, filed as though it were this
# package's. Nothing but the statement inside the signature stops this.
forged="$scratch/forged"
cp -r "$registry" "$forged"
write_geometry "$scratch/geometry-elsewhere" 1.0.0 100
elsewhere="$scratch/elsewhere"
mkdir -p "$elsewhere"
(
  registry="$elsewhere"
  mkdir -p "$scratch/units-1.0.0/src"
  cat >"$scratch/units-1.0.0/Slopium.toml" <<'EOF'
[package]
name = "units"
version = "1.0.0"
source = "src"
EOF
  echo '(export factor)

(fn factor () -> i32 1)' >"$scratch/units-1.0.0/src/lib.slp"
  mkdir -p "$scratch/units-1.0.0/.slopium"
  printf '[registry.default]\nindex = "%s"\n' "$elsewhere" >"$scratch/units-1.0.0/.slopium/config.toml"
  (cd "$scratch/units-1.0.0" && slopium publish --key "$key") >/dev/null
)
cp "$elsewhere/packages/units/units-1.0.0.sl.tar.sig" \
  "$forged/packages/geometry/geometry-1.0.0.sl.tar.sig"
sed -i 's/"signature":"[^"]*"/"signature":"'"$(cat "$forged/packages/geometry/geometry-1.0.0.sl.tar.sig")"'"/' \
  "$forged/index/ge/om/geometry.json"

write_consumer "$scratch/victim" "$forged" "$public"
refuses "a signature made for another package" SL1041 \
  env SLOPIC="$compiler" SLOPIUM_HOME="$scratch/fresh-forged" bash -c \
  "cd '$scratch/victim' && '$manager' build"

# Configuring keys is what turns signing on, so an unsigned package from a
# registry that has them is refused rather than quietly taken (`D-057`).
unsigned="$scratch/unsigned"
mkdir -p "$unsigned/packages/geometry" "$unsigned/index/ge/om"
cp "$registry/packages/geometry/geometry-1.0.0.sl.tar" "$unsigned/packages/geometry/"
sed 's/,"signature":"[^"]*"//' "$registry/index/ge/om/geometry.json" \
  >"$unsigned/index/ge/om/geometry.json"
write_consumer "$scratch/strict" "$unsigned" "$public"
refuses "an unsigned package where keys are configured" SL1040 \
  env SLOPIC="$compiler" SLOPIUM_HOME="$scratch/fresh-unsigned" bash -c \
  "cd '$scratch/strict' && '$manager' build"

# And the same registry with no keys configured is v0.4.4's behaviour, which is
# what every registry written before this release is in.
write_consumer "$scratch/lax" "$unsigned"
(cd "$scratch/lax" && SLOPIUM_HOME="$scratch/fresh-lax" slopium build) >/dev/null ||
  fail "an unsigned package is refused where no keys are configured"

# `D-060`: a key anybody else on the machine can read is not a key any more.
cp "$key" "$scratch/leaky-key"
chmod 644 "$scratch/leaky-key"
write_geometry "$scratch/geometry-1.1.0" 1.1.0 110
mkdir -p "$scratch/geometry-1.1.0/.slopium"
printf '[registry.default]\nindex = "%s"\n' "$registry" >"$scratch/geometry-1.1.0/.slopium/config.toml"
refuses "a world-readable signing key" "readable by somebody other than you" \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/geometry-1.1.0' && '$manager' publish --key '$scratch/leaky-key'"

# There is no upload protocol, because there is no server (`D-059`).
mkdir -p "$scratch/geometry-1.1.0/.slopium"
printf '[registry.default]\nindex = "https://example.invalid/index"\n' \
  >"$scratch/geometry-1.1.0/.slopium/config.toml"
refuses "publishing to a URL" "only a directory can be published to" \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/geometry-1.1.0' && '$manager' publish --key '$key'"

# `D-054` from the writing side: a manifest that cannot become an index entry is
# refused before the key is even read.
mkdir -p "$scratch/local-dep/src" "$scratch/local-dep/.slopium"
cat >"$scratch/local-dep/Slopium.toml" <<'EOF'
[package]
name = "local-dep"
version = "1.0.0"
source = "src"

[dependencies]
geometry = { path = "../geometry-1.0.0" }
EOF
echo '(fn unused () -> i32 0)' >"$scratch/local-dep/src/lib.slp"
printf '[registry.default]\nindex = "%s"\n' "$registry" >"$scratch/local-dep/.slopium/config.toml"
refuses "publishing a package that depends on a directory" SL1032 \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/local-dep' && '$manager' publish --key '$key'"

# --- a new version, and a dry run ---------------------------------------------

printf '[registry.default]\nindex = "%s"\n' "$registry" \
  >"$scratch/geometry-1.1.0/.slopium/config.toml"
before="$(find "$registry" -type f | sort)"
(cd "$scratch/geometry-1.1.0" && slopium publish --key "$key" --dry-run) >"$scratch/dry" ||
  fail "a dry run failed"
[ "$(find "$registry" -type f | sort)" = "$before" ] ||
  fail "a dry run wrote into the registry"
grep -q "Would publish geometry v1.1.0" "$scratch/dry" ||
  fail "a dry run does not say what it would publish"

publish "$scratch/geometry-1.1.0" >/dev/null || fail "cannot publish geometry v1.1.0"

# The pinned consumer does not move, and a fresh one takes the newer version.
output="$(cd "$scratch/consumer" && slopium run | tail -1)"
[ "$output" = "100" ] || fail "a new publication moved a pinned consumer: $output"
write_consumer "$scratch/newcomer" "$registry" "$public"
output="$(cd "$scratch/newcomer" && slopium run | tail -1)"
[ "$output" = "110" ] || fail "a fresh consumer did not take v1.1.0: $output"

# --- the store, and what verification is for ----------------------------------

# Editing an archive in the store is caught by its digest, before its signature
# is ever consulted — the digest is what the lock records (`D-055`).
digest="$(sed -n 's/.*"checksum":"\([^"]*\)".*/\1/p' \
  <<<"$(grep '"version":"1.0.0"' "$registry/index/ge/om/geometry.json")")"
stored="$SLOPIUM_HOME/archives/$digest.sl.tar"
[ -f "$stored" ] || fail "the store does not hold the archive it verified"
chmod -R u+w "$SLOPIUM_HOME"
printf 'tampered' >>"$stored"
refuses "an edited archive in the store" SL1010 \
  env SLOPIC="$compiler" bash -c \
  "cd '$scratch/consumer' && '$manager' verify"

# --- the committed fixture registry -------------------------------------------

# `tests/registry` is what the Nix bridge builds from, and regenerating it here
# is what keeps it honest: identical bytes out of an identical input is the
# archive format's whole promise (`D-039`), and Ed25519 signs deterministically,
# so a difference is a regression and not a timestamp.
fixture="$scratch/fixture"
mkdir -p "$fixture/registry"
printf 'ed25519-private:%s\n' "$fixture_seed" >"$fixture/key"
chmod 600 "$fixture/key"

write_geometry "$fixture/geometry" 1.0.0 100
mkdir -p "$fixture/geometry/.slopium"
printf '[registry.default]\nindex = "%s"\n' "$fixture/registry" \
  >"$fixture/geometry/.slopium/config.toml"
(cd "$fixture/geometry" && slopium publish --key "$fixture/key") >/dev/null ||
  fail "cannot publish the fixture package"

fixture_public="$(slopium key public "$fixture/key")"
write_consumer "$fixture/consumer" "../registry" "$fixture_public"
# The consumer is resolved where it will live, so the relative index in its
# configuration is the one the lock records.
mkdir -p "$fixture/staged"
cp -r "$fixture/registry" "$fixture/staged/registry"
cp -r "$fixture/consumer" "$fixture/staged/consumer"
(cd "$fixture/staged/consumer" && SLOPIUM_HOME="$scratch/fixture-home" slopium build) >/dev/null ||
  fail "the fixture consumer does not build"
rm -rf "$fixture/staged/consumer/target"

if [ "${SLOPIUM_UPDATE_FIXTURES:-}" = "1" ]; then
  rm -rf "$workspace_dir/tests/registry" "$workspace_dir/tests/consumer"
  cp -r "$fixture/staged/registry" "$workspace_dir/tests/registry"
  cp -r "$fixture/staged/consumer" "$workspace_dir/tests/consumer"
  echo "publish-check: regenerated tests/registry and tests/consumer"
else
  diff -r "$fixture/staged/registry" "$workspace_dir/tests/registry" ||
    fail "tests/registry is not what publishing produces; SLOPIUM_UPDATE_FIXTURES=1 to rewrite it"
  diff -r -x target "$fixture/staged/consumer" "$workspace_dir/tests/consumer" ||
    fail "tests/consumer is not what resolution produces; SLOPIUM_UPDATE_FIXTURES=1 to rewrite it"
fi

echo "publish-check: ok"
