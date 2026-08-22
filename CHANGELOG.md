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

## [0.15.1] - 2026-08-22

### Changed
- `(& mut T)` is refused by a message that says `&mut` is one token and `& mut`
  is two, in type position and where a value is borrowed alike, rather than by
  `invalid type` or by a complaint about a call head that is not a name. A
  sigil is matched as a whole atom, so the space a reader arriving from Rust
  writes turns one borrow into a borrow of `mut` standing before the operand,
  and that is what the help now says.

### Fixed
- A function whose code does not fit in the megabyte an AArch64 conditional
  branch reaches is refused with `SL0502` naming the function, instead of
  reaching the object writer and coming back as an internal error whose advice
  was to assemble the program by hand. `docs/architecture.md` says what the
  limit is and that it bounds one function rather than a module or a program.
- A `slopium build` that fails because the program does not compile says
  `build failed`, the way `check` does, instead of naming whichever module's
  object the loop happened to be asking for. That module had compiled fine and
  was usually one from the standard library, so the last line of the output
  sent a reader looking for a bug a long way from the file the diagnostic above
  it named. A build that fails for any other reason still names the module.
- A type that borrows a borrow is refused where it is written: `&&String` in a
  parameter, a result or a field is `SL0200` at the type, rather than a
  declaration that compiles and cannot be called. A generic reaching the same
  shape — a parameter `&T` whose `T` an earlier argument bound to `&String` —
  is refused at the argument that asked for it, instead of a mismatch naming a
  type no value can have.
- `slopium fmt` keeps the parentheses of `(& mut x)` rather than writing it
  `&mut x`, which is the exclusive borrow and a different program. `&` is the
  one sigil another begins with, so it is the one place where an abbreviation
  written short would fuse with what stands after it.
- `SL0800` marks a deprecated field at the field in all three places it can be
  named. A struct pattern used to carry the caret on the local the pattern
  introduces, which the author is free to call anything, where a construction
  and a read already pointed at the field itself. A struct pattern that names
  one field twice is marked the same way, on the keyword rather than on the
  binding beside it.


## [0.15.0] - 2026-08-22

### Added
- `deprecated` applies to a `struct`'s field, written before the name where
  every declaration with a keyword carries one: every read, construction and
  pattern that names the field warns with `SL0800` and the annotation's message
  as a note. A write goes through a pattern, so those three are all the places a
  field name appears. It is interface, so a module that names the field is
  rebuilt when the annotation is added or taken away.
- A borrow is written `&x` and `&mut x`, a sigil standing before the one form it
  applies to. The reader expands it before anything reads a tree, so the object
  a file compiles to is byte-identical whichever spelling it was written in, and
  `(& x)` stays legal — a sigil that opens a list its own operand ends is the
  head of that list. `slopium fmt` writes the short spelling, which is what
  respelt the 752 sites in the bundled library and the fixtures. `'`, `` ` ``
  and `,` are reserved for the macros this language has not built, and writing
  one is refused with `SL0006` naming what it is held for.
- `|)` closes every list a declaration left open, back to the top level, so the
  run of closing parens that ends one is written as the single token a reader
  can check without counting. `slopium fmt` writes it wherever that run is
  longer than three. It is a resynchronisation point as much as an
  abbreviation: a `)` lost inside a module used to swallow every declaration
  after it and surface as one `SL0004` at the end of the file, and now cannot
  leave the declaration it was written in. `|` ends a token wherever it appears,
  and the shipped Neovim plugin has its own indenter, because Vim's built-in
  Lisp one cannot be taught another closer.
- `$` opens a list that closes where the form holding it closes, so a chain of
  single-argument wrappers is written in the order the calls happen rather than
  in the order the parentheses close: `(a $ b $ c d)` is `(a (b (c d)))`. It is
  a row of the reader's abbreviation table beside `&`, expanded before anything
  reads a tree, and a sigil before one applies to everything after it —
  `(note $ & $ disagreement left right)` borrows the whole call. `slopium fmt`
  neither writes a `$` nor removes one.

### Fixed
- `slopium fmt` measures a form against the closing parens that will follow it
  on the same line, so a line whose form fitted only because the parens closing
  every enclosing form were not counted now breaks instead of running past the
  preferred width.


## [0.14.0] - 2026-08-21

### Added
- `core:builder` and `std:builder` build a string out of many pieces in one
  growing buffer, with `new`, `write-str`, `write-byte`, `write-i64`,
  `write-u64`, `size` and `build`, and `write-f64` in `std:float`. Accumulating
  with `concat` copies everything written so far on every piece; a document of
  40,000 records that took 41.0s to build that way takes 88ms through a builder.
- `std:time` reads the clock — `monotonic` for a duration and `realtime` for a
  timestamp, both nanoseconds in an `i64` — and `std:random` draws entropy with
  `bytes` and `number`. Both are Slopium over the runtime, and a call that fails
  is an `Err` carrying its `errno` rather than an abort.
- `std:process` starts a child: `spawn` leaves its output where this program's
  is, `capture` gives it a pipe and hands back the read end, and `wait` answers
  the exit status. A `Child` owns nothing, so the descriptor is closed by a
  `defer` written beside the call that opened it.
- A list has `insert`, `swap`, `clear` and `truncate`, in `core:list` and
  `std:list`, and `sort-by` is a merge sort over the indices with the
  permutation applied by `swap` instead of a selection sort that moved the tail
  of the list on every placement. Sorting 10,000 scrambled integers took 3,262ms
  and takes 21ms.

### Changed
- `slopium fmt` lays a form out instead of wrapping it: a form fits on its line
  or its arguments go one per line, a declaration's body starts below its
  signature, and an `export` packs rather than becoming a column. A break can no
  longer land inside an argument, which is what column-88 wrapping used to do.

### Fixed
- A refusal about ownership or borrowing is reported as `SL0300` wherever it is
  raised. Eight of them — using, moving or assigning to a value that is
  borrowed, and the four ways two borrows can conflict — used to arrive as
  `SL0200`, which `docs/diagnostics.md` reserves for names and types.


## [0.13.0] - 2026-08-21

### Added
- `=` and `!=` compare two values of an enum no variant of which carries
  anything, so asking whether a state is `(Status:Done)` no longer costs a
  `match`. Such an enum is now represented as its tag — one machine word,
  copied rather than owned — so nothing is allocated to build one, nothing is
  freed when it dies, and comparing one does not consume it. An enum that does
  carry something keeps the representation it had, and comparing two of those
  is still refused, now with a message pointing at `match`.
- `<<` and `>>` compose functions and take as many as they are given:
  `((<< f g h) x)` is `(f (g (h x)))` and `((>> f g h) x)` is the same chain
  written in the order it happens. Applied where it is written, a composition
  expands to the nesting and costs exactly what the nesting costs — two direct
  calls and no allocation. Left as a value it becomes a closure, and only the
  operands that are local are captured, because a top-level `fn` needs no
  closing over.

### Changed
- A release build calls a closure directly when the block it reads the address
  out of was built in the same straight line, instead of jumping through the
  block. The inliner then sees an ordinary call, so an unapplied composition of
  two top-level functions lowers to the same two direct calls the applied form
  does. The block is still allocated and released; only the indirection is
  gone.
- `scripts/project-tests.sh` says what a fixture directory with no manifest
  usually is — a build directory left behind by the branch that added the
  fixture — instead of reporting a missing file and leaving the reason to be
  worked out.

### Fixed
- Asking the compiler library for an object while also asking for debug
  information is refused rather than answered with a stripped object. The
  object writer emits no debug sections at all, so the request could never have
  been honoured; a debug build emits assembly and assembles it, which is what
  the `slopium` and `slopic` commands already do.


## [0.12.0] - 2026-08-20

### Added
- `(defer body ...)` runs its body when the enclosing scope ends, whatever
  ended it: falling off the end, a `break`, a `continue`, or the error arm of a
  `try`. Deferred expressions run in the reverse of the order they were
  written, and all of them run before the scope releases what it owns, so a
  file descriptor, a socket or a lock behind an `i64` is released where it was
  decided rather than wherever the scope happens to end.
- A manifest can say what a module *is* for each target, so
  `[target."x86_64-unknown-linux-gnu"]` names one file for `arch` and another
  triple names a different one. The program writes `(take arch ...)`
  once and never learns which file answered; the files that were not selected
  are not compiled, and the one that was is an ordinary module, checked like
  every other. A triple this toolchain cannot build for needs no special case.
  An entry naming a file that is not there is refused as `SL1102` rather than
  quietly doing nothing.
- A comment beginning `;;`, on the lines directly above a declaration, is that
  declaration's documentation, and the language server shows it on hover above
  the type. A single `;` is an ordinary comment and still means nothing. A
  blank line ends the block, and so does a comment sharing its line with code.
  The formatter leaves a `;;` block exactly as it was written.
- A declaration can say which target it is for: `(fn (target
  "aarch64-unknown-linux-gnu") ...)`, and the same for a `const`, a `struct`,
  an `enum`, an `extern` and a `test`. A declaration for another target is
  removed before anything types it, so nothing downstream carries code nobody
  compiles. The string is a target triple, spelled as `slopium targets` prints
  it. A name whose only declaration was for another target is an unknown name,
  and the refusal says which target declares it rather than leaving that to be
  worked out.

### Changed
- A change writes its changelog entry into its own file under `changelog.d/`,
  named for the issue it closes, instead of a line under `[Unreleased]` in
  `CHANGELOG.md`. Two changes in flight no longer collide there, and a release
  collects the files into that version's section. `CHANGELOG.md` itself is
  written only by a release.

### Fixed
- A terminator naming a block that does not exist is reported as `SL0700`
  rather than crashing the compiler in a release build, where the verification
  that would have caught it does not run.
- A release build no longer folds a constant its analysis had not finished
  proving. Constant propagation bails out at a bound, and until it settles its
  states are optimistic, so a `Branch` could be rewritten into a `Goto` the
  program never asked for; a run that reaches the bound now folds nothing and
  reports `SL0700` instead. The bound has never been reached by a real
  program, and reaching it would mean the bound is wrong.
- A debug or continuous-integration build no longer aborts on a program that is
  still worth optimizing after the last pipeline round. That was an assertion
  about a legitimate outcome; the pipeline stops and keeps what it achieved.
- An aggregate index and a `Drop`'s type are checked against the layout the
  module records. A field or payload index past the end of what it indexes
  became an address in both backends with nothing having bounded it, and a
  `Drop` picked its release helper from a type nothing compared to the local it
  was dropping. No Slopium program could express either, so what changes is that
  a mistake in the compiler is now an `SL0700` naming the instruction instead of
  an out-of-bounds heap access.


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
[0.15.1]: https://github.com/keepinfov/slopium/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/keepinfov/slopium/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/keepinfov/slopium/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/keepinfov/slopium/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/keepinfov/slopium/compare/v0.11.0...v0.12.0
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
