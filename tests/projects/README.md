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
| `path-dependencies` | Manifest aliases plus direct and transitive path dependencies |
| `diamond-dependencies` | One dependency reached through two packages, resolved under both namespaces |
| `custom-std` | Path replacement for `std` and manifest-defined language items |
| `process-io` | stdin, `read-line`, `read-i64`, `parse-i64`, environment, argv |

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
