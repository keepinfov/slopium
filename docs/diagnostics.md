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
text rather than JSON — `slopium` reports one error and exits. Packaging is the
only part of it with codes so far (`D-048`):

- `SL1001`: an archive entry names a path outside the package;
- `SL1002`: an archive entry is not a file or a directory;
- `SL1003`: an archive holds more than one package;
- `SL1004`: an archive is malformed;
- `SL1010`: a stored archive does not match the digest it is filed under;
- `SL1011`: a package is not in the store and cannot be fetched;
- `SL1012`: a vendored copy does not match its checksum.

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
