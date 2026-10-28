# Compact builder storage proposal

Status: approved by `D-167` on 2026-10-28; implementation remains separate.
Issue #157. The decision covers local packing, public-field migration and the
initial per-word push scope; general narrow collection storage stays deferred.

The comparison considers three alternatives: eight bytes per existing u64 list
word, a dedicated owning byte buffer, and general narrow-element packing. The
first has an executable Slopium prototype. The latter two have native C cost
models only; neither is implemented in Slopium by this research.

## Ownership and ABI matrix

| Dimension | Packed words in the builder | Dedicated owning bytes | General narrow lists |
| --- | --- | --- | --- |
| Owner/drop | Existing affine struct owns `List u64`; ordinary structural drop frees the list; scalar tail owns nothing | C model frees its allocation explicitly; production needs a specified affine owner and compiler-known drop | Existing list owns its payload; narrow primitive elements need no drop, aggregate element callbacks must keep working |
| Clone | Existing structural deep list clone plus scalar copies; mutations after clone are independent | C model deep-copies; production clone glue and allocation failure behavior are not implemented | Clone must copy exactly the stored width, retain element callbacks, and normalize signed/narrow values |
| Borrows/invalidation | Ordinary list/struct loans; growth may invalidate data addresses, so C sees a borrow only after pre-sizing; no pointer retained | Production owner must invalidate aliases on growth, prohibit retention and support exclusive mutable byte borrows | Existing loans remain necessary; all slice/element address calculations must use the new stride |
| Growth | Checked logical length, geometric word-list reserve; one partial word is kept outside the list | Checked byte capacity with geometric reserve; the C model allocates/copies/frees without `realloc` | Checked `capacity * element_width`; generic per-byte push remains unless a separate bulk operation is added |
| Alignment/order | Allocator-aligned u64 words; explicit low-byte-first packing and decoding; no native-endian reinterpretation | Byte alignment; raw byte order | Allocation alignment must satisfy every element type; loads/stores and signed extension must match each width |
| C pointer/count | Existing mutable `List u64` gives word pointer/count; separate tail, logical used count and source byte length; C cannot resize | Needs an agreed byte pointer/count and owner/transfer protocol; a raw `Ptr` is Copy and supplies no automatic resource drop | Current word-element C contracts become incompatible for narrow `T`; list and slice descriptors/entry points must agree on stride |
| Finalization | Research helper allocates a normal owned runtime String and decodes the full words plus tail | Model copies to a final owned allocation; a zero-copy production transfer is unimplemented | Model copies; any ownership transfer into String needs a separate move/ABI design |
| Ordinary collections | No layout change | No layout change if a distinct owner is introduced | Lists, arrays, slices, generic accesses and ABI consumers change |
| Backends/compiler | Uses existing word loads/stores, list/aggregate glue and C calls on both targets | New ownership/type/FFI support is required; existing `&mut String` is refused | Both backends and MIR/lowering must support narrow loads/stores and checked addresses; current get dereferences a word |
| Public Builder fields | Changing `bytes: List u8` into words/tail/length is a breaking field/constructor/pattern change | Changing the exported field/layout is also breaking | Could retain the field spelling, but changes the general collection ABI and physical element addresses |

The C models deliberately share a descriptor and use primitive elements only.
They do not prove compiler-managed ownership or general generic correctness. The
checker validates executable models; proposed compiler work remains work.

## Compatibility findings

Current external code can construct `Builder :bytes (list ...)`, borrow-match
its field, consume-match the field and clone it. The earlier isolated probe
compiled and ran in both native profiles. The representation is public.

The existing extern checker rejects `&mut String` with `SL0200`, explaining that
C cannot maintain its length/capacity. A String-backed mutable buffer is not a
library-only substitute under that contract. No read-only pointer is used to
bypass the restriction. Returning a raw pointer also does not create an affine
owner with drop/clone behavior.

The public compatibility change has separate approval in `D-167`. Keeping
function names does not preserve direct construction or destructuring. Existing
list/slice C APIs must not be silently repurposed as packed byte APIs.

## Decision and migration plan

The approved design is builder-local word packing (`D-167`). The
[checked comparison](2026-10-28.md) measures original-kernel RSS of 2588 KiB
against the 5120 KiB goal and a maximum 4 MiB-piece RSS of 9904 KiB against
16384 KiB. Original wall time improves 51.0% in both independent rounds and
every diagnostic append row improves. Indexed reads are slower, and short
integer/8 KiB-piece finalization has repeatable regressions. Those costs are
part of the proposal, not waived correctness or production validation gates.

This is the smallest executed Slopium alternative meeting the storage goals. The
dedicated-owner model is faster on bulk-piece copying/read/finalization, but
needs ownership/type/FFI support not implemented here. General narrow lists
retain per-element push cost and change collection loads/stores and ABI. Reject
both as prerequisites for the initial builder improvement; general collection
workloads can justify a separate design later. Native C model numbers are not
predictions of unimplemented Slopium compiler features.

The separate design and compatibility approval is now recorded in `D-167`,
superseding the builder-local deferral in `D-165`. Production #158–#160 follows
after this decision lands; this research changes no installed representation.

The packed-word design keeps the functions `new`, `write-byte`,
`write-str`, `write-u64`, `write-i64`, `size` and consuming `build`. Client code
that currently constructs the `bytes` field migrates to `new` and writes the
input bytes. A client that consumes/destructures the field migrates to `build`
and String byte access, or explicitly materializes a regular byte list if that
is required. A client borrowing the raw list cannot receive the same zero-copy
byte-list view of packed words; its migration cost must be explicit. No new
compatibility API is promised without approval.

Production work would stay split: checked growth/ownership (#158), append and
integer integration (#159), then owned finalization and library integration
(#160). Each change needs its own full gate and companions, serialized standard
library/consumer checksum regeneration, both targets/profiles and memory tools.
The public field break requires a pre-agreed breaking-change commit/fragment;
release versions still move only in a later release PR. The final comparison
(#161) must retain the original kernels and disclose remaining gaps.

## Bounded designs for the alternatives not implemented in Slopium

The dedicated-owner candidate would use a non-Copy owner of a header containing
byte length, capacity and data pointer. Drop releases payload/header through the
core hooks; clone allocates independent storage and copies only initialized
bytes. Reserve checks length addition and capacity multiplication before
geometric growth. Read returns a copied byte; no ordinary word-sized Slice is
manufactured over its byte storage. A whole-buffer loan forbids growth while
live, and an FFI payload pointer is valid only for that call. Internal resize
operations would need owner-header access under an exclusive loan; arbitrary FFI
borrowing would expose byte pointer/count without ownership transfer.
Finalization in this comparison copies to an owned String and consumes the
buffer. Zero-copy transfer is a separate, unmeasured optimization.

These semantics require compiler-known ownership/drop/clone and an agreed type
and FFI vocabulary. A Copy raw pointer cannot supply them, an ordinary scalar
field does not free a foreign allocation, and today's mutable-String extern is
explicitly refused. No new public type or C parameter rule is approved here. The
model verifies manual C ownership only. It does not establish that any proposed
owner is a small library change.

The general-packing candidate makes storage width a type property: narrow
integer/bool elements use their width, while pointer/owned aggregate elements
remain word-sized with their existing helpers. List/array/slice operations and
borrowed scalar loads/stores must all honor that width, including zero/sign
extension into register values and preserving canonical values in local slots.
Element callbacks must still see valid owned values, and growth must check
capacity times the stored width. Read-only/mutable C pointer-count pairs change
their element spelling and stride for narrow T; source-compatible field names do
not preserve those ABI contracts. The C model instantiates primitive u8 width
only and retains null callback slots; it proves none of the signed, aggregate,
generic, borrow-codegen or two-backend cases needed for production.

Current lowering finishes get with Step::Load, emitted as Qword on x86-64 and
Double on AArch64. Packing only the runtime payload while leaving those loads
unchanged is therefore not an implementation. This broader work also reaches
raw-byte String helpers, slices, generic collection operations and the runtime
ABI reference. It should be justified by general collection workloads, not
silently bundled into a builder improvement.

## Integration and migration limits

The executable packed prototype has a benchmark-local Buffer type. It does not
prove that production core:float integration works with changed Builder fields.
The repository's core:float and std:process use builder functions rather than
raw fields, so retaining the qualified production Builder type and function
signatures is the intended integration path; that path still needs production
fixtures, both targets/profiles and a serialized library checksum regeneration.

The manager hashes struct field names/types in module_interface. A real field
layout change must rebuild dependent modules and update the toolchain library
digest; it must not be deployed as a body-only compatible cache change. The
research changes neither those interfaces nor the compiler protocol.

Preserving today's Builder field layout while making its bytes packed is not a
compatible substitution: the same List u8 field currently exposes one element
per logical byte. A separate opt-in packed module/type could retain the old API,
but that is an explicit additional public API choice and would not make existing
std:builder users compact automatically. Default replacement and public-field
migration have separate breaking-change approval in `D-167`.

## Valid states of the proposed packed owner

For a value created by `new`, `used` is nonnegative, the word-list length is
`floor(used / 8)`, and only the lowest `used % 8` bytes of `tail` are
initialized; all higher bits are zero. Complete words are numeric low-byte-first
values, not a native-endian byte view. The copy to a runtime String includes one
NUL byte in its allocation and `cap`, while `len` excludes it.

The current language keeps struct fields constructible. Naming new fields does
not make an opaque type, and no privacy feature is assumed. The prototype checks
inconsistent word counts, partial-word high bits and length overflow at its C
bridge, but does not validate every forged state at every Slopium entry.
Production #158 must define and check these invariants at exposed operations,
cover malformed public construction and overflow, and measure the cost. The
prototype's timings do not establish that those additional checks are free.
Invalid construction must produce a checked failure rather than an unchecked
memory access or a silently reinterpreted ordinary byte list.

## Reproduce the compatibility constraints

An external client can construct and consume today's public field:

```lisp
(take std:builder Builder write-byte build)
(take std:string equals)
(take std:panic assert)
(fn main () -> i32
  (let mut out (Builder :bytes (list (as u8 65) (as u8 0))))
  (match &out
    ((Builder :bytes bytes)
      (assert (= (len bytes) 2) &"borrowed public field")))
  (let consumed (clone out))
  (let raw (match consumed ((Builder :bytes bytes) bytes)))
  (assert (= (get &raw 0) (as u8 65)) &"moved public field")
  (write-byte &mut out 255)
  (assert (equals &(build out) &"A\0\xff") &"public construction")
  0)
```

The existing extern restriction is observable independently:

```lisp
(extern "probe_copy" (copy-into (storage &mut String) (length i64)) -> unit)
(fn main () -> i32 0)
```

The latter is refused with `SL0200`: C cannot maintain String length/capacity; a
shared String and a fresh returned String are supported. The current
`extern_parameter_is_expressible` in the semantic analyzer permits shared
String/Slice references and mutable List/Array references, rather than an
arbitrary owning aggregate in a C parameter or return position.

An additional prototype loan probe keeps `held = &out`, attempts a write through
`&mut out`, then uses `held`. It is refused with `SL0300`, "cannot mutably
borrow `out` while it is shared-borrowed". This is evidence of the existing
owner-wide loan rule, not a new borrow-checker implementation.

## Approved initial per-word growth scope

The executable prototype still pre-fills an ordinary list with one `push` per
complete word. It removes the old one-call-per-byte pattern by a factor of eight
and meets the measured append/RSS goals, but it does **not** implement
#159's original acceptance item of one capacity decision per whole piece.
The existing mutable-list extern supplies payload pointer and element count; it
does not lend the list header or allow C to resize that allocation.

`D-167` approves the measured word-prefill strategy for the initial compact
builder: one push per completed word, one borrowed pack call per nonempty piece
and no per-byte growth call. This explicitly revises #159's initial acceptance;
one-reserve-per-piece remains deferred. Retaining that stronger target would
require a separately specified compiler reserve operation or owner-header FFI
protocol, with its language/ABI and companion changes. Neither is implemented
or justified by the C models here. No header is recovered from a borrowed
payload pointer and no new builtin is assumed. The measured gains justify the
bounded initial design and its remaining per-word capacity checks.

## Mutable raw-field clients also need migration

The current public field permits external `match &mut out` followed by `replace
bytes ...` and `push bytes ...`. An isolated probe changed the second byte from
NUL to `B`, appended `255`, then built `"AB\xff"`; it passed native/AArch64
dev/release. Field visibility covers mutation as well as construction and
destruction.

The proposed existing writer API only appends. Raw arbitrary-edit clients cannot
migrate by renaming a field: they must finalize and rebuild through writes, or
explicitly materialize/edit ordinary bytes and rebuild. A borrowed reader that
needs a String without consuming the builder can use `build (clone out)`, paying
for both a deep clone and finalization. No new indexed mutation/view adapter is
included by default. Such adapters need a separate API decision rather than a
claim of source compatibility.

## Approved scope and landing boundary

The approval in `D-167` covers default `core:builder:Builder` storage as `words:
(List u64)`, scalar `tail: u64` and logical `used: i64`; migration away from the
public `bytes: (List u8)` constructor/field/pattern; and the explicit
#159 adjustment accepting one push per completed word in the initial design.
It retains the qualified Builder type and writer/size/build functions, ordinary
collection layouts and the existing C borrow vocabulary. It adds two bounded
runtime bridges with the pointer/count semantics above; research `study_*`
symbols are not installed or promised production names.

Existing runtime entry points, including `sl_rt_builder_write_bytes`, must
retain their ABI obligations when the builder stops using them. The production
integration change must use the approved `!` subject and `BREAKING CHANGE:`
trailer, the issue fragment and explicit Russian migration guide. No release
version moves as part of these implementation tasks.

`D-167` records the separate representation, migration and initial growth-scope
approval. Production #158 can follow once the decision lands. This research
still activates no representation change. A future one-reserve-per-piece path
must specify its compiler/ABI extension and establish its benefit separately.

## Source audit anchors

- [Current Builder and its public fields](../../std/core/builder.slp).
- [Existing C borrow checker](../../crates/slopic-core/src/sema.rs),
  `extern_parameter_is_expressible`.
- [Builtin lowering](../../crates/slopic-core/src/lowering.rs), `get` and
  `Step::Load`; [x86-64 emission](../../crates/slopic-core/src/codegen.rs) and
  [AArch64 emission](../../crates/slopic-core/src/aarch64.rs).
- [Manager interface hashing](../../crates/slopium/src/main.rs),
  `module_interface` and struct fields.
- [Runtime C compile flags](../../crates/slopic-core/src/lib.rs),
  `cc_compile_flags`; [runtime ABI](../../docs/runtime-abi.md).
