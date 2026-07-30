# Packaging

How a Slopium package becomes bytes, what those bytes are addressed by, and
where they are kept. This document is the specification: an implementation that
disagrees with it is wrong, and two implementations that agree with it produce
identical archives.

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

## Checksums in the lock

`Slopium.lock` is format 2 and records a `checksum` for every package whose
bytes cannot change under it:

```toml
[[package]]
name = "std"
version = "0.4.2"
source = "toolchain"
checksum = "7a8d0238112ead5e7142c9e8714931cf51cc5254b4b47d1eab15d74bd982ee1a"
dependencies = []
```

A path dependency has none. It is a working tree, and hashing one would rewrite
the lock on every keystroke. The bundled library has one because it ships inside
the compiler as fixed bytes — which also means a lock notices a toolchain whose
library changed without its version doing so.

A lock this toolchain cannot read is regenerated, with a line saying so. It is
derived entirely from the manifests, so there is nothing to lose. Under
`--locked` it is an error instead, because `--locked` asked for exactly that.

## Vendoring

```sh
slopium vendor              # into ./vendor
slopium vendor --dir third-party
```

`vendor` copies every dependency that is not already a directory on this machine
into the vendor directory, one subdirectory per package, and writes the
redirection into `.slopium/config.toml`:

```toml
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

## `--offline`

`--offline` forbids reaching for bytes that are not already local, and
`--frozen` is `--offline` and `--locked` together. When a package is missing it
says which one and what digest it wanted (`SL1011`).

In this release every source is local — a directory on this machine, or the
library inside the compiler — so nothing is ever fetched and `--offline` cannot
change the outcome of a build. It is threaded through as one policy value to one
place, so that git and registry sources have somewhere to ask rather than a flag
to reinvent.
