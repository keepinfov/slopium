# The Slopium benchmark suite

The same seven kernels written five times — in Slopium, C, Rust, Python and
Java — built by this repository's `bench/run.py`, checked to agree on output,
and measured. The suite answers the question a comparison between languages
is always asked to answer honestly: same algorithm, same input, same output,
measured the same way, with every difference that remains a property of the
language and its toolchain.

Nothing here is a gate: `scripts/verify.sh` does not run it, because wall
time is not reproducible across machines. The tables below are a snapshot of
one machine; rerun the suite to see what your own hardware says.

## Running it

```sh
nix develop .#bench -c python3 bench/run.py
```

The bench shell carries every toolchain from one nixpkgs revision — GCC,
Rust, CPython, OpenJDK headless — which is what makes two runs on one machine
comparable. The harness finds the toolchain it benchmarks in this order: an
explicit `--slopic`, this repository's own `target/release/slopic`, `$SLOPIC`,
then `PATH`. A machine with the toolchain installed as a system package
should still see the local build win; `--quick` shortens a run to three
iterations while a kernel is being changed, and `--runs`, `--languages`,
`--kernels`, `--pin` and `--no-pin` narrow everything else.

Useful invocations:

```sh
python3 bench/run.py --languages slopium,c          # the compiler against cc
python3 bench/run.py --kernels fib,mandelbrot       # two kernels only
python3 bench/run.py --quick                        # while editing kernels
```

## What is measured

For every (kernel, language) pair, one warm-up run and then ten measured
runs. Each run is a fresh process, pinned to one core, timed end to end by a
small C wrapper (`rssrun.c`) that forks, execs, waits, and reports the
child's wall time and its peak RSS from the kernel's own accounting. The
wrapper is the floor under the RSS numbers, and it is deliberately tiny: a
Python harness cannot make this measurement, because every spawn primitive
it has leaves the child inheriting the interpreter's resident pages before
`exec`, and `ru_maxrss` keeps that peak.

- **Runtime** — the median of the measured runs, in milliseconds.
- **Peak RSS** — the median of `ru_maxrss` per run.
- **Artifact size** — the linked binary; for Python, the source it ships.
- **Compile time** — cold, artifact cache cleared first. C and Slopium
  compile one file per kernel; Rust and Java build all seven kernels in one
  invocation, whose total is reported under the table.
- **`slopium build`** — the project path through the manager, cold and warm,
  over `bench/slopium-project/fib`.

Every number, plus the environment metadata (CPU, kernel, tool versions),
lands in `results/<timestamp>.json` and a Markdown table beside it.
`results/` is build output and gitignored.

## The kernels

| kernel | what it exercises | size |
| --- | --- | --- |
| `hello` | process startup and exit: the cost of the runtime itself | — |
| `fib` | naive recursive Fibonacci, `n = 32`: call overhead | ~4.3M calls |
| `mandelbrot` | escape-time iteration, 200×200, 50 iterations: `f64` arithmetic | 40k pixels |
| `matmul` | naive i-j-k matrix multiply: `f64` plus memory access patterns | 160³ multiply-adds |
| `binary-trees` | build, walk and drop trees of depth 13, 14, 15: the allocator and the drop path | ~114k nodes |
| `hash-map` | 10,000 inserts then 10,000 lookups of xorshift-generated keys | 10k keys |
| `string-builder` | 50,000 records of `item <n>` appended to one growing buffer | ~540 KB |

The map keys come from a three-shift xorshift64 over a fixed seed, so every
language walks an identical key sequence. `binary-trees` leaves carry their
path tag, so the checksum is a function of the shape and not a node count.
Matrices are filled from formulas over the loop counters, because Slopium
has no integer-to-float conversion; the counters themselves are `f64` stepped
by `1.0` in every language, which keeps every value exact and every product
bit-identical.

## The fairness rules

- **One algorithm, one spelling.** The three loops of `matmul` run in the
  same order everywhere; the association of every floating-point expression
  is the same everywhere, because a last-ulp difference is a real
  difference. All five implementations of a kernel were checked to agree
  byte-for-byte before anything is measured — `mandelbrot` included, which
  means the float arithmetic agrees to the bit across five compilers. The
  one exception is `matmul`'s checksum, which each language prints with its
  own formatter and the harness compares with a 1e-9 relative tolerance.
- **Compiler flags are what the language normally ships.** `cc -O2`;
  `cargo build --release`; `slopic --emit exe --optimize --strip`, which is
  the release profile the manager drives; `javac` and `java` with default
  flags; CPython as it runs.
- **Every cost is kept.** Java's numbers include JVM startup; Slopium's
  arithmetic traps on overflow and its bounds checks never fold away; the
  `std:map` and `std:builder` entries measure the library written in
  Slopium, not a C library behind a façade. Where a language has an idiomatic
  fast path that is not the same data structure — CPython joins a list of
  parts rather than growing one buffer — it takes that path, and this file
  says so.

## Snapshot

One run, ten measured iterations per cell, on:

```text
2026-09-03, host "carbon" — Intel Core Ultra 7 265H (16 cores),
Linux 7.1.4, runs pinned to one core
slopic/slopium 0.17.0, gcc 15.3.0, rustc 1.97.0, CPython 3.14.6, OpenJDK 21.0.12
```

Runtime (median, ms):

| kernel | slopium | c | rust | python | java |
| --- | ---: | ---: | ---: | ---: | ---: |
| hello | 2.4 | 2.5 | 2.8 | 12.9 | 28.9 |
| fib | 12.1 | 3.8 | 5.7 | 145.3 | 43.7 |
| mandelbrot | 12.6 | 3.6 | 4.8 | 222.2 | 44.4 |
| matmul | 15.3 | 3.3 | 6.5 | 517.6 | 51.9 |
| binary-trees | 4.2 | 4.7 | 4.6 | 29.0 | 34.3 |
| hash-map | 87.2 | 10.5 | 2.8 | 20.4 | 49.9 |
| string-builder | 15.3 | 3.8 | 4.1 | 27.0 | 56.5 |

Peak RSS (median, MB):

| kernel | slopium | c | rust | python | java |
| --- | ---: | ---: | ---: | ---: | ---: |
| hello | 2.1 | 2.1 | 2.2 | 12.1 | 39.3 |
| fib | 2.1 | 2.1 | 2.2 | 12.3 | 40.4 |
| mandelbrot | 2.1 | 2.1 | 2.2 | 12.3 | 41.6 |
| matmul | 2.3 | 2.2 | 2.6 | 15.4 | 42.8 |
| binary-trees | 3.4 | 3.7 | 4.2 | 14.8 | 43.7 |
| hash-map | 3.3 | 17.5 | 2.6 | 13.0 | 42.1 |
| string-builder | 9.5 | 2.3 | 2.7 | 16.4 | 43.5 |

Artifact size (KB) and cold compile time (s):

| kernel | slopium | c | rust | python | java |
| --- | ---: | ---: | ---: | ---: | ---: |
| hello | 18.0 / 0.47 | 15.5 / 0.18 | 518.3 / — | 0.1 / — | 0.4 / — |
| fib | 18.0 / 0.45 | 15.5 / 0.20 | 518.5 / — | 0.2 / — | 1.0 / — |
| mandelbrot | 18.0 / 0.47 | 15.6 / 0.17 | 518.7 / — | 0.9 / — | 1.0 / — |
| matmul | 26.0 / 0.47 | 15.6 / 0.19 | 550.3 / — | 1.1 / — | 1.4 / — |
| binary-trees | 18.0 / 0.47 | 15.7 / 0.20 | 519.0 / — | 0.9 / — | 1.0 / — |
| hash-map | 22.0 / 0.47 | 15.6 / 0.20 | 533.5 / — | 0.9 / — | 1.1 / — |
| string-builder | 22.0 / 0.45 | 15.8 / 0.20 | 519.0 / — | 0.4 / — | 1.1 / — |

Rust builds all seven kernels in one `cargo build --release` (1.56 s here),
Java in one `javac` (0.50 s). The project path through the manager,
`slopium build --release` over `bench/slopium-project/fib`, is 1.09 s cold
and 0.01 s warm.

## What the numbers say

Read them as one machine's word, not a verdict — but several findings are
structural rather than incidental:

- **Startup and footprint are the strong story.** A Slopium program is
  speaking in ~2 ms and ~2 MB, with an 18 KB binary next to Rust's 518 KB of
  static standard library and Java's 40 MB of heap. A JVM pays for its
  runtime on every one of these kernels before its first line of user code
  runs.
- **The numeric kernels run 2–4× behind `cc -O2` and roughly at Rust `-O3`'s
  doorstep.** `fib` and `mandelbrot` measure call overhead and scalar
  `f64` throughput; the gap is the price of the checked arithmetic the
  language promises, bounds checks that never fold away, and a pipeline
  with no vectorizer. The element read of a list goes through a call, which
  is most of `matmul`'s distance from C — and that kernel reads through
  `get-ref` and `clone` rather than `get`, because `get` on a `(List f64)`
  currently misreads the stored word as an integer bit pattern.
- **Allocation is competitive today.** `binary-trees` — build, walk and drop
  114,000 nodes — lands level with `malloc` and Box's drop glue, which is
  the allocator doing its job rather than the optimizer's.
- **`hash-map` is measuring the library, not the language.** `std:map`'s
  rehash drains its bucket array through front `remove`s, each one a linear
  shift, so growing the table is quadratic in its own size. The kernel is
  sized at 10,000 keys so the comparison stays readable; at 100,000 the same
  kernel runs for tens of seconds. Rust's `HashMap` and CPython's `dict` are
  the speed of a tuned hash table, because that is what they are.
- **`string-builder` shows what a word-per-byte buffer costs.** `std:builder`
  grows a `(List u8)`, and every integer is one machine word wide whatever
  its type says — so the 540 KB document holds 4.3 MB of buffer, visible in
  the RSS column, and the byte-at-a-time writes are calls.

## Layout

```text
bench/
├── run.py              the harness: build, verify, measure, report
├── rssrun.c            the measuring wrapper (wall time, peak RSS)
├── slopium/            one .slp file per kernel, run through slopic
├── c/                  one .c per kernel, cc -O2
├── rust/               one bin per kernel, cargo build --release
├── python/             one script per kernel, CPython
├── java/               one class per kernel, javac + java
├── slopium-project/    a fib project for the manager's cold/warm numbers
└── results/            output of a run; gitignored
```

## Limits

Wall time on one laptop is evidence, not law: CPU frequency scaling,
thermal state, and whatever else the machine is doing all move these
numbers. The suite pins to one core and takes medians, but the honest
comparison is between rows of one table, on one machine, from one run —
never between this file and a number someone else measured tomorrow. The
kernel set is deliberately small and does not cover I/O, concurrency
(no language here has Slopium threads to compare against), or programs large
enough for whole-program optimization to matter.
