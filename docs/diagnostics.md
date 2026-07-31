# Diagnostic contract

Compiler diagnostics are emitted as human-readable text by default and as one
JSON object per stderr line with `--diagnostic-format json`.

Every diagnostic contains the compatibility fields `severity`, `message`,
`file`, `span`, and `help`, plus a stable `code`. Optional `labels`, `notes`,
and `suggestions` enrich newer consumers without breaking clients that ignore
unknown fields. Suggestions carry a byte span, replacement text, explanation,
and applicability.

Code families are stable:

- `SL00xx`: lexer and S-expression parser;
- `SL01xx`: declaration and expression shape;
- `SL02xx`: name resolution and types;
- `SL03xx`: ownership and borrowing;
- `SL04xx`: pattern matching and entry/test rules;
- `SL05xx`: target and ABI backend;
- `SL06xx`: compiler I/O and external toolchain;
- `SL07xx`: internal compiler errors.

`SL10xx` belongs to the project manager rather than the compiler, and is plain
text rather than JSON — `slopium` reports one error and exits. Packaging and
fetching are the only parts of it with codes so far (`D-048`):

- `SL1001`: an archive entry names a path outside the package;
- `SL1002`: an archive entry is not a file or a directory;
- `SL1003`: an archive holds more than one package;
- `SL1004`: an archive is malformed;
- `SL1010`: a stored archive does not match the digest it is filed under;
- `SL1011`: a package is not in the store and cannot be fetched;
- `SL1012`: a vendored copy does not match its checksum;
- `SL1020`: a `git` command could not be run, or failed;
- `SL1021`: a fetched package uses submodules, which v0.4 does not fetch;
- `SL1022`: a pinned commit no longer archives to the digest the lock records;
- `SL1023`: a git reference names no commit in the repository;
- `SL1030`: a registry a manifest or a lock names is not configured, or its
  index is not one this toolchain can reach;
- `SL1031`: one package name is required from two different sources (`D-038`);
- `SL1032`: a published package depends on something it may not — a directory,
  a repository, or a registry it names by a local nickname (`D-054`);
- `SL1033`: a fetched package's manifest disagrees with the index entry that
  selected it (`D-055`);
- `SL1034`: a downloaded archive does not hash to what the index published;
- `SL1035`: every version that would satisfy a requirement is yanked;
- `SL1036`: an index file is malformed;
- `SL1037`: an index or a package could not be fetched;
- `SL1040`: a registry has `trusted-keys`, and a package from it is unsigned;
- `SL1041`: a signature by a trusted key does not verify the package it is
  filed with (`D-056`);
- `SL1042`: a package is signed by a key that is not in `trusted-keys` — which
  is also what a publisher's key rotation looks like, so the message names the
  key to add;
- `SL1043`: a version is already in the index, and an index line is append-only
  (`D-059`);
- `SL1044`: the Nix bridge cannot fetch a locked source. It is thrown during
  evaluation rather than printed by `slopium`, because that is where the
  refusal happens (`D-061`).

The v0.2 package and language-core diagnostics reserve:

- `SL0450`: module resolution and visibility;
- `SL0451`: dependency graph or manifest dependency;
- `SL0452`: generic declaration or instantiation;
- `SL0453`: standard-library language-item contract.

`SL0700` reports a failed internal consistency check, such as MIR verification.
It is never caused by the source being wrong, only by a compiler bug, and
should be reported as one. MIR verification runs in debug builds and whenever
`SLOPIUM_VERIFY_MIR=1` is set, so a release compiler can be checked without
paying for the analysis by default.

Compile-fail fixtures store expected codes and primary byte/line spans
separately from rendered stderr. Intentional snapshot updates require:

```sh
SLOPIUM_UPDATE_SNAPSHOTS=1 cargo test -p slopic-core --test compile_fail
```
