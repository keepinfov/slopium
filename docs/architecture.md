# Compiler architecture

Slopium deliberately separates the project manager from the compiler.

Every `D-nnn` cited below is an entry in [`decisions.md`](decisions.md), the
project's decision log.

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
3. AST construction validates declaration and expression shapes. It is also
   where a declaration's annotations are read and checked: the list between a
   keyword and a name is an annotation, and one table decides which names
   exist, what each takes and which declarations each may sit on, so all six
   declaration forms refuse alike (`D-122`).
4. Package analysis derives module identities, resolves collected exports,
   imports, privacy, dependency namespaces, and rejects cycles.
5. Semantic analysis checks types, generics, nested match exhaustiveness, and
   affine ownership with control-flow-shortened borrows. It is where a use of a
   `deprecated` declaration becomes a warning, at the use rather than at the
   declaration, and warnings leave the compiler through a sink the caller
   passes in — because a warning belongs to the *compilation* rather than to
   the program, and which of them a run reports depends on what it was asked to
   build (`D-122`). Package analysis answers that: a warning about a
   dependency's own source is the dependency's, and a run building one codegen
   module reports that module's alone, or `slopium` would print every warning
   in the package once per object. It is also where a
   module-level `const` disappears: the declaration's literal is typed once and
   a use is a copy of it, so nothing after this pass knows the name existed
   (`D-121`).
6. Reachability-driven monomorphization materializes concrete generic
   functions and aggregate layouts.
7. Lowering produces whole-package target-independent MIR with locals, basic
   blocks, explicit moves, borrows, loops, calls, and structural drops.
   A `loop` that produces a value writes into one local on every break edge and
   reads it once past the exit, and a `when` guard is a second branch between
   the pattern test and the arm — a block that binds the pattern's names
   without taking the aggregate apart, so a guard that answers `false` leaves
   the scrutinee exactly as the next arm expects it. Neither costs an
   instruction that did not exist: a break value is `Assign` and `Goto`, and a
   guard is `Branch` (`D-121`).
   Assigning a field is `FieldStore` and its `EnumFieldStore` twin, the writing
   half of the two instructions that form a field's address: the place is known
   because the name came from a pattern this function wrote, so a borrow keeps
   the representation it always had and nothing at a call boundary changes
   (`D-120`). One write is a load of the old word, the store, and a drop of what
   came out — in that order, so the field never briefly holds two owners or
   none. A
   `lambda` body becomes a function of its own here and its environment becomes
   a struct, which is why a closure costs neither backend an instruction: the
   block is laid out as an aggregate, so the clone and drop helpers generated
   for every aggregate are its glue, and the runtime does nothing but read one
   out of the block and jump to it (`D-101`). Every statement carries the span
   of the expression it came from, and every block is terminated by
   construction.
8. The release profile runs an optimization pipeline to a fixpoint: bounded
   inlining, cross-block constant propagation — which stops tracking a local
   the moment its address is taken, because a C `extern` with an out-parameter
   writes through that address (`D-124`) — control-flow simplification, and
   dead code elimination. Two behaviours are preserved by construction —
   arithmetic that would trap is never folded away or removed, and drops are
   never deleted or moved across a branch. Inlining does not cross a module
   boundary, because a module's object is cached on its own body and would go
   stale if it contained a copy of another module's code. An `inline`
   annotation raises the two size ceilings for one callee and changes nothing
   else: every rule that makes inlining sound still decides, which is why the
   annotation is not part of a module's interface and adding one rebuilds
   nothing but the module it is written in (`D-122`).

   Two bounds keep a mistake in a pass from hanging the compiler, and they mean
   opposite things (`D-132`). The pipeline's bound is quiet: every pass is
   sound on its own, so a module still improving when the rounds run out is a
   correct module that was optimized less. Constant propagation's is not: its
   lattice is optimistic until it settles, so a run that reaches the bound
   folds nothing and reports `SL0700`. The block-target half of MIR
   verification runs in every profile for the same reason — the passes index
   blocks by a terminator's target, so an out-of-range one is a panic rather
   than a wrong answer, and the rest of the verifier is off in the only profile
   the pipeline runs in.
9. A verifier checks the result of each pass: identifier ranges, parameter
   layout, call arity, operand types, and that every read has a reaching
   definition. A call through a function value is checked against the `Fn` type
   of the block it passes along, because a wrong indirect call is the one kind
   that assembles and links (`D-092`, `D-101`). So is every read and address
   taken through a
   borrow: a borrow of a pointer-shaped value is that pointer and a borrow of
   anything else is the address of a slot, so the two field instructions are
   each garbage in the other's place, and getting it wrong reads an integer as
   a pointer rather than failing (`D-099`, `D-100`). So is every field write,
   against the layout it writes into: a word stored where a pointer belongs is a
   heap block the drop glue will follow, and the enum case can only be checked
   as far as the instruction names a layout, since the variant decides a payload
   slot's type and the instruction does not name one (`D-120`). So is every
   volatile
   access, whose width has to agree with what the pointer points at and with
   the local the value came from or went to: a width one size wrong does not
   fault, it reads or writes the bytes of the neighbouring device register
   (`D-067`). It reports `SL0700` internal errors rather than panicking, and
   runs in debug builds or under `SLOPIUM_VERIFY_MIR=1`.

   What the verifier deliberately does *not* check is that a volatile access
   was neither eliminated nor duplicated, because that is a statement about a
   module before and after a pass and the verifier is handed one module. The
   optimizer compares the counts itself, pass by pass, and `is_pure` is what
   stops dead-code elimination from removing an access whose result nothing
   reads (`D-114`).
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

Both backends hold every integer in a full machine word, canonical for its type
— sign-extended when signed, zero-extended when unsigned (`D-074`, `D-107`). A
narrow type therefore has no instruction selection of its own: it computes at 64
bits and is put back into its own width afterwards, and it overflows exactly
when that round trip changes the value, which is one compare-and-trap rather
than a bound constant per type. Only `u64` reaches for genuinely unsigned
instructions, because a zero-extended value below 2^32 compares and divides the
same either way. This is also what makes a conversion free of any MIR
instruction: `(as T v)` lowers onto a mask or a shift pair the backends already
emit, so there is no node for the two of them to disagree about. What the
invariant costs is one canonicalisation per narrow parameter in the prologue —
a Slopium caller always places a canonical word, but C leaves the upper half of
a narrow argument register undefined.

Memory is a machine word everywhere too — a frame slot, a struct field, an enum
payload, a list element — with exactly one exception. A volatile access through
a raw pointer reads or writes one, two, four or eight bytes, because a device
register has a width the program did not choose (`D-067`). That is where the
only sub-word encodings in either backend live: `movzx` from a byte and a half
and the `0x66`-prefixed store on x86-64, `ldrb`/`ldrh`/`strb`/`strh` and the
four-byte pair on AArch64. A narrow load zero-extends, which is already an
unsigned type's canonical word, so only a signed one is extended afterwards and
the two paths share the canonicalisation the conversions use. A `(List u8)` is
still one word per element; packing it is a performance decision with a
measurement in front of it, not part of having the type.

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
then into either patched bytes or relocations. A branch to a local label *in
the same section* is arithmetic the compiler can do; a call, or an address
anywhere else, is left to the linker under the same rules a linker expects.
`elf` turns the result into a file, and knows only three things per
architecture: the machine number, which relocation type spells each fixup, and
what a section of instructions is aligned to.

A section's *kind* is a closed set — code, constants, and the empty
`.note.GNU-stack` marker — and how many sections of a kind an object has is
not. A backend obtains one by asking for a kind and naming a function, never by
spelling a string, so the flags and the alignment come from the kind and the
object writer can still see every section that exists. That is what lets a
function own the `.text` its code sits in.

Linking is still the system linker's. It is what knows where the C runtime
lives, which dynamic loader to name, and how the platform starts a process —
none of which is a code generation question.

The link is asked to keep only what a program uses. The runtime is two C files
of every helper the language might call, so they are compiled with a section
per function and linked with `--gc-sections`: a program that never makes a
slice does not carry `sl_rt_slice_*`. That only ever removes unreferenced code, so it
is unconditional. **A Slopium function is granular the same way**: it owns the
`.text` its own code sits in, so a library module a program takes no longer
brings whatever that module calls. A module is still emitted whole — `emit` is
per module — and the linker is what drops the rest, which is why `basics`
defines seventy-three functions across its objects and links twelve.

Each function's panic trampolines sit inside its section rather than at the end
of a shared one. A trampoline is two or three instructions and only the ones a
function's arithmetic can reach are emitted, so the cost is small and it is
dropped along with the function it serves. The reason it is not shared is that
AArch64 reaches one with a 19-bit conditional branch: within a section that is
a displacement the compiler works out, and it can say so at compile time when
it does not fit, whereas across sections it becomes an `R_AARCH64_CONDBR19`
that no linker will supply a veneer for. Stripping the symbol table is a choice, because it removes
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
editor and the build cannot namespace a module differently. The bundled library
itself lives one level lower again, in `slopium-std`: the sources are ordinary
`.slp` files under `std/`, and the compiler hands them to name resolution while
the manager hashes them into the lock, so those two cannot disagree about what
the library contains (`D-076`). There are two bundled packages — `core`, which
carries `Option`, `Result`, `string`, `float`, `map`, `set` and the language
items, and `std`, which
carries `io`, `process` and `fs`, depends on `core`, and re-exports its
language items through a `prelude` module so that exactly one direct dependency
ever declares them (`D-082`). `std:string` re-exports `core:string` for the
same reason (`D-083`): a package that depends on `std` has no name for `core`. The language server
analyzes an open file as the member that owns it. `slopic` still types
and lowers the package as one semantic unit, then emits only the selected
owner module for each object invocation. Generated function/type/drop/clone
symbols use deterministic global names so objects link independently.

Object cache keys include the selected module body plus every module
interface. A body-only change keeps independent objects fresh; an interface
change invalidates consumers. Generic owner modules additionally include
consumer bodies because their reachable concrete instance set can change.

A module's interface is everything a *consumer's object* can depend on, which
is more than its signatures. A `const` is inlined at every use (`D-121`), so
its value is interface: a dependent that does not rebuild keeps the old number
compiled into it. A `deprecated` annotation is interface too, because the
caller is what warns. An `inline` annotation is not, because nothing inlines
across a module boundary (`D-122`).
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

A registry is a static index tree and an archive next to it, reached over
`file://` or through `curl`. It is the first source offering more than one
version of a package, so it is what puts weight on selection: the resolver takes
the newest version a requirement allows and backtracks when that makes some
other requirement unsatisfiable. Candidate requirements come out of the index,
because downloading every candidate to read its manifest is the cost an index
exists to avoid — and what is downloaded is then checked against both the entry
that selected it and the digest it was published under. Again the compiler is
handed a directory in the store, so the resolver is the only part of the system
that knows registries exist at all.

Signing sits at that same boundary, and at one point inside it. `slopium
publish` writes an Ed25519 signature over a statement naming the package, its
version and its digest — never over the digest alone, which would be
transplantable — into the registry beside the archive and into the index line.
Consumption is a policy of the consuming checkout: `[registry.<name>]
trusted-keys` lists who may sign, an empty list checks nothing, and there is no
trust on first use to make the first download a decision. The check happens in
the one function that turns a resolved package into a directory, so it runs on
every build rather than only on the download that filled the store, and adding a
key takes effect immediately. Nothing about it reaches the compiler either.

`lib.buildSlopiumPackage` inverts the direction: Nix reads a `Slopium.lock` this
resolver wrote and turns it into fixed-output derivations keyed by the very
checksums it recorded, then builds `--offline --locked`. There is one resolver
and it runs once, which is why the two build paths cannot disagree about a
graph.

## Runtime boundary

Generated programs link a small C runtime through the stable `sl_rt_*` ABI. It
provides allocation, strings, owned lists/arrays, borrowed slice descriptors,
printing, input, files, and process arguments. It does not evaluate source or
own language semantics, and since v0.5.3 it does not format or parse a number
either — that is Slopium, in `core:string` (`D-086`, `D-087`). A float is the
same: `sl_rt_f64_bits` and `sl_rt_f64_from_bits` reinterpret one machine word
and do nothing else, and every digit of a decimal expansion is computed in
`core:float` (`D-097`). Generated clone/drop helpers recursively copy and destroy
collection elements, structs, and enum payloads.

It is two translation units, and the seam between them is the boundary a
freestanding program would sit on (`D-066`).

`runtime/slop_rt_core.c` is the strings, the lists, the slices and the failure
paths. It includes no hosted header and calls exactly four symbols it does not
define: `sl_rt_alloc`, `sl_rt_free`, `sl_rt_abort` and `sl_rt_panic`
(`D-080`). The split is deliberately not "core never allocates" — a kernel has
an allocator; what it does not have is libc. So core is the code and hosted is
the providers.

`runtime/slop_rt_hosted.c` defines those four over `malloc`, `free`, `exit` and
`fprintf`, and adds stdio, `argv`, `getenv` and whole-file reads and writes
(`D-084`).

A hosted call that can fail has one channel for its result and none for its
error, because the FFI vocabulary has no out-parameter and no struct return.
So the hosted half keeps a status slot: every such call clears it on the way in
and sets it to an `errno` on the way out, `sl_rt_last_error` reads it, and the
library turns it into an `Option` or a `Result` in the form immediately after
the call (`D-085`). Zero is success, a positive value is an `errno`, and `-1`
is end of input.

Which units link is the environment's to say, and the environment is the
target's default overridden by `slopic --freestanding` (`D-081`). It decides
four things and no others: the runtime units, whether the `main(argc, argv)`
wrapper is emitted at all, whether a lone file's default library is `std` or
`core`, and what the link says — `-nostdlib -nostartfiles -static -no-pie`
beside the freestanding compile flags (`D-117` widens `D-081` by that fourth
one). `x86_64-unknown-none` is the row that supplies it, so a freestanding build
is a `--target` and not a mode; the override remains for the two hosted triples,
which is how a freestanding AArch64 object is still checked before there is a
freestanding AArch64 target.

The layout is the program's. `[build] linker-script` names a script inside the
package and the manager passes it as `-T`; without one the link takes the
toolchain's default. The entry point is the program's too, because no wrapper is
emitted: a freestanding program supplies `_start` — through `[package]
c-sources`, which has always handed a `.s` to `cc` and now has a fixture saying
so — and reaches its own entry by the name that entry links under. A program's
`main` keeps its bare name where every other function is qualified by its
module, so that name is `sl_fn_6d61696e`, and it is the seam a boot stub is
written against.

`scripts/core-check.sh` is the check that makes this real rather than
aspirational. It builds a `core`-only program — one that sends its answer out
through `core:string` and `core:float` and reads it back, so the string and
float libraries are covered too (`D-083`, `D-097`) — links it against
`slop_rt_core.o` with `-nostdlib` and a supplied
`_start`, requires `nm -u` to show nothing but the four hooks, and runs it. The runtime ABI freezes at v0.8,
and freezing a half nothing had ever linked would be freezing a guess.

It then links the same program a second time and lets the compiler write the
command line, over `x86_64-unknown-none` and with no `--library`, so that the
hand-written link and the shipped one have to agree about the flags and about
the entry point. `tests/projects/freestanding` makes the same claim about a
package, where the linker script and the entry stub come from a manifest.

`scripts/kernel-check.sh` is where the environment stops being a set of flags
and becomes a machine. It boots `tests/projects/freestanding/kernel` under
`qemu-system-x86_64`: a multiboot stub enters in 32-bit protected mode, zeroes
`.bss`, identity-maps the first 8 MiB, switches to long mode and calls
`sl_fn_6d61696e`, and the Slopium above it writes the VGA text framebuffer
through a volatile `(Ptr u16)` at `0xB8000`, **reads it back through the same
pointer**, and sends what it found out of a serial port through a UART driver
written in Slopium over an `extern` pair of port instructions. Port-mapped I/O
is the one thing a raw pointer cannot express — it is a separate address space
that no address names — so `in` and `out` cross the C boundary rather than
becoming operators, which keeps `lowering.rs` target-neutral (`D-025`).

The image QEMU is handed is a 32-bit re-wrap of the linked kernel, because
QEMU's multiboot loader refuses a 64-bit one. `objcopy` rewrites the container
and nothing else; the contents and the entry stay where the linker script put
them, and every address fits because the whole image lives at 1 MiB — which is
also what keeps the small code model the compiler emits correct, there being no
`-mcmodel=kernel` to offer. Interrupts are never enabled and no IDT is ever
loaded, which is what makes the red zone sound: only an asynchronous push could
corrupt it.

Generated hosted executables use a small C ABI `main(argc, argv)` wrapper.
Language functions, including the user `main`, retain compiler-mangled symbols.
Unrecoverable runtime errors print a normalized message and exit with status
101.

The host `cc` is used only to assemble and link emitted assembly. No LLVM,
Cranelift, VM, or interpreter is involved.
