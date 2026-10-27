# Builder phase probes

These diagnostics accompany issue #153. The original `string-builder` kernels
remain unchanged and are built and checked alongside the probes. No timing
threshold is added to correctness CI.

The [2026-10-26 baseline](2026-10-26.md) records measured phase shares,
profiling/allocation evidence and numerical goals for the next changes.

Run after building the compiler under investigation, with no verification or
other build in flight:

```sh
nix develop --command python3 bench/builder-phases/run.py \
  --slopic "$PWD/target/debug/slopic" --compiler-root "$PWD"
```

The debug/release mode of the Rust compiler executable affects compilation
speed, not the Slopium program's profile: all probes use `--optimize --strip`.
`--compiler-root` must name the source tree that built the executable and its
bundled library. The report records its HEAD, complete tracked patch, compiler
binary hash and library/runtime hash. Keep that tree unchanged during a run.
Generated sources and raw samples are saved beside `report.json` under the
ignored `bench/results/builder-phases/` directory. An explicit `--outdir` must
not exist yet. `--quick` validates one size with three samples; it is a smoke
check, not a performance result.

## Workloads and controls

- Records: 1,000, 50,000 and 250,000 copies of `item <i>\n`, preserving the
  original record format. The 50,000-record output is 538,890 bytes with byte
  sum 36,116,720; the runner computes expectations independently.
- Integers: the same count of successive decimal integers without separators,
  exercising integer formatting and append together.
- The paired runner also measures long integers: the same counts of unsigned
  values descending from `u64::MAX`, each twenty decimal digits. C uses
  unsigned `snprintf`, Rust unsigned `to_string`, and Python independently
  computes every expected byte count and checksum.
- Constant pieces: 5, 64 and 4,096 `a` bytes, repeated to approximately 8 KiB,
  512 KiB and 4 MiB. The exact length is `floor(size / piece_width) * piece_width`.
  Checksum therefore reads fixed identical input in each language. This matrix
  distinguishes short-piece overhead from long-piece behavior.
- Original control: the untouched Slopium, C and Rust 50,000-record kernels,
  checked against the same independent expected length and byte sum.

Each diagnostic measures contiguous intervals within one invocation: buffer
construction/append, owned String finalization, and checksum over every output
byte. It prints two deterministic correctness lines followed by four timing
lines; the runner validates every invocation and puts correctness and timing in
separate JSON fields. The fourth duration is a pair of consecutive clock
readings, giving the timer/return-path floor. Clock failures fail the probe.

C uses geometric `realloc` growth and `snprintf` for decimal digits. Rust uses
`String::push_str` and `to_string`. Both diagnostic controls explicitly copy into
an owned final String and release the builder buffer, matching Slopium's current
linear final copy. This is a diagnostic comparison: the original Rust kernel
keeps its native zero-copy finalization, and the original C kernel formats each
whole record with `snprintf`. Their original controls preserve those differences.
No external dependency or production runtime entry is introduced.

## Measurement limits

All languages use the same checked output and counts, but their safety checks,
growth policies and formatting algorithms differ. Internal intervals include
the clock call and its return path; Slopium's `std:time` also constructs a
`Result`. Report the clock-pair distribution without subtracting it as an exact
correction. Tiny finalization timings are near that floor.

Every measured process is pinned to one permitted CPU. Three warmups precede
fifteen samples, interleaved in a deterministic shuffled order (seed 153).
CPU/OS/tool versions, affinity, flags, full raw samples and medians are saved.
The existing `rssrun.c` fork/exec wrapper measures wall time and whole-process
peak RSS without inheriting Python's peak. An empty C program calibrates
startup/scheduling overhead. Wall time includes process startup, output and
cleanup outside the internal intervals. RSS includes the whole builder and
String lifetime and is not a per-phase allocation measurement.

Do not add independent probe times or subtract unlike workloads as an exact
decomposition of the original. Rank bottlenecks from the intervals measured
within records, supported by controlled piece-size and integer comparisons.
Use available profilers to strengthen attribution; record unavailable tools or
counter permissions explicitly, rather than silently omitting the limitation.
Do not activate a compiler optimization solely because one diagnostic is faster.
Refresh the pinned baseline after prerequisite fixes and bulk-copy work land.

For attribution without hardware counters, Valgrind's Callgrind is available
in the development shell. Keep profiling separate from timed samples:

```sh
target/debug/slopic bench/slopium/string-builder.slp --optimize -o /tmp/builder-profile
valgrind --tool=callgrind --callgrind-out-file=/tmp/builder.callgrind /tmp/builder-profile
callgrind_annotate /tmp/builder.callgrind
valgrind --leak-check=full --error-exitcode=99 /tmp/builder-profile
```

Do not strip this profiling executable: its symbols identify compiler-generated
functions. Callgrind reports instruction references, not native elapsed time;
Memcheck's heap summary reports allocation activity, not a per-phase RSS peak.

No portable C-parity promise is made. After a measured baseline, record explicit
goals for the selected phase, the unchanged original workload, short-piece
regressions and RSS before implementing further throughput changes.

## Paired revision comparisons

`compare.py` compares two frozen source trees whose `target/debug/slopic`
binaries were built from those trees. It builds the complete phase matrix
and untouched original kernels, validates every output, then interleaves the
two Slopium revisions and C/Rust controls within each workload. Two rounds
have separate three-pass warmups and fifteen measured samples, with recorded
ordering seeds 154 and 155. An empty C process records the startup floor.
No timing or RSS threshold is added to correctness CI.

```sh
nix develop --command python3 bench/builder-phases/compare.py \
  --before-root /path/to/baseline --after-root "$PWD" --pin 2 \
  --outdir bench/results/builder-phases/paired-comparison
```

The output directory must not exist. Finish all builds and checks before
sampling, keep both source trees unchanged until completion, and retain the
raw report and generated sources. The report records both compiler hashes,
tracked patches, library/runtime hashes, checked outputs and every sample.
Compare matching internal intervals and whole-process RSS; retain C/Rust and
startup variation when assessing changes close to the noise floor.

For noisy short workloads, select cases with repeatable `--case NAME` and
use `--repeat-document 100`. Diagnostics then repeat inside each process;
every document's result is checked, and the first five internal intervals
are discarded before averaging the remaining intervals. Process samples
remain the statistical unit. The original control is always unchanged and
runs once; repeated-process wall/RSS numbers describe different lifetimes.
The [bulk-copy report](2026-10-27-bulk-copy.md) records both fresh-process and
steady-state results, including uncertainty in small cases.

The [integer-digit report](2026-10-27-integer-digits.md) compares three
allocation-free alternatives and records the selected implementation’s full
paired matrix, exact piece-binary identity, profiling evidence and missed goals.
