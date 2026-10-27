# Builder storage study

Issue #157 researches storage before any production representation decision.
`D-165` remains deferred. Nothing here changes `core:builder`, the compiler,
installed runtime symbols, list layouts or release versions.

The runnable Slopium prototype stores complete words in an ordinary `List u64`
and keeps one partial word in its owner. Two research C entry points pack a
borrowed pre-sized word list and produce an owned runtime String. All word
packing/unpacking uses explicit low-byte-first shifts, independent of host
endianness. The compiler's existing list drop/clone and borrow rules apply.

Three native C cost models compare that strategy with a dedicated owning byte
buffer and a generic list whose primitive element width is one byte. **These are
C-only models, not implementations of new Slopium types or narrow lists.** Their
common descriptor retains generic-list callback slots but handles only primitive
elements (null callbacks). General aggregate ownership is not modeled. All
models finalize with a copy; zero-copy transfer is not measured.

## Reproduce

Build the compiler from the production sources under investigation, then run
inside `nix develop`. The compiler root must contain byte-identical `std/` and
`runtime/` files to this checkout. No other builds/checks should overlap timing.

```sh
bash bench/builder-storage/check.sh "$PWD/target/debug/slopic"
python3 bench/builder-storage/run.py \
  --compiler "$PWD/target/debug/slopic" --compiler-root "$PWD" --pin 2 \
  --outdir bench/results/builder-storage/full-study
```

The output directory must not exist. `--quick` is a three-sample smoke check,
not a performance result. Full runs use two independently warmed rounds, three
warmups and fifteen shuffled samples per case/variant (seeds 157/158). The
existing `rssrun.c` wrapper measures child wall/RSS without inheriting Python's
peak. An empty executable records process startup variation.

Diagnostics check length and two independent checksums: indexed reads of the
builder before finalization, then reads of every finalized String byte. Timing
intervals are append, builder read (including size/capacity queries), owned
finalization and final checksum, followed by a clock-pair calibration. No
interval is subtracted from an unlike workload. C and Slopium read loops have
different compiler optimizations; their times are not interchangeable.

The original Slopium kernel is copied unchanged for the baseline. Its packed
counterpart changes only the builder import, adding no read pass or timing
output. The untouched C/Rust original kernels remain controls. Native C storage
models run only the diagnostic workloads with their additional read pass.

Small, original-record and multi-MiB cases exercise short/long pieces, short
integers and full-width unsigned integers. Every process validates its outputs.
Raw reports include source/compiler/binary hashes, commands, affinity, tool
versions, sample order, all samples and RSS. Keep sources and compiler frozen
through each run; generated artifacts are ignored under `bench/results/`.

## Capacity and validation

Native models print their actual reserve plus an eight-byte partial-word cache
for the word model. The runner independently checks it against geometric growth.
For Slopium, capacity is calculated from the frozen runtime's four-element
initial reserve and doubling rule; the prototype never reads a hidden header
through an unsupported FFI. Its word count is `floor(logical_bytes / 8)`.
Capacity figures exclude allocator overhead and the remaining descriptors; RSS
measures the actual whole process. Small RSS values have a process floor.

`check.sh` requires both cross tools and all memory checkers; it never skips. It
runs native/AArch64 dev/release checks, core-only linkage on both targets,
Memcheck and ASan/UBSan, clone/drop/abandonment, every tail offset, embedded
NUL/high bytes/UTF-8, numeric limits and bounds failures. A freestanding helper
object may depend only on `sl_rt_alloc` and `sl_rt_panic`. It installs nothing
and regenerates no fixture. The ordinary full repository gate still applies to
changes submitted with this study.

The [design proposal](proposal.md) records the ownership/compatibility matrix
and the separate approval required before production work.

The [2026-10-28 report](2026-10-28.md) contains the corrected full paired
comparison, identities, all append/read/finalization rows and missed targets.
The prototype still pushes once per completed word, not once per piece; the
proposal requests explicit agreement on that #159 scope adjustment. Production
also needs public-state validation and actual float/library integration.
Slopium's C runtime/bridge use the compiler's flags with no explicit C
optimization level; native C models use `-O2` and are reported separately.
