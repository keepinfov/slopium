#!/usr/bin/env bash
# The packaging gate: archives, the store, vendoring, and `--offline`.
#
# Everything here runs against a scratch `SLOPIUM_HOME` and a scratch copy of a
# fixture, so it never touches the developer's own package store and can be run
# twice in a row. Where the toolchain claims something a standard tool also
# knows how to do — hashing bytes, reading a tar — the standard tool is asked,
# the way the object writer is checked against `as` (`D-029`).
set -euo pipefail

workspace_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scratch="$(mktemp -d)"
trap 'chmod -R u+w "$scratch" 2>/dev/null || true; rm -rf "$scratch"' EXIT

cargo build --quiet --workspace --manifest-path "$workspace_dir/Cargo.toml"

compiler="$workspace_dir/target/debug/slopic"
manager="$workspace_dir/target/debug/slopium"
export SLOPIUM_HOME="$scratch/home"

slopium() {
  env SLOPIC="$compiler" "$manager" "$@"
}

fail() {
  echo "package-check: $1" >&2
  exit 1
}

# A package with a dependency on the bundled library, which is the one source
# whose bytes do not change under the lock and so the one with a checksum.
project="$scratch/checkout-one/generics-std"
mkdir -p "$scratch/checkout-one"
cp -r "$workspace_dir/tests/projects/pass/generics-std" "$project"
rm -f "$project/Slopium.lock"
cd "$project"

# --- the archive is a function of the tree, and of nothing else ---------------

slopium package >"$scratch/package.out"
archive="$(awk 'NR==2 {print $2}' "$scratch/package.out")"
digest="$(awk 'NR==2 {print $1}' "$scratch/package.out")"
[[ -f "$archive" ]] || fail "package printed no archive path"

if [[ "$(sha256sum "$archive" | cut -d ' ' -f 1)" != "$digest" ]]; then
  fail "the digest slopium printed is not the one sha256sum computes"
fi
if ! tar -tf "$archive" >"$scratch/listing" 2>"$scratch/tar.err"; then
  fail "tar cannot read the archive: $(cat "$scratch/tar.err")"
fi
grep --quiet '^generics-std-0.2.4/Slopium.toml$' "$scratch/listing" ||
  fail "the archive holds no manifest"
grep --quiet '^generics-std-0.2.4/src/main.slp$' "$scratch/listing" ||
  fail "the archive holds no source"
if grep --quiet '^generics-std-0.2.4/target' "$scratch/listing"; then
  fail "the archive holds build output"
fi
# Nothing in it may carry a timestamp, an owner, or a mode beyond 0644 and 0755.
# In UTC, because a zero `mtime` renders as 1969 in any zone behind it.
TZ=UTC tar -tvf "$archive" >"$scratch/verbose"
if grep --quiet --invert-match '1970-01-01' "$scratch/verbose"; then
  fail "an archive entry carries a timestamp"
fi
if grep --quiet --invert-match ' 0/0 ' "$scratch/verbose"; then
  fail "an archive entry carries an owner"
fi
if grep --quiet --invert-match --extended-regexp '^(-rw-r--r--|drwxr-xr-x)' "$scratch/verbose"; then
  fail "an archive entry carries a mode that is not 0644 or 0755"
fi

cp "$archive" "$scratch/first.tar"
touch "$project/src/main.slp"          # a newer tree is the same package
slopium package >/dev/null
cmp "$scratch/first.tar" "$archive" ||
  fail "two runs of package produced different bytes"

# A second checkout at a different path is the same package, which is the whole
# point of the format.
mkdir -p "$scratch/checkout-two"
cp -r "$workspace_dir/tests/projects/pass/generics-std" "$scratch/checkout-two/generics-std"
rm -f "$scratch/checkout-two/generics-std/Slopium.lock"
(cd "$scratch/checkout-two/generics-std" && slopium package >/dev/null)
cmp "$scratch/first.tar" \
  "$scratch/checkout-two/generics-std/target/package/generics-std-0.2.4.sl.tar" ||
  fail "two checkouts produced different bytes"

# --- a manifest survives a key this toolchain does not know -------------------
#
# A manifest is read by every toolchain that ever sees the package, so a key it
# does not know is reported and ignored rather than refused (`D-128`). The
# archive carries the key verbatim: what is packaged here is what a later
# toolchain reads, and rewriting it on the way through would defeat the point.

cp "$project/Slopium.toml" "$scratch/manifest.known"
{
  printf 'edition = "2031"\n\n'
  sed 's/std = { toolchain = true }/std = { toolchain = true, features = ["io"] }/' \
    "$scratch/manifest.known"
  printf '\n[profile.dev]\nlto = "thin"\n'
} >"$project/Slopium.toml"

slopium check >"$scratch/unknown.out" 2>&1 ||
  fail "a manifest with an unknown key did not build: $(cat "$scratch/unknown.out")"
if [[ "$(grep --count 'SL1200' "$scratch/unknown.out")" != 3 ]]; then
  fail "the three unknown keys were not each reported once: $(cat "$scratch/unknown.out")"
fi
for key in 'edition' 'dependencies.std.features' 'profile.dev.lto'; do
  grep --quiet "sets .$key." "$scratch/unknown.out" ||
    fail "$key was not named: $(cat "$scratch/unknown.out")"
done

slopium package >/dev/null 2>&1
tar -xOf "$archive" generics-std-0.2.4/Slopium.toml >"$scratch/archived.toml"
grep --quiet 'edition = "2031"' "$scratch/archived.toml" ||
  fail "the archive dropped a key the toolchain does not know"

cp "$scratch/manifest.known" "$project/Slopium.toml"
slopium package >/dev/null
cmp "$scratch/first.tar" "$archive" ||
  fail "restoring the manifest did not restore the package"

# --- a package holds files and directories, and nothing else ------------------
#
# The reader refuses a `../` entry and a link entry alike, which the archive
# unit tests forge headers to prove. What a person actually runs into is the
# writer's half: a link in the tree they are packaging.

ln -s /etc/passwd "$project/src/stolen.slp"
if slopium package >"$scratch/symlink.out" 2>&1; then
  fail "a package containing a symbolic link was archived"
fi
grep --quiet 'SL1002' "$scratch/symlink.out" ||
  fail "a symbolic link was refused without SL1002: $(cat "$scratch/symlink.out")"
rm "$project/src/stolen.slp"

# --- the store verifies before it unpacks -------------------------------------

rm -rf "$SLOPIUM_HOME"
slopium vendor >"$scratch/vendor.out"
grep --quiet '^Vendored std ' "$scratch/vendor.out" ||
  fail "vendor did not vendor the bundled library"
[[ -f "$project/vendor/std/Slopium.toml" ]] || fail "the vendored library has no manifest"
[[ -d "$SLOPIUM_HOME/archives" ]] || fail "vendoring did not fill the store"

stored="$(find "$SLOPIUM_HOME/archives" -name '*.sl.tar' | head -n 1)"
std_digest="$(basename "$stored" .sl.tar)"
grep --quiet "checksum = \"$std_digest\"" "$project/Slopium.lock" ||
  fail "the lock does not record the digest the store filed the library under"

# A stored file is read-only, so a package is not edited by accident.
checkout="$SLOPIUM_HOME/store/$std_digest"
if [[ -w "$checkout/src/option.slp" ]]; then
  fail "a checked-out package is writable"
fi

# --- vendoring changes where bytes are read from, and nothing else ------------

cp "$project/Slopium.lock" "$scratch/lock.vendored"
slopium check --locked >/dev/null ||
  fail "vendoring changed what the project resolves to"
cmp "$scratch/lock.vendored" "$project/Slopium.lock" ||
  fail "a build after vendoring rewrote the lock"

slopium build --offline --locked >/dev/null ||
  fail "a vendored project does not build offline"

# --- an edited copy is not the package ----------------------------------------

printf '(export Option)\n' >"$project/vendor/std/src/option.slp"
if slopium check >"$scratch/edited.out" 2>&1; then
  fail "an edited vendored copy was accepted"
fi
grep --quiet 'SL1012' "$scratch/edited.out" ||
  fail "an edited vendored copy was refused without SL1012: $(cat "$scratch/edited.out")"
# And `vendor` can put it back, which is the only way out of that state.
slopium vendor >/dev/null
slopium check >/dev/null || fail "vendor did not repair the edited copy"

# --- an edited store entry is not the package either ---------------------------

python3 - "$stored" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
data = bytearray(path.read_bytes())
data[1024] ^= 1
path.write_bytes(bytes(data))
PY
rm -rf "$SLOPIUM_HOME/store" "$project/vendor"
if slopium vendor >"$scratch/tampered.out" 2>&1; then
  fail "a tampered store entry was unpacked"
fi
grep --quiet 'SL1010' "$scratch/tampered.out" ||
  fail "a tampered store entry was refused without SL1010: $(cat "$scratch/tampered.out")"
if [[ -d "$SLOPIUM_HOME/store/$std_digest" ]]; then
  fail "a tampered archive was unpacked before it was verified"
fi

echo "package-check: archives, store, vendor and offline ... ok"
