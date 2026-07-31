# Test trees

- `projects/` — end-to-end feature fixtures run by `scripts/project-tests.sh`.
  See its own README.
- `registry/` and `consumer/` — a whole published registry and a project that
  depends on it, both **generated** by `scripts/publish-check.sh`.

## Why the registry is committed

`nix flake check` builds `checks.lock-build`: the consumer, built by
`lib.buildSlopiumPackage` from its own `Slopium.lock`. Every registry entry
becomes a fixed-output derivation, and a fixed-output derivation needs a store
path to fetch from — so the registry has to be part of the source tree rather
than something a script makes at test time, the way `registry-check.sh` and
`git-check.sh` make theirs.

Committing it buys a second thing. `publish-check.sh` regenerates both trees on
every run and requires `diff -r` to be empty, which turns two claims into
assertions: that a package archive reproduces itself byte for byte (`D-039`) and
that Ed25519 signs deterministically. A difference here is a regression, not a
timestamp.

## Regenerating

```sh
SLOPIUM_UPDATE_FIXTURES=1 scripts/publish-check.sh
```

The signing key is a constant written into that script rather than a file kept
here, so nobody has to weigh up whether a key checked into git is a secret. It
is a visible pattern, it signs these two files, and it is public by
construction. Never publish anything real under it.

`consumer/.slopium/config.toml` is committed even though `.slopium/` is
otherwise ignored: it holds the index path and the trusted key the check
resolves against, so a clone without it cannot run the check.
