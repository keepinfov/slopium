# Diagnostic contract

Compiler diagnostics are emitted as human-readable text by default and as one
JSON object per stderr line with `--diagnostic-format json`.

Every `D-nnn` cited below is an entry in [`decisions.md`](decisions.md), the
project's decision log.

Every diagnostic contains the compatibility fields `severity`, `message`,
`file`, `span`, and `help`, plus a stable `code`. Optional `labels`, `notes`,
and `suggestions` enrich newer consumers without breaking clients that ignore
unknown fields. Suggestions carry a byte span, replacement text, explanation,
and applicability.

Code families are stable:

- `SL00xx`: lexer and S-expression parser, and the reader's abbreviation table
  between them — `SL0006` for a sigil that is reserved rather than built, whose
  note says what the character is kept for, and `SL0007` for an abbreviation
  with nothing to expand (`D-149`);
- `SL01xx`: declaration and expression shape;
- `SL02xx`: name resolution and types;
- `SL03xx`: ownership and borrowing, and `SL0301` for a raw-pointer operation
  written outside an `unsafe` block;
- `SL04xx`: pattern matching and entry/test rules;
- `SL05xx`: target and ABI backend;
- `SL06xx`: compiler I/O and external toolchain;
- `SL07xx`: internal compiler errors;
- `SL08xx`: **warnings** — a program that compiles, about which the compiler
  has something to say. `SL0800` is a use of a declaration somebody annotated
  `deprecated` (`D-122`).

Every family but the last is a refusal. A warning carries `severity` of
`warning` rather than `error`, renders as `warning[SL08xx]`, and leaves the
compilation successful: `slopic` prints it and exits `0`. Which compilation
reports a given warning is decided by what it was asked to build — a warning
about a dependency's own source belongs to the dependency, and when `slopium`
builds one object per module, only the module being compiled reports its own.
So a build prints each warning once, and a module whose object was already
fresh prints nothing.

`SL10xx` belongs to the project manager rather than the compiler, and is plain
text rather than JSON — `slopium` reports one error and exits. `SL12xx` is the
manager's **warning** family, which renders as `warning[SL12xx]` on standard
error and leaves the command running:

- `SL1200`: a manifest sets a key this toolchain does not know, and the key is
  ignored (`D-128`). It is raised for the manifests of the workspace being
  acted on and for no others, because a dependency's manifest is the
  dependency's business — the same rule `SL08xx` follows about a dependency's
  source.

A code marks **a refusal about something somebody wrote** — a manifest field, a
dependency entry, a selection, a graph that cannot exist — which is a thing to
look up. It does not mark a failure to *do* something: `cannot read
'/x/Slopium.toml': No such file or directory` is the operating system's own
explanation, and a number in front of it would add nothing. Uncoded manager
messages are that kind, deliberately (`D-071`).

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

The manifest, the workspace, the graph, the lock and the build:

- `SL1050`: a dependency entry names no source, or names several;
- `SL1051`: a git dependency's reference is wrong — two of `branch`/`tag`/`rev`,
  or a reference with no repository to take it from;
- `SL1052`: `workspace = true` cannot be satisfied — there is no workspace, no
  entry to inherit, or a source is named alongside it;
- `SL1053`: a manifest field is missing or has the wrong shape, including a
  package with no `entry` where a module to start from is needed;
- `SL1054`: a `[source.*]` table in `.slopium/config.toml` is incomplete or
  points at nothing;
- `SL1060`: the selection is ambiguous or contradictory — `--workspace` with
  `--package`, or several members and neither;
- `SL1061`: a named package is not a member of this workspace;
- `SL1062`: a package sits inside a workspace without being listed in
  `[workspace] members`;
- `SL1063`: `members` is malformed, names a directory that is not there, or two
  members share a name;
- `SL1070`: a dependency cycle;
- `SL1071`: the key in `[dependencies]` is not the name of the package found
  there (`D-040`);
- `SL1072`: one name is required at two versions;
- `SL1073`: no published version satisfies a requirement;
- `SL1074`: two packages define `[language-items]` (`D-041`);
- `SL1075`: a replaced or vendored package is missing, or is a different
  package;
- `SL1076`: a git package declares a `path` dependency (`D-051`);
- `SL1077`: the toolchain source is named for something other than a bundled
  package — they are `core` and `std` (`D-082`);
- `SL1078`: a lock entry needs a checksum and has none;
- `SL1080`: `Slopium.lock` is malformed;
- `SL1081`: `Slopium.lock` is a format version this toolchain does not write;
- `SL1082`: `--locked` was given and the lock would have to change;
- `SL1090`: the compiler and the manager disagree about the protocol version;
- `SL1100`: a `c-sources` entry is absolute or leaves the package;
- `SL1101`: `[build] linker-script` is absolute or leaves the package;
- `SL1102`: a `[target."<triple>"] modules` entry names no file, leaves the
  package, or is absolute.

Resolution is spread over two families: `SL103x` keeps the registry errors that
happen during resolution, because a stable code that moves is not stable
(`D-071`).

The v0.2 package and language-core diagnostics reserve:

- `SL0450`: module resolution and visibility;
- `SL0451`: dependency graph or manifest dependency;
- `SL0452`: generic declaration or instantiation;
- `SL0453`: standard-library language-item contract.

`SL0700` reports a failed internal consistency check, such as MIR verification
or an optimizer analysis that did not settle within its bound. It is never
caused by the source being wrong, only by a compiler bug, and should be
reported as one. MIR verification runs in debug builds and whenever
`SLOPIUM_VERIFY_MIR=1` is set, so a release compiler can be checked without
paying for the analysis by default — with one exception: the check that every
terminator names a block that exists runs in every profile, because the release
optimizer indexes blocks by those targets and a bad one is a panic rather than
a diagnostic (`D-132`).

Compile-fail fixtures store expected codes and primary byte/line spans
separately from rendered stderr. Intentional snapshot updates require:

```sh
SLOPIUM_UPDATE_SNAPSHOTS=1 cargo test -p slopic-core --test compile_fail
```
