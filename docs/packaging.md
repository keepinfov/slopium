# Packaging

How a Slopium package becomes bytes, what those bytes are addressed by, and
where they are kept. This document is the specification: an implementation that
disagrees with it is wrong, and two implementations that agree with it produce
identical archives.

Every `D-nnn` cited below is an entry in [`decisions.md`](decisions.md), the
project's decision log.

## The archive

A package archive is a **ustar** tar file with every source of variation removed
(`D-039`, `D-045`). It is written by `slopium package` to
`target/package/<name>-<version>.sl.tar`.

Everything that could differ between two machines is pinned:

| field | value |
| --- | --- |
| entry order | sorted by path, byte order |
| `mtime` | `0` |
| `uid`, `gid` | `0`, with empty `uname` and `gname` |
| mode | `0644` for files, `0755` for directories |
| entry types | regular files and directories only |
| prefix | one top-level directory, `<name>-<version>/` |
| trailer | two zero blocks, padded to 10240 bytes |

There is no compression in the format. The digest is over the tar, so it can be
checked with `sha256sum` and two archives can be compared with `cmp`. A
transport is free to compress; that is a property of the transport, not of the
package.

Every directory on the way to a file is an entry of its own, so unpacking never
has to invent one and two archives of the same tree cannot differ over whether
a walk happened to report a directory. Empty directories carry nothing and are
not archived.

Paths longer than 100 bytes use ustar's 155-byte prefix field, split at the last
`/` that makes both halves fit. A path that cannot be split that way is an error
rather than a GNU extension header.

### What goes in

The manifest and the source tree. Left out, always:

- `target/` — build output;
- `.git/` — version control;
- `.slopium/` — this machine's configuration, including any vendoring;
- whatever `[source.*] directory` in `.slopium/config.toml` points at, because
  a vendored copy travels with the configuration that names it, and that stays
  behind;
- `Slopium.lock`, for a library. A library is built inside somebody else's
  graph and its own lock says nothing about how it will resolve there
  (`D-044`). An executable keeps its lock.

Everything else travels, which includes the C a package's `extern` declarations
need. `[package] c-sources` names those files, they are relative to the package
root and may not leave it (`SL1100`), and the default walk therefore always
carries them. An `include` that leaves one out is refused rather than published:
a package missing its C fails at every consumer's link, not at its author's.

`[build] linker-script` is carried the same way and refused the same way
(`SL1101`), and it sits in `[build]` rather than beside `c-sources` because the
two are not the same kind of thing (`D-117`). A package's C is additive: every
dependency's is compiled and linked, and a longer list is a correct answer. A
linker script describes one whole image, so a list of them would be a conflict —
and `[build]` is already the table read from the root package alone, which is
what makes a dependency's script ignored by construction rather than by a rule.
A missing script is worse than a missing C file, because the link succeeds with
the default layout instead of failing.

`[package] exclude` adds to that list; `[package] include` replaces the whole
question with an explicit answer. Giving both is an error — `include` already
says what the package is. `Slopium.toml` is always packaged; an archive without
a manifest is not a package, so `include` does not have to remember it.

Patterns match the path relative to the package root. `*` matches within one
path component and `**` matches across them, so `src/*.slp` is one directory's
modules and `**/*.slp` is all of them. Naming a directory takes everything under
it, which is what makes `exclude = ["notes"]` mean what it reads as.

```toml
[package]
name = "geometry"
version = "1.4.0"
source = "src"
exclude = ["benchmarks", "**/*.png"]
```

### What is refused

A symbolic link, hard link, device node or fifo, in the tree being packaged or
in an archive being read (`SL1002`). An entry whose path is absolute, contains
`..`, or is a file at the archive's top level (`SL1001`). An archive holding two
top-level directories (`SL1003`). A header that fails its own checksum, a
truncated archive, a size that runs past the end (`SL1004`).

A package is source text. Nothing in it should be able to write outside the
directory it is unpacked into, and the reader is written on the assumption that
somebody is trying.

## The store

Fetched packages live in `$SLOPIUM_HOME`, which defaults to
`${XDG_CACHE_HOME:-~/.cache}/slopium`. It is a cache in the strict sense —
deleting it costs a re-fetch and nothing else.

```
$SLOPIUM_HOME/
  archives/<digest>.sl.tar     the package: the exact bytes that were hashed
  store/<digest>/              its unpacked form, a convenience
  git/db/<name>-<hash>/        a bare repository, never checked out
```

The archive is the authority and the tree is derived from it. So:

- an archive is **verified before it is unpacked**, never after — bytes that
  fail their digest never get to write a file (`SL1010`);
- the archive is verified again every time the tree is used, because the tree
  is what a build reads;
- unpacking goes to a temporary directory and is renamed into place, so a build
  arriving halfway through another one's extraction sees the finished tree or
  nothing at all;
- files in the tree are left read-only, so nobody edits a package by accident.
  Directories stay writable: the point is not to make a stale checkout
  impossible to delete.

An archive already in the store is left exactly as it is. Its digest names its
bytes, so rewriting could only be a no-op — or a way to quietly repair a store
somebody has edited, which is the one thing verification exists to notice.

## Git dependencies

```toml
[dependencies]
geometry = { git = "https://example.com/geometry.git" }              # default branch
geometry = { git = "https://example.com/geometry.git", branch = "main" }
geometry = { git = "https://example.com/geometry.git", tag = "v1.4.0" }
geometry = { git = "https://example.com/geometry.git", rev = "0f2c1a9" }
```

Fetching runs `git` (`D-037`). A bare repository is kept per URL under
`$SLOPIUM_HOME/git/db/`, fetched with an explicit refspec so nothing depends on
how a particular git is configured, and never checked out. `GIT_CONFIG_GLOBAL`
and `GIT_CONFIG_SYSTEM` are held away from it: a `url.*.insteadOf` rule would
send a fetch somewhere the lock does not name.

The package itself is `git archive` of the resolved commit, read for its paths
and contents and for nothing else, then written back out through the ordinary
archive writer above (`D-050`). So a git package is the same kind of object as a
published one, addressed the same way, and its digest can be re-derived from the
repository by anyone.

**Resolution always pins a full commit.** A branch is how a commit is found
once, never what is recorded:

```toml
[[package]]
name = "geometry"
version = "1.4.0"
source = "git+https://example.com/geometry.git?branch=main#7c1e0a2f…"
checksum = "9b3f…"
dependencies = []
```

The reference the manifest asked for stays in the query and the commit is the
fragment (`D-049`). Keeping the reference is what lets a manifest that moves
from `branch = "main"` to `branch = "next"` disagree with its own lock; the
commit alone could not. A dependency naming the default branch records no query
at all.

A pinned dependency is **not resolved again**. Once the lock names a commit, the
branch is not consulted and `git` is not run — moving a branch does not move a
build, whether or not `--locked` was given. `--locked` says *do not write a new
lock*; the lock itself says *do not go looking again*.

Some consequences, each of them deliberate:

- a `version` requirement alongside `git` is **checked** against the fetched
  manifest, never used to select — there is one candidate, and it is the commit;
- two dependents naming one package from the same repository by different
  references is an error, the way two registries for one name is (`D-038`);
- submodules are not fetched in v0.4, and a fetched tree holding `.gitmodules`
  says so at resolve time (`SL1021`) rather than building something incomplete;
- a git package may not declare a `path` dependency (`D-051`). It is unpacked
  into the store, so a relative path from it would either escape the package or
  name a directory whose absolute path no lock could portably record.

## Registries

A registry is a directory that some file server serves. There is no protocol
beyond "fetch this path", which is why any file server — or a directory, or
`file://` — is one, and why no registry server lives in this repository.

```
index/<prefix>/<name>.json          one JSON object per line, one line per version
packages/<name>/<name>-<version>.sl.tar
```

`<prefix>` fans the index out by name length: `1/` for one-character names,
`2/`, `3/<first>/`, and `<first two>/<next two>/` beyond that. The index file is
JSON *per line* rather than one document, so publishing appends. A line that
cannot be read is an error naming the file (`SL1036`); an unknown field is
ignored, so a later format can add one without older clients refusing the index.

```json
{"name":"geometry","version":"1.4.0","dependencies":[{"name":"units","requirement":"^2"},{"name":"std","requirement":"^0.4","source":"toolchain"}],"checksum":"9b3f…","yanked":false,"signature":"ed25519:1a2b…:0b4c…"}
```

A dependency naming no `source` means **the registry the entry came from**
(`D-054`), never the consumer's default — that is what stops a package published
to an internal index from being made to reach a public one by how a consumer is
configured. `"source": "toolchain"` is the bundled library. Nothing else is
accepted yet.

### Naming one

```toml
[dependencies]
geometry = "^1.2"                                    # the `default` registry
physics = { version = "^2", registry = "internal" }
```

Registries are configured per checkout, and **this toolchain ships no registry
URL** (`D-053`):

```toml
# .slopium/config.toml
[registry.default]
index = "https://packages.example.com"

[registry.internal]
index = "file:///srv/registry"
```

An unconfigured registry — including `default` — is an error (`SL1030`), not a
download from somewhere nobody chose. `https://` goes through `curl` with a
fixed argument list no configuration can extend; a value with no scheme is a
path relative to the workspace root, because a local registry is a directory;
and `http://` is accepted only for a loopback host, since whoever answers a
plaintext index chooses what a first resolution pins.

The **index URL is the identity**, so the lock records it and never the local
name. Two developers who call one index by two names still produce one lockfile:

```toml
[[package]]
name = "geometry"
version = "1.4.0"
source = "registry+https://packages.example.com"
checksum = "9b3f…"
```

### Selecting a version

The registry is the first source that offers more than one version of a package,
so it is the first that makes selection mean anything. Selection is maximal with
backtracking (`D-036`): the newest version satisfying a requirement, and when
that leaves some other requirement unsatisfiable, an older one. A diamond whose
newest dependent needs a major nobody else accepts resolves to the older
dependent rather than failing.

Requirements come out of the index during the search, because downloading every
candidate to find out what it needs is the cost an index exists to avoid. What
gets built is checked against the archive: a package whose manifest disagrees
with the entry that selected it is refused (`SL1033`), and so is one whose bytes
do not hash to what was published (`SL1034`). The index is trusted to make
resolution fast and for nothing else (`D-055`).

A pinned dependency is not resolved again, exactly as for git — and a fully
pinned graph never reads an index at all, which is what lets `--offline` work
against a populated store.

A yanked version is not selected, but is still built when the lock already names
it: yanking is a statement about new resolutions, and a lockfile that stops
working when somebody edits an index is not a lockfile. A requirement whose only
candidates are yanked says so (`SL1035`).

### Deliberate limits in v0.4

- a published package depends only on its own registry and the toolchain
  (`D-054`). A registry name in a manifest is a local nickname and means nothing
  on the machine that fetched it, and a directory or repository is `D-051`'s
  problem again. All three are `SL1032`.
- two dependents naming one package from two sources is an error (`SL1031`),
  whether the disagreement is registry against registry, registry against git,
  or registry against path.
- a workspace resolves each member separately, so two members whose requirements
  select different versions of one package is an error rather than a joint
  solve. Requirements that overlap select the same version and are unaffected.

### Editing the manifest

```sh
slopium add geometry@^1.2               # from the default registry
slopium add geometry --git https://example.com/geometry.git --tag v1.4.0
slopium add helper --path ../helper
slopium remove geometry
slopium update                          # move every pin its source allows
slopium update -p geometry              # move exactly one
slopium update -p geometry --precise 1.3.0
```

`add` and `remove` edit `Slopium.toml` as text rather than reprinting it from
its parse: a manifest is something a person wrote, and a tool that reformats it
on every touch is one people stop using. The one form they will not touch is a
dependency written as `[dependencies.<name>]`, which they refuse rather than
half-edit.

`update` is what moves a lock, so `--locked` refuses it. `-p` throws away
exactly one pin and leaves the rest, which is what makes the lock's own diff the
proof of what moved.

`slopium package --index-entry` prints the index line for the archive it wrote,
which is what putting a package into a static tree takes. It is also where
`D-054` is enforced from the writing side: a manifest that depends on a
directory or a repository cannot become an index entry.

### A key the toolchain does not know

A manifest is read by every toolchain that ever sees the package, and not only
by the one that wrote it. So a key this toolchain does not know is **reported
and ignored**, not refused (`D-128`):

```text
slopium: warning[SL1200]: `/w/Slopium.toml` sets `edition`, which this toolchain does not know; it is ignored
```

The archive carries the key verbatim — nothing is rewritten on the way through
— and the ignored key changes nothing about what was resolved or built. What
this costs is the typo check that refusing gave for free, which is why the key
is named rather than dropped in silence: a setting that quietly does nothing is
worse than one that is refused.

The warning belongs to the manifests of the workspace being acted on. A
dependency's manifest is the dependency's business, and a consumer that cannot
edit it has nothing to do with the message.

`.slopium/config.toml` is the exception and still refuses. It is this
checkout's own file, written by whoever runs the build and shipped nowhere, so
a key nobody knows there is a mistake rather than a message from a later
version.

## Signing and publishing

```sh
slopium key new ~/.slopium/signing-key   # prints the public half to paste
slopium key public ~/.slopium/signing-key
slopium publish --key ~/.slopium/signing-key
slopium publish --key ~/.slopium/signing-key --registry internal --dry-run
```

`publish` is `package`, plus a signature, plus three files written into a
directory: the archive, a detached `<name>-<version>.sl.tar.sig` beside it, and
one appended index line. Only a directory can be published to. There is no
upload protocol because there is no server (`D-059`) — an `https://` index is
published to by whatever puts files where it serves them.

Before it signs, `publish` unpacks the archive and packs it again and requires
identical bytes. That is what the format was specified for (`D-039`), and the
moment before a signature asserts that these bytes *are* the package is the
moment to find out.

A version already in the index is refused (`SL1043`). An index line is
append-only: somebody's lock may already name that version and that digest, and
a republished version is the one change no lockfile can notice. Yanking is what
exists for taking a version back.

### What a signature says

Ed25519 over a statement, not over the digest alone:

```
slopium-package-v1\n<name>\n<version>\n<digest>\n
```

Signing a bare hash makes a signature transplantable — an attacker chooses the
contents of the package they publish, so two archives can be made identical, and
a signature lifted onto another name or another version would still verify.
Naming the package inside the signed message costs a newline and stops that
(`D-056`).

Keys and signatures are written `<what>:<hex>`: `ed25519:<key>` is a public key,
`ed25519-private:<seed>` is the whole of a key file, and
`ed25519:<key>:<signature>` is a signature. A signature carries the key that
claims to have made it, because 64 opaque bytes cannot say who produced them and
"somebody you have not listed signed this" is a different thing to be told from
"this does not verify". That key is a claim: it is checked against the trusted
list *before* it verifies anything, so a signature can never introduce the key
that makes it acceptable.

A key file is mode 0600 and `publish` refuses one that is not (`D-060`). Key
material is never an argument — `/proc/<pid>/cmdline` is world-readable — and
never an environment variable, which every subprocess a build runs would
inherit.

### Trusting a key

```toml
# .slopium/config.toml
[registry.default]
index = "https://packages.example.com"
trusted-keys = ["ed25519:1a2b…", "ed25519:3c4d…"]
```

A registry with `trusted-keys` admits only archives signed by one of them: no
signature is `SL1040`, a signature that does not verify is `SL1041`, and a
signature by an unlisted key is `SL1042`. A registry with no `trusted-keys` does
not check signatures at all.

There is no third state, and in particular **no trust on first use** (`D-057`).
Remembering whichever key signed the first download would make the first fetch
the trust decision, and the first fetch is exactly the one an attacker who can
answer for an index gets to choose. Rotating a key is adding the new one to the
list, which is why it is a list.

The check runs at every checkout, not only at the download that filled the
store: one `$SLOPIUM_HOME` is shared by every project on a machine, so checking
only on arrival would let a project that trusts nobody leave bytes behind that a
project which does trust somebody then builds unverified (`D-058`). It costs
nothing measurable, works with `--offline`, and means adding a key takes effect
on the next build rather than on the next cache eviction.

```sh
slopium verify          # re-check every locked package: digest, then signature
```

`verify` goes through the same checkout a build does, so what it checks is what
a build would use — and on a machine whose store is empty it fills it, which
makes it the command to run first in a fresh checkout.

## Building from a lock with Nix

`lib.buildSlopiumPackage` reads `Slopium.lock` and builds `--offline --locked`.
Nix does no version selection at all (`D-061`): every registry entry becomes a
fixed-output derivation whose hash is the checksum the lock already records,
those fill a package store, and the build reads it.

```nix
slopium.lib.${system}.buildSlopiumPackage {
  pname = "consumer";
  version = "0.1.0";
  src = ./.;
  # A lock records the index string a manifest was configured with, which may be
  # a relative path; this says where that is.
  registries."../registry" = ./registry;
}
```

That is what makes "Cargo and Nix resolve identical locked graphs" true by
construction rather than by comparison: there is one resolver, it ran once, and
both builds read what it wrote.

Two limits. A git entry is refused by name (`SL1044`) — its checksum is the
digest of an archive the toolchain normalizes out of an exported tree, which Nix
cannot reproduce without running the toolchain, so no fixed-output derivation
can be written for it; `slopium vendor` turns such a graph into one the bridge
builds. And a signature has no hash in the lock, so it is copied out of a
registry directory rather than fetched: a registry reachable only over `https://`
arrives unsigned under Nix, and a build that requires signatures needs a local
copy of it.

## Checksums in the lock

`Slopium.lock` is format 2 and records a `checksum` for every package whose
bytes cannot change under it — the bundled library, a git commit, and a
published version:

```toml
[[package]]
name = "std"
version = "0.6.1"
source = "toolchain"
checksum = "6dbbb0a51db54af6b2e0382db44b087098353f301709a3ee1c9179b365552072"
dependencies = [
    "core",
]
```

The toolchain ships two packages, so a project that depends on `std` locks
`core` beside it: `std` carries `io`, `process` and `fs` and depends on `core`,
which carries `Option`, `Result`, `string` and the language items (`D-082`,
`D-083`). A freestanding project depends on `core` alone. Each has its own
checksum, which is what lets a lock say which of the two changed.

A path dependency has none. It is a working tree, and hashing one would rewrite
the lock on every keystroke. The bundled library has one because it ships inside
the compiler as fixed bytes — which also means a lock notices a toolchain whose
library changed without its version doing so. A git package has one because a
commit names a tree, and the digest of what that tree archives to is what makes
the pin verifiable without trusting git's own hashing: a repository rewritten
under a pinned commit is caught (`SL1022`) rather than believed.

A lock this toolchain cannot read is regenerated, with a line saying so. It is
derived entirely from the manifests, so there is nothing to lose. Under
`--locked` it is an error instead, because `--locked` asked for exactly that.

## Vendoring

```sh
slopium vendor              # into ./vendor
slopium vendor --dir third-party
slopium vendor -p full      # only what one workspace member needs
```

`vendor` copies every dependency that is not already a directory on this machine
into the vendor directory, one subdirectory per package, and writes the
redirection into `.slopium/config.toml`:

```toml
[source.git]
replace-with = "vendored"

[source.toolchain]
replace-with = "vendored"

[source.vendored]
directory = "vendor"
```

Only packages with a checksum are vendored — a path dependency is already a
directory here, and copying it would make a second one that can drift from the
first. The copies come out of the store, so what lands in the vendor directory
has been verified against its digest and unpacked by a reader that refuses to
write outside itself.

**Replacement is invisible to resolution** (`D-047`). A replaced package keeps
its name, its version, its source and its lock entry; only the bytes handed to
the compiler come from somewhere else. Vendoring cannot change what a project
resolves to, and `slopium check --locked` passes across it.

A vendored copy is checked on every build by re-archiving it and comparing
digests (`SL1012`). The format has no room for anything but names and contents,
so the tree that produced a digest is the only tree that reproduces it — which
is what a vendored copy is worth: bytes anyone can re-derive, rather than a
checksum taken on trust. Edit one and the build stops until `slopium vendor`
puts it back.

`vendor` itself resolves as though nothing were vendored. It is what produces
the copies, so requiring them to be intact first would leave an edited copy
unrepairable by the only command that can repair it.

### Vendoring one member

`-p` copies one workspace member's share of the graph instead of all of it. The
lock is unchanged either way — `-p` narrows what is copied, never what was
resolved.

The redirection is workspace-wide, because it names sources rather than
packages, so a member left out of a partial copy stops building `--offline`.
`vendor -p` prints which members those are rather than leaving it to be found
later.

Running `vendor` again over a redirection `vendor` itself wrote appends the
sources the earlier run did not cover, which is what going from `-p` to the
whole workspace needs. A `[source]` table that redirects somewhere else, or does
something this command does not understand, is still refused rather than
rewritten: guessing which half of a hand-written configuration to keep is not
something to do to somebody's checkout.

## `--offline`

`--offline` forbids reaching for bytes that are not already local, and
`--frozen` is `--offline` and `--locked` together. What stays available is the
lock, the package store, any vendored copies, and the index cache below; what is
forbidden is running `git` and downloading anything. When something is missing
it says which package and what digest it wanted (`SL1011`).

So a project that has resolved once builds offline from the store, and a project
that has been vendored builds offline with no store, no `git` and no `curl`
installed at all — the copies in `vendor/` are the whole answer, checked against
the lock's checksums on the way in.

### Resolving offline

An index file this machine has already fetched is kept in
`$SLOPIUM_HOME/index/<digest of the index url>/`, laid out the way the registry
lays out its own `index/` tree, with a `url` file at the top saying which
registry the directory belongs to. A `--offline` run reads that cache, so
`slopium add` and a new requirement in `Slopium.toml` can select a version
without a network — as long as some earlier run fetched that package's index
file.

Three rules keep the cache from being a way to build against something stale:

- an online run always fetches and always overwrites. The cache is a fallback,
  never a shortcut, because a version that has just been published is the whole
  reason to read an index at all;
- an online run that finds a package gone from the index deletes the cached
  copy, so `--offline` never contradicts the last online run;
- a registry that is a directory — `file://` or a relative path — is read
  directly, offline or not. It is already local, and reading it was never a
  network operation.

What is still impossible offline is a package no run has ever fetched the index
of (`SL1011`, naming the path it looked in), and any git dependency the lock
does not already pin: which commit a branch names today is a question only the
repository can answer.
