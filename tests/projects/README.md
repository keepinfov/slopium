# End-to-end feature projects

These fixtures exercise Slopium through the real `slopium` project manager.
They complement compiler unit/native tests with manifest discovery,
multi-module resolution, dependency graphs, separate objects, runtime startup,
and the generated test harness.

Run the complete suite from the repository root:

```sh
scripts/project-tests.sh
```

## Passing projects

| Project | Covered surface |
| --- | --- |
| `basics` | Scalars, arithmetic, comparisons, recursion, inference, mutation, `if`, `do`, calls with stack arguments, printing |
| `ownership-borrows` | Moves, shared/mutable borrows, last-use shortening, strings, structural clone |
| `aggregates-patterns` | Structs, field access, enums, bool/int/enum match, owned and nested patterns |
| `collections` | Copy/owned lists, all list operations, `Option`, arrays, slices, clone/drop |
| `loops` | `while`, `loop`, `break`, `continue` |
| `everyday-forms` | A module-level `const` across a module boundary, a `let` that carries its value's type, a `loop` that produces a value, `match` guards, and a name bound twice in one scope |
| `annotations` | A declaration's annotation slot, `deprecated` warning at a use across a module boundary, and an `inline` hint that reaches the optimizer |
| `generics-std` | Generic functions/structs/enums, bundled `Option`/`Result`, successful and propagated `try` |
| `modules` | Nested path modules, exports, `take` aliases, re-exports, qualified calls, separate objects |
| `module-tests` | A `(test ...)` in a module that is not the entry module and in one nested below it, with checked arithmetic, a string literal and a `lambda` in the test bodies |
| `path-dependencies` | Direct and transitive path dependencies, each under its package name |
| `diamond-dependencies` | One dependency reached through two packages, resolved once under its own name; lockfile and `tree` |
| `custom-std` | A path package supplying `[language-items]` in place of the bundled `std` |
| `workspace` | A root package with a library member: inherited version and dependency, one lock, one `target/`, per-package tests |
| `process-io` | stdin, `read-line`, `read-i64`, `parse-i64`, environment, argv |
| `defer` | A `defer` on each of the four ways a scope ends, registration order against running order, a nested scope, and a C handle with no destructor |
| `target-modules` | One module name that is a different file per target across three triples, built and run for both the host and the cross target, with a third file for a target this toolchain cannot build |
| `composition` | `<<` and `>>` applied and unapplied, both directions, three operands, one operand, and two locals of `Fn` type composed |
| `fieldless-enums` | An enum with no payload anywhere: copied not moved, compared with `=`, matched, and an `Option` beside it keeping the old representation |
| `target-selection` | A `fn` and a `const` selected by target across three triples, and an unannotated declaration that every target gets; built and run for both the host and the cross target |

`workspaces/virtual-root` is not a passing project, because its root defines no
package to run: it has its own phase in the runner, which asserts that a command
without `-p` or `--workspace` fails, that `members = ["crates/*"]` expands, that
`exclude` keeps a directory out of the lock, and that `-p` builds one member.

A passing project may also carry an `expected.stderr`, whose lines must all
appear in what `check` wrote to standard error. That is where a warning is
asserted: a program the compiler has something to say about still compiles, so
there is no exit status to read it from (`D-122`).

Every passing project must pass `fmt --check`, `check`, `run`, and `test`.
Selected projects also build and run under the release profile.
The runner additionally checks `new`, `targets`, `compiler`, and `clean`, plus
body-only versus public-interface cache invalidation for separate module
objects. The low-level compiler is exercised in `check`, HIR, MIR, assembly,
object, executable, release, test-harness, and JSON-diagnostic modes.

## Expected failures

`compile-fail` projects assert stable diagnostic codes or deterministic
manager errors for ownership, borrows, types, matching, modules, dependencies,
generics, standard-library contracts, entry points, borrowed-value escape, and
removed v0.1 syntax.

`runtime-fail` projects compile successfully, then assert native exit status
101 and the normalized runtime message for bounds, input, process API, and
arithmetic failures. `deliberate-panic` is the one that fails because it
decided to rather than because it tripped over a trap: `assert` and `panic`
exit with the same 101 and say why.

`test-fail` projects have a failing test in them on purpose, and assert what
the harness printed. Every other fixture asserts that its tests pass, which is
the one case where a failure has nothing to report, so what a *failing* test
says was held to nothing until `test-fail` existed. `failing-comparison`
covers a number, an unsigned number, text in quotes, and a bare condition with
nothing to say.

## Freestanding projects

`freestanding` projects build for `x86_64-unknown-none`, where there is no C
library, no `main(argc, argv)` wrapper and no `std`. They cannot be passing
projects: a program with no `std:io` has nothing to print with, so the answer
leaves through the exit status, and `run` and `test` are not shapes this target
supports at all.

| Project | Covered surface |
| --- | --- |
| `bare` | `[build] target = "x86_64-unknown-none"` and `[build] linker-script`, an entry stub in `.s` and the four runtime hooks through `c-sources`, the core half of the runtime alone |
| `kernel` | a multiboot stub that enters long mode, a VGA write through a volatile `(Ptr u16)` at `0xB8000` read back through the same pointer, and a UART driver written in Slopium over an `extern` pair of port instructions |

Each one is asserted to build, to leave nothing undefined under `nm -u`, to have
had its linker script applied — the fixture's script discards `.comment`, which
every default link keeps — and to have `test` refused with a message rather than
silently accepted. Each sets `strip = false`, because `nm -u` finding nothing is
only a claim while there is a symbol table to look in.

A fixture carrying an `expected.status` is also run here and its status
compared. `kernel` carries none, because it cannot be executed on the host at
all: it is entered by a loader in 32-bit protected mode, and
`scripts/kernel-check.sh` boots it under `qemu-system-x86_64` and compares what
arrives on the serial port against `expected.serial`. The split is deliberate —
the kernel is still built, linked and inspected on every run of this script,
including on a machine with no emulator, so it cannot rot silently behind a
skipped boot.
