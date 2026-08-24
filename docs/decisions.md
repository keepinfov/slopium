# Decision log

Every design decision this project has taken, in the order it took them, with
the reasoning that produced it. A commit or a document that cites `D-042` means
the entry numbered `D-042` here.

An entry says what was decided, why, and what follows from it. `Status` is
`approved` for a decision in force, `deferred` for one taken but not built,
`rejected` for one that was considered and turned down, and `superseded` when a
later entry replaced it. Entries are not rewritten as the code changes: a
decision that stopped being true is superseded by a new one, because the record
of what was believed at the time is the part worth keeping.

## Index

- [D-001 — a native compiler, without LLVM](#d-001--a-native-compiler-without-llvm)
- [D-002 — the compiler and the manager are separate programs](#d-002--the-compiler-and-the-manager-are-separate-programs)
- [D-003 — planning is private and local](#d-003--planning-is-private-and-local)
- [D-004 — module syntax needs its own comparison first](#d-004--module-syntax-needs-its-own-comparison-first)
- [D-005 — editor integration evolves toward a language server](#d-005--editor-integration-evolves-toward-a-language-server)
- [D-006 — nothing is published without being asked](#d-006--nothing-is-published-without-being-asked)
- [D-007 — the daily-tooling milestone ships as patch releases](#d-007--the-daily-tooling-milestone-ships-as-patch-releases)
- [D-008 — scalar process APIs before owned collections](#d-008--scalar-process-apis-before-owned-collections)
- [D-009 — modules are derived from paths, and symbols are colon-qualified](#d-009--modules-are-derived-from-paths-and-symbols-are-colon-qualified)
- [D-010 — v0.2 is an incremental series, and breaks on purpose](#d-010--v02-is-an-incremental-series-and-breaks-on-purpose)
- [D-011 — the standard library is a replaceable dependency](#d-011--the-standard-library-is-a-replaceable-dependency)
- [D-012 — parametric generics before traits](#d-012--parametric-generics-before-traits)
- [D-013 — the v0.2 collection, loop and pattern forms](#d-013--the-v02-collection-loop-and-pattern-forms)
- [D-014 — the project fixture suite is a release gate](#d-014--the-project-fixture-suite-is-a-release-gate)
- [D-015 — entry-point validation depends on the analysis context](#d-015--entry-point-validation-depends-on-the-analysis-context)
- [D-016 — v0.3 ships as a patch-release series](#d-016--v03-ships-as-a-patch-release-series)
- [D-017 — "SSA-like MIR" means analyzable, not phi nodes](#d-017--ssa-like-mir-means-analyzable-not-phi-nodes)
- [D-018 — inlining never crosses a module boundary](#d-018--inlining-never-crosses-a-module-boundary)
- [D-019 — optimization preserves traps and drops](#d-019--optimization-preserves-traps-and-drops)
- [D-020 — a local gets one register, or one slot, for the whole function](#d-020--a-local-gets-one-register-or-one-slot-for-the-whole-function)
- [D-021 — a calling function allocates only callee-saved registers](#d-021--a-calling-function-allocates-only-callee-saved-registers)
- [D-022 — an address-taken local is pinned to memory](#d-022--an-address-taken-local-is-pinned-to-memory)
- [D-023 — debug information is line tables and nothing else](#d-023--debug-information-is-line-tables-and-nothing-else)
- [D-024 — debug information may not change the instruction stream](#d-024--debug-information-may-not-change-the-instruction-stream)
- [D-025 — anything two backends must agree about lives outside both](#d-025--anything-two-backends-must-agree-about-lives-outside-both)
- [D-026 — two backends are kept honest by differential execution](#d-026--two-backends-are-kept-honest-by-differential-execution)
- [D-027 — float comparison excludes the unordered case](#d-027--float-comparison-excludes-the-unordered-case)
- [D-028 — a backend emits a stream of items, not text](#d-028--a-backend-emits-a-stream-of-items-not-text)
- [D-029 — the object writer is checked against the assembler, not trusted](#d-029--the-object-writer-is-checked-against-the-assembler-not-trusted)
- [D-030 — a release binary carries only what it uses, and no names](#d-030--a-release-binary-carries-only-what-it-uses-and-no-names)
- [D-031 — dead trap strings out, panic messages an opt-in, checks untouchable](#d-031--dead-trap-strings-out-panic-messages-an-opt-in-checks-untouchable)
- [D-032 — the flake is the installation interface, and the scripts stay](#d-032--the-flake-is-the-installation-interface-and-the-scripts-stay)
- [D-033 — v0.4 ships as a patch-release series](#d-033--v04-ships-as-a-patch-release-series)
- [D-034 — one manifest and one resolver, outside both consumers](#d-034--one-manifest-and-one-resolver-outside-both-consumers)
- [D-035 — a package is identified by name and version, and appears once](#d-035--a-package-is-identified-by-name-and-version-and-appears-once)
- [D-036 — semver ranges, with maximal selection](#d-036--semver-ranges-with-maximal-selection)
- [D-037 — our checksums, borrowed transports, one crypto crate](#d-037--our-checksums-borrowed-transports-one-crypto-crate)
- [D-038 — a package name resolves from exactly one source in a graph](#d-038--a-package-name-resolves-from-exactly-one-source-in-a-graph)
- [D-039 — an archive is a plain, sorted, zero-timestamp tar](#d-039--an-archive-is-a-plain-sorted-zero-timestamp-tar)
- [D-040 — the key in `[dependencies]` is the package name](#d-040--the-key-in-dependencies-is-the-package-name)
- [D-041 — the standard library is whichever dependency declares the items](#d-041--the-standard-library-is-whichever-dependency-declares-the-items)
- [D-042 — a workspace is the unit of resolution, and a lone package is one member](#d-042--a-workspace-is-the-unit-of-resolution-and-a-lone-package-is-one-member)
- [D-043 — inheritance is taken whole, and paths belong to the root](#d-043--inheritance-is-taken-whole-and-paths-belong-to-the-root)
- [D-044 — a library is a package with no entry point, and tests are per package](#d-044--a-library-is-a-package-with-no-entry-point-and-tests-are-per-package)
- [D-045 — a package is its archive, and the archive is its digest](#d-045--a-package-is-its-archive-and-the-archive-is-its-digest)
- [D-046 — a library may omit `entry`](#d-046--a-library-may-omit-entry)
- [D-047 — replacement is invisible to resolution](#d-047--replacement-is-invisible-to-resolution)
- [D-048 — the manager's diagnostics start at `SL10xx`](#d-048--the-managers-diagnostics-start-at-sl10xx)
- [D-049 — a git package is named by its repository, its reference and its commit](#d-049--a-git-package-is-named-by-its-repository-its-reference-and-its-commit)
- [D-050 — a git package's bytes are `git archive`, normalized](#d-050--a-git-packages-bytes-are-git-archive-normalized)
- [D-051 — a git package declares no `path` dependencies](#d-051--a-git-package-declares-no-path-dependencies)
- [D-052 — a registry is a static index tree, identified by its index URL](#d-052--a-registry-is-a-static-index-tree-identified-by-its-index-url)
- [D-053 — there is no built-in default registry](#d-053--there-is-no-built-in-default-registry)
- [D-054 — a published package depends only on registries and the toolchain](#d-054--a-published-package-depends-only-on-registries-and-the-toolchain)
- [D-055 — the index is a hint, and the fetched manifest is the authority](#d-055--the-index-is-a-hint-and-the-fetched-manifest-is-the-authority)
- [D-056 — a signature covers a statement, not a digest](#d-056--a-signature-covers-a-statement-not-a-digest)
- [D-057 — trusted keys are configuration, and no keys means no signatures](#d-057--trusted-keys-are-configuration-and-no-keys-means-no-signatures)
- [D-058 — the signature is checked where bytes are used, not where they arrive](#d-058--the-signature-is-checked-where-bytes-are-used-not-where-they-arrive)
- [D-059 — publishing writes a static tree and never rewrites one](#d-059--publishing-writes-a-static-tree-and-never-rewrites-one)
- [D-060 — a signing key is a file, and its mode is part of the check](#d-060--a-signing-key-is-a-file-and-its-mode-is-part-of-the-check)
- [D-061 — the Nix bridge reads the lock and never resolves](#d-061--the-nix-bridge-reads-the-lock-and-never-resolves)
- [D-062 — v0.5 ships as a patch-release series](#d-062--v05-ships-as-a-patch-release-series)
- [D-063 — the compiler stops owning input and output](#d-063--the-compiler-stops-owning-input-and-output)
- [D-064 — an `extern` declaration is the safety boundary](#d-064--an-extern-declaration-is-the-safety-boundary)
- [D-065 — the FFI vocabulary is closed, and an `extern` borrows](#d-065--the-ffi-vocabulary-is-closed-and-an-extern-borrows)
- [D-066 — core requires an allocator and does not supply one](#d-066--core-requires-an-allocator-and-does-not-supply-one)
- [D-067 — `unsafe`, raw pointers and volatile, designed before they are built](#d-067--unsafe-raw-pointers-and-volatile-designed-before-they-are-built)
- [D-068 — traits are settled by a written gate, not by taste](#d-068--traits-are-settled-by-a-written-gate-not-by-taste)
- [D-069 — freestanding before 1.0, UEFI after](#d-069--freestanding-before-10-uefi-after)
- [D-070 — `--offline` means no network, not no resolution](#d-070----offline-means-no-network-not-no-resolution)
- [D-071 — a code marks a refusal, not a failure](#d-071--a-code-marks-a-refusal-not-a-failure)
- [D-072 — a package with no `entry` is entered through `<source>/lib.slp`](#d-072--a-package-with-no-entry-is-entered-through-sourcelibslp)
- [D-073 — an `extern` reaches the backends as a table and a shared plan](#d-073--an-extern-reaches-the-backends-as-a-table-and-a-shared-plan)
- [D-074 — the C boundary is narrowed at the call site, and is not variadic](#d-074--the-c-boundary-is-narrowed-at-the-call-site-and-is-not-variadic)
- [D-075 — `c-sources` belong to the package, and they are cache inputs](#d-075--c-sources-belong-to-the-package-and-they-are-cache-inputs)
- [D-076 — the bundled library is files on disk, owned by one crate](#d-076--the-bundled-library-is-files-on-disk-owned-by-one-crate)
- [D-077 — a lone file is a package of one module, and it gets the library](#d-077--a-lone-file-is-a-package-of-one-module-and-it-gets-the-library)
- [D-078 — input and output are monomorphic functions, because there are no traits](#d-078--input-and-output-are-monomorphic-functions-because-there-are-no-traits)
- [D-079 — the runtime's string entry points take a pointer and a length](#d-079--the-runtimes-string-entry-points-take-a-pointer-and-a-length)
- [D-080 — the seam is four symbols, because a message is a contract](#d-080--the-seam-is-four-symbols-because-a-message-is-a-contract)
- [D-081 — the environment is the target's default and the command line's choice](#d-081--the-environment-is-the-targets-default-and-the-command-lines-choice)
- [D-082 — `std` depends on `core`, and re-exports what makes it the library](#d-082--std-depends-on-core-and-re-exports-what-makes-it-the-library)
- [D-083 — `string` belongs to `core`, and `std:string` re-exports it](#d-083--string-belongs-to-core-and-stdstring-re-exports-it)
- [D-084 — a file is read and written whole, because a handle has no destructor](#d-084--a-file-is-read-and-written-whole-because-a-handle-has-no-destructor)
- [D-085 — a C failure crosses as a status slot, read after the call](#d-085--a-c-failure-crosses-as-a-status-slot-read-after-the-call)
- [D-086 — the integer printers stay and stop being C, and the `i32` ones go](#d-086--the-integer-printers-stay-and-stop-being-c-and-the-i32-ones-go)
- [D-087 — parsing yields an `Option`, and the library stops aborting](#d-087--parsing-yields-an-option-and-the-library-stops-aborting)
- [D-088 — traits are refused for 1.0, and the gate's real answer is `=`](#d-088--traits-are-refused-for-10-and-the-gates-real-answer-is-)
- [D-089 — `=` compares scalars, and `D-012` stops being half false](#d-089---compares-scalars-and-d-012-stops-being-half-false)
- [D-090 — a conversion is a form with a target type, `(as i64 value)`](#d-090--a-conversion-is-a-form-with-a-target-type-as-i64-value)
- [D-091 — `clone` crosses a borrow, and refuses to do nothing](#d-091--clone-crosses-a-borrow-and-refuses-to-do-nothing)
- [D-092 — a function is a value](#d-092--a-function-is-a-value)
- [D-093 — the library grows to the point of being worth freezing](#d-093--the-library-grows-to-the-point-of-being-worth-freezing)
- [D-094 — `Map` and `Set` return, parameterised rather than constrained](#d-094--map-and-set-return-parameterised-rather-than-constrained)
- [D-095 — a generic function can take a generic type](#d-095--a-generic-function-can-take-a-generic-type)
- [D-096 — an empty collection literal takes its element type from context](#d-096--an-empty-collection-literal-takes-its-element-type-from-context)
- [D-097 — formatting an `f64` is `core`, in Slopium, over the bits](#d-097--formatting-an-f64-is-core-in-slopium-over-the-bits)
- [D-098 — a printed `f64` is plain decimal, seventeen digits, ties to even](#d-098--a-printed-f64-is-plain-decimal-seventeen-digits-ties-to-even)
- [D-099 — `match` works through a shared borrow](#d-099--match-works-through-a-shared-borrow)
- [D-100 — reading a borrow is `clone`, and there is no dereference](#d-100--reading-a-borrow-is-clone-and-there-is-no-dereference)
- [D-101 — a function value is an owned closure](#d-101--a-function-value-is-an-owned-closure)
- [D-102 — a lambda names what it closes over](#d-102--a-lambda-names-what-it-closes-over)
- [D-103 — a list element can be replaced, and that is the only write there is](#d-103--a-list-element-can-be-replaced-and-that-is-the-only-write-there-is)
- [D-104 — `Map` and `Set` are library types over a hash and an equality](#d-104--map-and-set-are-library-types-over-a-hash-and-an-equality)
- [D-105 — an expectation is substituted, normalized, and passed inward](#d-105--an-expectation-is-substituted-normalized-and-passed-inward)
- [D-106 — the operator and literal vocabulary is completed before the freeze](#d-106--the-operator-and-literal-vocabulary-is-completed-before-the-freeze)
- [D-107 — integers get width and signedness, and the library does not double](#d-107--integers-get-width-and-signedness-and-the-library-does-not-double)
- [D-108 — concurrency is shared-nothing, after 1.0, and the freeze reserves for it](#d-108--concurrency-is-shared-nothing-after-10-and-the-freeze-reserves-for-it)
- [D-109 — macros are deferred, and the freeze reserves the namespace](#d-109--macros-are-deferred-and-the-freeze-reserves-the-namespace)
- [D-110 — a microcontroller is a word size before it is an instruction set](#d-110--a-microcontroller-is-a-word-size-before-it-is-an-instruction-set)
- [D-111 — a freeze that changes something is not a freeze](#d-111--a-freeze-that-changes-something-is-not-a-freeze)
- [D-112 — the six calls the vocabulary decision left open](#d-112--the-six-calls-the-vocabulary-decision-left-open)
- [D-113 — the calls the integer axis left open](#d-113--the-calls-the-integer-axis-left-open)
- [D-114 — the volatile invariant splits, because a verifier sees one module](#d-114--the-volatile-invariant-splits-because-a-verifier-sees-one-module)
- [D-115 — the object model and the environment are separate patches](#d-115--the-object-model-and-the-environment-are-separate-patches)
- [D-116 — a panic trampoline lives in the section that branches to it](#d-116--a-panic-trampoline-lives-in-the-section-that-branches-to-it)
- [D-117 — the linker script is `[build]`'s, and an environment decides four things](#d-117--the-linker-script-is-builds-and-an-environment-decides-four-things)
- [D-118 — port-mapped I/O crosses the C boundary, and does not become an operator](#d-118--port-mapped-io-crosses-the-c-boundary-and-does-not-become-an-operator)
- [D-119 — the booted image is a 32-bit re-wrap, because the loader refuses a 64-bit one](#d-119--the-booted-image-is-a-32-bit-re-wrap-because-the-loader-refuses-a-64-bit-one)
- [D-120 — a field is assigned through a place, not through an address](#d-120--a-field-is-assigned-through-a-place-not-through-an-address)
- [D-121 — the everyday forms, and where a type is written down](#d-121--the-everyday-forms-and-where-a-type-is-written-down)
- [D-122 — an annotation is a list before the name, and a warning belongs to a compilation](#d-122--an-annotation-is-a-list-before-the-name-and-a-warning-belongs-to-a-compilation)
- [D-123 — the version belongs to the release, and the decision log is public](#d-123--the-version-belongs-to-the-release-and-the-decision-log-is-public)
- [D-124 — the C boundary opens by three rows, and none of them is a slice](#d-124--the-c-boundary-opens-by-three-rows-and-none-of-them-is-a-slice)
- [D-125 — `format` is reserved at the freeze, and nothing is built](#d-125--format-is-reserved-at-the-freeze-and-nothing-is-built)
- [D-126 — a temporary is borrowed where a call takes it, and dies there](#d-126--a-temporary-is-borrowed-where-a-call-takes-it-and-dies-there)
- [D-127 — a body is as many expressions as it needs, and a one-sided condition is `when`](#d-127--a-body-is-as-many-expressions-as-it-needs-and-a-one-sided-condition-is-when)
- [D-128 — a manifest survives a key it does not know, and a config does not](#d-128--a-manifest-survives-a-key-it-does-not-know-and-a-config-does-not)
- [D-129 — hexadecimal is two functions and an uppercase table](#d-129--hexadecimal-is-two-functions-and-an-uppercase-table)
- [D-130 — failing on purpose, and a failing test that says what it compared](#d-130--failing-on-purpose-and-a-failing-test-that-says-what-it-compared)
- [D-131 — the release page is generated from the titles that were merged](#d-131--the-release-page-is-generated-from-the-titles-that-were-merged)
- [D-132 — an optimistic analysis that did not settle is an error, not a result](#d-132--an-optimistic-analysis-that-did-not-settle-is-an-error-not-a-result)
- [D-133 — a deferred expression runs before the scope's drops, and may not move](#d-133--a-deferred-expression-runs-before-the-scopes-drops-and-may-not-move)
- [D-134 — `;;` above a declaration is documentation, read out of the trivia](#d-134--above-a-declaration-is-documentation-read-out-of-the-trivia)
- [D-135 — the manifest says what a module is for each target](#d-135--the-manifest-says-what-a-module-is-for-each-target)
- [D-136 — a declaration says which target it is for, and `cfg` is not built](#d-136--a-declaration-says-which-target-it-is-for-and-cfg-is-not-built)
- [D-137 — a changelog entry is a file, because two of them are not a conflict](#d-137--a-changelog-entry-is-a-file-because-two-of-them-are-not-a-conflict)
- [D-138 — the verifier bounds every index it can, and says which it cannot](#d-138--the-verifier-bounds-every-index-it-can-and-says-which-it-cannot)
- [D-139 — composition is a form, and applied it is the nesting](#d-139--composition-is-a-form-and-applied-it-is-the-nesting)
- [D-140 — a fieldless enum is its tag, and `=` reaches one](#d-140--a-fieldless-enum-is-its-tag-and--reaches-one)
- [D-141 — a call through a block built in the same breath names its symbol](#d-141--a-call-through-a-block-built-in-the-same-breath-names-its-symbol)
- [D-142 — an object with no debug information is refused, not handed back](#d-142--an-object-with-no-debug-information-is-refused-not-handed-back)
- [D-143 — a form has two shapes, and a body starts on its own line](#d-143--a-form-has-two-shapes-and-a-body-starts-on-its-own-line)
- [D-144 — every refusal about ownership carries an `SL03xx` code](#d-144--every-refusal-about-ownership-carries-an-sl03xx-code)
- [D-145 — a string is built in one buffer, and the buffer is a `(List u8)`](#d-145--a-string-is-built-in-one-buffer-and-the-buffer-is-a-list-u8)
- [D-146 — a list can be written into, and the sort is a merge sort over indices](#d-146--a-list-can-be-written-into-and-the-sort-is-a-merge-sort-over-indices)
- [D-147 — the clock and the entropy are the operating system's, in nanoseconds and bytes](#d-147--the-clock-and-the-entropy-are-the-operating-systems-in-nanoseconds-and-bytes)
- [D-148 — a child process is started with its arguments joined by NUL, and its output is a descriptor a `defer` closes](#d-148--a-child-process-is-started-with-its-arguments-joined-by-nul-and-its-output-is-a-descriptor-a-defer-closes)
- [D-149 — a sigil stands before the form it applies to, and the reader expands it](#d-149--a-sigil-stands-before-the-form-it-applies-to-and-the-reader-expands-it)
- [D-150 — `$` nests the rest of the form it is written in](#d-150---nests-the-rest-of-the-form-it-is-written-in)
- [D-151 — one token closes every list a declaration left open](#d-151--one-token-closes-every-list-a-declaration-left-open)
- [D-152 — `deprecated` applies to a struct field, and warns wherever the field is named](#d-152--deprecated-applies-to-a-struct-field-and-warns-wherever-the-field-is-named)
- [D-153 — a borrow of a borrow is refused where it is written and where it is instantiated](#d-153--a-borrow-of-a-borrow-is-refused-where-it-is-written-and-where-it-is-instantiated)
- [D-154 — the manager reads the compiler's exit code, and stops naming a module it cannot blame](#d-154--the-manager-reads-the-compilers-exit-code-and-stops-naming-a-module-it-cannot-blame)
- [D-155 — a function longer than a conditional branch reaches is refused, and the limit is written down](#d-155--a-function-longer-than-a-conditional-branch-reaches-is-refused-and-the-limit-is-written-down)
- [D-156 — a test is emitted by the module that owns it, and the harness by the entry module](#d-156--a-test-is-emitted-by-the-module-that-owns-it-and-the-harness-by-the-entry-module)
- [D-157 — a deprecated name lives out its major version, and removing it costs the next](#d-157--a-deprecated-name-lives-out-its-major-version-and-removing-it-costs-the-next)
- [D-158 — an edition is named by a year, and a missing key means the first](#d-158--an-edition-is-named-by-a-year-and-a-missing-key-means-the-first)
- [D-159 — the compiler protocol stays internal at 1.0](#d-159--the-compiler-protocol-stays-internal-at-10)
- [D-160 — a fix is a named rule, tied to the release that made it necessary](#d-160--a-fix-is-a-named-rule-tied-to-the-release-that-made-it-necessary)

## D-001 — a native compiler, without LLVM

Status: approved · 2026-07-29

Slopium has its own MIR and its own architecture backends. LLVM is not a
dependency. The system assembler and linker driver stay as a small external
dependency.

## D-002 — the compiler and the manager are separate programs

Status: approved · 2026-07-29

`slopic` compiles what it is handed, as `rustc` does. `slopium` manages
projects, profiles, cache and tests, with Cargo's responsibilities but not
necessarily its syntax or its configuration design.

## D-003 — planning is private and local

Status: approved · 2026-07-29

Roadmap, plans, ideas, handoffs and coordination live in `.notes/`, which is
gitignored and excluded from build contexts. `AGENTS.md` describes how to find
and maintain them without exposing their content. Superseded in part by
`D-123`, which moves this log itself into the tracked documentation.

## D-004 — module syntax needs its own comparison first

Status: deferred · 2026-07-29

Rust's `mod`/`use` is not assumed. The parser, resolver, manifest and file
layout stay as they are until the alternatives have been compared on the same
examples and one has been chosen. Resemblance to Rust is not itself an
argument. Settled by `D-009`.

## D-005 — editor integration evolves toward a language server

Status: approved · 2026-07-29

The lightweight regexp syntax and omnifunc completion stay as a fallback. A
separate `slopium-lsp` process supplies semantic highlighting, scoped
completion, hover, definition, rename and diagnostics. Neovim integration keeps
working when that binary is absent.

## D-006 — nothing is published without being asked

Status: approved · 2026-07-29

Committing, pushing, publishing a package and opening a pull request are
actions taken on request and never on initiative. Staging implementation files
is ordinary work.

## D-007 — the daily-tooling milestone ships as patch releases

Status: approved · 2026-07-29

The correctness and tooling milestone is four independently verified releases —
diagnostics, formatting, the language server, and runtime and ABI closure —
each passing its own acceptance gate before the next is considered.

## D-008 — scalar process APIs before owned collections

Status: approved · 2026-07-29

Until `(List String)` has generated ownership glue, process input is scalar
builtins: `read-line`, `parse-i64`, `args-len` and `arg`. A runtime failure a
program can trigger prints a normalized message and exits 101 rather than
aborting.

## D-009 — modules are derived from paths, and symbols are colon-qualified

Status: approved · 2026-07-29

A module's identity comes from its `.slp` path relative to the package source
root. One colon separates namespace segments and enum constructors, as in
`geometry:vector:Point` and `Message:Text`, and a leading `:name` stays a
constructor field keyword. `(export ...)` collects public names and
`(take module Name (other :as alias))` makes file-wide aliases. Qualified
access observes privacy; there are no wildcard imports and no module
initialization, and every module cycle is rejected.

## D-010 — v0.2 is an incremental series, and breaks on purpose

Status: approved · 2026-07-29

v0.2 ships as independently verified patch releases: package modules,
dependencies and separate codegen, generics and the standard library, owned
collections, then borrow and control-flow extensions. The `::` separator and
atom-shaped reference types may be replaced outright, with machine-applicable
migration diagnostics rather than a parallel syntax kept alive.

## D-011 — the standard library is a replaceable dependency

Status: approved · 2026-07-29

`std = { toolchain = true }` resolves the library bundled with the compiler and
`std = { path = "..." }` goes through the ordinary path dependency. A compiler
special form such as `try` binds through validated language-item paths declared
in the library's manifest, rather than through a fixed source layout or a guess
at the shape of an enum.

## D-012 — parametric generics before traits

Status: approved · 2026-07-29

A generic declaration carries a parameter list after its name and type
applications are S-expressions. Bodies are checked parametrically: an
unconstrained type variable acquires no arithmetic, comparison or clone
ability. Reachable concrete uses are monomorphized. Traits and explicit bounds
stay deferred.

## D-013 — the v0.2 collection, loop and pattern forms

Status: approved · 2026-07-29

`(array values...)` makes an owned `(Array T N)` and `(slice (& collection)
start end)` makes a borrowed `(Slice T)`. A list's `pop` returns the configured
`(Option T)` and is unavailable without the `option` language item. Loops are
`(loop ...)`, `(while condition ...)`, `(break)` and `(continue)`. Patterns
recurse through positional enum payloads and named struct fields. Traits,
iterator syntax, array patterns and a value-carrying `break` stay deferred.

## D-014 — the project fixture suite is a release gate

Status: approved · 2026-07-29

Documented language and project behaviour is covered by standalone
`Slopium.toml` fixtures as well as by Rust tests. A passing fixture is
formatted, checked, run natively and put through the generated test harness; a
failing one asserts a stable diagnostic or native status 101 with a normalized
message. The categories are `pass`, `compile-fail`, `runtime-fail` and reusable
`dependencies`, and `scripts/project-tests.sh` is part of `scripts/verify.sh`.

## D-015 — entry-point validation depends on the analysis context

Status: approved · 2026-07-29

A compiler or manager build validates the executable entry point. Editor
analysis does not, so a module without a local `main` keeps its typed symbols,
completion and navigation. For that purpose an entry named `lib.slp` is an
ordinary library and a manifest defining `[language-items]` is a standard
library; neither needs `main`. This is an analysis mode rather than new package
syntax, and an executable entry module still reports `SL0401`.

## D-016 — v0.3 ships as a patch-release series

Status: approved · 2026-07-29

The backend milestone is delivered the way `D-010` delivered v0.2: MIR
foundation, then the pass framework, then liveness and linear-scan allocation,
then DWARF line tables, then target-independent builtin lowering and the
AArch64 backend, then the relocatable ELF writer. The order is forced by real
dependencies — live intervals need the CFG utilities, DWARF needs per-statement
spans, a second backend needs builtin lowering hoisted out of the first, and
writing objects directly needs two backends to prove the interface.

## D-017 — "SSA-like MIR" means analyzable, not phi nodes

Status: approved · 2026-07-29

The requirement is MIR that is verified, def/use-analyzable and has computable
live intervals. It is not SSA construction, phi nodes or block parameters.
Numbered locals stay, because linear scan needs live intervals over a linear
block order and the CFG utilities give that directly. Full SSA would mean
rewriting the lowering of `if` and `match` and the drop-elaboration merge that
compares per-branch liveness, which is where ownership correctness physically
lives. Revisit only if a specific pass is shown to need it.

## D-018 — inlining never crosses a module boundary

Status: approved · 2026-07-29

`slopium` emits one object per owner module and keys its cache on that module's
body plus every module's interface, so a body-only change to one module does
not rebuild another's object. Inlining across the boundary would leave the
caller's object holding a stale copy with nothing able to notice, and the
obvious fix — every body in every key — ends incremental builds. So the inliner
requires caller and callee to share an owner module, derived from the symbol
prefix before the final colon. The cost is that the entry function cannot
inline its own module's helpers, because it carries no prefix.

## D-019 — optimization preserves traps and drops

Status: approved · 2026-07-29

Integer overflow and division by zero panic with a normalized message and exit
101, so constant folding answers "not a constant" rather than wrapping, and
dead code elimination never removes a `Binary`: the trap is the effect.
`Drop` and `Free` are how memory is released, so they are never dead, a `Drop`
counts as a use of its operand, and no pass moves a drop across a branch.
Blocks merge only when statement order is preserved exactly.

## D-020 — a local gets one register, or one slot, for the whole function

Status: approved · 2026-07-30

Live intervals are never split and no reload is inserted inside one. That is
what makes register allocation a substitution of operands inside the existing
instruction selection rather than a rewrite of it, and it makes spilling free:
a local that loses the scan keeps the frame slot it would have had anyway, so a
spill can neither fail nor add an instruction. The cost is allocation quality —
a value live across a long stretch it is never mentioned in holds its register
throughout. Revisit on a measurement, not on the literature.

## D-021 — a calling function allocates only callee-saved registers

Status: approved · 2026-07-30

A function whose code contains a call that returns may allocate only `rbx` and
`r12`–`r15`. They survive a call by ABI, so allocation needs no clobber model
and no save or restore around a call site: the question is answered once, in
the prologue. A function that calls nothing takes `r10` and `r11` first — they
are caller-saved, this backend never touches them, and they are never argument
registers, which matters because storing a parameter into a register a later
one is still arriving in would corrupt it. Leafness is judged conservatively:
claiming a call that is not emitted costs an opportunity, claiming a leaf that
is not one corrupts `r10`.

## D-022 — an address-taken local is pinned to memory

Status: approved · 2026-07-30

`lea` has no register form, so any local whose address the backend hands to
something else stays in a frame slot: a borrowed scalar, each element a
collection constructor copies in, a value pushed onto a list, the destination
the runtime pops into. The pinning scan and the `lea` sites are two lists that
must agree and nothing in the type system makes them, so a test compiles the
whole corpus in both profiles and asserts that no emitted `lea` names a
register.

## D-023 — debug information is line tables and nothing else

Status: approved · 2026-07-30

`--debug` emits `.file` and `.loc` and lets the assembler build the line table.
It does not describe where a variable lives. Line tables cost one directive per
statement and use the span MIR already carries; variable locations would have
to name a register or a frame slot per local and would go stale the moment
allocation learns to split intervals. Two consequences are accepted rather than
worked around: a backtrace frame names the ELF symbol, so a Slopium function
appears as `sl_fn_<hex>`, and compiler-generated glue inherits the line before
it rather than being marked as having no source.

## D-024 — debug information may not change the instruction stream

Status: approved · 2026-07-30

Assembly built with `--debug` equals assembly built without it once the `.file`
and `.loc` lines are removed, and a test asserts exactly that. The alternative
is a debugger showing a program other than the one that ships. It is also cheap
to keep: the only place it was nearly lost is the peephole that deletes a `mov`
undoing the one before it, which stopped firing when a `.loc` landed between
the pair and now looks past location directives.

## D-025 — anything two backends must agree about lives outside both

Status: approved · 2026-07-30

Symbol names, the runtime helper that releases or copies a given type,
aggregate sizes and the lowering of every builtin are decided once, in
`lowering.rs`, and read by each backend. None of that is a property of a
machine, and a second backend that re-derived it would be free to derive it
differently — producing a name that does not link, or worse, one that links and
hands a value to the wrong drop helper. Neither is visible in either backend
alone, because each is internally consistent. What stays per backend is the
machine: register file, instruction selection, calling convention, assembly
syntax.

## D-026 — two backends are kept honest by differential execution

Status: approved · 2026-07-30

`scripts/cross-check.sh` compiles the corpus for both targets, runs the AArch64
build under `qemu-aarch64`, and requires identical stdout and exit status. It
inspects no generated code, because agreement on behaviour is the property
worth having. Three things the corpus does not reach are covered separately:
programs that panic, because the overflow and division checks are built out of
different material on the two targets; float comparison, because that is where
the two were in fact found to disagree; and the ABI, by linking Slopium
functions against a C caller with more arguments than either register class
holds. Agreement is not correctness, so a check whose expected output is a fact
about the language writes that output down rather than only comparing two runs.

## D-027 — float comparison excludes the unordered case

Status: approved · 2026-07-30

`(< a b)`, `(> a b)` and `(= a b)` on `f64` are false whenever either side is a
NaN, on every target and in the constant folder. The x86-64 backend did not do
this and the differential suite found it: `ucomisd` reports unordered with the
same flags it uses for below and for equal, so `setb` and a bare `sete` both
answered true for a NaN, while the folder — which uses Rust's own operators —
already answered false. The same program therefore gave different results
folded and unfolded, which is a stronger argument than the appeal to IEEE 754.
Less-than now compares the operands the other way round and asks `seta`, and
equality ands in the parity flag; AArch64 needed no change, because `fcmp`
reports unordered as a fourth outcome that `mi`, `gt` and `eq` all decline.

## D-028 — a backend emits a stream of items, not text

Status: approved · 2026-07-30

A backend produces `Vec<Item<Inst>>` — sections, labels, symbol attributes,
data and typed instructions. Assembly text and a relocatable ELF object are two
readings of that one stream; neither is derived from the other. The alternative
was an assembler that parsed the text back, which would have been a smaller
change and was rejected because an interface whose currency is text makes every
syntax detail a contract between two places that can drift. With a typed
stream, a form the encoder does not implement fails to compile rather than to
assemble. The refactor was gated as `D-025`'s was: 40 compilations across the
corpus, both targets, both profiles, byte-identical assembly against the
previous compiler, twice.

Three consequences. The peephole moved with it and now asks
`Instruction::undo`, which each architecture answers for its own instruction
set. Object writing stops at line tables, so a `--debug` build falls back to
the assembler automatically rather than silently losing its line table. And
linking stays external on purpose: assembling was code generation and has been
taken back, but linking is where the C runtime, the loader and the platform's
idea of starting a process live.

## D-029 — the object writer is checked against the assembler, not trusted

Status: approved · 2026-07-30

Every encoding in both instruction modules is written down as the bytes `as`
produced for the same text, and `scripts/object-check.sh` re-checks the corpus
both ways on every run: byte for byte on AArch64, instruction by instruction on
x86-64, plus relocations, symbol tables and execution. Byte-for-byte works on
AArch64 because instructions are fixed width and there is no choice to make; on
x86-64 it would mean matching the assembler's relaxation and its
accumulator-specific immediate forms, so the comparison is per instruction with
addresses normalized away. This is a higher standard than any proof this
project could carry out, and it is what found a 32-bit `movz` bug — an encoding
correct in its result and wrong in its bits, which no test written from the
same understanding that wrote the encoder would have caught.

## D-030 — a release binary carries only what it uses, and no names

Status: approved · 2026-07-30

Three subtractions, all at link time, all resting on the compiler now knowing
exactly which symbols exist. The runtime is compiled with
`-ffunction-sections -fdata-sections` and linked with `--gc-sections`, so a
program that never touches a slice does not carry `sl_rt_slice_*`; on the
corpus a small program drops from about 22 KB to about 14 KB and 27 of 32
runtime functions disappear. Stripping is a flag rather than a profile:
`slopic` exposes `--strip` and decides nothing, while a manifest profile
carries `opt-level`, `debug` and `strip`, with `strip` defaulting to the
opposite of `debug`. And a test body is emitted only under `--test`, which is a
fix rather than a flag — an ordinary release binary used to carry every
`sl_test_*` function.

The CLI contract moved with it, so `COMPILER_PROTOCOL` went from 3 to 4 and a
mismatched pair fails the handshake instead of erroring on an unknown flag. Two
things were considered and not done: per-function sections, which need the
object model to carry many `.text` sections and were left for `D-030`'s
successor, and obfuscation, which is security through obscurity, fights the
differential and debug machinery the compiler is built on, and would be hard to
keep correct across two backends.

## D-031 — dead trap strings out, panic messages an opt-in, checks untouchable

Status: approved · 2026-07-30

Three things that are easy to run together are separated. A check and its
message are different: removing a bounds, overflow or zero-divisor check trades
a few bytes for undefined behaviour, and no profile may ask for it. Dead trap
messages should not ship, so `lowering::trap_usage` says which trampolines a
function's arithmetic can reach and each backend emits only those — pure
subtraction of dead data, with a trapping program still trapping and still
saying the same thing. Dropping the message text is a choice, so it is a knob:
`panic = "abort"` reaches `slopic --panic-abort`, the trampolines call a
message-less `sl_rt_abort`, and the runtime is compiled to route its own
failures through the same bare exit. The default stays `"message"`, because a
crash that says nothing is a worse default than a few bytes of string.

## D-032 — the flake is the installation interface, and the scripts stay

Status: approved · 2026-07-30

The package expression is one function used twice, so `packages.<system>` and
`overlays.default` cannot drift. The machine and the account get different
modules: `nixosModules.default` installs the three binaries and the overlay, and
editor support lives in `homeModules.default`, because a module that assumes the
importer runs home-manager fails for the importer who does not. Completions are
generated by the binaries during `postInstall` rather than checked in, since a
committed completion script is a copy of a CLI that nothing verifies. The
version is read from `Cargo.toml`, which was already one release behind when it
was written twice. The install scripts stay as the answer for a non-NixOS
checkout and for developing the plugin against the working tree; running them
alongside the module is a conflict, because both own the same paths.

## D-033 — v0.4 ships as a patch-release series

Status: approved · 2026-07-31

The package-management milestone is delivered the way `D-010` and `D-016`
delivered theirs: one manifest, one resolver and one lock; then workspaces;
then reproducible archives, the store and vendoring; then git dependencies;
then the registry client; then signing, publishing and the Nix bridge. The
order is forced by dependencies — the lock needs package identity, the store
needs a specified archive, git needs the store, the registry needs the
resolver, and signing needs something to publish.

## D-034 — one manifest and one resolver, outside both consumers

Status: approved · 2026-07-31

`Slopium.toml` had two readers: the manager's, and a second one in the language
server that parsed a subset and walked the dependency graph again. The crate
`slopium-manifest` owns the manifest schema, semver, package identity, the
resolver, the lockfile and the bundled library, and both binaries consume it.
This is `D-025`'s rule applied to the manifest, and it does not weaken `D-002`:
the crate is manager-side, and the compiler still receives nothing but
`--dependency ALIAS=ROOT`.

## D-035 — a package is identified by name and version, and appears once

Status: approved · 2026-07-31

Resolution used to give a dependency the namespace of the path it was reached
by, so a package reached two ways became two packages with two namespaces and
two copies in the binary. A resolved graph now holds each package once, keyed
by name and version, and its namespace is its package name. Without that a
lockfile cannot describe the graph unambiguously, a content-addressed store has
nothing stable to address, and a registry has no identity to resolve. It is a
breaking change — generated symbols, `--dependency` aliases and the visibility
of transitive dependencies all move — so `COMPILER_PROTOCOL` went from 4 to 5.

## D-036 — semver ranges, with maximal selection

Status: approved · 2026-07-31

Requirements are `^`, `~`, `=`, `>=` and `<`, comma-joined, and a bare `1.2.3`
means `^1.2.3`. Selection takes the highest compatible version and backtracks
on conflict. A prerelease is never selected unless a requirement names one.
Minimal version selection is the simpler algorithm and was rejected: it makes
every dependent's build depend on the lowest version anyone declared, which is
not what the manifest shape this language borrows has taught people to expect.
Two incompatible majors of one name in one graph is an error, because allowing
them would reintroduce the duplicate instances `D-035` removed.

## D-037 — our checksums, borrowed transports, one crypto crate

Status: approved · 2026-07-31

SHA-256 is implemented in-tree and checked against `sha256sum` on every run,
the way the object writer is checked against `as` (`D-029`). The build-cache
hash stays FNV-1a and stays a freshness check rather than a security boundary.
`git` and `curl` are invoked as external programs for the reason `cc` is: they
know about transports, credentials, proxies and shallow fetch, and `D-001`
forbade a mandatory compiler backend rather than calling out at all.
`ed25519-dalek` is the one new runtime dependency, because hand-writing field
arithmetic is where writing it ourselves stops paying: a wrong ELF byte fails
loudly against the assembler, and a wrong scalar multiplication does not.

## D-038 — a package name resolves from exactly one source in a graph

Status: approved · 2026-07-31

Dependency confusion is the failure where a name resolves somewhere the author
did not mean, and the protections against it are structural rather than
heuristic. There is no built-in default registry URL, so an unconfigured
default is an error and there is nothing to typosquat out of the box. Two
dependents naming one package from different registries is an error rather than
a silent merge. A path or git dependency never satisfies a registry requirement
of the same name unless the root package says so. And the lock records the
source, so a changed source invalidates `--locked`. Each case has its own
diagnostic code and its own fixture.

## D-039 — an archive is a plain, sorted, zero-timestamp tar

Status: approved · 2026-07-31

ustar, entries sorted by path, `mtime` and ownership zero, modes normalized to
0644 and 0755, no symlinks or device nodes, one `<name>-<version>/` prefix, and
no compression: the checksum is over the tar, and a transport may compress if
it likes. Made this way, reproducible is a property `cmp` decides rather than
one anybody argues about, and the store can address an archive by the hash of
exactly the bytes it received.

## D-040 — the key in `[dependencies]` is the package name

Status: approved · 2026-07-31

`D-035` makes a package's namespace its name, so a dependent cannot choose a
different key for it: `math = { path = "../mathlib" }` promises a namespace
nothing produces. The key equals the dependency's `[package] name`, and a
mismatch is an error naming both. Renaming is deliberately not offered — it only
makes sense once two packages can legitimately want one key, which is a registry
problem, and it would reopen what `D-035` closed.

## D-041 — the standard library is whichever dependency declares the items

Status: approved · 2026-07-31

Resolution used to read `[language-items]` only from a dependency whose alias
was literally `std`, which made the library an ordinary dependency with a magic
name. Language items now come from whichever direct dependency of the root
declares them, and two such dependencies in one graph is an error. Only the
root's direct dependencies are consulted: a library that pulled in its own
standard library would otherwise change the language for whoever depends on it,
which is not a decision a dependency gets to make.

## D-042 — a workspace is the unit of resolution, and a lone package is one member

Status: approved · 2026-07-31

`[workspace] members` sits in a root manifest, which need define no package of
its own, and there is one lock and one `target/` at that root. Resolution covers
every member at once, because the lock does: if `build -p a` locked only what
`a` reached, `build -p b` would rewrite it and `--locked` would fail for
whichever ran second. One name resolves to one version across the workspace, and
two members disagreeing is an error naming both. A lone package loads as a
workspace of one member, which is what removes the special case rather than
adding one. A package inside a workspace directory but not listed in `members`
is not a member and loads as its own workspace, because silently adopting it
would put it under a lock it never asked for.

## D-043 — inheritance is taken whole, and paths belong to the root

Status: approved · 2026-07-31

`version.workspace = true` and `dep = { workspace = true }` take the workspace's
value entire; writing `{ workspace = true, version = "^2" }` would leave one
entry owned by two files, so it is an error rather than a precedence rule
nobody should have to remember. An inherited `path` is rebased onto the
workspace root, since that is what the root manifest wrote it relative to.
`members` understands a trailing `*` and nothing else: `crates/*` is what people
write, and a general glob language is a dependency and a surface area.

## D-044 — a library is a package with no entry point, and tests are per package

Status: approved · 2026-07-31

A package entered through `lib.slp` is a library, and `slopic --library`
compiles one without requiring `main`; `COMPILER_PROTOCOL` went 5 to 6 so a
mismatched pair fails the handshake rather than an unknown flag. `build` on a
library means `check`, because it has nothing to link, and `run` on one is an
error — erroring on `build` instead would make `build --workspace` useless in
any workspace holding a library. A package's test harness carries that package's
tests only: codegen emits a test body in the object that owns the module, so a
harness that counted a dependency's tests emitted calls no object defined.

## D-045 — a package is its archive, and the archive is its digest

Status: approved · 2026-07-31

`D-039`'s format, implemented: normalized modes, zero ownership and timestamps,
sorted entries, filled-in parent directories, one prefix, and links and device
nodes refused in both directions. The store keeps two things and the difference
matters — `archives/<digest>.sl.tar` is the package, the bytes somebody hashed,
and `store/<digest>/` is only its unpacked form, a cache that exists because a
compiler reads files. So the archive is what is verified, before anything is
unpacked and again whenever the tree is used, and the tree can be deleted
without losing anything. Extraction goes to a temporary directory and is renamed
into place, so a concurrent build sees the finished tree or nothing.

`Slopium.lock` gained `checksum` and moved to format 2. Only a source whose
bytes cannot change under the lock carries one; a path dependency is a working
tree and gets none. A lock this toolchain cannot read is regenerated with a
message rather than refused, being a build product derived from the manifests —
except under `--locked`, which asked for the opposite.

## D-046 — a library may omit `entry`

Status: approved · 2026-07-31

`[package] entry` is optional, and omitting it says the package is a library:
the same thing `lib.slp` said, said directly. A library has no module a build
starts from and is entered through whichever of its modules a dependent takes
from. This is what lets the bundled library be written out as an ordinary
package with an honest manifest, which is what vendoring needs.

## D-047 — replacement is invisible to resolution

Status: approved · 2026-07-31

`slopium vendor` writes `[source.<name>] replace-with` into
`.slopium/config.toml` and copies the packages into a vendor directory. A
replaced package keeps its identity, its source and its lock entry; only the
bytes handed to the compiler come from elsewhere, so vendoring cannot change
what a project resolves to and `check --locked` passes across it. A vendored
copy is verified by re-archiving it and comparing digests — the format has room
for nothing but names and contents, so the tree that produced a digest is the
only tree that reproduces it — and it is left writable and checked on every
build rather than made read-only and trusted. `vendor` itself resolves with
replacements ignored, since it is what produces the copies.

## D-048 — the manager's diagnostics start at `SL10xx`

Status: approved · 2026-07-31

Packaging is the first part of the manager that refuses input for reasons a
person needs to look up — a path that escapes an archive, a checksum that does
not match — so its errors carry stable codes in the `SL10xx` range. The rest of
the manager's messages stay prose until their own patch; converting them halfway
while adding a feature is worse than either state.

## D-049 — a git package is named by its repository, its reference and its commit

Status: approved · 2026-07-31

A git source id is `git+<url>#<40 hex>`, with the reference the manifest asked
for kept in a query and nothing at all for the default branch. The commit is the
fragment because it is what was resolved; the reference is in the id because
dropping it would leave a manifest that changed branches pinned to the old
commit forever, there being nothing in the lock left to disagree with. Two
dependents naming one package from one repository by different references is an
error, the way two registries for one name is. Resolution always pins a full
commit: a branch is how a commit is found once, never what is recorded.

## D-050 — a git package's bytes are `git archive`, normalized

Status: approved · 2026-07-31

Fetching shells out to `git`: a bare repository under `$SLOPIUM_HOME/git/db/`,
an explicit refspec, then `git archive` of the pinned commit. That tar is read
for what the package format can hold — files, directories, contents — and
everything else is discarded, then written back out through the ordinary archive
writer, so a git package is the same kind of object as a published one and is
addressed the same way. The lock records the commit and the archive digest, so
anyone with the repository can re-derive the second from the first, and a stored
copy is checked against the digest rather than against a repository that may
have been rewritten. Submodules are not fetched: a tree containing `.gitmodules`
says so at resolve time rather than building something quietly incomplete.

## D-051 — a git package declares no `path` dependencies

Status: approved · 2026-07-31

A package fetched from git is unpacked into the content-addressed store, so a
`path` dependency it declares resolves either to a machine-specific absolute
path under `$SLOPIUM_HOME` — which is exactly what a lock must not record — or,
with a `..`, to somewhere outside the package. Both have real answers that are
not in this scope, and half an answer written into the lock format is worse than
none, so such a package is refused with a message. A git package may still
depend on the toolchain and on other git packages.

## D-052 — a registry is a static index tree, identified by its index URL

Status: approved · 2026-07-31

A registry is a directory a file server serves: `index/<prefix>/<name>.json`,
one JSON object per line and one line per version, beside
`packages/<name>/<name>-<version>.sl.tar`. The prefix fans the index out by name
length so a large index is a tree rather than one enormous directory, and a line
that cannot be parsed is an error naming the file, because a half-understood
index is worse than an unread one.

The index URL is the identity and the local nickname is not: the lock records
`registry+<index url>`, so two developers who call one index by two names still
produce one lockfile, and the identity an attacker would have to change to
redirect a build is exactly what `--locked` refuses. `file://` and `https://`
are transports and a scheme-less value is a path relative to the workspace root;
HTTPS goes through `curl` with a fixed argument list no configuration can
extend. `http://` is accepted only for a loopback host, because whoever answers
a plaintext index chooses which version a first resolution pins and supplies the
checksum that would have caught tampered bytes. Unknown fields in an index entry
are ignored, so a later format can add one without an older client refusing to
read the index.

## D-053 — there is no built-in default registry

Status: approved · 2026-07-31

`dep = "^1.2"` means the registry configured as `default`, and neither that name
nor any other ships with a URL, so a checkout that configures nothing gets an
error rather than a download. It is the cheapest dependency-confusion protection
there is: a name can only be taken from a registry somebody on this machine
wrote down, and there is no ambient index a private package name could be
silently resolved against.

## D-054 — a published package depends only on registries and the toolchain

Status: approved · 2026-07-31

A package taken from a registry may declare neither a `path` dependency, for
`D-051`'s reason, nor a `git` one: a git URL inside an index entry is a fetch
the index's author chooses and the consumer's lock cannot describe as a version
of anything. A dependency in an index entry that names no registry means the
registry the entry came from, never the consumer's default, so a package
published to an internal index cannot be made to reach a public one by a
consumer's configuration — which is the shape most dependency-confusion attacks
take. That is also why there is no cross-registry dependency at all: a registry
name in a manifest is a local nickname, and a fetched manifest's nicknames mean
nothing on the machine that fetched it.

## D-055 — the index is a hint, and the fetched manifest is the authority

Status: approved · 2026-07-31

Selection reads requirements out of the index, because downloading every
candidate to find out what it needs would defeat the point of having one. What
is built is checked against the archive: a fetched package whose manifest
disagrees with the index entry that selected it, or whose bytes disagree with
the checksum the index published, is refused. So the index is trusted to make
resolution fast and for nothing else, an index that lies is caught at the moment
the lie would first matter, and the lock records the digest of the archive
rather than anything the index said about it. A yanked version is not selected
but is still built when a lock already names it, because yanking is a statement
about new resolutions and a lockfile that stops working when somebody else edits
an index is not a lockfile.

## D-056 — a signature covers a statement, not a digest

Status: approved · 2026-07-31

Ed25519 signs the bytes `slopium-package-v1\n<name>\n<version>\n<digest>\n`
rather than the digest alone. Signing a bare hash makes a signature
transplantable: two packages whose archives are identical — easy to arrange,
since an attacker controls the contents of the one they publish — would share
one, and a signature lifted onto another name or version would still verify. The
statement names what is being asserted, and the prefix is what a second
statement format would have to change.

One encoding shape is used everywhere: `ed25519:<key>` is a public key and is
what `trusted-keys` holds, `ed25519-private:<seed>` is the whole content of a
key file, and `ed25519:<key>:<signature>` is a signature. A signature carries
the key that made it, because 64 opaque bytes cannot say who claims to have
produced them, and that would collapse "signed by somebody you have not listed"
— the ordinary case of a publisher rotating a key — into "does not verify". The
key in a signature is a claim rather than a grant: it is checked against
`trusted-keys` before it is used to verify anything.

## D-057 — trusted keys are configuration, and no keys means no signatures

Status: approved · 2026-07-31

A registry with `trusted-keys` admits only archives carrying a signature by one
of them, with separate codes for no signature, a signature that does not verify,
and a signature by a key nobody listed. A registry without them checks
signatures at all. There is no third state and in particular no trust on first
use: remembering whichever key signed the first download would make the first
fetch the trust decision, and the first fetch is exactly the one an attacker who
can answer for an index gets to choose. Rotating a key is adding the new one,
which is why it is a list.

## D-058 — the signature is checked where bytes are used, not where they arrive

Status: approved · 2026-07-31

The detached signature follows the archive into the store and is verified on
every checkout against the keys the current build trusts. Checking only on
arrival would put the decision in the wrong place: one `$SLOPIUM_HOME` is shared
by every project on a machine, so a project that configured no keys would admit
bytes that a stricter project then builds without ever verifying. Verifying at
checkout costs nothing worth measuring, works offline, and makes adding
`trusted-keys` to an existing checkout take effect on the next build rather than
on the next cache eviction.

## D-059 — publishing writes a static tree and never rewrites one

Status: approved · 2026-07-31

`slopium publish` is `slopium package` plus a signature plus three file writes —
the archive, the signature and one appended index line — into a directory
registry. There is no upload protocol because there is no server, and inventing
one to reach an `https://` index would be inventing the server too. A version
already in the index is refused: an index line is append-only, because somebody
else's lockfile may already name that version and digest, and a republished
version is the one change no lockfile can notice. Before signing, the archive is
unpacked and packed again and the bytes must be identical, which is the moment
to check the property the format was specified for.

## D-060 — a signing key is a file, and its mode is part of the check

Status: approved · 2026-07-31

`slopium key new` writes `ed25519:<hex seed>` at mode 0600 and prints the public
half to paste into `trusted-keys`; publishing refuses a key file readable by
group or other. Key material never appears in an argument, because
`/proc/<pid>/cmdline` is world-readable on the only platform this toolchain
targets, and never in an environment variable, because that is inherited by
every subprocess a build runs. The seed comes from `/dev/urandom`, which keeps
the single new cryptographic dependency to the signature scheme itself rather
than to a random-number stack underneath it.

## D-061 — the Nix bridge reads the lock and never resolves

Status: approved · 2026-07-31

`lib.buildSlopiumPackage` parses `Slopium.lock` in Nix, turns every registry
entry into a fixed-output derivation keyed by the checksum the lock already
records, assembles a package store from them, and builds offline and locked. Nix
does no version selection at all, which is what makes two identical locked
graphs a fact rather than a coincidence to be tested: there is one resolver, it
ran once, and both builds read its output. A git entry is refused by name — its
checksum is the digest of an archive this toolchain normalizes out of a git
export, which Nix cannot reproduce without running the toolchain — and the
message says that vendoring turns such a graph into one the bridge builds.

## D-062 — v0.5 ships as a patch-release series

Status: approved · 2026-07-31

Four independently verified patches: the `extern` declaration and the C FFI;
input and output moving out of the compiler into the library; the core and
hosted split in the runtime and the library; then the library growing to the
minimum plus files. The order is forced, and not in the direction the roadmap
implied. While `println` is a compiler builtin and the bundled library is two
modules, "core versus hosted" is a property of the compiler and a split drawn
there would be drawn in the wrong place. The FFI is what lets the library hold
input and output at all, so it goes first. v0.5 is also the last milestone that
may break the language: everything in it that breaks has to happen before the
freeze or it is a 2.0.

## D-063 — the compiler stops owning input and output

Status: approved · 2026-07-31

`print`, `println`, `read-line`, `read-i64`, `parse-i64`, `env`, `arg` and
`args-len` leave the builtin table and reappear as ordinary library functions
written in Slopium over `extern`. The `sl_rt_*` implementations stay in C; what
changes is that the compiler no longer knows their names. Collections and
strings stay builtins: they are a much larger surface, their lowering carries
the drop and clone glue `lowering.rs` owns for both backends, and moving them
buys nothing this milestone needs. The line is drawn at what a freestanding
target must not have rather than at what could theoretically move — a target
cannot decide it has no `stdout` while `stdout` is a keyword. The cost is that
every `.slp` in the repository, the fixtures and the documentation acquires an
import.

## D-064 — an `extern` declaration is the safety boundary

Status: approved · 2026-07-31

Before raw pointers exist there is no `unsafe` keyword, and what marks the edge
of the guarantee is the `extern` declaration itself: writing one vouches for the
function behind it. The consequence worth stating is that bare metal is
reachable from there — an MMIO poke, `outb`, `lidt` and the body of an interrupt
handler are written in a C file the project supplies and declared `extern`, so
the dangerous code lives where it lived anyway and every line of Slopium in such
a program is still covered. What is given up is writing the poke in Slopium,
which `D-067` is the decision about taking back. An `extern` declaration is
trusted input in the security model, in the same list as a path dependency, and
so is `[package] c-sources`, which is a build script by another name.

## D-065 — the FFI vocabulary is closed, and an `extern` borrows

Status: approved · 2026-07-31

The set of C signatures the language can express is enumerable rather than open,
so it is enumerated: the scalars map to their obvious C counterparts, with
`size_t` and `unsigned` documented mismatches rather than hidden ones. Three
cases are not scalars. `(& String)` renders as `const char *`, which is free
because a Slopium string already keeps a terminating NUL that nothing had a use
for. `(& (Slice T))` renders as a pointer and a length in two consecutive
arguments. A returned `String` is owned by the caller and must have been
allocated through `sl_rt_string_new`, without which the library's own input
functions cannot be written.

An `extern` may not take ownership of an argument: borrows and scalars only.
Ownership crossing into C would run the drop glue where the compiler cannot see
it, and there is no version of that the checker can still call a guarantee. An
`extern` call is opaque to the optimizer — never inlined, reordered or dropped —
which extends `D-019` to a call whose effects cannot be enumerated.

## D-066 — core requires an allocator and does not supply one

Status: approved · 2026-07-31

The runtime becomes two translation units. `slop_rt_core.c` holds the string,
list and slice logic and the panic trampolines; it calls `sl_rt_alloc`,
`sl_rt_free` and `sl_rt_abort` and defines none of them. `slop_rt_hosted.c`
defines those three over `malloc`, `free` and `exit` and adds stdio, `argv` and
`getenv`. The wording is the whole decision: the obvious split — core never
allocates — would have forced lists and strings into the hosted half, which is
exactly the half a kernel cannot have. A kernel has an allocator; what it does
not have is libc. So core is the code, hosted is the providers, and the seam is
three symbols.

The library splits the same way: `core` carries `Option`, `Result` and the
language items, `std` carries input, output and process and depends on `core`,
re-exporting the items rather than declaring its own so that exactly one direct
dependency declares them either way. The split is verified before any
freestanding target exists — a package depending only on `core` links with
`-nostdlib` and leaves no undefined libc symbol beyond the three hooks —
because this project does not freeze contracts it has not checked.

## D-067 — `unsafe`, raw pointers and volatile, designed before they are built

Status: approved · 2026-07-31

`D-064` puts the escape hatch in C; this is the shape of the version that takes
it back into the language, written while nothing depends on the answer, because
deciding it under the pressure of a specific kernel task is how such things go
wrong. It is a raw pointer type, a volatile read and write, and a block form
that permits them: a permission rather than a second type system. It turns off
the borrow checker's aliasing rules for values reached through a raw pointer and
nothing else — in particular not bounds or overflow checks, which `D-031` calls
untouchable. A volatile access must survive constant propagation, CFG
simplification and dead code elimination untouched and in order, which is the
same class of invariant as `D-019` and belongs in the verifier rather than in a
comment. And the security document's central claim acquires a condition, which
has to be written down before the feature exists rather than after: a guarantee
that quietly narrows is worse than one that was never claimed.

## D-068 — traits are settled by a written gate, not by taste

Status: approved · 2026-07-31

Traits are deferred, and the question of when to revisit is settled at the end
of v0.5 by a written record: every place in the library where a concrete type
was chosen because a bound could not be expressed, and every place where
`D-012`'s refusal to infer capabilities for an unconstrained type variable
forced duplication. Then the verdict — traits become the next milestone, or the
refusal is written down and 1.0 ships parametric. Without that, the question
would be settled by whoever was tired of working around it.

## D-069 — freestanding before 1.0, UEFI after

Status: approved · 2026-07-31

`x86_64-unknown-none` lands ahead of the freeze, not because bare metal is the
point of the project but because it is the only thing that exercises `D-066`
for real: no libc, no entry wrapper, a core-only library, a panic hook the
program supplies. Freezing the runtime ABI with that half unproven would be
freezing a guess. The x86-64 backend is reused unchanged — what differs is the
environment rather than the architecture — and what is new is the link step, the
entry point and section placement, which also unblocks per-function sections
deferred in `D-030`. UEFI stays out of 1.0: it needs a PE32+ object writer
beside the ELF one and the Microsoft x64 calling convention beside System V,
which is the size of the whole backend milestone. Legacy BIOS is a permanent
non-goal.

## D-070 — `--offline` means no network, not no resolution

Status: approved · 2026-07-31

Refusing to read any index offline was one rule covering two unlike things. A
registry that is a directory is read directly, offline or not, because reading a
directory was never a network operation. A registry that is a URL is read from a
per-index cache under `$SLOPIUM_HOME` that an online run fills as a side effect
of fetching. Three rules keep that from becoming a way to build against
something stale: an online run always fetches and overwrites, an online run that
finds a package gone deletes the cached copy, and nothing else is cached. A
cache write that fails is not a build failure, because the resolution it belongs
to has already succeeded.

What stays impossible offline is a package no run has ever fetched the index of,
and any git dependency the lock does not pin. The index cache is where a digest
comes from rather than something a digest checks, so editing it changes what an
offline resolution selects — the same power as answering an index over
plaintext, bounded by the next online run overwriting it and by signatures being
checked at every checkout.

## D-071 — a code marks a refusal, not a failure

Status: approved · 2026-07-31

A stable code marks a refusal about something somebody wrote — a manifest field,
a dependency entry, a selection, a graph that cannot exist — which is a thing to
look up. It does not mark a failure to do something, where the operating
system's own words are the whole explanation and a code would be a number in
front of "permission denied". The manager's families are `SL105x` for the
manifest, `SL106x` for the workspace, `SL107x` for resolution, `SL108x` for the
lockfile and `SL109x` for the compiler handshake. `SL103x` keeps the registry
errors that happen during resolution rather than moving, because a stable code
that moves is not stable.

## D-072 — a package with no `entry` is entered through `<source>/lib.slp`

Status: approved · 2026-07-31

`D-046` said a library may omit `entry`, and only resolution believed it: the
manager asked for an entry path unconditionally, so the shortest way to write a
library was a package nothing could build. Of the three ways out — build every
module under the source root, imply `<source>/lib.slp`, or narrow `D-046` to
mean resolvable rather than buildable — the second adds nothing new, since
`lib.slp` already means library. The file must exist and the message names it: a
package that declares no entry and has no `lib.slp` has not said what it is, and
the useful thing to report is the file that was looked for rather than the field
somebody deliberately did not write.

## D-073 — an `extern` reaches the backends as a table and a shared plan

Status: approved · 2026-07-31

`MirModule` gains an `externs` table and `Instruction::Call` does not change,
because `Call` is read by six things that have nothing to do with the FFI — the
purity test, the def/use sets, the inliner's call graph, the printer, the
verifier and the serialized form the language server reads — and a
discriminator on the instruction would have touched all six to say something
none of them asks about. The two consumers that do care look the name up:
`call_symbol` returns the raw C symbol or the mangled one, and
`extern_arguments` expands a borrowed string into the word at the pointer's
offset and a borrowed slice into a pointer and a length. Both live in
`lowering.rs` for `D-025`'s reason. Expanding the arguments in MIR instead was
rejected: MIR has no way to say "the word 16 bytes into this value", and giving
it one is a raw pointer under another name in the milestone whose safety
argument is that there is no raw pointer.

## D-074 — the C boundary is narrowed at the call site, and is not variadic

Status: approved · 2026-07-31

Every callee the compiler had ever called was also generated by the compiler,
which keeps an `i32` sign-extended for its whole life, so the backends took a
call result as a full register and were right to. C only defines the low half
for a 32-bit return, and the upper half is whatever the callee left there. An
`extern` returning `i32` therefore sign-extends at the call site and a `bool` is
read from the low byte. A variadic C function cannot be declared: System V
requires the vector-register count in `al`, which neither backend sets, and the
same call is well-formed against a fixed prototype and undefined against
`printf`. A project that needs one wraps it in the C it already supplies, which
is the half C is allowed to get wrong.

## D-075 — `c-sources` belong to the package, and they are cache inputs

Status: approved · 2026-07-31

`[package] c-sources` lists paths relative to the package and a path that leaves
it is refused. They are compiled with the `cc` the link already selects and they
ship in the archive without anyone writing an include list. The part that is not
obvious is that their contents are hashed into the artifact cache key: the
manifest text is already hashed, so declaring a C source invalidates the build,
but editing one would not, and a stale link against a month-old object is the
kind of wrong answer that survives a full test run. `c-sources` is not
workspace-inheritable, because it names files inside one package's directory.
Two refusals came out of building it, both because the failure lands on somebody
other than the author: an entry that is not actually in the archive is refused,
and a dependency's C is compiled and linked by whoever builds the executable,
there being no other link for it to end up in.

## D-076 — the bundled library is files on disk, owned by one crate

Status: approved · 2026-08-01

The library was two Rust string literals in the manifest crate, with a second
copy in the compiler because the compiler cannot depend on the manager's crate,
and nothing compared them. Two similar modules was survivable; four modules of
real Slopium is not. The sources become ordinary `.slp` files under `std/`, the
way the runtime is an ordinary `.c` file under `runtime/`, and a new crate with
no dependencies embeds them and names the language items, which both the
compiler and the manifest crate depend on. Pointing the dependency the other way
was rejected: it hands the compiler the manifest schema and a TOML parser when
`D-002` says the compiler does not read manifests. A library written in the
language it is for should be readable as that language, and it is now checked as
one — a test formats every bundled module and requires the result to match disk.

## D-077 — a lone file is a package of one module, and it gets the library

Status: approved · 2026-08-01

Once printing lives in the library, a lone `.slp` file has no manifest to
declare a dependency in, so it gets the bundled library and `--no-std` opts out.
A package is untouched, because what a package depends on is what its manifest
says. The consequence worth writing down is that a lone file is compiled as a
package of one module named after the file, so its declarations canonicalize to
`<stem>:name` and C code calling into it has to spell the module in. That
happens whether or not the library came along: a flag about which library is
available has no business renaming the program's own functions.

## D-078 — input and output are monomorphic functions, because there are no traits

Status: approved · 2026-08-01

The builtin printer was generic by inspecting its argument in the checker, which
a library function cannot do. So `std:io` exports `print` and `println` over
`(& String)` and separate names for each scalar width. This is the first place
`D-068`'s refusal costs something concrete, and it is recorded as evidence for
that gate rather than worked around; once the library can turn an integer into a
string the widths collapse into one call. A generic printer with a runtime type
tag was rejected: that is a trait object with the trait left implicit, and it
would answer `D-068` by accident.

## D-079 — the runtime's string entry points take a pointer and a length

Status: approved · 2026-08-01

A borrowed string crosses the FFI as a `const char *`, which cannot carry a
length, and a Slopium string may contain a NUL byte — from reading a line off a
pipe, or from a string C allocated. A library printer built on `strlen` would
print less than the caller asked for and say nothing about it. So the runtime's
string entry points take a pointer and a length, which is also what lets the
environment lookup keep refusing a name with an embedded NUL instead of looking
up the prefix. For the library to supply that length, `len` accepts a borrowed
string: strings are still compiler builtins, so the byte length is the
compiler's to provide.

## D-080 — the seam is four symbols, because a message is a contract

Status: approved · 2026-08-04

`D-066` put the seam between the core runtime and the hosted one at three
symbols; writing the split showed it is four. Core's own failures carry a
message, and nine fixtures assert those messages, but a message-less abort
cannot carry one. Keeping the seam at three would have meant either losing
runtime error messages in every hosted build, or compiling core twice per target
so the manager could pick — sacrificing a diagnostic contract to a number in an
older decision, or doubling the runtime's build matrix to avoid one declaration.
So core declares a panic entry point and, like the other three, defines it
nowhere; hosted defines it over `fprintf`, and a panic-abort build still expands
the failure at the call site so no literal reaches the binary.

## D-081 — the environment is the target's default and the command line's choice

Status: approved · 2026-08-04

The environment field exists before any freestanding target does, so both
targets set it to hosted and `slopic --freestanding` selects the other value:
the target supplies the default and the command line overrides it. That is what
makes the field load-bearing today rather than at the milestone that adds the
target, after which a `-none` triple is a table entry rather than a new
mechanism. The environment decides exactly three things — which runtime units
are materialized, whether an entry wrapper is emitted, and which toolchain
library a lone file gets — and not the calling convention or the object format.
If it ever wants to decide one of those, it has become a target and should be
one.

## D-082 — `std` depends on `core`, and re-exports what makes it the library

Status: approved · 2026-08-04

The bundled library becomes two toolchain packages: `core` with `option` and
`result` and no dependencies, and `std` with input, output and process, which
depends on it. `std` declares the language items and points them at a prelude
module that takes `core`'s and re-exports them. The alternative — `std`
declaring none, and every project depending on both — is simpler in the resolver
and worse everywhere else: every manifest would carry two lines where it carried
one, and `D-041`'s "exactly one direct dependency declares the items" would hold
by luck rather than by construction. `core` and `std` have separate checksums,
so an older lock names a `std` that no longer contains what moved; locks are
regenerated rather than migrated, and the checksum is what notices.

## D-083 — `string` belongs to `core`, and `std:string` re-exports it

Status: approved · 2026-08-06

Formatting an integer and parsing one are byte work: they need an allocator and
nothing else, so they belong in the half of the library a freestanding program
can have. `core:string` is the module, and the four things it cannot write for
itself — a byte at an index, a substring, a concatenation, and a string built
out of bytes — are entry points in the core runtime, where the string layout
already lives. `std` carries four lines of re-export for the same reason the
prelude exists: a package that depends on `std` has no name for `core`.
`scripts/core-check.sh` keeps it honest by taking its answer through the string
library with `-nostdlib` on both targets, so a primitive that reached for libc
fails there before it reaches a review.

## D-084 — a file is read and written whole, because a handle has no destructor

Status: approved · 2026-08-06

`std:fs` has `read`, `write`, `exists` and `remove`, each taking a path, and no
`open` or `close`. A file descriptor would have to live in a Slopium value, and
the language has no way for a user type to run code when its value dies: drop
glue is generated for the runtime's own types, and a struct wrapping an integer
owns nothing, so nothing runs. An open/close pair would be a leak by
construction, kept closed only by the programmer remembering, in a language
whose argument is that the compiler remembers ownership. Streaming waits for
destructors, which should be asked as "what does a resource type look like"
rather than answered by accident here.

## D-085 — a C failure crosses as a status slot, read after the call

Status: approved · 2026-08-06

The FFI vocabulary has no out-parameter, no struct return and no pointer type,
so a hosted call that can fail has one channel for its result and none for its
error, and no sentinel works — an empty string is both an empty file and a
missing one. The runtime keeps a slot instead: every hosted entry point that can
fail clears it on entry and sets an `errno` on the way out, and the library
reads it immediately after the call and builds an `Ok` or an `Err` from what it
finds. This is `errno` spelled out, which is the argument for it: it is the
convention every one of these functions is already sitting on, and inventing a
second one would mean translating twice. The cost is that the slot is global and
a call in between would clobber it; the library never does that, and the rule is
written down for anyone declaring their own `extern` over the same slot.

## D-086 — the integer printers stay and stop being C, and the `i32` ones go

Status: approved · 2026-08-06

`D-078` predicted that a string formatter would collapse the print widths back
into one call over a borrowed string. It does not, because only a named binding
can be borrowed, so the collapse would cost a `let` at every call site that
prints a number. The names stay and their bodies become Slopium over the
formatter. The `i32` pair is deleted instead, because there is nothing to write
it over: the language has no widening conversion, and adding one through the FFI
would be a cast smuggled in as a foreign function. Both halves are evidence for
`D-068`, and the second is evidence that the gate is asking about the wrong
feature — the gap that hurt is conversions, not bounds.

## D-087 — parsing yields an `Option`, and the library stops aborting

Status: approved · 2026-08-06

Parsing an integer used to be C that killed the program on a byte that was not a
digit. It returns an `Option` now, and reading an integer is reading a line and
parsing it. A library written in the language should not have a failure mode the
language cannot express, and this one always could be expressed: `Option` is a
language item and a list's `pop` has returned one for three milestones. Two
consequences: the overflow check is now the library's, so a digit string past
the integer's range is `None` rather than a panic, and the two `runtime-fail`
fixtures that asserted the old messages are deleted because the failure moved
rather than the coverage.

## D-088 — traits are refused for 1.0, and the gate's real answer is `=`

Status: approved · 2026-08-07

This is the entry `D-068` requires, answered from the written library rather
than from taste. Six places chose a concrete type where a bound would have done,
and only the first is a trait's fault: the print widths, which a `Display` bound
would halve; string equality, which is concrete because `=` has no meaning to
give; `find` and `contains`, which need that first; `split`, which returns an
eager list where an iterator belongs and an iterator is a closure problem before
it is a trait problem; the conversion names, which carry a type because there is
no conversion vocabulary at all; and a library error type a caller cannot carry
into its own, which wants a `From`. The absent combinators on `Option` and
`Result` look like the trait-shaped hole and are not one: there is no function
type, so a combinator would have nothing to take.

`D-012` was half false, and that is the finding. Arithmetic, ordering and clone
are refused on an unconstrained parameter, but `=` compiles and compares machine
words — so the one operator `D-012` forbids is the one it acquires, with the
wrong meaning: identity where every reader reads structure. The verdict is no:
1.0 ships parametric, because nothing 1.0 promises needs a generic container
over a comparable key, the measured cost of refusing is four extra functions,
the debts that were paid repeatedly are conversions and equality, and traits are
additive to a parametric language where a freeze is not. The milestone that
would have built them changes subject to those debts: `=` restricted to scalars,
explicit conversion, and first-class functions recorded as the prerequisite the
gate never asked about.

## D-089 — `=` compares scalars, and `D-012` stops being half false

Status: approved · 2026-08-07

Every other operator enumerated the types it accepts; `=` accepted anything that
was not `unit`, lowered to one machine-word comparison, and was waved through
the verifier on purpose. Since a struct is one word and every local is a handle,
two strings holding the same text were not equal. `=` is restricted to `bool`,
`i32`, `i64` and `f64` and refused on everything else, string equality becomes
the only way to compare text, and `D-012` becomes true for the first time since
generics existed. Copyability is not the predicate it looks like, because it
answers true for a borrow, which is precisely the case that has to go.

Writing the old meaning into the reference instead was rejected: after the
freeze the language would owe forever an `=` that answers identity in a syntax
every reader takes for structure. Generating structural equality glue was
rejected too — it is buildable without traits, but it hands an unconstrained
parameter a capability by inference, which is what `D-012` forbids; it also
stays available, since widening what an operator accepts is compatible after 1.0
and narrowing is not. The blast radius was measured first: of seventy uses of
`=` in the tree, every one compares scalars.

## D-090 — a conversion is a form with a target type, `(as i64 value)`

Status: approved · 2026-08-07

The reference had promised since v0.2 that numeric conversions are never
implicit, and the language kept the promise by having no conversions at all,
which is what cost the `i32` printers. The form is `(as i64 value)`, a special
form beside `let`, `if` and `while`, with the target type read by the existing
type parser and carried in its own node — not a call, which would have to
smuggle a type through a variable reference and would make the node the
formatter and the language server see lie about what it is. Exactly one pair is
allowed to begin with, because the pair table is what the freeze covers and
adding a row later is cheaper than adding a keyword. The type name as callee was
rejected for reserving every scalar name in callee position forever, and a
`widen` spelling for being wrong the moment there are two conversions.

## D-091 — `clone` crosses a borrow, and refuses to do nothing

Status: approved · 2026-08-07

`clone` returned its argument's type unchanged, so cloning a borrow produced a
borrow: a no-op that reads like a copy, which the library had already paid for
by writing a whole-string substring to get an owned copy. It now crosses a
borrow of either kind and refuses an owned scalar, which is copied by being
used, so the two cases that differ are told apart rather than run together.

## D-092 — a function is a value

Status: approved · 2026-08-08

`D-088` refused traits and recorded the order any reopening would take: function
type, then closures, then bounds. This builds the first two, and it is not a
step towards traits — it is what makes refusing them honest, because every
combinator the refusal costs becomes writable. Three facts decided the shape,
established by probing rather than reading: a function type already parsed and
died at normalization, functions and variables were separate namespaces, and
neither backend had an indirect call. So the checker gained a fallback in each
direction and two typed variants, and everything below it followed.

One field of the AST did have to change. A pass rewrites a call's callee to its
qualified name but leaves a bare variable alone, because a bare name is usually
a local and that pass has no scopes; so the signature table was keyed by the
qualified name and the fallback never matched it, and every function value was
an undefined variable the moment it was written in a real package rather than
probed in isolation. A variable now carries what the name would mean as a
top-level item in the module it was written in, filled in by the only pass that
knows the imports. It is advisory, and the environment is consulted first, so a
local always wins.

A function value is one machine word, which is the invariant the type system
rests on, so a closure's environment-and-code pair is boxed to one word rather
than allowed to be two: two words would reach register allocation, the C
boundary and every pointer-like test, where boxing costs an allocation and drop
glue the compiler already generates. The spelling is `(Fn (i64 i64) bool)`, with
parameters grouped in their own list so every arity has exactly one form and
nothing is counted from the right; the arrow spelling loses because an arrow is
not a type and a type parser would have to special-case an atom in the middle of
a list. Merging the two namespaces was rejected, because a fallback preserves
every existing program by construction; an explicit apply form was rejected for
costing a keyword and making a higher-order call read unlike every other call.

## D-093 — the library grows to the point of being worth freezing

Status: approved · 2026-08-08

The library is 401 lines, and the freeze makes the reference normative about it.
Freezing what is already there is a promise about a language nobody can write
anything in, so the scope is chosen by what would be embarrassing to promise
rather than by what would be convenient to have. The clearest case is that
`f64` is unobservable: the word appears nowhere in the library, so the language
has a scalar type with literals, arithmetic and equality whose value no program
can print without supplying its own `extern`. `Option`, `Result` and `List` get
their combinators, `sort-by` takes a comparator — which is what a trait language
gets from an ordering bound — and `split` keeps returning an eager list, frozen
with that meaning, because a lazy sequence later is a new name rather than a
change to this one. The library is written over function pointers before
closures exist, and every place it wants a capture and cannot have one is
recorded, because that record is what the open half of `D-092` is decided from.

## D-094 — `Map` and `Set` return, parameterised rather than constrained

Status: approved · 2026-08-08

A generic container over a comparable key was held to need traits. It does not:
it needs a hash function and an equality function, and after `D-092` those are
values. `(map-new hash equals)` constructs a `(Map K V)` with no bound anywhere,
which answers the gate by construction rather than by argument. It is the last
patch of its milestone and the one to cut if the milestone runs long, because it
is included as proof that the parametric bet was right — and if it turns out
awkward to use, that is worth discovering before 1.0 promises the bet was sound.
A string-keyed special case was rejected: it answers the motivating use and
nothing else, and puts a concrete type in the library's most general container.

## D-095 — a generic function can take a generic type

Status: approved · 2026-08-10

Inside a generic body a generic type stays an unresolved application, because
there is no instance to make yet — but every consumer of an aggregate matched on
a resolved name, so `match`, field access and unification could not see it. A
generic function had therefore never been able to take, match, build or return a
generic aggregate, and nothing had said so because no line of the library or the
suite had tried. That is on the critical path rather than beside it: every
combinator the traits refusal owes is exactly that shape, so the cost `D-088`
accepted could not have been paid without this.

The fix is small because the architecture anticipated it and only the typing
half was ever written: the pass that runs after monomorphization already turned
a now-concrete application into a resolved name and generated the instance on
demand, and was simply unreachable. Four sites learn to read an application the
way they read a name. Two consequences: a pattern's enum name is settled from
the scrutinee rather than from the pattern, which computes the same answer for
concrete code; and `try` still refuses a generic `Result`, with a diagnostic
rather than a miscompile, because it reads the error type out of an instance
table a generic body has no entry in.

## D-096 — an empty collection literal takes its element type from context

Status: approved · 2026-08-10

An empty list literal was an error with no way to say what it would have held,
which is why the library's `split` documents that it always yields at least one
part, and why `map` and `filter` could not be written at all — both must return
an empty list when handed one. An empty literal is now legal wherever the
expected type says what it holds and an error only where nothing does, which is
the rule the absent `Option` case already follows and the same plumbing carries.
It also removes two diagnostics for one cause, since the invented `unit` element
type that kept the checker going is gone with it. A type-annotated binding was
rejected as a larger language change wanted in one place only.

## D-097 — formatting an `f64` is `core`, in Slopium, over the bits

Status: approved · 2026-08-11

Three reasons, in the order they decide it. The next milestone is freestanding
and `%.17g` lives in libc, so hosting the formatter would leave a scalar type
the reference promises whose values a kernel cannot print — the hole moved
rather than filled. The algorithm does not need C: a double is a 53-bit integer
times a power of two, which is a finite decimal exactly, so it is one bignum
multiplied by a one-digit factor repeatedly, with nothing divided, nothing
compared, and one rounding at the end. And what the library genuinely cannot do
is look inside an `f64`, so two runtime primitives read and write the bit
pattern and do nothing else — the same reason the four string primitives exist.
They are a union pun, so the core object still has no undefined symbol.

Those two are private to the float module and are not a conversion: reading the
bits of a double is not turning it into a number, and exposing them would be a
different decision under a different name. Where the hosted names live was
settled by measurement: putting them beside the integer ones made the smallest
program that prints a word grow 2.3 times, because a module is one section and
taking one brings everything it calls, so they live in a module of their own and
a program pays for a float when it mentions one.

## D-098 — a printed `f64` is plain decimal, seventeen digits, ties to even

Status: approved · 2026-08-11

The output is an optional sign, digits, a point and digits, with at least one
digit on each side, and `nan`, `inf` and `-inf` for the three values that have
no digits. There is no exponent form: the language has no exponent literal, so a
printed exponent would be a spelling the compiler cannot read back, and plain
decimal is complete because every double is a finite decimal. The cost is length
at the extremes — 309 digits for the largest double, 342 characters for the
smallest subnormal — probed rather than assumed. The digits are the exact
expansion rounded to seventeen significant digits, ties to even, with trailing
fractional zeros removed and one always kept, so `1.0` prints as `1.0`.
Seventeen is the round-trip width; fifteen would print a shorter `0.1`, and that
ugliness is the argument for seventeen rather than against it, because this is
the only way to observe an `f64` and a printer that drops bits makes them
unrecoverable by any means.

## D-099 — `match` works through a shared borrow

Status: approved · 2026-08-12

Refusing a borrowed scrutinee cost `core:option` its `is-some` and forced every
combinator in the library to take its value by ownership, because asking which
variant a value is consumed it. That is a strange thing for a language with
borrows to be unable to do, and it is the surface the freeze covers. So the
scrutinee may be a shared borrow of an enum or a struct, every binding a pattern
makes is a shared borrow of the field it names, the scrutinee is not moved, and
nothing in an arm is dropped.

The binding type is uniform, and that is forced rather than chosen: binding a
copyable payload by value and any other by borrow reads better at every concrete
type and cannot be written down at a generic one, because whether a parameter is
copyable is not known until monomorphization and the type of a binding has to be
known before it. It costs no new machine code — an enum is already a heap
pointer, so borrowing one copies the pointer — beyond one MIR instruction for
the address of a payload slot that is not itself pointer-like. A separate
matching form was rejected: a second keyword whose difference from the first is
the type of its argument.

## D-100 — reading a borrow is `clone`, and there is no dereference

Status: approved · 2026-08-12

`D-091` made cloning an owned scalar an error because a scalar is copied by
being used. That was written about an owned value and applied to a borrowed one,
and a borrowed scalar is the case where it is wrong: there was no way at all to
get the integer out. In a generic body it was worse than refused — cloning a
borrowed element compiled and returned a pointer where an integer was asked for,
because the "nothing to clone" test ran on the generic parameter and the
specialized instance was never asked again. That is a silent miscompile, found
by probing rather than by any test.

So `clone` through a shared borrow yields an owned value for every type,
including a scalar, decided from the concrete type after substitution rather
than from the generic one before it. This is the dereference, and no form is
added for it: a language whose read form is spelled `clone` says the thing that
is true of it, which is that reading out of a borrow costs a copy. A dedicated
dereference form was rejected as a second name for what `clone` already means
across a borrow, differing only at the scalar case — which is precisely the bug.

## D-101 — a function value is an owned closure

Status: approved · 2026-08-14

A value that can be copied cannot own an allocation: two copies, one
environment, two frees. So either a closure owns nothing, or a function value
stops being copyable, and everything follows from which is picked. A closure
that owns nothing must borrow what it captures, may not outlive the frame it was
written in, and needs the type to say so — which a shared function type cannot,
so it would have to be said conservatively, and that refuses a function value in
a return type, a struct field and a collection element. All three compile
already, and a table of function pointers is what a language without traits
reaches for.

So a function value is owned: one machine word pointing at a heap block holding
the code address, a drop helper, a clone helper, then one word per capture.
Calling through a value does not consume it; using one twice as an argument is
two moves, and the answer is the one the language already has — take it by
borrow. The block is a struct, which is why this costs almost no code: the
struct clone and drop helpers both backends already generate are the closure
glue, and two runtime shims load the helper out of the block and jump to it. A
bare function is boxed too, because the alternative is a static block holding a
code address and this object writer has relocations in `.text` and nowhere else.
A tag bit distinguishing raw address from closure was rejected: it keeps copying
and then the closure cannot own its captures after all, so the escape question
returns needing an analysis instead of a type.

## D-102 — a lambda names what it closes over

Status: approved · 2026-08-14

`(lambda (captures...) ((param ty)...) -> result body)`. The captures are names
from the enclosing scope, each moved into the environment; the body sees the
captures and the parameters and nothing else. The form is a declaration with the
name dropped and one list changed — where a declaration says what it is
parameterised over, this says what it closes over — and both are the things that
must be settled before the body means anything.

It is explicit because a capture is a move and every other move in this language
is written down. Inferring captures would end a binding's life on a line that
never names it. Inside the body a capture keeps its name and type, and what it
may not do is move one back out: the environment owns it and drops it, and the
second call is where that damage would show, so it is refused by name rather
than by ownership state. A capture may not be a borrow, and the first draft
allowed one: an environment is an aggregate that outlives its frame and is
reached through a value that can be returned, which is all three of the places a
borrow is already kept out of, and nothing else would have caught it because a
borrow is copyable and so capturing one moves nothing. A closure capturing a
borrowed integer and returned from the function that owned it compiled, ran, and
printed the right answer, which is what a dangling read does until it does not.

## D-103 — a list element can be replaced, and that is the only write there is

Status: approved · 2026-08-15

`(replace (&mut list) index value)` puts the value in the slot and returns what
was there. It exists because no mutable container could be written in this
language without it, which nothing had noticed because nothing had tried:
assignment reaches a name and nothing else, there is no assignment to a field or
an element, and a `match` looks through a shared borrow only, so the only ways
to change a list were at its ends. A hash table's whole point is to touch one
bucket, and without this a map is a linear scan wearing a hash function's
clothes.

It returns the old element rather than dropping it, which is what makes it an
ownership operation rather than an assignment: both values are owned, a slot
holds exactly one, and the caller decides what happens to the one that came out.
An assignment form would have had to drop the old value silently, which the
language does nowhere else. Assignment through a place expression was rejected
as three features to buy what one runtime call buys, and a builtin map with its
bucket array in C was rejected for answering the parametric-container question
by not asking it.

## D-104 — `Map` and `Set` are library types over a hash and an equality

Status: approved · 2026-08-15

Both earlier refusals reasoned from the premise that such a container needs a
bound. It needs a hash function and an equality function, and those are values,
so they are two parameters and nothing in the module knows what a key is. That
answers the traits gate by construction.

Everything that writes consumes the map and gives it back, which is not a taste
for functional containers but `D-103`'s finding one level up: a method taking an
exclusive borrow could not reach the fields inside it, because a match looks
through a shared borrow and there is no field assignment. The table is separate
chaining, buckets doubling when the entries reach the bucket count; open
addressing would need a way to write an empty element of a type the module
cannot construct. A new map has no buckets at all and the first insert gives it
four, because an empty list of lists needs its type from somewhere and a struct
field being built is somewhere a `let` is not. `Set` is a map whose value is
ignored, because a set is what a map is when only the key matters. The string
hash multiplies by 31 under a prime modulus, because the usual constants are
written for a language where overflow wraps and arithmetic traps in this one.
What it does not have: iteration beyond a fold, key and value lists, and a
borrowed value out of a lookup, since a reference cannot leave the function that
made it.

## D-105 — an expectation is substituted, normalized, and passed inward

Status: approved · 2026-08-15

Three holes in generic inference, all found by writing the map against the
compiler that shipped before it, and all the same shape: the checker knew what a
thing had to be and did not say so. A generic call's result was substituted but
never normalized, so the same type under the two spellings monomorphization
gives it did not unify — reachable without any of this work, in a concrete call
to the library's own combinator. What a call site expected did not reach the
arguments, because the expected result was unified after they were typed rather
than before. And a parameter that had been substituted still counted as unbound,
because the check was by name: inside a generic function whose own parameter is
also called `K`, a `K` substituted by the caller's `K` looked like a `K` nothing
had decided, which is invisible until two generic functions choose the same
letter, which is always.

Normalization is conditional on the type being fully bound, which is the bug the
first draft had: normalizing a type with an unbound parameter instantiates a
monomorphized struct named after the parameter. An unbound parameter is not a
type and must not be treated as one.

## D-106 — the operator and literal vocabulary is completed before the freeze

Status: approved · 2026-08-16

The operators were `+ - * / < > =` and nothing else: no remainder, no `<=` or
`>=`, no `and`, `or` or `not`, no bitwise anything, no unary negation, and an
integer literal was decimal only. The library is the evidence rather than the
argument — a remainder written as three operations, a whitespace test as four
nested conditionals, `(- 0 x)` standing in for negation everywhere. That is not
a limitation the language chose; it is a list nobody finished. It goes before
the freeze because the freeze makes the reference normative, and a normative
reference that promises a language without these freezes the workarounds into
the promise; it goes with the freestanding milestone because a kernel is masks,
flags and memory-mapped registers.

What lands: `%`, truncated so that division and remainder agree, trapping on
zero as division does; `<=`, `>=` and `!=`, each accepting what its counterpart
accepts; `and` and `or` as forms rather than calls, because a call evaluates its
arguments and short-circuiting is the entire point, with `not` as an ordinary
operator; the six bitwise operations spelled out in words, because `&` is the
borrow form and a language where `(& a b)` is a bitwise and while `(& a)` is a
borrow is a language with a trap in it; unary negation; hexadecimal, binary and
digit-separated literals, because a kernel that writes an address in decimal
acquires a wrong constant reviews do not catch; and the `\0` and `\xNN` escapes,
since a string may hold a NUL and had no way to write one. None of it touches
the grammar: in an S-expression language an operator is a name in head position,
so there is no precedence table and no parse to break.

## D-107 — integers get width and signedness, and the library does not double

Status: approved · 2026-08-16

A memory-mapped register is an 8, 16 or 32-bit quantity at a fixed address, and
reading one as an `i64` is not a description of the hardware; C's narrow and
unsigned types had no spelling either. The full axis is done once — eight types
— because stopping halfway is arbitrary in a way the language would have to keep
explaining, and the cost is one table of widths rather than eight decisions.

The library does not grow by eight: text and parsing exist for `i64`, `u64` and
`f64`, and every narrower type travels through `as`. Without traits the
alternative is thirty more printer names. `as` becomes a real table and may
narrow, because a truncation between fixed widths is a defined operation with a
written result, unlike the float conversions the same decision refused where the
question is which rounding; truncation and reinterpretation are explicit rows
and never the implicit result of an assignment. The backends pay the honest cost
— unsigned is not a label but a second division instruction, a second set of
conditional branches, zero-extension beside sign-extension and an overflow bound
that depends on the type. There is no `usize`: indices and lengths stay `i64`,
but the name is reserved, because a pointer-sized integer becomes a real need
the moment a 32-bit target appears. `f32` is not in this decision: it is a
second float rather than a width of an integer.

## D-108 — concurrency is shared-nothing, after 1.0, and the freeze reserves for it

Status: approved · 2026-08-16

The finding is that the language already satisfies the hard precondition for
threads without having tried to. There are no global variables of any kind, an
exclusive borrow is exclusive and checked, there is no shared ownership and no
interior mutability, and a capture may not be a borrow — which is the property a
spawn demands, already enforced for an unrelated reason. So the marker traits
that separate what may cross a thread from what may not are not needed here:
they exist to describe reference counting, cells and statics, none of which this
language has. Moving a string, a list or a map to another thread is moving one
machine word, and message passing over owned values is expressible without
traits.

What is actually in the way is one line of C: the process-wide error slot every
hosted call clears and sets, which with two threads is a race that returns a
plausible `errno` from the wrong call. So threads come after 1.0 and the
reservations come at the freeze, because the ABI and the grammar freeze first
and this is the class of thing that becomes impossible rather than merely
expensive. The slot becomes thread-local before the ABI is written down; `async`
and `await` become reserved words defined as nothing, while `spawn`, `channel`,
`mutex` and `atomic` are not reserved because they are library names; the ABI is
written with an additive rule rather than as a closed list, since atomics are
instructions and what they need is permission to grow; and what a panic in a
thread means is stated even though no thread exists — it ends the process, like
every other panic. Async stays out beyond 1.0 as well: it is either a compiler
transform of a body into a state machine or a protocol written by hand at every
call site, and either way it needs a scheduler and a reactor inside the runtime
the freeze covers.

## D-109 — macros are deferred, and the freeze reserves the namespace

Status: deferred · 2026-08-16

The mechanism is as cheap as it will ever be — the source is already a vector of
S-expressions and an expander would sit between the parser and the builder — and
it is deferred anyway. What it would cost is the diagnostics: every error here
carries an exact span and the fixtures assert them as numbers, so a macro that
expands one form into forty has to give every one a span pointing at something
the programmer wrote, or the language acquires a second class of error message.
Hygiene is the same story from the other end, and the rename has to be followed
by the analysis layer so that go-to-definition and rename keep working. The
expander is a week; the spans and the hygiene are the feature. The case that
will want macros first is microcontroller register definitions, and those are
generated from a description file in every language rather than written, which
needs no language feature at all.

Reserved at the freeze, because it is the part that cannot be added later: the
words `macro` and `define-syntax`, and the namespace rule — one namespace, a
macro is a name resolved where names are resolved — because adding a second
namespace afterwards changes what an existing import means. If macros are ever
built they are pattern macros over the syntax tree with mandatory span
propagation and hygiene, not procedural ones, which need an interpreter for the
language inside the compiler.

## D-110 — a microcontroller is a word size before it is an instruction set

Status: approved · 2026-08-16

Supporting a small board reads like a backend question and is not one. Both
backends assume a 64-bit machine in the middle rather than in the instruction
selection: every list element crosses the runtime as an eight-byte word, `i64`
is the default integer and the type of every index and length, and there is no
pointer-sized type. On a 32-bit target a 64-bit add is two instructions and a
carry, a multiply is a runtime helper, and some cores have no divide instruction
at all. So the first 32-bit target is expensive whatever its ISA is and every
one after it is cheap, and the milestone is "the compiler stops assuming a
64-bit word".

Then the instruction sets, ordered by what unlocks what. RV32IMAC is small and
orthogonal, its assembler and emulator exist so both existing gates transfer
unchanged, and it reaches a whole family of parts including one whose RISC-V
cores sit beside its ARM ones. Thumb-2 is what the most common hobbyist part
actually needs, and the AArch64 backend does not help: a Cortex-M is variable
length, 32-bit, and shares little with A64 beyond the name. Xtensa is last,
because its calling convention uses register windows that neither existing
frame model resembles. What a microcontroller needs beyond a backend is mostly
already there — the allocator is a hook, the bit operations and volatile access
are scheduled, and hand-written assembly links because C sources are handed to
`cc` by path rather than by extension. An interrupt handler is a calling
convention and therefore a declaration form and therefore grammar, so it is
reserved at the freeze even though no target needs it yet.

## D-111 — a freeze that changes something is not a freeze

Status: approved · 2026-08-16

The plan had one milestone freezing the edition, the runtime ABI, the package
format and the diagnostic codes while also being the last chance for anything
that costs a keyword or a form. That cannot be one milestone: a freeze is a
milestone that changes nothing, whose whole content is writing down what is
already true and adding the tests that fail when it stops being true, and a
milestone that adds a form and then freezes the grammar in the same breath
freezes a grammar nobody has used. So the series renumbers — the language
finishes first, in a milestone that ends with a language nobody wants to change,
and the freeze follows with no feature in it at all.

## D-112 — the six calls the vocabulary decision left open

Status: approved · 2026-08-16

A shift by a bad amount says so in its own words: reusing the overflow message
would have misdescribed the only bug a driver author writes there, so there is a
third trap kind with its own trampoline in both backends, emitted only when a
shift can reach it. The check is one unsigned comparison against the width,
which catches a negative amount in the same branch. A shift does not trap when
bits leave the top — `(shl 1 63)` is the smallest integer and that is the answer
— because a shift describes a pattern of bits rather than a magnitude, and a
language that trapped there could not write a mask.

A hexadecimal or binary literal is a bit pattern where a decimal one is a
number, so an all-ones mask is written as such rather than as a negative
decimal, which is exactly how the float module came to take a sign bit off by
adding a power of two twice. A malformed literal is a diagnostic rather than a
name: the builder used to fall back to a variable, so a bad literal in a match
arm bound a variable, matched everything, and made every arm below it
unreachable in silence. `\xNN` is exactly one byte, which is what makes string
literals carry bytes from the lexer to the object file, with a C symbol and a
test name staying text and saying so. And `and` and `or` take two operands or
more and stop at the checker: every operand is checked against `bool`, so a
wrong one is complained about rather than a constant the compiler invented, and
what comes out is nested conditionals, so no backend learns anything.

Unary operators cost no instruction: negation is a subtraction, `not` is a
comparison against false, and `bit-not` is an exclusive or with all ones, so
all three lower onto binary shapes that already exist. Rewriting `<=`, `>=` and
`!=` as negations was rejected because on a float it is a silent miscompile: a
NaN is neither less nor greater nor equal, and is unequal to everything.
Rotations were left out because adding an operator after the freeze is
compatible while writing them over shifts is not — a rotate by zero needs a
shift by the full width, which now traps.

## D-113 — the calls the integer axis left open

Status: approved · 2026-08-16

Every integer is held canonical in a full machine word — sign-extended when
signed, zero-extended when unsigned, at every width — which generalises what was
already true of the one narrow type. The alternative was native narrow
instruction selection, and the x86-64 backend is where that dies: no 16-bit path
in the tree and a byte-register table of four names. Under the invariant none of
it has to be built: loads and stores stay 64-bit, a frame slot stays eight
bytes, a list of bytes stays a word per element, and the narrow unsigned types
ride the existing signed comparison and division paths because a zero-extended
value below 2^32 answers the same either way. This is the claim most worth
attacking if the patch is ever found wrong.

A narrow operation overflows exactly when canonicalising its result changes it,
so no per-type bound constant is written down anywhere: it is one
compare-and-trap at six widths and both signednesses, reusing the
canonicalisation a conversion needs anyway. A conversion therefore costs no MIR
instruction, being a mask or a shift pair both backends already emit, which is
why the protocol number did not move. The source's signedness extends and the
target's width truncates, all 64 pairs are legal, and none of them traps,
because a conversion describes a pattern of bits. The literal range check moved
to the checker, because only the checker knows the target type. Negating an
unsigned value is a compile error rather than a value that traps, since it can
only answer for zero.

Two things the work found. A narrow parameter arriving from C is not canonical,
because the ABI leaves the upper half of a narrow argument register undefined
and the old backend never noticed; the invariant has to hold of values a
function was handed and not only of ones it computed, so the prologue
canonicalises them. And `bit-not` on a signed narrow type wants all ones rather
than the width's mask, because the operand is sign-extended and the bits above
the type have to flip with it.

## D-114 — the volatile invariant splits, because a verifier sees one module

Status: approved · 2026-08-17 · amends `D-067`

`D-067` put "never eliminated, never reordered" in the verifier. That is not
implementable as written: the verifier is handed one module, and both claims are
about a pair of modules, before and after a pass. The obvious repair — an
identity per access, checked for uniqueness and monotonicity — has legal
counterexamples on both halves, since inlining duplicates a callee's accesses at
every call site and unreachable-block removal deletes one a folded branch made
unreachable.

So the invariant splits and the intent is kept whole. The optimizer owns what
only a before-and-after comparison can see: a count of volatile accesses per
function, snapshotted after inlining so that inlining is explicitly outside the
invariant, required to be unchanged across constant propagation and dead code
elimination and non-increasing across CFG simplification. The verifier owns what
a single module can decide, which is the half that prevents a miscompile rather
than a missed optimisation: that the width of an access agrees with what the
pointer points at and with the local the value came from, since a width one size
wrong does not fault but reads or writes the neighbouring register.

Two supports underneath both. The purity test returning false is necessary and
sufficient against elimination, and the match has no wildcard, so a new
instruction has to be classified. And the def/use sets must report a volatile
load's destination: constant propagation marks what an instruction defines as
varying, so a load that reported nothing would leave its destination holding an
earlier value — a device register folded to a constant.

## D-115 — the object model and the environment are separate patches

Status: approved · 2026-08-17

One patch held two things that share no file and no failure mode. The object
model is the compiler's alone: no manifest key, no flag, no diagnostic, no
protocol bump, and everything it can get wrong is caught against the platform
assembler. The environment is almost entirely the manager's: a target row, a
manifest key with validation and archive carriage, a cache key, and the runtime
materialization. Landing them together would have produced one commit whose
subject cannot name what it did, with a failure in either half blocking the
other for no reason. The milestone's exit condition is unchanged.

## D-116 — a panic trampoline lives in the section that branches to it

Status: approved · 2026-08-17

Every checked operation branches to a trampoline that loads a message and
panics, and there were three per program at the end of the text section. Once a
function owns its own section they cannot stay there, for an encoding reason
rather than a preference: AArch64's conditional branches carry a 19-bit
displacement, and no linker synthesizes a veneer for that relocation, so a
shared trampoline would have turned a compile-time diagnostic into a link-time
"relocation truncated to fit" about a branch nobody wrote.

So each function carries its own copies, named after it, emitted after the
epilogue and before the size directive so a backtrace stopping in one still
names the function whose check reached it. Only the trampolines that function's
arithmetic can reach are emitted, so a function that cannot trap grows by
nothing, and section-based garbage collection now drops a trampoline with the
function it serves. It also closes a cliff that was already there and never
reached: the old distance was to the end of the module's whole text section,
which grows with the library, and the new one is bounded by one function. The
alternative — inverting each condition to branch over an unconditional jump —
is correct and stays in reserve, but it changes the instruction sequence of
every checked operation in the language. x86-64 does not need any of this and
gets it anyway, because one rule is better than two with an architecture in
each.

## D-117 — the linker script is `[build]`'s, and an environment decides four things

Status: approved · 2026-08-17 · amends `D-081`

The key is `[build] linker-script` rather than `[package]`'s, although it is
otherwise modelled on the C sources key down to the validation, the archive
carriage and the content hash. The divergence is the point: a package's C is
additive, so a longer list is a correct answer, while a linker script describes
one whole image, so a list of them is a conflict with no rule about whose wins
that is better than not asking. `[build]` is already read from the root package
alone, so a dependency's script is ignored by construction rather than by a rule
somebody has to remember. It gets its own refusal code rather than reusing the C
sources one, because a diagnostic naming the wrong key sends its reader to the
wrong line.

An environment decides four things and not three: the runtime units, whether an
entry wrapper is emitted, which toolchain library a lone file gets, and the link
flags. The fourth belongs to the environment by `D-081`'s own test, since the
two x86-64 targets share an architecture, an ABI and an object format and differ
in exactly this. Static and non-relocatable are not tidiness: a toolchain
defaulting to position-independent output emits an object asking for an
interpreter, which is the one thing a program with no C library under it cannot
be given. The entry point needs no new mechanism — a freestanding start routine
calls the name the entry links under, and a program's `main` is the one function
that keeps its bare name, so both gates spell that symbol out rather than
compute it, and changing the mangling fails at a link instead of resolving to
something else.

## D-118 — port-mapped I/O crosses the C boundary, and does not become an operator

Status: approved · 2026-08-17

`D-067` took the dangerous half of a bare-metal program back into the language,
and the first kernel written on it found the edge of that on the first line of
its serial driver: a PC serial port is not memory. The `in` and `out`
instructions address a separate space that no pointer names, so a volatile write
cannot reach a port however the address is spelled. Making port access a pair of
builtins would have put them where a reader would look for them, and it was
refused: those instructions are x86 and nothing else, and a builtin only one
backend can lower is a hole in `D-025` rather than a row in a table — the other
backend would have to grow a diagnostic refusing a word the language claims to
have. The cost of refusing is two lines of assembly in the fixture and one
`extern` declaration, which is `D-064`'s arrangement unchanged. It also made the
FFI vocabulary load-bearing in a way `D-065` did not anticipate: a 16-bit port
and an 8-bit datum cross because the integer axis exists, and the pair would
have needed a 32-bit type on both sides a milestone earlier.

## D-119 — the booted image is a 32-bit re-wrap, because the loader refuses a 64-bit one

Status: approved · 2026-08-17

The emulator's multiboot loader will not load a 64-bit kernel, and it says so
before anything the toolchain produced has run, so no arrangement of sections,
entry points or linker scripts changes it. That was found by reading the shipped
emulator while planning rather than by booting, which is the only reason this
work did not begin by debugging a long-mode transition that was never reached.
The image handed to the emulator is therefore an object-copy of the linked
kernel into a 32-bit container: nothing about the program changes, and every
address fits because the whole image lives at 1 MiB. The re-wrap belongs to the
gate and not to the toolchain, which keeps it a fact about one loader rather
than a target property the freeze would have to cover. Building an ISO through a
bootloader that does read a 64-bit image was refused for its cost in the dev
shell and on every run of the suite.

Two consequences worth keeping. The kernel loads at 1 MiB, which is what keeps
the small code model the compiler emits correct — there is no kernel code model
to offer, and a higher-half kernel is the first thing that would need one. And
the red zone is sound only because interrupts are never enabled and no interrupt
table is ever loaded: the compiler emits ordinary System V code and may use the
128 bytes below the stack pointer, which only an asynchronous push corrupts.

## D-120 — a field is assigned through a place, not through an address

Status: approved · 2026-08-18

An aggregate field had no write at all, so a value inside a struct could only be
changed by taking the struct apart and building a new one, which is why every
write in the library's map consumed the map and gave it back. `D-099` had
decided the shape and deferred half of it: a match looked through a shared
borrow, and an exclusive one was refused because nothing could be done with the
binding. A match now looks through an exclusive borrow, every name a pattern
binds under it is an exclusive borrow of the field it names, and assignment
writes such a name, dropping what was there.

It is a place rather than an address. The store takes the aggregate and the
field's index, known at compile time because the name came from a pattern in
this function, so a borrow keeps exactly the representation it always had and
nothing about address-taking, the builtins over an exclusively borrowed list, or
the C boundary moves. The alternative — making an exclusive borrow uniformly the
address of a slot with a general store — is tidier on paper and would have
changed both backends, pinned every mutably borrowed local to memory, and put a
load in front of four builtins, all to reach a place the pattern already named.
The spelling is the assignment form the language already has, chosen by the
binding's type, which is `D-100`'s reasoning in the other direction: through a
borrow, the form the language already has is the dereference.

An exclusively borrowed parameter is not a place, because assignment writes a
field of an aggregate this function took apart itself; allowing it would have
worked for a scalar referent and not for a pointer-shaped one, which is
pointer-likeness leaking out of the backend into a language rule. Writing a
field through a path expression was considered and left out: it is a second
syntactic shape for assignment, it answers only the case where the aggregate is
a local this function owns rather than the borrow the library actually has, and
it is additive after the freeze in a way the exclusive match is not.

Three things fell out of building it. `clone` had to stop refusing an exclusive
borrow, because reading a field to write it back is the shape every counter in
the library now has. The weakening of an exclusive borrow to a shared one had to
become a property of types rather than of the checker, because the MIR verifier
compares argument types against parameters and rejected what the checker had
accepted — one rule stated in two files is how the two drift. And two refusals
shared a sentence that fitted only one of them: taking an exclusive borrow of an
already shared-borrowed value said "more than once", which describes a different
mistake.

## D-121 — the everyday forms, and where a type is written down

Status: approved · 2026-08-18

Five things landed together because all five are grammar and grammar is what the
freeze covers: a module-level constant, a `let` that can carry its value's type,
a `break` with a value, guards on a match arm, and a decision about shadowing.
None of them is deep, and that is why they had to be now — adding a form after
the freeze is compatible and changing one is not.

A type is written after the value, with a colon. The two rejected spellings say
why: putting the annotation between the name and the thing it describes reads
backwards, and three positional slots where two are expressions reads worse.
Writing it last makes it an annotation of the value rather than of the binding,
which is the honest reading, since inference reaches everywhere it can and this
is what is written where it cannot. A constant is a literal and nothing else —
no arithmetic, no reference to another constant, no use in a pattern — and what
it replaces is a nullary function returning a number, of which the kernel had
seven. A `break` with a value belongs to `loop` alone, because a `while` ends
when its condition is false and there is no value on that edge; it costs no MIR
instruction, being one local written on each break edge and read past the exit.

A guard is flat, four elements where an arm was two, and two rules make it sound
rather than special: a guarded arm proves nothing about exhaustiveness, and a
guard may only read the names its pattern bound, since it runs before the arm is
taken and a move there would consume a value the later arms still match against.
Shadowing is allowed, decided by what stays possible — permitting it later is
compatible and forbidding it later is not — which also settles a split nobody
had decided, because the old refusal only looked at the innermost scope and a
nested block could already rebind. A parameter, a capture and a pattern binding
are still refused twice, because each of those is a list somebody wrote wrong
rather than a name they reused.

## D-122 — an annotation is a list before the name, and a warning belongs to a compilation

Status: approved · 2026-08-19

There was no mechanism for annotating a declaration, and every annotation the
language will ever want needs one. An annotation is a list written between a
declaration's keyword and its name, and a declaration may carry several. The
slot ends at the first element that is not a list, which is the declaration's
own name — an atom for most forms and a string for two — so nothing any
declaration could already write became ambiguous, and one sentence covers six
forms. A wrapper form was rejected for costing every annotated declaration an
indentation level, and an annotation that is itself the wrapper for reserving a
top-level head word each and turning a misspelling into an unknown declaration.
Every form carries the slot, including the ones no annotation applies to yet:
after the freeze a new annotation stays additive and a new form does not.

One table decides the rest — the name, its arguments, the declarations it may
sit on, and whether it is interface — so all six forms refuse alike and the two
annotations that use it are rows rather than code. A deprecation warns at the
use rather than at the declaration, and an inline hint raises the optimizer's
two size ceilings for one callee and moves nothing that makes inlining sound.

A warning belongs to the compilation and not to the program: the severity had
existed for eight milestones with nothing to construct it, because the result of
a compilation has no room for a diagnostic about a program that compiled. It is
a sink the caller passes in, because which warnings a run reports depends on
what it was asked to build — a warning about a dependency's own source is the
dependency's business, and a run that names a codegen module reports that
module's alone, or a build would print every warning once per object.

Building it turned up a constant missing from the interface the manager caches
modules against, and missing since constants shipped: a constant is inlined at
every use, so changing its value changes every dependent's object, and a
dependent kept the old number until something else forced a rebuild. Reproduced
with two modules and a constant changed from three to seven, where the program
went on exiting three. A deprecation is interface and an inline hint is not: the
caller is what warns, and nothing inlines across a module boundary.

## D-123 — the version belongs to the release, and the decision log is public

Status: approved · 2026-08-19

Every `feat` used to be a release: it moved `workspace.package.version` in its
own commit and took a tag. That holds while one person commits to `main` and
fails the moment two branches are open — both would claim the next number,
whichever merged second would be claiming a version that already exists, and
both would have regenerated the committed consumer's lock, which carries the
version and the bundled library's checksums, into a conflict. So the version is
a property of the last release rather than of the last commit: no ordinary
commit touches it, a change owes a line under `[Unreleased]` in `CHANGELOG.md`
instead, and a release is its own pull request that moves the manifest, the
lockfiles and the changelog together and is tagged on the merge commit.
`scripts/release-check.sh` decides the mechanical half of that, on every pull
request and again at the tag, before anything is built.

Work reaches `main` through a pull request and merges as a merge commit rather
than a squash. The merge commit's subject is the pull request's title, so it
obeys the commit contract like any other subject, and its body is empty, because
the prose belongs to the commit underneath it and a review checklist is not
history: `git log --first-parent` then reads one line per change with the detail
one level down.

And this log moves into the tracked documentation. Almost every commit body in
this history cites an identifier from it, and while it lived in a gitignored
directory those citations pointed at a document no reader of the repository
could open. Publishing it is what makes them resolvable, and it costs the
entries a rewrite rather than a copy: the record is what was decided and why,
addressed to nobody, with no path into anything a clone does not contain.
`.notes/` keeps what it is for — status, roadmap, plans, handoffs, and drafts of
decisions not yet taken.

## D-124 — the C boundary opens by three rows, and none of them is a slice

Status: approved · 2026-08-19

`D-065` closed the FFI vocabulary on purpose and the closure held for four
milestones, at the cost of three things the language could not otherwise reach:
C could not write into a buffer, so every hosted primitive was shaped "C
allocates and returns"; there was no spelling for an out-parameter; and a
function pointer could not cross, which is what a thread library over
`pthread_create` would be written on. Each is one row, and the rows are opened
before the freeze because widening a table afterwards is compatible while the
shape of what the rows *mean* is not.

**C fills what you own, borrowed exclusively.** A `(&mut (List T))` or a
`(&mut (Array T N))` crosses as the element pointer and the element count, read
out of `SlList` at fixed offsets the way a `Slice` already was. A `(Slice T)` is
deliberately not offered on the writing side: it does not record whether it was
made from a shared or an exclusive borrow, so a safe call could have C write
through a loan somebody else is reading — and closing that would mean either a
flag on a binding that does not survive the slice being moved, or mutability in
the type, which is a change to what a slice *is*. The collection states its
exclusivity in the type, so the property survives being passed through a Slopium
signature, and reading is unaffected: that is what `(& (Slice T))` is for.
Every element is one machine word whatever `T` is (`D-113`), so what C is handed
is an array of `int64_t` and a byte-at-a-time API needs a shim; the alternative,
passing the element size as a third word, was refused for making the writing row
disagree with the reading one about what a buffer looks like.

**An out-parameter is a whole machine word.** `(&mut i64)`, `(&mut u64)`,
`(&mut f64)` and `(&mut (Ptr T))` cross as `int64_t *`, `uint64_t *`, `double *`
and `T **`. The narrow integers and `bool` are refused by name, because an
integer is held canonical in a full word: C writing four bytes through an
`int32_t *` would leave the upper half of the slot holding what was there, and
the next read of it would be a different number. That refusal costs `int *out`,
which is common in POSIX, and it is still the right one — the answer for a
narrow slot is a `(Ptr i32)` and an `unsafe` write, and widening this row later
is additive where getting it wrong now is not.

**A callback is a named function.** A `(Fn ...)` in a declaration is a C
function pointer and the argument at that position must name a top-level `fn`,
which lowers to the symbol's address and nothing else. A `lambda` is refused by
name, and so is a local holding a function value: since `D-101` a function value
is a heap block with a header and an environment, and a `void (*)(...)` has
nowhere to carry one. The callback is entered with Slopium's own convention, so
its own parameters and result are scalars — a borrowed string would arrive as a
pointer to a runtime structure where C would have passed a `char *`, and an
owned result would move ownership out of a function C called, which `D-065`
refuses in the other direction for the same reason.

**Aggregates stay refused, by name.** A Slopium struct is not a C struct — every
field is a machine word and the value is a heap block — so passing one is not a
row in a table but a foreign record: a second kind of type with C's layout,
needing `repr`, its own field access and its own drop story. By value it is
worse, being the eightbyte classification and its AAPCS64 counterpart. It
arrives with the target that needs it (`D-110`).

None of this costs a MIR instruction or a backend line. The two new words are
`ExternWord::Indirect` at another offset, the address of a scalar is the
`AddressOf` that a borrow already emits, and a function pointer is the `FnAddr`
that `D-092` added — which is why the compiler protocol does not move.

What the work uncovered is worth more than the rows. **Constant propagation kept
folding a local after its address had been taken**: `AddressOf` marked its
destination varying and said nothing about its source, so a `(let mut half 0.0)`
handed to C as an out-parameter still read as `0.0` afterwards in a release
build. Nothing could write through a scalar borrow before this — a `(&mut T)`
parameter is refused as an assignment target (`D-120`) — so the hole was
unreachable rather than absent, and it was reachable the moment C could write.
The cross-backend suite caught it on the first release build that had one, which
is `D-026` doing exactly what it is for: the dev build agreed with the release
build about everything else, and the two disagreed here.

## D-125 — `format` is reserved at the freeze, and nothing is built

Status: approved · 2026-08-19 · implementation deferred to the freeze

The first report from somebody writing a real program rather than a fixture
named string building as the second-heaviest friction in the language: `concat`
takes exactly two arguments, so a line of output is a stack of nested calls and
a column of intermediate bindings. The answer taken for it is a `StringBuilder`
over a `(List u8)`, which is where the cost actually is — a template is sugar
over a buffer, and building the sugar first would build it twice.

**But not reserving the word would decide more than that.** Three rules meet
here and leave no third way. A new form cannot be added after the freeze, which
is `D-122`'s rule and the reason every declaration form already carries an
annotation slot. A variadic library function cannot exist: there are no variadic
functions and, without traits (`D-088`), no way to write one argument list that
takes an `i64` here and a `String` there. And a pattern macro cannot do it
either, because `D-109` admits only pattern macros over `SExpr` and formatting
dispatches on the *static type* of each argument, which a pattern has no way to
ask about. So the choice at the freeze is between reserving the word and
deciding that interpolation never arrives — and the second is a decision that
should be taken deliberately if it is taken at all.

The word is therefore reserved, and the shape is written down with it, because a
reservation that does not say what it is holding open is not one:

```lisp
(format "task #{}: {}" id title)
```

The template is a **literal**, so the holes are counted and the expansion
decided at compile time; `{}` is a hole and `{{` is a brace; each argument is
converted by its static type through the library's own conversions and appended
to a builder; the result is an owned `String`. A template whose hole count
disagrees with its argument count is a diagnostic rather than a run-time
surprise, and an argument whose type has no conversion is refused by name the
way the C boundary refuses a type it cannot spell (`D-065`).

Reserving costs one identifier. `format` appears nowhere in the bundled library,
the fixtures or the examples, so nothing in the tree has to move, and after the
freeze a declaration or a binding may not take the name — which is what makes
adding the form later a change nobody's program can notice.

`StringBuilder` needs no reservation: it is a library type and a library
function, and both are additive at any time.

## D-126 — a temporary is borrowed where a call takes it, and dies there

Status: approved · 2026-08-20

Only a named binding could be borrowed, so `(& "hello")` and
`(& (from-i64 id))` were refused and the shape that forced — a column of `let`s
whose only purpose is to give a value a name — was the first thing an outside
program complained about. It was in the library too: six printers each spent a
line naming a string they immediately borrowed.

**A temporary may now be borrowed where a call takes it as an argument, and it
dies when that call returns.** The lifetime rule is the whole decision, and the
alternative — extending the temporary to the end of the enclosing scope, as if
the compiler had written the `let` — was rejected: it keeps an allocation alive
somewhere a reader cannot see, and in a loop body it grows memory until the
scope ends. Dying at the end of the expression is what a reader would guess,
and it is what the lowering already knew how to do.

**The argument position is not a restriction on top of the rule; it is the rule
made checkable.** The temporary dies at the end of the expression the borrow
appears in, so there has to *be* such an expression — a call is one, and a
`let` value is not, because the binding outlives it. So `(let text (& "x"))` is
refused with a message that says to name the value, and everything else is
reached through an argument anyway.

The checker recognises the position with a depth counter rather than a flag,
because a call inside an argument list is ordinary: `(println (& (concat (& a)
(& b))))` opens three, and each releases what was borrowed inside it. The count
is reset around a `lambda` body, which is a function that happens to be written
inside an argument rather than part of one — a borrow *inside* that body is
still fine, because it is an argument of the call the body makes.

Lowering cost nothing new. `clone` already collected the owned temporaries
handed to it and dropped them after the call, so this is that mechanism with
one more producer: the borrow pushes what it owns onto a stack, and the
expression wrapper drains the stack after any call it lowered, innermost first.
Neither backend learned anything, and no MIR instruction was added — the borrow
is the `AddressOf` a borrow always was.

## D-127 — a body is as many expressions as it needs, and a one-sided condition is `when`

Status: approved · 2026-08-20

A `match` arm was one expression and an `if` branch was one expression, so an
arm that did two things was written `(do ...)` and a condition with no second
branch was written `(if condition (do ...) ())`. Both are grammar, both had to
be settled before the freeze, and they are one decision because the second is
only worth having once the first exists: `when` without a multi-expression body
would be a word that saves a `()` and still costs a `do`.

An arm takes a pattern, an optional guard and then as many expressions as it
needs, answering the last. That is why the guard is found by the word `when`
after the pattern rather than by counting the arm's elements — an arm of four
elements is now a body of three, and the old rule would have read it as a
guard.

An `if` puts its extra expressions in the tail, which is the `else` branch;
`then` stays a single expression. The asymmetry is deliberate. With an `else`
that is never optional there is no second boundary to find, and the shape the
language actually writes is a function answering early: the short answer above,
the work below. The bundled library agreed before the rule existed — twenty-six
`if`s had a block in the `else` and none had one in both branches — so the tail
form fits what was already there rather than inviting a new layout.

`when` runs its body when the condition holds and answers `unit` either way. It
is the same word as the arm guard, in head position, which is decided here
rather than discovered later: a guard follows a pattern and a `when` form
begins one, so no arm and no expression is ambiguous. Nothing downstream
learned the word — it is parsed into an `if` whose `then` is a `do` with a
`unit` after it and whose `else` is `unit`, so a body ending in a value drops
it exactly where a `do` would, and neither MIR nor either backend has a case
for it.

Rewriting the library into the new forms is what showed the size of the thing:
fifty-seven blocks and twenty-eight `()` branches disappeared, and three of
those branches were the *first* one — `(if condition () work)` — which is now
`(when (not condition) work)` and says what it means. The formatter needed no
change and agreed with the mechanical result line for line, which is the
strongest evidence available that the new shapes are the ones the layout rules
were already describing.

## D-128 — a manifest survives a key it does not know, and a config does not

Status: approved · 2026-08-20

`deny_unknown_fields` on every manifest struct made a key this toolchain does
not know an error, which is the wrong default for the one file in this project
that more than one version of it reads. A manifest travels with the package: a
manifest written for a later toolchain is read by every earlier one that
resolves it, so refusing an unknown key makes every field added after 1.0 a
breaking change to the format, and features, dev-dependencies and
target-specific dependencies impossible to add at all.

So an unknown key in a manifest is reported and ignored. Reported, because the
other thing an unknown key can be is a typo, and `deny_unknown_fields` was
catching those for free; a key dropped in silence is a setting that quietly
does nothing, which is worse than either. The warning is `SL1200`, the
manager's first, and it names the dotted path — `profile.dev.lto` rather than
"an unknown key somewhere". The archive carries the key verbatim, because what
is packaged here is what a later toolchain reads and rewriting it on the way
through would defeat the point.

Which manifests report is the rule `SL08xx` already follows about a
dependency's source: the workspace being acted on, and nothing below it. A
consumer cannot edit a dependency's manifest, so the warning there would be
addressed to somebody who is not reading it.

`.slopium/config.toml` keeps refusing, and the difference is what the file is.
It belongs to the checkout rather than to the package, it is shipped nowhere,
and nothing older than the toolchain running now will ever read it — so a key
nobody knows there is a mistake, and refusing it is help rather than
obstruction.

The mechanism is `#[serde(flatten)]` capturing whatever a table did not
recognise, so what counts as known stays the struct definition rather than a
second list beside it that could disagree with it. The one thing the change
cost was message quality in a single place, and it was bought back: a
dependency entry that names no source now says so *and* names the keys nobody
knew, because `geometry = { pth = "../geometry" }` is a typo rather than a
message from the future, and the refusal that used to name `pth` is the one
worth keeping.

## D-129 — hexadecimal is two functions and an uppercase table

Status: approved · 2026-08-20

`core:string` wrote decimal and nothing else, so a program printing a mask, an
address or a device register — which is most of what a freestanding program
prints — had to build the digits itself. It writes base sixteen now, in `core`
rather than `std` for the same reason the rest of the module is there: the
caller who wants it most has no C library under it.

Three things were decided while writing it, and none is deep enough to be
anywhere but here. **The `0x` is a second function rather than a `bool`.** The
shape the issue described was one function taking a value, a width and whether
to include the prefix, and in a language with no named arguments that reads
`(hex-from-u64 mask 8 true)` at the call site, where `true` says nothing.
`hex-from-u64` and `hex-prefixed-from-u64` cost one name and say which is
wanted by which is called.

**The width is a floor.** Fewer digits than it are padded with zeros; a value
needing more keeps all of them. A width that truncated would turn a number
into a different number that still looks like a number, which is the one thing
a formatter must never do, and zero means the natural width so that the common
case needs no thought.

**The glyphs are uppercase**, because that is how this language writes a
hexadecimal literal and how the compiler reads one back, so a value printed
here can be pasted into a program and mean what it printed. For the same
reason there is no signed variant: a hexadecimal literal is a bit pattern
rather than a number (`D-112`), so a signed value is rendered as
`(hex-from-u64 (as u64 value) 16)`, and a conversion between the two widths is
exact.

The loop divides by sixteen rather than shifting by four, which is slower and
is the point: it is `digits-of` over a different base, and a shift would be the
only place in the module where an unsigned value is taken apart a second way.
Nothing here is on a path where the difference is measurable.

## D-130 — failing on purpose, and a failing test that says what it compared

Status: approved · 2026-08-20

There were two ways out of a program that had gone wrong: a trap it did not ask
for, and `std:process:exit`, which says nothing. Neither is what a function
does when its caller broke the contract, so `panic`, `assert` and `unreachable`
are `core:panic`, written over `sl_rt_panic`, which the `extern` vocabulary
could already declare. They are `core` because the failure path is (`D-080`): a
freestanding program supplies the hook and gets the same behaviour, and
`core-check.sh` links one that calls in. A panic exits 101, the status every
deliberate failure in this language already exits with, and there is no
catching it — a `Result` carries the failure a caller can answer (`D-087`), and
this is the other kind.

**All three answer `unit`.** The honest type is one meaning "never", and the
language has none. Adding one to spell three functions would be the larger
change by far — every `match` arm, every branch merge and the whole of
inference would have to learn a type that is a subtype of all others — so the
functions were written to the language rather than the language to the
functions. What it costs is that a panic goes where a statement goes: under a
`when`, at the end of a branch, and not where a value is expected. The `when`
of `D-127` is what makes that read as intended rather than as a hole.

**A failing test is a separate mechanism, and that is the point.** A failed
assertion ends the program, so a suite built on `assert` reports one problem
per run and stops. `std:test` instead gives `equal-i64`, `equal-u64` and
`equal-text`, which answer exactly as `=` does and, on a mismatch, leave a note
that the harness prints beside `FAILED` — so the run continues and every
failure names both sides. It is `std` rather than `core` because the harness
is; a freestanding program has nothing to print with.

The note lives in the hosted runtime as one bounded slot, copied on the way in
and cleared as the verdict is printed. One slot is enough because tests run one
after another, and copying is required rather than convenient: the `String` the
message was built in is dropped on the way out of the call that made it. That
is a length error away from memory corruption, which is why `runtime-check.sh`
now runs a deliberately failing suite under ASan and valgrind, with a note
longer than the slot among them — and why a test that leaves no note is in
there too, since a note printed against the wrong test is the other way this
fails.

`tests/projects/test-fail` exists for the same reason. Every other fixture
asserts that its tests pass, which is precisely the case where a failure has
nothing to report, so what a failing test says was held to nothing at all until
a fixture failed on purpose.

## D-131 — the release page is generated from the titles that were merged

Status: approved · 2026-08-20

The release notes were the `CHANGELOG.md` section and nothing else, which meant
a pull request's title was read once, by whoever merged it, and then never
again. The title is already load-bearing — it becomes the merge commit's
subject, which is what `git log --first-parent` prints — so the cheapest way to
make it matter was to publish it.

The first half of a release page is now one line per pull request the tag adds,
taken from the merge commits with `git log --first-parent` between the previous
tag and this one. Nothing rewrites it afterwards, deliberately: a title nobody
thought about appears verbatim in front of everybody who downloads the
toolchain, and the place to fix that is the title, before the merge. It is a
lever on the one piece of writing every change already has to produce, and it
costs a shell step.

The second half stays the changelog's own section, because a subject line
cannot carry why a change was worth making and a generated list of them is a
table of contents rather than an account. So the page reads as the machine's
half over the person's half, and the version's section still has to exist —
`release-check.sh` already refuses a tag whose changelog section is missing.

Two details fall out of the walk. The release commit is dropped from the list,
because `chore(release): vX.Y.Z` is the bookkeeping that made the tag rather
than something the tag delivers. And the job that builds the page needs the
whole history and every tag, where every other job in that workflow is happy
with a shallow clone: a walk from the previous tag has neither end of it
otherwise.

## D-132 — an optimistic analysis that did not settle is an error, not a result

Status: approved · 2026-08-20

The release pipeline carried two iteration bounds written as if they meant the
same thing, and they do not. The pipeline's bound is over passes each of which
preserves observable behaviour on its own, so stopping early leaves a correct
module that was optimized less; it was nonetheless a `debug_assert!`, which
aborted a debug or continuous-integration build whenever a program was merely
large enough to still be paying off after eight rounds. Constant propagation's
bound is over a dataflow whose lattice starts at the top and is lowered as
predecessors are visited, so an unfinished run leaves states that are too
optimistic — a local reading `Known` where a predecessor not yet propagated
would have made it `Varying` — and the rewrite then folded on them and turned a
`Branch` into a `Goto`. Bailing out of a pessimistic analysis is a missed
optimization; bailing out of an optimistic one and trusting the result is a
miscompile, and it was reachable only under `optimize`, which is the profile MIR
verification is off in.

So the two bounds are spelled differently now. The pipeline's is quiet and
keeps what the rounds it got achieved. The dataflow's folds nothing at all and
reports `SL0700` naming the function, because a bound that is actually reached
means the bound is wrong, and that is a thing to learn from a failing build
rather than from a program whose output does not match its source.

The same argument decides a third thing. The passes index `function.blocks` by
a terminator's target and renumber blocks through a table indexed the same way,
so a target past the end is an out-of-bounds index rather than a bad program —
a panic, which is the one failure mode `D-031` does not allow the compiler.
The verifier already refuses it and does not run in a release build, so that
one check is split out and runs in every profile, while everything else the
verifier says stays where it was: being wrong about a type produces a bad
program, which is what the gated pass is for.

## D-133 — a deferred expression runs before the scope's drops, and may not move

Status: approved · 2026-08-20

Memory and aggregates are released deterministically and an external resource is
not: a file descriptor, a socket, a lock and an `mmap` all live behind an `i64`
that owns nothing, so nothing runs when one dies. `D-084` decided there is no
destructor — a struct wrapping an integer owns nothing, so drop glue has nothing
to generate — and left the resource question open rather than answering it by
accident. `(defer expr)` answers it without a resource type: the work is named
where it is decided and it happens on every way out of the scope.

**Deferred expressions run before the scope's own drops**, in the reverse of the
order they were written. That order is not a preference. A deferred expression
names bindings of the scope it was written in, and half the point is to log or
report something on the way out, so running one after the scope had released
what it holds would be a use-after-free the compiler wrote rather than the
program. The reverse order is the same argument one step further: a later
`defer` may depend on what an earlier one set up.

**Nothing is captured and nothing is evaluated at the point it is written.** Go
evaluates a deferred call's arguments immediately and runs the call later, which
is two rules where one will do; here the whole body runs at the end, so it reads
whatever its operands hold then. What it answers is dropped, exactly as a `do`
drops everything but its last expression, and the body takes as many expressions
as it needs because every body has since `D-127`.

Nothing in that body may leave the scope it is running out of. A `try` returns
from the function, and a `break` or a `continue` jumps out of the loop whose exit
is running the body — each would ask that exit to run the same body again, which
before it was refused was an infinite recursion in the lowering rather than a
program that misbehaved. A loop the body opened for itself is a different scope,
and that is the distinction the refusal draws.

Two rules keep the ordering honest, and both are the same hazard from opposite
sides: the scope is still going to drop what it owns, after the deferred
expression has run. A `defer` may not move a name it found already in scope,
refused where the move is written — the shape a `when` guard already uses, for
the same reason. And every name it reads that owns something is share-borrowed
for the rest of the scope, so nothing written below can take the value away
first. That borrow has no `&` on the page to point at, so the refusal names the
`defer` instead of reporting a borrow the reader cannot find. What it costs is
that a deferred expression cannot consume a value; the resources this exists for
are integers C handed out, which it copies.

The exits are the four a scope has: falling off the end, a `break`, a
`continue`, and the error arm of a `try`. There is no `return` form, so that is
all of them. Each one lowers the expression again where it leaves — there are no
landing pads and no unwinder here, and a few duplicated instructions per exit is
the price of not having either. What this is not is a destructor: a program that
ends inside the scope takes none of those exits, and `panic` and `exit` still
end it where they are.

Writing it found that the compiler had two unwinds. `drop_scope_except` walked a
scope's bindings in reverse, and the error arm of a `try` walked the live set
instead and reproduced that order by hand, with a comment saying so. The second
one had nowhere to put a deferred expression, because it did not know where a
scope began. Both are `unwind_scopes` now, which ends a range of scopes two
phases at a time, and the `try` arm reaches it with the same range a `break`
does.

## D-134 — `;;` above a declaration is documentation, read out of the trivia

Status: approved · 2026-08-20

Hover showed a type and never a sentence, and `;;` meant nothing — a comment
starting with one more semicolon than it needed. Claiming it now is free,
because every program that wrote `;;` wrote a comment and still has one;
claiming it after the freeze would be a compatibility break, since a program may
have used the spelling for something else. So the syntax is taken now, and what
it costs is one rule about where a block begins and ends.

The block is the run of `;;` lines immediately above a declaration. A blank line
ends it, because a comment separated by one is about the file rather than about
what follows — the header at the top of every module in this repository is
exactly that. So does a comment sharing its line with code: `(fn a ...) ;; note`
belongs to the line it is on and not to whatever is declared next, which is a
mistake worth refusing rather than a case worth supporting.

**It is read out of the lossless tokens rather than out of the syntax tree.**
The tree carries no trivia, so an AST field would have meant threading a
`doc` through six declaration structs, the parser, and every pass that rebuilds
one. The tokens are already in `Analysis`, and `SymbolIndexBuilder` already
walks them to narrow a declaration's span to its name; the same walk, one step
backwards, is the whole implementation. The two hold identical information, so
promoting it to an AST field when a `slopium doc` wants one stays additive.

Two things fall out. The formatter is untouched, which is the point: what hover
shows is the bytes somebody wrote, and a pass that reflowed or re-indented a
block would be editing the sentence. And a `;;` inside a form — above a struct
field or an enum variant — is still an ordinary comment, because the walk
backwards from a field reaches its parent's open parenthesis rather than a
comment. Reading those later is additive too, and it wants a rule about what a
comment inside a form means, which nothing needs yet.

## D-135 — the manifest says what a module is for each target

Status: approved · 2026-08-20

There is no conditional compilation in the language, and the library's answer —
the `core`/`std` split — works because that split is a *dependency* rather than
a condition. It does not scale to a hardware abstraction layer with a file per
chip. The package format freezes with everything else, so the shape had to be
decided even though the second architecture that needs it is further out.

**The manifest says what a module is, not merely which files to keep.** A
`[target."<triple>"]` table maps a module name to a file, and the rest of the
program writes `(take arch ...)` once. Selection alone was tried first and does
not work: files keep the names their paths give them — `arch:x86-64` and
`arch:aarch64` — so a program still has to name the one it wants, which is the
problem this was supposed to remove. Every file any target names is out of the
build unless the target naming it is the one selected; the file that is selected
is compiled as an ordinary module, checked and exported like every other, which
is the property a `(target "...")` annotation on a declaration deliberately does
not have (`D-136`).

A triple this toolchain cannot build for needs no special case: its file is
never the selected one. That is what makes the table forward-compatible with no
version to check, and `D-128` covers the other direction — a toolchain older
than the key warns `SL1200` and builds for the targets it knows. Each package's
own table is read rather than the root's alone, unlike `[build] target` and
`linker-script`, and the difference is the one `D-117` already draws: those
describe the single image being built, this describes which of a package's own
files it is made of. A library with a module per chip is the case, and a library
is never the root.

**The compiler still reads no manifest.** It discovers a package's modules by
walking a source root it was handed (`D-002`), so the manager, which read the
manifest, hands the answer over on the command line: `--exclude-module` for what
this build is not made of, and `--module NAME=PATH` for the file a named module
is, with `--dependency-exclude` and `--dependency-module` for the same about a
dependency. The alias is a separate field rather than qualified into one string
because a module name already holds colons and `a:b:c` would not say which part
is which. Path derivation still names every file nobody said anything about,
which is every file in almost every package.

A path naming no file is refused, `SL1102`, whatever target is being built — the
standard `D-128` set for a key nobody knows, because a selection that quietly
does nothing is worse than one that is refused. The archive carries every
target's file: a package is one thing wherever it is built, and what a build
leaves out is decided when it runs.
## D-136 — a declaration says which target it is for, and `cfg` is not built

Status: approved · 2026-08-20 · supersedes the refusal recorded in `D-122`'s
neighbourhood and in the tracker

There was no conditional compilation anywhere. The library's answer is the
`core`/`std` split, which works because that split is a *dependency* rather than
a condition, and it does not scale to two definitions of one function that
differ by a word. The planning notes and the tracker both refused a `(cfg ...)`
**form** for three reasons: it is grammar, it makes every pass carry code nobody
compiles, and it makes the formatter and the language server work on text that
is not part of the program. Two of those do not survive contact with the
annotation slot.

`D-122` built that slot — a list between a declaration's keyword and its name —
and gave it to all six declaration forms, including the four where nothing
applied yet, precisely so that a new annotation stays additive after the freeze
and a new form does not. So `(target "<triple>")` costs **no grammar**, which
answers the first reason. And the declarations it does not select are removed
between `build_program` and `sema`, so nothing types them, nothing monomorphizes
them, no MIR carries them and neither backend learns the word — which answers
the second.

The third reason survives and is written down rather than argued away: a
declaration that is not selected is never checked, so it can rot silently. That
is the standard complaint about conditional compilation in every language that
has one, and it is why whole-file selection stays a separate mechanism rather
than being replaced by this one: a file selected by a manifest is compiled as an
ordinary module for the target it belongs to, checked like every other.

**It is spelled `target` and not `cfg`, and that is the decision inside the
decision.** `cfg` is a promise of a predicate language — `any`, `all`, `not`, a
feature namespace, an architecture separate from an operating system — and none
of that is being built. A word that promises what it delivers is worth more than
one borrowed from a language whose version of it is ten times larger. What it
tests is a target triple, spelled exactly as `slopium targets` prints it, as
`[build] target` takes it and as `--target` takes it, so one vocabulary covers
the manifest, the command line and the annotation. Widening the argument later
to name an architecture or an environment rather than a whole triple stays
compatible, because an annotation's arguments are ours.

The one thing this does better than the languages it is borrowed from is the
failure. A name whose only declaration was for another target is an unknown
name, and everywhere else that is where the story ends — the reader is left to
work out that the definition is in the file in front of them and simply not
theirs. The `Program` keeps a record of what was removed and which target asked
for it, so the refusal says so.

## D-137 — a changelog entry is a file, because two of them are not a conflict

Status: approved · 2026-08-20

Every change wrote its line at the same place, the line after `## [Unreleased]`,
so every two changes in flight conflicted there. Five pull requests merged in
one afternoon cost three resolutions in `CHANGELOG.md` and not one of them was a
disagreement: the entries were independent and git stopped only because they had
been inserted at a single anchor. One of those resolutions was also wrong, and
nothing caught it — an entry about a new form was concatenated under `### Fixed`
and read as a bug fix until the next person opened the file.

`D-123` already made this argument one file over. A `feat` used to bump
`workspace.package.version` in its own commit, and that could not survive more
than one branch at a time, so the number stopped being written where branches
collide and is decided at the release instead. A changelog entry has the same
shape and takes the same answer: a change writes
`changelog.d/<issue>[-<discriminator>].<kind>.md`, and a new file collides with
nothing.

The fragment holds **one bullet, exactly as it will be published** — the marker,
the two-space continuation, wrapped at 80. A collator that added the marker
would own the wrapping too, and then the width a writer sees would not be the
width that ships; the same argument `D-134` makes about the formatter leaving a
`;;` block alone. It is named for the issue because §5 already requires one, so
the number exists before the entry does and no two changes can pick the same
name. The discriminator is for a change that owes two entries of one kind, which
the very first batch already did.

`[Unreleased]` keeps its heading and holds nothing. `release-check.sh` already
refused a *release* that left entries under it; that rule is now the invariant
rather than a release-time one, and `changelog-check.sh` decides it on every
pull request. A release collates with `scripts/changelog-collate.sh`, which
orders by kind and then by issue number so that the same fragments always
produce the same section, writes the link reference the release check asks for,
and empties the directory — and the release check refuses a version that left a
fragment behind, which is the one way collation can be forgotten.

Two cheaper answers were weighed and both lose. A `merge=union` driver in
`.gitattributes` costs one line and silently produces two `### Fixed` headings
when two branches each add one, which is the same failure as the wrong
resolution above with nobody to notice it. Generating the changelog from commit
subjects is what `D-131` deliberately declined: it generates the release page's
first half from titles and keeps the changelog as the half a person writes,
because a subject line cannot carry why a change was worth making.

## D-138 — the verifier bounds every index it can, and says which it cannot

Status: approved · 2026-08-20

`FieldLoad`, `FieldAddr`, `EnumFieldLoad` and `EnumFieldAddr` became `index * 8`
in both backends with nothing having bounded the index; `StructNew` and
`EnumNew` were handed a field list nothing sized; and `Drop`'s `ty` selected the
runtime release helper through `lowering::drop_function` with nothing comparing
it to the local being dropped, so a mismatch freed a `String` with the list
dropper. `verify_field_stores` already did this work for a struct's *writes*,
which is what made the gap visible: the same lookup, in a pass already holding
the same layouts, was missing for every read and every construction.

Nothing but `mir::lower` produces MIR, so all of it was a guarded convention
rather than a live defect. `D-031`'s standard is that the compiler defends its
invariants rather than assuming them, and an out-of-bounds heap access is a poor
thing to be assuming in a language whose argument is that bounds checks are
untouchable.

**What cannot be checked is written down rather than skipped silently.** An
enum's payload instructions carry an index and no tag, because which variant is
present is a run-time question that `match` answers — so a payload index is
bounded by the *widest* variant, which is the bound that keeps the access inside
the block, and its type is not checked at all. `EnumNew` is the exception and is
checked exactly, because it is the one enum instruction that names its tag. The
previous rule expressed this as `if enumeration { continue; }`, which said the
same thing by saying nothing.

Two shapes the checks had to learn rather than refuse. A field read through a
borrow answers with a borrow of the field's type, not the field's type, because
borrowing a pointer-shaped value copies the word and the load *is* the borrow
(`D-099`) — the first draft rejected `core:map`'s own lowering over it. And a
lifted lambda's capture block is deliberately typed `i64` so that a body does
not drop what it was handed (`D-101`), so a `FieldLoad` base does not always
name a layout; where the type does not say which aggregate it is, nothing is
claimed about it.

## D-139 — composition is a form, and applied it is the nesting

Status: approved · 2026-08-20

Functions have been values since `D-092` and a `lambda` captures them by move
(`D-102`), so composition already worked as a library function — and two things
made that insufficient, both of which point at a form. It cannot be variadic:
there are no variadic functions and, without traits, no way to write one, so a
library `compose` is fixed at two arguments and three functions means
`(compose f (compose g h))`, which is the nesting this exists to remove. And it
cannot be free: measured on the optimized MIR, it cost three allocations and
three indirect calls where `(f (g x))` is two direct calls and nothing else.

`<<` and `>>` are both n-ary, and each is the other reversed. `<<` keeps the
order of names a nested call has and `>>` reads in the order things happen. The
spellings are free precisely because `D-106` wrote the shifts as `shl` and
`shr`; `.` was refused, because it is field access and
`tests/projects/pass/function-values` already has a struct whose field holds a
function, where a second meaning for the dot typechecks two ways and only the
names decide which.

**Applied where it is written, it expands in the parser.** The issue proposed
expanding in `sema`; the parser is earlier and enough, because the expansion is
`(f (g (h x)))` and needs no types. So nothing downstream — not `sema`, not
MIR, not either backend — ever learns that a composition was written, and the
MIR of `((<< double increment) n)` is byte-for-byte the MIR of
`(double (increment n))`. That is the whole of the cost argument above,
answered.

**Unapplied it becomes a `lambda`, by being written as one.** `sema` synthesizes
the AST for `(lambda (captures) ((x A)) -> C (f (g (h x))))` and types it
through the ordinary path rather than assembling a `TExprKind::Lambda` by hand,
so the capture moves, the environment swap, the borrow refusals and the lifting
all behave as they do everywhere else without being reimplemented. Only the
operands that are *local bindings* are captured: a top-level `fn` is callable
from inside a lambda without being closed over, so composing two of them — which
is nearly every composition — closes over nothing.

**The operands are names.** What an unapplied composition becomes closes over
them, and a `lambda` captures names, so an expression could not be an operand
however the form were written; a composition of something without a name is a
`let` away from being one. Each is held as the `Var` it is, so that a
module-qualified operand is resolved by the pass that resolves every other name
rather than by a second rule. Composing one function is that function — refusing
it would be a rule about arity where there is no question of meaning — and every
operand takes exactly one argument, because a composition passes one value along
the chain.

What this does not do is make the unapplied form free; that is still a lifted
lambda, and removing it is the devirtualization issue's business, not this one's.
## D-140 — a fieldless enum is its tag, and `=` reaches one

Status: approved · 2026-08-20

`D-089` restricted `=` to `bool`, the integer types and `f64` because everything
else was one machine word holding a *handle*, so a wider comparison would have
answered about identity while looking like it answered about contents. A
fieldless enum is the case that rule was written before it existed: an enum no
variant of which carries anything has nothing to point at, so the word can hold
the value itself.

So it does. Such an enum is its tag — one machine word, copied rather than
owned, with no block to allocate, free or clone, and no drop or clone helper
generated for it. Building one is writing a constant, matching on one compares
the value where it used to read a word out of a block, and `=` is an ordinary
`i64` comparison, which is the same thing `branch_pattern` has always done with
a tag. That last part is why the MIR verifier needed no widening: the comparison
never reaches it as a comparison of a named type.

**Per enum and never per variant.** `Option`'s `None` carries nothing and its
`Some` carries a value; which one a local holds is precisely the run-time
question `match` exists to answer, so a representation that differed between
them would need that answer before the question could be asked. It also keeps
`Step::WrapOption` — where both backends hand-assemble an `Option` block — true
without a line changing.

The obstacle was that nothing could answer "is this enum fieldless" where the
answer was needed. `Type::is_copy` is asked of a type alone and says `false` for
every `Named`, because a type does not know what it names; the MIR builder is
handed one function and never the module. So the answer is recorded where the
type is built — on `TypedEnum` and on `MirEnum` — computed once in `mir::lower`
and threaded into each builder, and asked through `Builder::is_copy` and
`Analyzer::is_copy_type` rather than of `Type`. Three predicates moved onto the
builder with it, `reads_through_borrow` among them, because whether a borrow is
the word or the address of a slot now depends on the module too.

What this deliberately does not do is make `(clone status)` an error. A
fieldless enum joining `is_nothing_to_clone` would refuse a program that
compiles today, and a representation change is a poor reason to break source;
cloning one is a copy that costs nothing, which is what it already was.

## D-141 — a call through a block built in the same breath names its symbol

Status: approved · 2026-08-20

A call through a function value was indirect even when the block it read the
address out of had been built three instructions earlier, because the callee was
a local and nothing looked at where the local came from. When it came from a
`StructNew` of a closure layout whose code word is a known `FnAddr`, the symbol
is known at that point and the call names it.

The order matters more than the rewrite. It runs **before** inlining, because an
ordinary `Call` is the only shape the inliner matches — so a devirtualized call
becomes an inlining candidate, and the body it was going to jump to gets spliced
in instead. That is where the win is: an unapplied composition of two top-level
functions now lowers to the same two direct calls the applied form does, which
is the cost `D-139` explicitly left on the table.

**Only within one basic block, and only in order.** Tracing across blocks would
have to prove the definition dominates the call and that nothing reassigned it
on the way; a straight line needs neither. A closure built and called in the same
breath — every composition, every `map` over a lambda written where it is used —
is a straight line, and a callee that arrives as a parameter is not traceable by
anything and stays indirect.

**A direct call adopts the callee's declared parameter types, and that is not a
detail.** A lifted body takes its block as an `i64` so that returning does not
drop what it was handed (`D-101`), while the value passed is typed as the `Fn`
it is: the same word under two conventions, and the two are checked by different
rules — `verify_indirect_calls` against the callee local's `Fn` type,
`verify_direct_calls` against the callee's parameters. Rewriting without
retyping produced MIR that failed verification immediately, which is the
verifier doing precisely what it is for.

**What this does not do is remove the allocation.** The block is still built and
still dropped, because a `Drop` is observable and `opt.rs` has never removed one
— that rule is what keeps memory released where the program says it is. Taking
the allocation as well means proving the block's captures own nothing and that
its only remaining use is that drop, which is dead-allocation elimination rather
than devirtualization, and is worth doing on its own terms.
## D-142 — an object with no debug information is refused, not handed back

Status: approved · 2026-08-20

`compile_to_object` and `compile_package_to_object` hardcoded the absence of
debug information: a caller who set `options.debug` received a stripped object,
with no error and no warning. The library had three answers available — emit
DWARF, refuse, or say nothing — and it was giving the third, which is the only
one that cannot be right.

It refuses. Emitting DWARF from the internal object writer is a piece of work
rather than a fix, and it is not what this decision is about; `D-028` already
says the writer emits no `.debug_*` section, and the refusal points at the path
that does carry line tables — emit assembly and assemble it, which is exactly
what `slopic` itself does. That is why the gap was invisible: the command-line
compiler routes a debug build through `as` and never reaches these entry points,
so only a caller of the library could meet it.

The same paragraph in `docs/architecture.md` now says the other two things
`--debug` costs, both of which were true and unwritten. It reacquires the
external toolchain dependency `D-028` removes, so a debug build can fail where
the same build without it succeeds. And with `--target` it hands foreign
assembly to whatever `cc` resolved to, because `slopic`'s `--cc` defaults to
`"cc"` whatever the target is. Neither is a bug to fix here; both are behaviour
somebody should be able to read about before it surprises them.

## D-143 — a form has two shapes, and a body starts on its own line

Status: approved · 2026-08-21

`format_source` was not a layout algorithm. It walked the token stream, indented
by nesting depth, kept whatever line breaks the author had written, and wrapped
only when the next token would cross column 88 — so a break landed wherever that
column fell, between two arguments as readily as inside one. Deep nesting was
unreadable for that reason and not because it was deep.

It lays the source out from the tree `parse_lossless` already builds, and every
form has two shapes: it fits on the line it starts on, or it does not. When it
does not, one of four things happens. A **head-line table** says how many
arguments stay beside the head — a `fn` keeps its name, its generics, its
parameters and its return type, an `if` keeps its condition, a `match` keeps
what it matches — and the rest go one per line at the body indent. A form whose
arguments have no structure under them, an `export` or a `take` or a literal
list, **packs** as many per line as fit. Anything else **aligns** its arguments
under the first one, when every one of them has a flat shape that fits there,
and otherwise puts each on its own line at the body indent with the first
staying beside the head when it fits.

**A body starts on its own line, however short it is.** `fn`, `test`, `when`,
`while`, `loop`, `do`, `match` and `enum` are laid out that way because what
follows their head line is a sequence rather than an argument, and a reader
looking for what a loop does is looking down the left margin. `if` joins them
only when it takes a fourth argument, because with three it is the expression
`(if flag 1 0)` is; `lambda` never does, because it is an argument far more
often than it is a declaration; and a body that is a single literal is not a
body, so `(fn com1-data () -> u16 0x3F8)` names a constant on one line.

Two things the tree does not say are marked before the layout runs. A `match`
arm's head is a list exactly as a struct's field list is, so the form above it
is what says which is which, and a guard belongs to the arm's question rather
than to its answer — `when` and what it tests stay on the pattern's line. And
`->`, `:` and a field keyword bind to what follows them, so a return type, a
declared type and a named field are never split across two lines.

**The tree is not changed.** The output parses to the same parens, atoms,
strings and comments in the same order, which is what makes `fmt --check` a
statement about whitespace rather than about the code, and what makes it safe on
every save. Rewriting an expression into a different one is a different tool
with a different guarantee. A `;;` block is emitted as it was written, because
`D-134` says hover reads the bytes rather than the tree.

The migration is asserted rather than described: `std/` is byte-identical to
what the formatter produces, every fixture is `fmt --check`ed, and two
properties run over the bundled library and the hundred refused sources beside
it — laying out a source whose indentation has been taken away gives the same
bytes as laying out the source, and a layout changes nothing but the whitespace.

## D-144 — every refusal about ownership carries an `SL03xx` code

Status: approved · 2026-08-21

`docs/diagnostics.md` says what the families mean: `SL02xx` is name resolution
and types, `SL03xx` is ownership and borrowing. Eight refusals that are plainly
about ownership went through `Sema::error`, which defaults to
`codes::NAME_OR_TYPE`, and shipped as `SL0200` — using a value while it is
mutably borrowed, moving one while it is borrowed, assigning to one while it is
borrowed, borrowing a moved value, borrowing an immutable binding exclusively,
borrowing one exclusively twice, borrowing exclusively over a shared loan, and
share-borrowing over an exclusive one. Their siblings in the same file pass
`codes::OWNERSHIP` explicitly. So the same kind of mistake was reported under
two families depending on which line raised it.

They pass `codes::OWNERSHIP`. Two neighbours are deliberately untouched: "only a
binding can be borrowed here" is about what a borrow may be taken of rather than
about a loan, and "`{name}` is captured more than once" is about a list written
twice.

The reason it is this version and not a later one is `#23`. A frozen code never
changes meaning, so after the freeze moving these is a compatibility break
rather than a correction, and leaving them is a permanent promise that `SL02xx`
sometimes means ownership.

Five of the eight were pinned by no snapshot anywhere, which is how they drifted
in the first place, and each now has a `compile_fail` case of its own. The
document did not change: this makes it true.

## D-145 — a string is built in one buffer, and the buffer is a `(List u8)`

Status: approved · 2026-08-21

`concat` takes two strings, allocates a third and copies both into it. That is
the only way the library had to join anything, so accumulating a document was
written as `(set out (concat (& out) (& piece)))` — which copies everything
accumulated so far on every piece, and costs the sum of the prefixes. Building
40,000 JSON records that way takes 41.0 seconds and 1.07 MB of output; through
a builder it takes 88 milliseconds, and the curve is a straight line rather than
a parabola.

`core:builder` is a `Builder` over one `(List u8)` that grows in place, with
`new`, `write-str`, `write-byte`, `write-i64`, `write-u64`, `size` and `build`.
Everything that writes takes `(&mut Builder)`; `build` takes the builder by
value, because the bytes leave with the string and a buffer that has been handed
over is not one to keep writing to.

**The buffer is a `(List u8)` and that is a word per byte.** Every integer in
this language is one machine word wide whatever its type says (`D-107`), so a
byte list is eight times the size of the string it will become. That is the
price of not adding a byte-packed collection to a language whose type vocabulary
is closed, it is paid only while the string is being built, and the alternative
— a growable buffer inside the runtime — would be a resource with no destructor
in a language whose argument is that it has none (`D-084`). It also means the
module needs no runtime entry point of its own: `sl_rt_string_from_bytes`
already takes one word per byte, and `sl_rt_string_byte` is how a piece is read.

`write-i64` and `write-u64` put digits into the buffer directly rather than
calling `core:string`'s `from-i64`, which builds its answer by prepending one
`String` per digit. That is also why the module does not `take core:string`: a
whole Slopium module is one `.text` section (`D-097`), and a program that builds
strings should not link a parser and a splitter to do it.

**`write-f64` lives in `core:float`,** for the same reason `std:float` is a
module rather than five names in `std:io` and `std:string`: the decimal
formatter is not small, and a program that builds a string without mentioning a
float should not link one. It is the one write that allocates, because the
formatter's answer is a `String` and there is nowhere else for the digits to go.

`(format "task #{}: {}" id title)` is not built on top of this. It is reserved
as a form at the freeze (`D-125`), and the cost this issue was about is the
copying rather than the spelling.

## D-146 — a list can be written into, and the sort is a merge sort over indices

Status: approved · 2026-08-21

A list could be pushed, popped, removed from and replaced into, and that was
all. `insert`, `swap`, `clear` and `truncate` are the four things `replace`
(`D-103`) makes possible and nothing had written yet. All four take
`(&mut (List T))` and answer `unit`, because what they change is the list they
were handed.

`swap` is the one worth describing. `replace` puts a value in and takes one out
in the same breath, so a swap has nowhere to start: exchanging two slots needs
one element held outside the list, and the only element that can leave without
moving the rest is the last. So `swap` takes the last one out, rotates through
`replace`, and pushes what falls out — five runtime calls and no element moved
but the two.

**It uses `remove` at the last index and not `pop`, and that is a constraint on
the whole of `core`.** `pop` answers an `Option`, so it needs the `option`
language item, which the toolchain registers for the bundled `core` and nothing
registers for a `core` vendored into a registry as an ordinary package —
`scripts/registry-check.sh` builds exactly that, and it is where the first
version of this was caught. Removing the last element shifts nothing, so the
two cost the same, and `core` keeps the property that it builds without the
toolchain having introduced itself first. An
index outside the list ends the program the way an out-of-range `get` does,
which is why `core:list` takes `core:panic` and is the only thing it takes it
for: a bad index is a bug in the caller, not an answer it can be given.

**The sort is a merge sort over the indices, and the permutation is applied with
`swap`.** What it replaces was a selection sort that compared every remaining
pair for every position and then moved each element out with `remove`, so it was
quadratic in comparisons and moved the tail of the list on every placement. Its
own comment said a better algorithm needed a way to overwrite one element, which
`replace` had been for two versions.

Sorting the elements directly is what a language with a default value would do,
and this one has none: filling a slot means having something to put in it, and
an owned `T` has nothing to leave behind. An index does. So a `(List i64)` of
positions is merged bottom-up into a second list of the same length — two lists
that trade places on every pass, which is why one is a clone rather than
something built empty — and the elements are moved once each at the end, by
following the permutation's cycles with `swap`. That is `O(n log n)` comparisons
and at most `n` swaps.

Merge and not quicksort, because `sort-by` already promises that equal elements
keep the order they arrived in. Merge sort keeps it by taking from the left run
on a tie; quicksort does not keep it at all, and would also want a random access
pattern this list is worse at than a sequential one.

Sorting 10,000 scrambled integers took 3,262 milliseconds and takes 21. Four
times the elements used to cost twenty-seven times the time; it now costs about
five.

An insertion sort in place over `swap` was written first and measured **slower**
than the selection sort it replaced — 1,881 milliseconds against 577 at 4,000
elements — because the number of moves stays quadratic and each move became five
runtime calls instead of one memory copy. That is the whole argument for
measuring a performance claim rather than reasoning about it, and the reason
this entry describes an algorithm rather than a refactor.

## D-147 — the clock and the entropy are the operating system's, in nanoseconds and bytes

Status: approved · 2026-08-21

A program had no way to ask what time it is and no way to get a byte nobody can
predict. Both come from the operating system, so both are `std` and neither is
`core`: a freestanding program has no clock to read and no kernel to ask.

`std:time` is `monotonic` and `realtime`, and there are two because they answer
different questions. A monotonic reading only ever goes forward and is what a
duration is measured with; a wall-clock reading is what a timestamp is written
from and can move backwards when something corrects it, so measuring an interval
with it is a bug that surfaces about once a year. Both are **nanoseconds in an
`i64`** rather than seconds in an `f64`: a wall-clock reading is around
1.7 × 10^18 nanoseconds and an `f64` stops holding consecutive integers at 2^53,
so a float would have been rounding the clock away since 1970. An `i64` of
nanoseconds is exact until 2262.

`struct timespec` is not in the `extern` vocabulary (`D-065`) and adding a row
for it would open the boundary to records for the sake of one call, so the
runtime is where a reading is flattened into one number. That is the same
division `std:fs` already makes: what crosses is scalars, and the shape stays in
C.

`std:random` is `bytes` and `number`, over `getrandom` with `/dev/urandom` as
the fallback a kernel too old for it still offers. There is no seed and no
generator, because a sequence a caller can reproduce and one nobody can predict
are different things and only the second is safe to reach for without being
asked for by name; a reproducible one is a few lines over `core:list` and
belongs to whoever needs it. `number` shifts eight bytes together with `shl` and
`bit-or` rather than multiplying, because arithmetic traps on overflow (`D-031`)
and the top byte of a random word overflows every time.

The buffer is filled by C into slots the library made first: `(&mut (List u8))`
hands over the elements and their count and C may not resize the collection
(`D-124`), so the length is decided in Slopium and the bytes are written into
it. A negative count is refused with `EINVAL` rather than clamped to nothing,
because asking for minus one byte is a mistake and an empty list would hide it.

Neither module aborts (`D-087`). A call that fails is an `Err` carrying the
`errno` the status slot held (`D-085`), which is what `std:fs` and `std:process`
already do.

## D-148 — a child process is started with its arguments joined by NUL, and its output is a descriptor a `defer` closes

Status: approved · 2026-08-21

`std:process` could read its own arguments, read the environment and exit. It
could not start anything. It can: `spawn`, `capture`, `wait`, `read-output`,
`close-output` and `pid-of`, over `fork`, `execvp`, `waitpid`, `pipe` and
`close`, in the shape `std:fs` and the rest of `std:process` already are — a
Slopium module over `extern`s, with a failure carried through the status slot
(`D-085`) as an `Err` rather than an abort (`D-087`).

**Both, and not one.** Inheriting the parent's output is what a build tool
wants; capturing is what everything else does, and it is the half that cannot be
worked around from outside. `spawn` leaves the child's standard output where the
parent's is; `capture` gives the child the write end of a pipe and hands the read
end back.

**The argument vector crosses as one buffer of NUL-separated pieces with its
length beside it.** A list of strings is not in the `extern` vocabulary
(`D-065`), and adding a row for one would open the boundary to collections of
owned values for the sake of this call. The buffer is the shape `D-079`
established for every string the runtime is handed, and the only one an embedded
NUL does not truncate; joining the pieces is three lines over `core:builder`,
which is what `D-145` built. `argv[0]` is the program's own name, added by the
runtime, so what a caller passes is the arguments and not the convention.

**The descriptor is closed by a `defer`, and that is why this is expressible
now.** A `Child` carries a pid and a descriptor, and neither owns anything: a
struct wrapping an `i64` has no drop glue, so nothing runs when it dies
(`D-084`). Before `D-133` the honest minimum was inheriting the parent's output,
because a pipe held in a value nothing closes is a leak the compiler cannot see.
`close-output` is written beside the call that opened it and runs however the
scope ends, which is the same thing `tests/projects/pass/defer` demonstrates
against a C handle. A `Child` that inherited carries `-1`, and closing that is a
no-op, so the same `defer` is written either way.

Two behaviours are the shell's rather than this library's invention, because a
caller comparing a status against zero already handles both. A child something
killed reports `128 + signal`. A program that could not be `exec`'d reports
`127`, and it arrives as an ordinary exit status rather than an `Err`, because a
failed `exec` after a successful `fork` is already a running process and the news
is the child's to deliver.

Capturing means reading before waiting: a child writing more than a pipe holds
blocks until somebody drains it. That is a property of pipes and not of this
design, and it is written down here and in the module because the deadlock it
causes is the first thing anybody meets.

## D-149 — a sigil stands before the form it applies to, and the reader expands it

Status: approved · 2026-08-21

Uniform syntax is paid for in parentheses and a one-argument form pays every
time it is written. The borrow was the extreme case: 752 `(& …)` and `(&mut …)`
frames across the 132 tracked `.slp` files, each costing a paren pair that told
a reader nothing the `&` did not. A borrow is now written `&x` and `&mut x`,
and the mechanism is a rule rather than a carve-out — the reader has a table of
**abbreviations**, a token standing for structure nobody typed. `&` and `&mut`
are the rows that expand; `'`, `` ` `` and `,` are rows that are refused by
name, so the macros `D-109` deferred inherit a mechanism instead of finding
their characters taken. Expansion happens in `reader::expand`, between the
lexer and the parser, so `ast.rs` and everything under it never learn a
spelling was used — the same place `D-139` put applied composition, and the
reason `respelling_a_borrow_emits_the_same_object` can assert that the object
file is byte-identical.

**The unabbreviated form stays legal, and one rule buys that.** A sigil that
opens a list whose end is its own operand is the head of that list rather than
a prefix inside it, so `(& x)` is the borrow it has always been and no existing
source moved. With anything after that operand the sigil abbreviates like any
other, which is what makes `(&T &T)` — the parameter list of a `Fn` — the two
elements it looks like. The price is one place where the short spelling says
something else: a list holding a single borrowed type keeps its parentheses,
`(Fn ((& String)) i64)`, because `(&String)` there is the borrow. Writing it
short is refused where the `&` lands in type position on its own, with a help
naming the spelling that was meant. Requiring the sigil to be adjacent to its
operand would have freed the head instead, and was rejected: `&mut` and its
operand are necessarily apart, so adjacency would put a carve-out inside the
mechanism built to avoid carve-outs.

Two things fell out that the issue did not predict. A text literal did not end
an atom, so `&"literal"` lexed as the atom `&"literal` and the rest of the line
came apart; `"` ends an atom now, which nothing could depend on before, because
nothing could be written immediately before a quote. And the layout measured a
form against the preferred width without the run of closing parens that hugs
its last line, which no corpus file had ever reached — until the abbreviation
made a form short enough to be kept on a line whose tail then carried five of
them. `render_item` is handed that count now, and a form that fits only when
the parens are not counted breaks instead.

## D-150 — `$` nests the rest of the form it is written in

Status: approved · 2026-08-22

Nesting inside a body is not branching. Across the 132 tracked `.slp` files,
runs of closing parens that do not end a declaration numbered 1738 of length 1,
1055 of 2 and 451 of 3, then fell off to 57 at length 4 — and reading the
outliers showed what they were: chains of single-argument wrappers, each call
taking the next as its only interesting argument and each step paying a paren
back at the end. `$` opens a list that closes where the form holding it closes,
right-associative as in Haskell, so `(a $ b $ c d)` is `(a (b (c d)))` and the
chain reads in the order the calls happen rather than in the order the
parentheses close.

It is a row of `D-149`'s table rather than a form, so it expands in the reader
and nothing downstream learns it was written; the list it synthesizes spans
from the `$` to the end of its last element, which is text that exists, because
a `compile_fail` snapshot asserts spans as numbers. The two rows compose: a
sigil whose operand is a `$` applies to everything after it, which is what
makes `(note $ & $ disagreement left right)` a borrow of the whole call rather
than of the `$`. Three placements are refused with `SL0007` — a `$` as a form's
first element, where the head belongs; a `$` with nothing after it; and a `$`
with no form around it at all.

**What `$` means inside a declaration's header is what it means anywhere.** The
reader does not know what a list is for, and one that guessed would be reading
the grammar `ast.rs` owns, so a `$` in a `lambda`'s parameter list expands there
like everywhere else and lands on the shape error that list already produces.
That is the answer the issue asked for, and it is written as a `compile_fail`
case rather than as a rule.

**The formatter neither writes a `$` nor removes one.** Which grouping a person
meant by one is not recoverable from the tree, and a formatter that guessed
would be having opinions about structure rather than about layout. Two things
the layout does do: it keeps a `$` off the end of a line, and it leaves a sigil
written before one as the separate token it is — folding `& $` into `&$` would
be folding the sigil into an operand that is not its operand. It does not
compete with the terminal closer either: `$` produces no run of closing parens,
it prevents one, so code written with it reaches that rule with nothing to
collapse.

`.` was rejected for the same reason `D-139` spelled composition `<<` and `>>`:
it is field access, and `tests/projects/pass/function-values` has a struct whose
field holds a function.

## D-151 — one token closes every list a declaration left open

Status: approved · 2026-08-22

A declaration unwinds all at once at its end, and the parens that do it carry no
information a reader can check without counting them. Across 833 declarations
the terminal run was 389 of length 1, 171 of 2, 144 of 3, 100 of 4, and on up to
a nine in `std/core/map.slp` — already the residue of two de-nesting passes, so
what is left is structural. `|)` closes every list still open, back to the top
level, written where the pile would be and needing no opener.

**Top level only, deliberately.** A paired opener usable mid-body was designed
and dropped: with one, a `|)` written a line early silently ends the whole
`match`, the remaining arms become sibling expressions, they parse without
complaint and fail somewhere else in type checking. Without one, the rest of
the declaration lands at the top level, where `build_program` refuses it on the
spot.

**It improves the diagnostic it appears to weaken.** Parentheses are an
error-detecting code and a close-everything token does delete redundancy — but
a `)` lost inside a long module used to swallow every following declaration and
surface as one `SL0004` at the end of the file, pointing hundreds of lines up.
`|)` forces the depth to zero, so a lost paren cannot leave the declaration it
was written in, and the parser gains a resynchronisation point it did not have.
`SL0004`'s help names `|)` beside `)`, and a `|)` with nothing open is `SL0003`.

**`|` ends a token wherever it appears**, which is what makes `(c d|)` a name
and a closer. A closer built from a character a name may contain cannot work:
`<` and `*` are ordinary names that can be passed as values, so `(>> f <)` is
legal today and `<)` would force a meaningful space into it — the trap `D-106`
refused when it declined `(& a b)`. `|` occurs nowhere in the tree, in code, in
a comment or inside a literal, which is why it was free to take.

**The formatter writes it wherever the run is longer than three.** Three is not
taste: runs of one and two are the two most populous buckets, so a lower
threshold would rewrite half the tree and flicker under ordinary editing,
because every edit that changes nesting depth by one would cross it. The
collapse is a rewrite of the rendered tail rather than a decision inside the
layout, because a `|)` closes whatever is open at that point regardless of which
forms those are.

**The editor needed a real indenter, and that is the largest piece of this.**
`editors/nvim/ftplugin/slopium.lua` set `lisp` and delegated to Vim's built-in
Lisp indenter, which counts `(` and `)` in Vim's own C source and cannot be
taught another closer: a file holding a `|)` indents as if that declaration
never ended, and every declaration after it walks to the right. `lisp` is off
now and `editors/nvim/indent/slopium.lua` installs an `indentexpr` over
`lua/slopium/indent.lua`. It reads `lispwords` from the buffer rather than
carrying a copy, so the editor keeps the one list the language server already
checks against the words the language has — `head_line_groups` in `syntax.rs`
remains a second copy of that fact, and this deliberately did not become a
third.

`scripts/nvim-check.sh` runs the indenter in Neovim over the bundled library and
asserts what the closer threatens: every declaration stays at the margin, and
the indentation is idempotent. It asserts the margin rather than byte-identity
with `fmt`, because an editor indents a continuation line without measuring what
a whole form would cost — the layout drops a call to the body indent when its
arguments plus the parens closing every enclosing form would pass the preferred
width, which an indenter reading one line cannot know. Two lines of the tree
differ that way. Neovim joins valgrind, gdb, qemu and the cross toolchain in the
dev shell, and the check skips itself without one like the thirteen before it.

## D-152 — `deprecated` applies to a struct field, and warns wherever the field is named

Status: approved · 2026-08-22

`deprecated` applied to a `fn`, an `extern` and a `const` (`D-122`) and could
not apply to a field, because a field is exactly `(name type)` and the parser
said so. It carries the slot now, written before the name, which is where every
declaration with a keyword carries one. The space was free: nothing meant
anything else there, so no program could have been relying on it. It is grammar,
which is why it had to be decided before the freeze — after it a form that was
never written down cannot be added by an edition.

**Deprecating the type is the wrong instrument for the common case.** The
migration that actually happens is that a record is fine and one field of it is
going away. Saying that by annotating the whole `struct` warns at every mention
of a type that is mostly healthy — in parameter types, in return types, in
patterns that never touch the field in question — and a warning nobody can act
on is a warning that stops being read.

**A field name appears at exactly three places, and all three are literal**: a
read `(. p label)`, a construction `(P :label "x")`, and a pattern
`(P :label label)`. There is no fourth, because `set` writes a field only
through a binding a pattern introduced, so the pattern case covers writes for
free. That is what makes this cheaper than deprecating a type, whose name
reaches several passes. The annotation is looked up by the declaration's name
rather than an instance's, so a generic record carries it once however many
times it is instantiated.

**It is interface**, for the reason every `deprecated` is: what warns is the
module that names the field, so a dependent that does not rebuild is a warning
nobody sees. `module_interface` in the manager hashes a field's annotations
beside its type, and `tests/projects/pass/annotations` declares the record in a
module of its own so the warning has a boundary to cross.

Deliberately not included, each for its own reason. **A variant of an `enum`**:
a bare variant is an atom — `(enum E A B)` — so there is nowhere to put an
annotation without wrapping it, and `((deprecated) A)` is then structurally a
different thing from `A`; no spelling exists that is not a wart, and deprecating
the enum or the payload field of a variant that has one is the answer. **A
parameter**: the slot is offered where a `struct`'s fields are read and nowhere
else, because a parameter is the caller's business and deprecating one says
nothing a caller could act on. **`target` on a field**: a record whose fields
differ per target is a layout that differs per target, which is a much larger
question than an annotation slot. **`inline`**: meaningless on a field. Each of
the three is refused by the table `D-122` already had, and by name.

## D-153 — a borrow of a borrow is refused where it is written and where it is instantiated

Status: approved · 2026-08-22

`sema` has refused the expression `(& (& x))` for as long as borrows have
existed, with a message saying that a borrow of a borrow says nothing the
borrow did not say already. The *type* was never asked the question: `ast::ty`
built `&&String` without complaint, so `(fn holds ((value &&String)) -> i64 0)`
compiled and every call to it was refused instead — a diagnostic about the call
for a mistake in the signature, naming a value that looks fine to whoever wrote
it. The rule existed and was applied at one of the two places it belongs.

**It is applied at two places rather than one, and they are different
questions.** Where a type is spelled, `ast::ty` refuses it, because that is
where the caret belongs: the mistake is four characters long and the reader is
looking at them. Where an instance is decided, the generic call refuses it,
because that shape is one nobody wrote — `(fn pair (T) ((first T) (second &T))
...)` with `T` bound to `&String` by the first argument asks its second for
`&&String`, and before this the mismatch named that type as though a reader
could go and produce one. Neither site subsumes the other, and the message is
built by one function so that the two cannot come to disagree about a rule they
both apply.

**`SL0200` is the code either way**, which is why this does not move anything
`#23` will freeze. The refusal at the call also suppresses the mismatch it
replaces: reporting both would say the argument has the wrong type and then say
that the type it was measured against is not a type, and the second sentence is
the only one worth reading.

**It was cheap before the sigils and cheaper to fall into after them.**
`(& (& String))` is a shape nobody reaches for; `&&String` is what a hand
writes when it means "a borrow of the thing that was already borrowed", which
is a thought a reader arriving from a language with references has had before.
`D-149` did not create the hole. It made the hole easy to find.

## D-154 — the manager reads the compiler's exit code, and stops naming a module it cannot blame

Status: approved · 2026-08-22

`build` asks `slopic` for one object per module, and each invocation is handed
the whole package and checks the whole program before emitting anything. So a
program that does not compile fails every invocation, and the summary line
named whichever module's object the loop happened to be asking for when it
stopped — the first in the order on a cold build, the first whose cache stamp
went stale afterwards. The same error in the same file reported `core:list` on
one run and `core:builder` on the next, and the module it named had compiled
perfectly well.

**A misleading line is worse than a missing one.** The diagnostic above it was
always correct and always pointed at the real file; the summary contradicted it
by naming something in the standard library, and a reader who trusts the last
line of the output goes looking there for a bug in the file directly above.
`check` never had the problem, because it does not go module by module and says
`check failed` without naming anything.

**The two cases are told apart by the exit code, which already distinguished
them.** `slopic` exits `1` when the program does not compile, having printed
every diagnostic itself, and `2` for anything else — a bad argument, a file it
could not read, a `cc` that would not run. So the manager says `build failed`
for `1`, the way `check` does, and keeps `codegen for module `X` failed` for
every other status, which is the case that message was written for and the only
one where the name carries information. That makes the two codes a contract
between the programs rather than an accident, and both sides now say so where
they are read and written.

Nothing richer was built. A protocol in which the compiler reports which module
it was working on would be a second channel to keep correct, and it would say
nothing the diagnostics do not already say: every one of them carries a file and
a span. The question this decision answers is not "which module failed" but
"does the manager have anything to add", and for a program that does not
compile the answer is no.

## D-155 — a function longer than a conditional branch reaches is refused, and the limit is written down

Status: approved · 2026-08-22

AArch64's `b.cond`, `cbz` and `cbnz` carry a signed 19-bit word displacement, so
one reaches a megabyte of code. Every checked operation branches forward to a
trampoline at the end of its own function and every `while` and `if` branches
within one, so **a single function's code may not reach 1 MiB on this target**.
Nothing a person reads said so, and what they met instead was `SL0700` — an
internal compiler error whose help advises assembling the program by hand,
which fails the same way for the same reason — or, on the assembly path, a
message from `as` about a branch out of range in code they did not write.

**The limit is per function, and it used to be per module.** `D-116` moved the
panic trampolines inside the section that branches to them, and said in passing
that this "closes a cliff that was already there and never reached: the old
distance was to the end of the module's whole text section, which grows with
the library". So the ceiling worth writing down is not the one the tracker
described. A module and a program are unbounded; one function is not, and a
function of a megabyte is not a thing anybody has written.

**The refusal bounds the function rather than any one branch.** The exact worst
case is the distance from a particular branch to a particular label after the
layout is final, and computing it would mean a second pass to say something
more precise about a number nobody is near. A bound that can be stated in one
sentence — a function's code stays under 1 MiB — is worth more than one that is
exact, and it errs toward refusing a program that would have squeezed through,
which is the safe direction for a limit whose other outcome is an object nobody
can link.

**The fix is not this.** Branch islands, or inverting each condition to branch
over an unconditional jump, would remove the ceiling; `D-116` already holds the
second in reserve and says why it was not taken then — it changes the
instruction sequence of every checked operation in the language. Neither races
the freeze, and a limit that is written down and diagnosed is not a limit that
hurts anybody. x86-64 needs none of this: its conditional branches carry 32
bits.

## D-156 — a test is emitted by the module that owns it, and the harness by the entry module

Status: approved · 2026-08-22

A `--test` build asked one flag two questions. `codegen_options` set
`test_harness` for the module that emits the entrypoint and nothing else, and
both backends read it to decide whether to emit **test bodies** as well as
whether to emit the harness. The two conditions could only ever hold together,
so the entry module emitted the tests it owned and every other module emitted
none, while the harness — which is generated from the whole package — counted
and called all of them. What a person met was `undefined reference to
sl_test_5f5f736c6f705f746573745f30`, a mangled symbol from `ld` in a program
the compiler had said nothing about.

**The flag is two flags.** `test_bodies` says this is a `--test` build, so the
module emits the tests it owns; every module carries it. `test_harness` says
this module emits the harness, and stays the entry module's alone. Ownership
needed nothing: `partition_codegen` already marks a test for exactly the module
whose name prefixes it, and a test body's `.rodata` strings and its panic
trampolines follow the body, which is why the string collection and the trap
survey read `test_bodies` too.

**A test lives beside what it tests**, which is why the fallback the tracker
offered — refuse a test outside the entry module — was not taken. `D-135`'s
per-target module is the shape that suffers most: one module name resolving to
a different file per target is the natural home for that target's tests, and
there the functions worked and the tests did not. Refusing them would have made
the one arrangement the language provides for target-specific code the one
arrangement that cannot be tested.

**Nothing links differently.** A test body was already emitted with global
linkage like any other function, and the harness already referenced it by name,
which is why the reference existed before anything defined it. The release path
is unchanged: without `--test` neither flag is set, so no `sl_test_*` function
reaches an ordinary binary.

## D-157 — a deprecated name lives out its major version, and removing it costs the next

Status: approved · 2026-08-24

`deprecated` warned (`D-122`, `D-152`) without saying for how long, which made
`SL0800` a mood rather than a contract: a warning whose consequence has no
date is one a reader can postpone forever, and rightly. The policy now stands
in `docs/language.md`, beside the annotation it governs, and the warning's
help points at it, so the first `SL0800` a person meets says where the
contract is written.

**The lifetime is the major version, because that is what a caret requirement
already protects.** A bare requirement is a caret requirement (`D-036`) and a
caret holds the leftmost non-zero field, so a program that wrote `^1.2`
receives every later minor without asking; a name removed in a minor breaks a
program that never chose to move, which is the exact breakage semantic
versioning exists to fence. So removal is a breaking change and waits for a
major version, nothing is removed that a published release did not deprecate
first, and before 1.0 the minor plays the major's role — the same field the
caret holds — so a name deprecated in `0.x` lives through every `0.x.y`.

**A program that ignores the warning keeps everything and gains nothing.**
Within the major it is entitled to compilation and to unchanged behaviour:
`SL0800` never hardens into an error on its own, because a warning that
escalates on a schedule is a removal performed by the compiler instead of by
the author. Past the major it is entitled to nothing, and a use of a removed
name is an ordinary `SL0200` with no memory of the deprecation — an interface
that kept a tombstone for every name it ever shed would grow forever in order
to describe what it no longer contains.

**The pointer lives in the help rather than in a longer message.** `D-122`
gave every `SL0800` the same first line and kept the annotation's own string
as the note, and both survive: the help is the one slot left, and what it
names is the file rather than a section anchor, because a heading can be
reworded without anybody thinking of a string in the compiler.

## D-158 — an edition is named by a year, and a missing key means the first

Status: approved · 2026-08-24

A package says which edition of the language it is written in: `edition` in
`[package]`, validated against the editions the toolchain has, of which there
is exactly one, `2026`. Nothing branches on it yet, and that is the point —
after the freeze a form that exists cannot change meaning, so the only way a
language change survives 1.0 is a mechanism that lets a later toolchain read
an old program as what it was (`D-111`). The mechanism has to exist before the
first program is published, because a key added afterwards could not tell an
old manifest from one that merely forgot it.

**The identifier is a year.** An edition names when the language meant that,
not what changed or how much: a counted edition reads as a version and drags
version arithmetic behind it — whether `2` is compatible with `3`, what a
range of them would mean — none of which applies to a set whose members are
simply enumerated. A year orders itself, cannot run out, and is what a person
already says when they mean the language as of some point. The value stays a
string the toolchain matches against a list, and nothing anywhere parses it as
a number.

**A missing key means the first edition, and an unknown one is a refusal**
(`SL1055`). The default reads a manifest from before the key existed as what
it meant when it was written, which is why it is the oldest edition rather
than the newest — defaulting to the newest would re-interpret every existing
manifest the day a second edition appears. The refusal is the deliberate
opposite of `D-128`: a key nobody knows is a message from a later toolchain
that this one can skip, while an edition nobody knows says the program means
something this toolchain does not carry, and building it anyway would be a
guess. The message names the editions the toolchain has, because the fix is to
pick one, and points at the value. `slopium new` writes the newest edition, so
a new program starts from what the language is now.

## D-159 — the compiler protocol stays internal at 1.0

Status: approved · 2026-08-24

`D-002` separates the manager from the compiler, and the freeze does not pull
the seam between them into the promise: the command-line protocol `slopium`
speaks to `slopic` stays internal, `COMPILER_PROTOCOL` stays a handshake
number the pair bump freely, and `slopium` is the interface a user of the
toolchain holds. What 1.0 freezes is what leaves a machine — the archive, the
lockfile, the index and the signature statement (`D-039`, `D-045`, `D-052`,
`D-056`, and `docs/packaging.md` for the promise entire). A protocol spoken
only between two binaries installed together has no compatibility to keep,
and the handshake already turns a mismatched pair into a message rather than
an unknown flag.

Freezing it would cost exactly what the separation bought. The flags `slopic`
takes have moved repeatedly behind a `slopium` whose surface did not, and
each move was invisible because nothing outside this repository is entitled
to notice one; a frozen protocol would turn every such move into a format
change. A tool that wants what the compiler knows goes through `slopium`, or
through `slopic-core` as a library — the door the language server already
uses.

## D-160 — a fix is a named rule, tied to the release that made it necessary

Status: approved · 2026-08-24

`slopium fix` rewrites a program an old toolchain accepted into the current
spelling. It is not a general migration engine: a fix is a named rule, owned
by the release whose change made it necessary, knowing exactly what that
release moved and what the move's mechanical remainder is. The deprecation
policy (`D-157`) promises that a name lives out its major version; what makes
the promise more than a stay of execution is a tool that performs the move a
deprecation announces, which is why the freeze's exit condition asks for a
fixer that has been run on a real program rather than merely designed.

**A fix promises three things.** It is meaning-preserving: it only adds what
makes an old spelling name the thing it always named, and it never chooses
among meanings — a name whose successor answers differently is refused, not
guessed at. It is whole-file-or-nothing: a file the rule cannot mend
completely is left untouched, with `SL1110` naming the file and the reason,
because a half-rewritten program is wrong in a way neither the old toolchain
nor the new one can explain. And it is idempotent by construction: a fix
fires only where the old spelling is still in evidence, so on a mended or
never-broken tree it writes nothing and `fix --check` exits zero — which is
also how the test suite asserts that a second run changes nothing.

**The first rule is the `v0.5.1` move**, because it is the one change in the
history a program cannot survive unedited: `print`, `println`, `read-line`,
`read-i64`, `env`, `args-len` and `arg` stopped being builtins and became
exports of `std:io` and `std:process`, so a pre-move program calls them bare
with no `take` to say where they now live. The fix inserts the missing `take`
declarations — through the same lossless syntax `fmt` reads, laid out as
`fmt` would lay them, at the line where the file's imports sit — and only for
a name the file uses bare where nothing defines, imports, or binds it.
`parse-i64` is the refused remainder: its successor `std:string:to-i64`
returns an `(Option i64)` where the builtin aborted, and what a non-number
now means is the program's decision. The manifest is the program's own
business too — a package that never depended on `std` is told so rather than
edited.
