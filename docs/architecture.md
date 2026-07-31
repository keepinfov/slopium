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
8. The release profile runs an optimization pipeline to a fixpoint: bounded
   inlining, cross-block constant propagation, control-flow simplification, and
   dead code elimination. Two behaviours are preserved by construction —
   arithmetic that would trap is never folded away or removed, and drops are
   never deleted or moved across a branch. Inlining does not cross a module
   boundary, because a module's object is cached on its own body and would go
   stale if it contained a copy of another module's code.
9. A verifier checks the result of each pass: identifier ranges, parameter
   layout, call arity, operand types, and that every read has a reaching
   definition. It reports `SL0700` internal errors rather than panicking, and
   runs in debug builds or under `SLOPIUM_VERIFY_MIR=1`.
10. Backward liveness dataflow yields live intervals over a linear block order,
    and a linear-scan allocator places every local in a register or a frame
    slot. Allocation runs in both profiles: it is part of code generation
    rather than something the release profile turns on.
11. A `Backend` partitions MIR by owner module and consumes `MirModule` plus a
    `TargetSpec`. With debug information requested it also emits `.file` and
    `.loc` directives from the span each statement carries, which the assembler
    turns into a DWARF line table.

MIR keeps numbered locals rather than SSA form. The `cfg` module derives what
SSA would have carried implicitly — successors, predecessors, reverse
postorder, reachability, def/use, liveness, and live intervals. Drop
elaboration is fused into lowering: the builder compares per-branch liveness
maps and inserts drops at merges, so ownership correctness lives in the
lowering code rather than in a later pass. `--emit mir-text` renders the whole
module for reading.

Register allocation is whole-interval: a local lives in one register for its
entire function or in its frame slot for the entire function, with no interval
splitting and no mid-interval reload. A local that loses the scan simply keeps
the slot it would have had, so spilling can neither fail nor add instructions.
Two rules make the register choice safe rather than merely fast. A local whose
address the backend takes — a borrowed scalar, a collection element, a pop
destination — is pinned to memory, because `lea` has no register form. And a
function that calls anything draws only on callee-saved registers, so no
clobber set has to be tracked; a function that calls nothing draws first on
caller-saved registers, which cost no prologue at all.

Debug information is a line table and nothing else. `--debug`, which `slopium`
passes for any profile whose `debug` is on — the default for `dev` — attributes
each instruction to the expression it was lowered from, so a debugger can set a
breakpoint by file and line, step by statement, and produce a backtrace whose
frames name their own module. Adding it changes no instruction: the emitted
assembly is the assembly of a build without it, interleaved with directives.

There are no variable locations. A line table needs only spans, but describing
where a variable lives has to survive register allocation, which gives a local
one register or one slot for a whole function rather than a fixed frame offset,
so it would have to choose between `DW_OP_reg` and `DW_OP_fbreg` per local from
the `Allocation`. Generated code — clone and drop helpers, the entry wrapper,
the panic trampolines — carries no location of its own and inherits the row
before it, because the assembler discards the `.loc` that would say "not in the
source".

## Backends

There are two: Intel-syntax x86-64 for System V AMD64, and AArch64 for AAPCS64.
Both are chosen by triple from a target table, and everything before them —
parsing, typing, ownership, MIR, the optimizer, liveness, and the register
allocator — is the same code producing the same result for either.

What a backend supplies is a register file, an instruction selection, a calling
convention, and an assembly syntax. What it does not supply is anything the two
have to agree about. Symbol names, which runtime helper releases or copies a
given type, aggregate sizes, and what each builtin call actually does all live
in one module both backends read. That is the point of the split: two backends
deriving those separately would be free to derive them differently, and the
result would be a linker error at best and a value released by the wrong helper
at worst.

The differences that remain are the machine ones. x86-64 lets most instructions
name a frame slot, so its backend decides per instruction whether an operand is
a register or memory. AArch64 is load/store, so its backend reads an operand
into a scratch register whenever the allocator left it in memory. x86-64 gets
overflow from a hardware flag and division faults from the hardware; AArch64
sets the flag with `adds`, checks multiplication against the high half of the
product, and has to reject a zero divisor itself because `sdiv` does not fault.
The frame is upside down between them: x86-64 addresses locals below `rbp` and
pushes stack arguments, AArch64 addresses them upward from `sp` and reserves an
outgoing-argument area at the bottom of its own frame, because `sp` anchors the
locals and cannot move between two statements.

Agreement is checked rather than assumed. `scripts/cross-check.sh` compiles the
whole corpus for both targets, runs the second under `qemu-aarch64`, and
requires identical stdout and exit status, including for the programs that
panic. A separate ABI conformance program links Slopium functions against a C
caller built by the platform toolchain, with more arguments than either
register class holds, so the two only agree if both placed them where the ABI
says.

## Objects

A backend does not produce text. It produces a stream of items — sections,
labels, symbol attributes, data, and instructions — and that one stream is
either rendered as assembly or encoded into a relocatable ELF object. Both
readings come from the same description of the program, so there is nothing for
them to disagree about, and an instruction with no encoding is a compile error
in the compiler rather than a surprise at assembly time.

`asm` owns the half neither architecture decides: section identity, label
scope, symbol binding, and the layout pass that turns labels into addresses and
then into either patched bytes or relocations. A branch to a local label inside
`.text` is arithmetic the compiler can do; a call, or an address in another
section, is left to the linker under the same rules a linker expects. `elf`
turns the result into a file, and knows only two things per architecture: the
machine number, and which relocation type spells each fixup.

Linking is still the system linker's. It is what knows where the C runtime
lives, which dynamic loader to name, and how the platform starts a process —
none of which is a code generation question.

The link is asked to keep only what a program uses. The runtime is one C file
of every helper the language might call, so it is compiled with a section per
function and linked with `--gc-sections`: a program that never touches a slice
does not carry `sl_rt_slice_*`. That only ever removes unreferenced code, so it
is unconditional. Stripping the symbol table is a choice, because it removes
the mangled `sl_fn_*` and runtime names a debugger needs — so it is a flag
(`slopic --strip`), not something the compiler decides. A test body is code
only the harness reaches, so a build without `--test` does not emit it at all,
which is also why `sl_rt_test_result` is absent from an ordinary binary. The two
trap messages are emitted the same way: `"division by zero"` and `"integer
overflow"` are written only when a check can actually reach them, so a program
with no division carries neither string nor trampoline. Which trap a function's
arithmetic can reach is decided once, in `lowering::trap_usage`, so the two
backends cannot disagree about it (`D-025`). `slopic` and `slopium` link through
the same flag list (`slopic_core::cc_flags`), so a package binary and a
standalone one shrink alike. Together these take a small program from roughly
22 KB to 14 KB on disk.

`slopic` is mechanism, not policy: it has `--optimize`, `--debug`, `--strip`,
and `--panic-abort`, and no notion of a "release" — it does not decide when to
optimize, strip, or drop panic messages, only how. The manager holds the policy.
A `Slopium.toml` profile sets `opt-level`, `debug`, `strip`, and `panic`, and
`slopium` resolves each into the flag it passes; `strip` defaults to the
opposite of `debug`, because a binary you can debug and one you ship are opposite
intents. The build cache hashes the resolved answers, so flipping any of them in
the manifest rebuilds.

`panic = "abort"` is the one that removes checks' *messages* without removing
the checks. A trap still fires — the bounds, the overflow, the zero divisor are
all still tested — but the trampoline calls a message-less `sl_rt_abort` and the
runtime is compiled with `-DSLOPIUM_PANIC_ABORT`, which routes its own failures
through the same bare exit. The result carries no error strings and does not
pull in `fprintf`; it also says nothing when it dies, which is why the default
is `"message"`. Removing a *check* is never on the table: a skipped bounds or
allocation test trades a few bytes for undefined behaviour, which no build
profile is allowed to ask for.

Two things send a build back to the platform assembler. Debug information is
one: line tables are built from the `.file` and `.loc` directives, and the
object writer emits no DWARF. The other is `SLOPIUM_OBJECT_WRITER=external`,
which exists so a bug in an encoder has a way around it that is not a different
compiler.

The encoders are checked against the assembler rather than trusted.
`scripts/object-check.sh` compiles the corpus both ways for both targets and
compares the results: byte for byte on AArch64, where fixed-width instructions
make that achievable, and instruction by instruction on x86-64, where this
compiler always uses a 32-bit jump displacement and the assembler shortens the
ones that fit. Relocations and symbol tables are compared on both, and both
objects are linked and run.

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
dependencies into a set of packages, each identified by name and version and
appearing once however many dependents reach it. A package's namespace is its
name, so the key in `[dependencies]` must be the package name. The resolved
graph is written to `Slopium.lock`.

Several packages may share one workspace: `[workspace] members` at a root
manifest that need not define a package of its own. A workspace has one
`Slopium.lock` and one `target/`, and resolution covers every member at once, so
building one member cannot rewrite what another recorded. A member reached as a
`path` dependency resolves to that member rather than being read again, which is
what lets it inherit `version` and `[dependencies]` entries from the root. A
lone package is loaded as a workspace of one, so there is one code path rather
than two.

Resolution, the manifest schema, semantic versions, workspaces and the lockfile
live in `slopium-manifest`, which both `slopium` and `slopium-lsp` consume — the
editor and the build cannot namespace a module differently. The language server
analyzes an open file as the member that owns it. `slopic` still types
and lowers the package as one semantic unit, then emits only the selected
owner module for each object invocation. Generated function/type/drop/clone
symbols use deterministic global names so objects link independently.

Object cache keys include the selected module body plus every module
interface. A body-only change keeps independent objects fresh; an interface
change invalidates consumers. Generic owner modules additionally include
consumer bodies because their reachable concrete instance set can change.
Objects live under `objects/<package>/`, because two members can compile a
module of the same name.

A package's test harness carries that package's tests only. A dependency's
tests belong to the dependency: codegen emits a test body only in the object
that owns it, so collecting a dependency's tests would leave the harness calling
functions no object defines.

A package with immutable bytes records a `checksum` in the lock — the digest of
its archive, a ustar tar with timestamps, owners and entry order removed so that
the same tree always hashes the same (`docs/packaging.md`). `slopium package`
writes one; `slopium vendor` copies such packages into a vendor directory
through the content-addressed store in `$SLOPIUM_HOME`, where an archive is
verified before it is unpacked. A vendored copy is checked on every build by
re-archiving it. Replacement is invisible to resolution: a replaced package
keeps its identity, its source and its lock entry, so vendoring cannot change
what a project resolves to.

A git dependency is fetched by running `git`, which the manager treats the way
it treats `cc`: an external program that already knows about transports and
credentials. What comes back is `git archive` of the resolved commit, normalized
into that same archive format and stored under its digest — so a fetched package
is the same kind of object as a published one, and the lock records the commit
*and* the digest. Resolution pins a full commit and never asks again: a
dependency the lock names is not re-resolved, which is what makes a moved branch
unable to move a build. The compiler is still handed a directory, so nothing
below the manager knows a repository was involved.

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
