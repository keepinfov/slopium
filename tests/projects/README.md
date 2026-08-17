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
| `generics-std` | Generic functions/structs/enums, bundled `Option`/`Result`, successful and propagated `try` |
| `modules` | Nested path modules, exports, `take` aliases, re-exports, qualified calls, separate objects |
| `path-dependencies` | Direct and transitive path dependencies, each under its package name |
| `diamond-dependencies` | One dependency reached through two packages, resolved once under its own name; lockfile and `tree` |
| `custom-std` | A path package supplying `[language-items]` in place of the bundled `std` |
| `workspace` | A root package with a library member: inherited version and dependency, one lock, one `target/`, per-package tests |
| `process-io` | stdin, `read-line`, `read-i64`, `parse-i64`, environment, argv |

`workspaces/virtual-root` is not a passing project, because its root defines no
package to run: it has its own phase in the runner, which asserts that a command
without `-p` or `--workspace` fails, that `members = ["crates/*"]` expands, that
`exclude` keeps a directory out of the lock, and that `-p` builds one member.

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
arithmetic failures.

## Freestanding projects

`freestanding` projects build for `x86_64-unknown-none`, where there is no C
library, no `main(argc, argv)` wrapper and no `std`. They cannot be passing
projects: a program with no `std:io` has nothing to print with, so the answer
leaves through the exit status, and `run` and `test` are not shapes this target
supports at all.

| Project | Covered surface |
| --- | --- |
| `bare` | `[build] target = "x86_64-unknown-none"` and `[build] linker-script`, an entry stub in `.s` and the four runtime hooks through `c-sources`, the core half of the runtime alone |

Each one is asserted to build, to leave nothing undefined under `nm -u`, to have
had its linker script applied — the fixture's script discards `.comment`, which
every default link keeps — to exit with the status in `expected.status`, and to
have `test` refused with a message rather than silently accepted. The fixture
sets `strip = false`, because `nm -u` finding nothing is only a claim while there
is a symbol table to look in.
