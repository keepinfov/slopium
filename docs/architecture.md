# Compiler architecture

Slopium deliberately separates the project manager from the compiler.

```text
Slopium.toml → package DAG → slopic → module objects → cc → ELF
```

`slopium` owns manifests, profiles, target selection, caching, execution, and
tests. `slopic` consumes an explicitly supplied source root and dependency
roots; it does not discover manifests or access the network. Their
command-line protocol is internal and versioned.

## Compiler stages

1. The lossless syntax layer retains exact tokens, comments, and whitespace
   for formatting and editor features.
2. The semantic lexer and balanced S-expression parser produce a structural
   tree with byte and line/column spans.
3. AST construction validates declaration and expression shapes.
4. Package analysis derives module identities, resolves collected exports,
   imports, privacy, dependency namespaces, and rejects cycles.
5. Semantic analysis checks types, generics, nested match exhaustiveness, and
   affine ownership with control-flow-shortened borrows.
6. Reachability-driven monomorphization materializes concrete generic
   functions and aggregate layouts.
7. Lowering produces whole-package target-independent MIR with locals, basic
   blocks, explicit moves, borrows, loops, calls, and structural drops. Every
   statement carries the span of the expression it came from, and every block
   is terminated by construction.
8. An optional release pass folds constants.
9. A verifier checks the result of each pass: identifier ranges, parameter
   layout, call arity, operand types, and that every read has a reaching
   definition. It reports `SL0700` internal errors rather than panicking, and
   runs in debug builds or under `SLOPIUM_VERIFY_MIR=1`.
10. A `Backend` partitions MIR by owner module and consumes `MirModule` plus a
    `TargetSpec`.

MIR keeps numbered locals rather than SSA form. The `cfg` module derives what
SSA would have carried implicitly — successors, predecessors, reverse
postorder, reachability, def/use, and live intervals — which is what a
linear-scan register allocator needs. Drop elaboration is fused into lowering:
the builder compares per-branch liveness maps and inserts drops at merges, so
ownership correctness lives in the lowering code rather than in a later pass.
`--emit mir-text` renders the whole module for reading.

The first `Backend` uses stack slots and emits Intel-syntax x86-64 assembly for
the System V AMD64 ABI. Adding an architecture means implementing another
backend and target specification; parsing, typing, ownership, and MIR remain
unchanged.

`Analysis` combines syntax, diagnostics, optional typed HIR, and a semantic
symbol/occurrence index. `slopium-lsp` discovers `Slopium.toml`, overlays all
open unsaved buffers on the package source tree, re-runs package analysis, and
maintains cross-file definitions, references, rename targets, completion, and
UTF-16 span translation. Editor analysis requires an executable entry point
only for the package entry module. An entry named `lib.slp` and a package that
defines `[language-items]` are analyzed as libraries, so they do not receive a
spurious missing-`main` diagnostic.

## Packages and incremental objects

`slopium` resolves an acyclic graph of path and bundled-toolchain
dependencies. Each dependency alias becomes a namespace. `slopic` still types
and lowers the package as one semantic unit, then emits only the selected
owner module for each object invocation. Generated function/type/drop/clone
symbols use deterministic global names so objects link independently.

Object cache keys include the selected module body plus every module
interface. A body-only change keeps independent objects fresh; an interface
change invalidates consumers. Generic owner modules additionally include
consumer bodies because their reachable concrete instance set can change.

## Runtime boundary

Generated programs link a small C runtime through the stable `sl_rt_*` ABI.
The runtime provides allocation, strings, owned lists/arrays, borrowed slice
descriptors, printing, input, and process arguments. It does not evaluate
source or own language semantics. Generated clone/drop helpers recursively
copy and destroy collection elements, structs, and enum payloads.

Generated executables use a small C ABI `main(argc, argv)` wrapper. Language
functions, including the user `main`, retain compiler-mangled symbols.
Unrecoverable runtime errors print a normalized message and exit with status
101.

The host `cc` is used only to assemble and link emitted assembly. No LLVM,
Cranelift, VM, or interpreter is involved.
