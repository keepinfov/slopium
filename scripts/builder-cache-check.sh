#!/usr/bin/env bash
# Isolate field-layout invalidation with an unchanged compiler and consumer.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
slopic="${SLOPIC:-$root/target/debug/slopic}"
manager="${SLOPIUM:-$root/target/debug/slopium}"
target="${1:-x86_64-unknown-linux-gnu}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/src"
cat > "$work/Slopium.toml" <<'TOML'
[package]
name = "builder-cache"
version = "0.1.0"
entry = "src/main.slp"
[dependencies]
std = { toolchain = true }
TOML
cat > "$work/src/main.slp" <<'SLP'
(take builder Builder new answer)
(take std:io println-i64)
(fn main () -> i32
  (let original (new))
  (let copied (clone original))
  (println-i64 (answer &copied))
  0)
SLP
cat > "$work/src/builder.slp" <<'SLP'
(export Builder new answer)
(struct Builder ((bytes (List u8))))
(fn new () -> Builder (Builder :bytes (list (as u8 42))))
(fn answer ((value &Builder)) -> i64
  (match value ((Builder :bytes bytes) (as i64 (get bytes 0)))))
SLP
build() {
    if ! SLOPIC="$slopic" "$manager" --manifest-path "$work/Slopium.toml" \
        build --target "$target" > "$work/build.log" 2>&1; then
        cat "$work/build.log" >&2
        exit 1
    fi
    [[ "$("$work/target/$target/dev/builder-cache")" == 42 ]]
}
build
stamp="$work/target/$target/dev/objects/builder-cache/6d61696e.slop-cache"
before="$(sha256sum "$stamp" | cut -d ' ' -f 1)"
cat > "$work/src/builder.slp" <<'SLP'
(export Builder new answer)
(struct Builder ((words (List u64)) (tail u64) (used i64)))
(fn new () -> Builder
  (Builder :words (list (as u64 65)) :tail 32 :used 10))
(fn answer ((value &Builder)) -> i64
  (match value
    ((Builder :words _ :tail tail :used used) (+ (as i64 (clone tail)) (clone used)))))
SLP
build
after="$(sha256sum "$stamp" | cut -d ' ' -f 1)"
if [[ "$before" == "$after" ]]; then
    echo 'builder-cache-check: field layout did not invalidate the consumer' >&2
    exit 1
fi
build
[[ "$after" == "$(sha256sum "$stamp" | cut -d ' ' -f 1)" ]]
echo 'builder-cache-check: changed fields rebuild clone consumers; repeated build is fresh ... ok'
