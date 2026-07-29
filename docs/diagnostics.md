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
- `SL06xx`: compiler I/O and external toolchain.

The v0.2 package and language-core diagnostics reserve:

- `SL0450`: module resolution and visibility;
- `SL0451`: dependency graph or manifest dependency;
- `SL0452`: generic declaration or instantiation;
- `SL0453`: standard-library language-item contract.

Compile-fail fixtures store expected codes and primary byte/line spans
separately from rendered stderr. Intentional snapshot updates require:

```sh
SLOPIUM_UPDATE_SNAPSHOTS=1 cargo test -p slopic-core --test compile_fail
```
