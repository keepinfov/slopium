# Changelog

Notable changes to the language, the toolchain and the standard library.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
with the pre-1.0 reading in `AGENTS.md` §7: a minor for a capability, a patch
for a behaviour fix inside one.

A change adds its line under `[Unreleased]`; a release renames that heading and
opens a new one. Everything up to `0.9.2` was reconstructed from the tags, where
each version is exactly one commit, so those entries say what the version did
rather than everything it touched.

## [Unreleased]

### Added
- A declaration can say which target it is for: `(fn (target
  "aarch64-unknown-linux-gnu") ...)`, and the same for a `const`, a `struct`,
  an `enum`, an `extern` and a `test`. A declaration for another target is
  removed before anything types it, so nothing downstream carries code nobody
  compiles. The string is a target triple, spelled as `slopium targets` prints
  it. A name whose only declaration was for another target is an unknown name,
  and the refusal says which target declares it rather than leaving that to be
  worked out.

## [0.11.0] - 2026-08-20

### Added
- `core:string` writes hexadecimal, re-exported by `std:string`:
  `hex-from-u64` for the digits and `hex-prefixed-from-u64` for the same under
  `0x`. The width pads and never truncates, and the glyphs are uppercase, so
  what is printed can be pasted back into a program as a literal.
- A program can fail on purpose: `core:panic` — `std:panic` for a hosted
  package — has `panic`, `assert` and `unreachable`, each ending the program
  with status 101 and a message on standard error.
- A failing test says what it compared. `std:test` has `equal-i64`,
  `equal-u64` and `equal-text`, which answer as `=` does and leave a note the
  harness prints beside `FAILED`.
- `when` is the one-sided conditional: `(when condition body ...)` runs the body
  when the condition holds and answers `unit` either way, so `()` no longer
  stands in for a branch nobody wrote.
- A `match` arm and the `else` branch of an `if` take as many expressions as
  they need and answer the last, which is where `(do ...)` used to be written.
- A temporary can be borrowed where a call takes it: `(println (& "hello"))` and
  `(println (& (from-i64 id)))` are programs now. The value lives until that
  call returns and is dropped there, so an argument is the only position it is
  allowed in, and a borrow of a temporary bound with `let` is refused with a
  message that says to name the value instead.

### Changed
- A manifest key this toolchain does not know is reported as
  `warning[SL1200]` and ignored, instead of refusing the manifest, so a package
  written for a later toolchain still resolves and builds. The archive carries
  the key unchanged. `.slopium/config.toml` still refuses one.
- The library's six integer and float printers, and the float formatter, lost
  the bindings that existed only to give a value a name.
- The standard library, the fixtures and the examples are written in the new
  forms: fifty-seven `(do ...)` blocks are gone, and not one branch is written
  `()` any more.

## [0.10.0] - 2026-08-20

### Added
- The C boundary carries three more shapes: C can fill a `(&mut (List T))` or a
  `(&mut (Array T N))`, write through a `(&mut ...)` out-parameter of a
  word-width scalar, and call back into a named `fn` passed as a `(Fn ...)`.
  Aggregates by value, an exclusively borrowed `(Slice T)`, a narrow
  out-parameter and a closure as a callback are refused by name, each with a
  note saying why.
- `docs/decisions.md`: the project's decision log, `D-001` onwards, so that the
  identifiers cited across the documentation and the commit history resolve to
  something a clone contains.

## [0.9.2] - 2026-08-19

### Added
- A declaration can carry annotations, written as lists between the keyword and
  the name: `(fn (inline) hot ((x i64)) -> i64 ...)`. Every form has the slot.
- `inline` raises the optimizer's size ceiling for a function; `deprecated`
  warns at every use, with its message as a note.
- Warnings exist as a severity: a program that compiles can still be reported
  on, under the `SL08xx` family, and each one is printed once per build.

### Fixed
- A `const` was not part of a module's interface, so changing its value left
  every dependent holding the old number until it was rebuilt for other reasons.

## [0.9.1] - 2026-08-18

### Added
- A module-level `const` over a literal, inlined at every use.
- A `let` and a `const` can carry the type of their value: `(let total 0 : u8)`.
- `(break value)` makes a `loop` an expression.
- Match guards, written flat: `((pattern) when condition body)`.

### Changed
- Shadowing is allowed: `(let x ...)` twice in a scope rebinds rather than
  failing.

## [0.9.0] - 2026-08-18

### Added
- A `match` looks through a `(&mut ...)` and binds each field as a `(&mut ...)`
  of itself; `set` writes to such a name, dropping what was there.

### Changed
- `core:map` and `core:set` write through a borrow — `map-insert`, `map-delete`,
  `set-add` and `set-discard` take `(&mut ...)` and return `unit` instead of
  consuming the container and handing it back.

## [0.8.5] - 2026-08-17

### Added
- A kernel fixture that boots on bare metal under `qemu-system-x86_64`, prints
  through a volatile framebuffer and drives a serial port, with the UART driver
  written in Slopium.

## [0.8.4] - 2026-08-17

### Added
- `x86_64-unknown-none`: a freestanding build is a `--target` rather than a
  mode, and the toolchain performs the link itself.
- `[build] linker-script` in the manifest, validated, archived and hashed into
  the build cache key.

### Fixed
- A freestanding `slopium test` used to produce a binary that ran no test and
  said nothing. It refuses now, and says why.

## [0.8.3] - 2026-08-17

### Changed
- Every function owns the section its code sits in, so `--gc-sections` drops
  what nothing calls. A sample program linked 12 of the 73 functions its objects
  defined.

## [0.8.2] - 2026-08-17

### Added
- Raw pointers: `(Ptr T)` over a scalar pointee, an `unsafe` block,
  `volatile-read`, `volatile-write`, `ptr-offset`, and `as` between a pointer
  and an integer in both directions.
- `(Ptr T)` joins the `extern` vocabulary, and both backends do 1, 2, 4 and
  8-byte memory for raw pointers.

## [0.8.1] - 2026-08-16

### Added
- The eight integer types `i8` through `u64`, with `as` as a real conversion
  table over all 64 pairs.
- `digits-of`, `from-u64`, `to-u64`, `print-u64` and `println-u64` in the
  library.

### Fixed
- A narrow parameter arriving from C is canonicalised on entry; the System V ABI
  leaves the upper half of a narrow argument register undefined.

## [0.8.0] - 2026-08-16

### Added
- The operators a kernel needs: `%`, `<=`, `>=`, `!=`, `and`, `or`, `not`,
  `bit-and`, `bit-or`, `bit-xor`, `bit-not`, `shl`, `shr`, and unary `(- x)`.
- Hexadecimal, binary and digit-separated integer literals, and the `\0` and
  `\xNN` escapes.

### Changed
- A string literal carries bytes rather than text, lexer to object file.

## [0.7.5] - 2026-08-16

### Added
- `Map` and `Set` as library types written in Slopium over a hash function and
  an equality function.
- `replace` on a list: the one write an element did not have.

### Fixed
- Three holes in generic inference, found by writing the containers above.

## [0.7.4] - 2026-08-15

### Added
- `lambda` closures, which name what they capture and move it in.

### Changed
- A function value is owned rather than `Copy`, so passing one twice needs a
  borrow.

## [0.7.3] - 2026-08-12

### Added
- A borrow can be read with `clone` and matched through without taking the value
  apart.

### Fixed
- A miscompile that had shipped since 0.7.0.

## [0.7.2] - 2026-08-11

### Added
- `core:float` and `std:float`: an `f64` prints as plain decimal, seventeen
  significant digits, ties to even, no exponent, with no C library under it.

## [0.7.1] - 2026-08-10

### Added
- The library combinators that refusing traits had cost — over lists, `Option`
  and `Result`, written over function values.

## [0.7.0] - 2026-08-09

### Added
- A function is a value: `(Fn ...)` types, function references, and calls
  through one on both backends.

## [0.6.1] - 2026-08-07

### Added
- `(as i64 value)` conversions, and `clone` across a borrow.

## [0.6.0] - 2026-08-07

### Changed
- `=` compares scalars and nothing else.

## [0.5.3] - 2026-08-07

### Added
- `core:string`, `std:fs`, and a filled-out `std:process`.

### Changed
- Nothing in the library aborts: end of input, a byte that is not a digit, a
  missing argument and a failed file operation are all values now.

## [0.5.2] - 2026-08-05

### Changed
- The runtime and the library each split into a core half and a hosted half, so
  a freestanding program links the half that needs no libc.

## [0.5.1] - 2026-08-04

### Changed
- Input and output left the compiler for `std:io` and `std:process`, written in
  Slopium over the C FFI. `print`, `println`, `read-line` and their neighbours
  are library names rather than builtins.

## [0.5.0] - 2026-07-31

### Added
- `extern` declarations: a C function with a Slopium signature, and
  `[package] c-sources` to compile the C beside it.

## [0.4.6] - 2026-07-31

### Added
- `--offline` resolves from a cached index rather than refusing to resolve.

### Changed
- Every refusal the manager makes carries a stable `SL10xx` code.

## [0.4.5] - 2026-07-31

### Added
- Ed25519 signing and verification of published packages, and a build from a
  lock alone.

## [0.4.4] - 2026-07-31

### Added
- Registry dependencies, resolved by backtracking search, with `slopium add`,
  `remove`, `update` and `tree`.

## [0.4.3] - 2026-07-31

### Added
- Git dependencies, pinned to a commit and to the digest of their archive.

## [0.4.2] - 2026-07-31

### Added
- `slopium package` and `slopium vendor`, over a content-addressed store, with
  reproducible archives.

## [0.4.1] - 2026-07-31

### Added
- Workspaces: one lock and one `target/` at the root, `-p` and `--workspace`
  selection, and inherited dependency tables.

## [0.4.0] - 2026-07-31

### Added
- Packages resolved by name and version, and `Slopium.lock`.

## [0.3.7] - 2026-07-30

### Changed
- Trap strings nothing can reach are no longer shipped, and a panic can abort
  instead of unwinding a message.

## [0.3.6] - 2026-07-30

### Changed
- Smaller binaries, and a compiler that is handed its roots rather than
  discovering them.

## [0.3.5] - 2026-07-30

### Added
- A relocatable ELF object writer, so an ordinary build needs no external
  assembler.

## [0.3.4] - 2026-07-30

### Added
- The AArch64 backend, checked against the x86-64 one program by program.

## [0.3.3] - 2026-07-30

### Added
- `--debug`: DWARF line tables, breakpoints by `file:line`, stepping and
  backtraces.

## [0.3.2] - 2026-07-30

### Added
- Linear-scan register allocation in both profiles.

## [0.3.1] - 2026-07-29

### Added
- The release optimizer: bounded inlining, constant propagation, CFG
  simplification and dead code elimination, verified after every pass.

## [0.3.0] - 2026-07-29

### Added
- MIR: a verified intermediate representation between the checker and the
  backends, printable with `--emit mir-text`.

## [0.2.4] - 2026-07-29

The baseline this changelog was reconstructed from: the language core, the
package manager, the language server and the Neovim plugin as they stood when
tagging began.

[Unreleased]: https://github.com/keepinfov/slopium/compare/v0.11.0...HEAD
[0.11.0]: https://github.com/keepinfov/slopium/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/keepinfov/slopium/compare/v0.9.2...v0.10.0
[0.9.2]: https://github.com/keepinfov/slopium/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/keepinfov/slopium/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/keepinfov/slopium/compare/v0.8.5...v0.9.0
[0.8.5]: https://github.com/keepinfov/slopium/compare/v0.8.4...v0.8.5
[0.8.4]: https://github.com/keepinfov/slopium/compare/v0.8.3...v0.8.4
[0.8.3]: https://github.com/keepinfov/slopium/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/keepinfov/slopium/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/keepinfov/slopium/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/keepinfov/slopium/compare/v0.7.5...v0.8.0
[0.7.5]: https://github.com/keepinfov/slopium/compare/v0.7.4...v0.7.5
[0.7.4]: https://github.com/keepinfov/slopium/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/keepinfov/slopium/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/keepinfov/slopium/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/keepinfov/slopium/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/keepinfov/slopium/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/keepinfov/slopium/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/keepinfov/slopium/compare/v0.5.3...v0.6.0
[0.5.3]: https://github.com/keepinfov/slopium/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/keepinfov/slopium/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/keepinfov/slopium/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/keepinfov/slopium/compare/v0.4.6...v0.5.0
[0.4.6]: https://github.com/keepinfov/slopium/compare/v0.4.5...v0.4.6
[0.4.5]: https://github.com/keepinfov/slopium/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/keepinfov/slopium/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/keepinfov/slopium/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/keepinfov/slopium/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/keepinfov/slopium/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/keepinfov/slopium/compare/v0.3.7...v0.4.0
[0.3.7]: https://github.com/keepinfov/slopium/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/keepinfov/slopium/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/keepinfov/slopium/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/keepinfov/slopium/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/keepinfov/slopium/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/keepinfov/slopium/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/keepinfov/slopium/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/keepinfov/slopium/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/keepinfov/slopium/releases/tag/v0.2.4
