# Builder integer digits — 2026-10-27

Issue #155 replaces the per-number digit list with bounded recursive output.
The original kernel and its checksum remain unchanged. The word-sized builder
buffer and its RSS are unchanged; compact storage remains deferred in `D-165`.

## Algorithm selection

Three allocation-free alternatives were compiled from current main,
`d69ba3682a51adf9048dd481db1de2934697f664`, changing only `write-u64`.
All generated programs use `--optimize --strip`. Before any timing, all
alternatives and the baseline were compiled into separate executables.
Each process was pinned to CPU 2, with three warmups and fifteen interleaved
samples in each of two independently warmed rounds (seeds 155 and 156).
Every invocation checked independently computed length and byte sum.

- Recursive: emit `value / 10` first when `value >= 10`, then append
  `48 + value % 10`. Zero emits one digit and `u64::MAX` needs twenty calls.
- Decimal divisor: discover the highest power of ten by dividing a copy of
  the magnitude, then emit `remaining / divisor`, update the remainder and
  divide the divisor by ten. The discovered divisor never exceeds `10^19`.
- In-buffer reversal: append remainders to the existing builder, recording
  its original length, then swap bytes inward using two scalar temporaries
  and `replace`. No additional list or imported module holds the digits.

Both integer workloads write 50,000 numbers: short values 0 through 49,999,
and long unsigned values descending from `u64::MAX`. Times below are medians
in milliseconds; paired entries are independent rounds. Original control
times include process startup, checksum, output and cleanup.

| Algorithm | Short append | Long append | Original wall |
| --- | ---: | ---: | ---: |
| baseline | 5.175 / 5.086 | 21.007 / 20.808 | 15.655 / 15.520 |
| recursive | 3.213 / 3.198 | 15.319 / 14.695 | 13.639 / 13.753 |
| divisor | 4.155 / 3.888 | 18.234 / 18.231 | 13.934 / 13.645 |
| reverse | 3.867 / 3.645 | 18.814 / 18.410 | 14.394 / 13.928 |

Recursion wins both integer lengths in both rounds and was selected (`D-166`).
This selection is followed by a fresh comparison of the complete final diff
against the unchanged post-bulk baseline, including C and Rust controls.
It does not select a new storage representation.

Selection raw data and per-variant sources/executables are saved locally in
`/tmp/slopium-155-experiments/`, outside tracked source. Raw report SHA-256:
`04592ffa4ccd1ffab533dd370817c07be610c6fb928562d836f1d8544fb85567`.
The paired runner described below reproduces the selected implementation
comparison; the alternatives above state their complete digit algorithms.

## Allocation and profiling evidence

Separate untimed Valgrind 3.27.1 runs of the unchanged original kernel
compare the post-bulk baseline with recursive output. Both returned
538890 bytes and checksum 36116720, with no errors or live blocks.

| Metric | Before | After |
| --- | ---: | ---: |
| Memcheck heap allocation calls | 240111 | 100113 |
| Total allocated bytes (not peak RSS) | 25381506 | 18821586 |
| Callgrind list constructions | 50000 | 1 |
| Callgrind list pushes | 827778 | 538890 |
| Callgrind String byte reads | 788890 | 788890 |
| Callgrind instruction references | 216992897 | 164173201 |

The 49,999 temporary digit-list constructions disappear, leaving the builder
buffer. Digit staging pushes disappear rather than moving to another heap
object. Instruction references are not native wall-time shares.

The predeclared 60% total-allocation goal is **missed**: the measured decrease
is 58.3%. Callgrind still counts 50,000 `sl_rt_string_new` calls directly from
the original loop, each constructing its `"item "` literal with two runtime
allocations. Those 100,000 allocations dominate the remaining 100,113.
Changing literal allocation would be a separate compiler optimization;
neither the control nor that compiler behavior changes in this issue.
The temporary-list removal goal is fully met.

## Final paired matrix

The full corrected strict gate finished before this run. The baseline compiler
is the clean #165 head `3cfdd35b14c925d0919ff06516651fa6adf94cbc`;
its production library/runtime match main at
`d69ba3682a51adf9048dd481db1de2934697f664`.
The candidate is that main plus the tracked #155 patch captured in the raw
report. No source or compiler changed during sampling. Only this report and
its index link were added afterwards.

Reproduce after building both compiler executables:

```sh
nix develop --command python3 bench/builder-phases/compare.py \
  --before-root /path/to/post-bulk --after-root "$PWD" --pin 2 \
  --outdir bench/results/builder-phases/integer-digits
```

The machine was `carbon`, Intel(R) Core(TM) Ultra 7 265H,
Linux 7.1.4, native `x86_64-unknown-linux-gnu`. GCC 15.3.0, Rust 1.97.0,
Python 3.14.6, Slopium 0.17.0; programs use `--optimize --strip`, C `-O2`
and Rust `-O`. CPU 2 was allowed within affinity 0–15. Two rounds each use
three independent warmups and fifteen interleaved samples, seeds 154 and 155.
There are 2,280 measured workload processes, 456 warmups and 30 separate
empty-process calibrations. Every result is checked before it is retained.

The raw report and generated sources/executables are in the ignored directory
`bench/results/builder-phases/2026-10-27-integer-digits/`. Raw report SHA-256:
`a12f4c2cb7aaaaab33e32f03c5dc5f1aff7058e5e3dee1491554b7a6b92d3dee`.

| Identity (SHA-256) | Before | After |
| --- | --- | --- |
| compiler binary | `e1128df447d275d46629c1e0bc9c29e42a87a39e551394d261de8c8bc6f6a09d` | `780ab60a8686718c44910833b92d77ea711bb1410ef965622d29077103502b5d` |
| library/runtime | `01895f9fb2cc6b47e349148c02ced3571c9dfe91099ddd207a8f1bcb978d11da` | `dc6da3d4455bf056bf999b5135e5a780fb98eef1718c65f9935c2ce95cf9b4d5` |

Candidate tracked-patch SHA-256:
`c190355fa37d34359cf5869d0dcf1514c7fc3fe8f10fabbde6b92c82867bfc3f`.
Runner SHA-256:
`85642ee0ba6617487144d84dced0fde6cc0a5d5ea4700c8ac122d4b6e793ea6a`.

### Append intervals

Entries are median milliseconds, round 1 / round 2. These are contiguous
append intervals within each diagnostic, not differences between workloads.
C/Rust retain the same independent owned-finalization controls as the baseline.

| Workload | Before | After | After change | C | Rust |
| --- | ---: | ---: | ---: | ---: | ---: |
| records-1000 | 0.216 / 0.207 | 0.176 / 0.169 | -18.6% / -18.4% | 0.055 / 0.045 | 0.027 / 0.021 |
| integers-1000 | 0.087 / 0.075 | 0.068 / 0.055 | -21.2% / -26.1% | 0.038 / 0.041 | 0.019 / 0.021 |
| records-50000 | 11.746 / 12.077 | 9.360 / 9.882 | -20.3% / -18.2% | 1.704 / 1.595 | 0.957 / 0.878 |
| integers-50000 | 5.194 / 5.257 | 3.220 / 3.380 | -38.0% / -35.7% | 1.360 / 1.492 | 0.768 / 0.756 |
| records-250000 | 56.670 / 56.315 | 46.523 / 46.762 | -17.9% / -17.0% | 7.887 / 7.725 | 4.511 / 4.269 |
| integers-250000 | 32.448 / 32.278 | 23.041 / 22.328 | -29.0% / -30.8% | 6.220 / 6.161 | 3.649 / 3.510 |
| long-integers-1000 | 0.534 / 0.505 | 0.387 / 0.451 | -27.5% / -10.6% | 0.061 / 0.062 | 0.028 / 0.032 |
| long-integers-50000 | 20.568 / 20.434 | 14.764 / 14.271 | -28.2% / -30.2% | 2.208 / 2.105 | 1.111 / 1.020 |
| long-integers-250000 | 126.193 / 125.706 | 93.634 / 93.620 | -25.8% / -25.5% | 10.320 / 10.105 | 5.092 / 4.990 |
| pieces-8192-5 | 0.115 / 0.117 | 0.108 / 0.120 | -6.2% / +3.1% | 0.006 / 0.006 | 0.004 / 0.005 |
| pieces-8192-64 | 0.095 / 0.079 | 0.100 / 0.095 | +5.5% / +19.1% | 0.007 / 0.006 | 0.004 / 0.004 |
| pieces-8192-4096 | 0.080 / 0.121 | 0.092 / 0.089 | +16.1% / -26.4% | 0.007 / 0.007 | 0.003 / 0.004 |
| pieces-524288-5 | 6.766 / 7.184 | 6.525 / 6.970 | -3.6% / -3.0% | 0.319 / 0.273 | 0.321 / 0.301 |
| pieces-524288-64 | 4.961 / 4.680 | 4.967 / 5.272 | +0.1% / +12.6% | 0.238 / 0.245 | 0.252 / 0.268 |
| pieces-524288-4096 | 5.106 / 4.813 | 4.849 / 5.049 | -5.0% / +4.9% | 0.254 / 0.221 | 0.291 / 0.259 |
| pieces-4194304-5 | 54.020 / 54.718 | 54.939 / 54.164 | +1.7% / -1.0% | 2.041 / 1.913 | 2.057 / 1.904 |
| pieces-4194304-64 | 41.516 / 42.301 | 41.251 / 41.236 | -0.6% / -2.5% | 1.793 / 1.729 | 1.802 / 1.708 |
| pieces-4194304-4096 | 42.483 / 42.260 | 41.027 / 42.372 | -3.4% / +0.3% | 1.519 / 1.598 | 1.857 / 1.773 |

### Original control and RSS

Every invocation prints 538890 bytes and checksum 36116720. Wall time
includes startup, checksum, output and cleanup. Entries are round 1 / round 2.

| Language | Wall ms | RSS KiB |
| --- | ---: | ---: |
| before | 15.252 / 15.939 | 9756 / 9756 |
| after | 13.231 / 14.197 | 9756 / 9756 |
| c | 4.096 / 4.012 | 2340 / 2340 |
| rust | 3.792 / 3.522 | 2768 / 2768 |

The original control improves **13.3% and 10.9%**, meeting the 10% wall
goal in both independently warmed rounds. The 50,000-short-integer append
interval improves **38.0% and 35.7%**, meeting its 25% goal; twenty-digit
integers improve **28.2% and 30.2%**. Small 1,000-number cases are closer
to timer/startup and cold-process variation; their gains are not generalized
to every count. Integer-only correctness pairs at 50,000 values are
238890 / 12466720 (short) and 1000000 / 52458080 (long).

Median RSS is unchanged in every before/after row: 9756 KiB for the original,
3752 KiB for 50,000 short integers, 10280 KiB for 50,000 long integers,
and up to 38448 KiB in the 4 MiB piece matrix. The 250,000-long-integer
case reaches 67240 KiB in both revisions. This removes scratch allocation
traffic; it does not achieve the deferred compact-storage RSS goals.

### Piece variance investigation

The 8 KiB / 64-byte row appears slower in both rounds, while other small
piece rows and 512 KiB rows fluctuate. All nine optimized piece executables
are **byte-identical before and after**: their constant mode eliminates the
integer path, and the unused recursive formatter is discarded at link time.
The apparent piece differences therefore measure repeated runs of identical
code, not a changed copy path. No time correction or row exclusion is applied.
The following hashes independently identify those identical pairs.

| Piece case | Both executable SHA-256 values |
| --- | --- |
| pieces-4194304-4096 | `a8f85553a6910b994561158716a7ab2703c4cc43d2e018df3c8bede432135760` |
| pieces-4194304-5 | `35a992f59fe6e8061b9f6fa33001e458d42661b0c9d01d989cf2ab827f1f0e4e` |
| pieces-4194304-64 | `67c939ffe0d9193175caf444d30c9cf87dba130942db34063e0ec720a639c18a` |
| pieces-524288-4096 | `417b424597b6b2e6a87c1d0bacc722456918055510aea2f438b6f7b1c9a57a81` |
| pieces-524288-5 | `1e67f2ffd8c2899d100c8dbb22c32f0d7b154a8a560a6f302107c740caa8d8eb` |
| pieces-524288-64 | `d40a78e2dcd294f4cf250c18b9dc12cba3dafd3871a9b328948e70ca0c2c4e67` |
| pieces-8192-4096 | `a0ee34e07302acf9f408fb4c54d842783d99b03a79612d51fd61311e8c217899` |
| pieces-8192-5 | `c92f7b04bd76f84d55a61a2e88c79448873b21d3cf6c89fa7751926b9efb9e83` |
| pieces-8192-64 | `ad4732fc1fc1fa82e11caf7f1ac2ccf933442e58fa472d26956577f0a10362db` |

The empty-process wall median is 2.208 ms, range
1.898–2.763 ms. The candidate 50,000-record clock-pair
median is 29 ns. Neither is subtracted as an exact correction.
Host load averages before/after were 5.16/4.52 on sixteen
logical CPUs. CPU pinning does not isolate the host; timing variability,
particularly in sub-millisecond cold processes, remains a limitation.

## Validation and limits

`SLOPIUM_STRICT=1 scripts/verify.sh` completed from its first stage through
Nix, exit 0, with no skips. This includes workspace tests/clippy, the whole
project corpus, packaging/trust checks, ASan and Valgrind, freestanding
linkage and kernel boot, gdb, both backends, object/assembler parity and
Neovim. The first attempt stopped on canonical std formatting; the changed
library file was corrected, its checksum regenerated, and the entire gate
was restarted successfully. Existing expectations were preserved.

Host adaptations are the established private mount namespace supplying
`/bin/{sh,true,false,echo}`, with capabilities dropped for permission tests,
and an equivalent clean source snapshot for Nix (excluding ignored build
output), built locally with substitution disabled. ASan used the script’s
default leak-detection setting; Valgrind supplies the leak check.

The expanded builder fixture passes all eleven tests in both profiles on
both x86-64 and AArch64. It covers zero, every representable decimal power
and both neighbors, positive/negative signed boundaries, signed/unsigned
extremes, prefix/suffix preservation, repeated growth, bytes and float
integration. The freestanding and sanitizer programs now exercise exact
signed minima/maxima, unsigned maximum and zero. Consumer checksum
regeneration is byte-identical on the gate’s rerun. Release versions and
dependencies are unchanged.

Untimed profiling was repeated on the canonically formatted implementation
after the final timing matrix. Exact commands for a locally built original:

```sh
target/debug/slopic --optimize bench/slopium/string-builder.slp -o original
valgrind --leak-check=full --error-exitcode=99 ./original
valgrind --tool=callgrind ./original
```

Hardware `perf` remains unavailable in the development shell; the available
Valgrind tools provide allocation/call attribution instead. The 60% allocation
goal is missed and RSS remains unchanged, as shown above. Bounded recursion
uses at most twenty formatter frames; no new runtime entry, dependency,
unsafe operation or buffer representation is introduced.
