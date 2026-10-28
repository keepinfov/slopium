# Public compact builder integration — 2026-10-28

Issue #160 installs the representation approved by `D-167`: complete bytes in
ordinary `List u64` words, a scalar partial tail and checked logical length.
The existing qualified `Builder` and public functions remain. Raw `bytes`
constructors/patterns are an approved breaking migration, documented in the
Russian guide. Every earlier runtime ABI entry remains available.

This comparison uses the actual installed public builders and owned String
converters in both compiler revisions. It contains no research finalizer or
temporary private-owner export. The original kernel is unchanged and prints
`538890` and `36116720` in every run. All process statuses and outputs are
checked, including the empty startup control. C and Rust controls are unchanged.

## Results and limits

Original wall medians fall from 16.060/15.032 ms to 8.044/7.983 ms
(49.9%/46.9% faster), and RSS falls from 9756 to 2588 KiB. The original
5120 KiB memory goal and the 4 MiB-document 16384 KiB goal are met; the latter
peaks at 9904 KiB. Every append row improves in both fresh-process rounds.
C's original control is 4.332/4.492 ms and Rust's is 4.024/3.697 ms;
native elapsed-time parity is not achieved.

Small cold finalization intervals regress in several 8 KiB rows. Fresh-process
wall time is mixed for tiny cases: integers-1000 is -5.9%/+6.8%, and 8 KiB
with eight-byte pieces is +1.9%/+8.7%. These observations are retained below.
An independent two-round repeat of 100 documents per process, discarding its
first five intervals, reproduces no append or whole-process regression above
3% in those small scenarios. One-byte append is nearly unchanged when warm
(-1.8%/+2.1%); five-byte and larger pieces remain faster, and warm 8 KiB
finalization improves 85.9–89.2%. That does not erase the cold observations
or establish an exact allocator, page-fault or scheduling cause.

## Reproduction and identities

```sh
python3 bench/builder-phases/compare.py \
  --before-root /path/to/before --after-root "$PWD" \
  --outdir bench/results/builder-storage/integration-final --pin 2 --compact
```

The baseline is #159 source commit `bd9d8a6`, production-identical to its merge
`f7445995b4a48e75be3a70a8fbf1aa40819beb5b`. The candidate was measured before
commit; the raw report records its complete tracked patch, compiler hash and
combined library/runtime hash. Executable sources match the passed strict-gate
snapshot. Only Markdown was added or corrected afterwards.

The matrix has 31 cases: records and short/long integers at 1000/50000/250000
items, 1/5/7/8/9/64/4096-byte pieces at 8 KiB/512 KiB/4 MiB, and the original
control. Piece counts round down. Two separately warmed rounds each use three
warmups and fifteen interleaved samples of before/after/C/Rust: 3720 checked
measured processes, 744 warmups and 30 startup controls. CPU 2 is pinned;
local builds/checks finished before timing, while unrelated host activity was
not controlled. Wall time includes startup/output/cleanup, with no subtraction.
Stage intervals come from the same diagnostic record; they are not added to
unlike independent workloads as an exact decomposition of the original.

Host: Intel(R) Core(TM) Ultra 7 265H; Linux-7.1.4-x86_64-with-glibc2.42.
Tools: gcc (GCC) 15.3.0; rustc 1.97.0 (2d8144b78 2026-07-07) (built from a source tarball); Python 3.14.6.

| Input | SHA-256 |
| --- | --- |
| baseline compiler | `93d1fd8bf841d01b4aa4aba7fbf4a350cd0c73d1a766e6234931985377aa5b5d` |
| candidate compiler | `d1522744263ab6c78aca1fb88810a2df491927d8df1afe0b86b75d6fec4e4286` |
| baseline library/runtime | `4995f5a79927f80e71ed66866db78d150211bb601100e2b5bca15b401ced03f2` |
| candidate library/runtime | `2a2efc2b2ae71864b6d6ca94d596ffbe0958f559711396b87860b93a8be6c5d0` |
| full matrix report.json | `4277f6ae8c06e339062c7d600e8da22895368882cf50f99f5b552df5ef9d6ea4` |
| small repeat report.json | `6a60e2ab1308e6ed4f2e66432bcf5299c85e2e952256b5f371da1136d7b1b7dd` |
| strict gate log | `b3e9cd01b39f736d63fa1faf70fc319411e6c45db4869576490762ddc3179f8a` |
| original before binary | `1df2c764cc29a77cdc0f26926308cfd2e1117e77e7f91b5e51a83e0dec4126c3` |
| original after binary | `06a98d3fe8dfa771f3c3efa1d2beb8a072c8a194f7e55dfd0152f5adfdea8dae` |
| original c binary | `7723b89ef4e653199fe747683650d36f45ff7181701810a6f6f08639e69bc31f` |
| original rust binary | `9384d2419d81e9339cc0ff59c9ef032177958d520269b08a38a7bbaaf43f5005` |

Full raw reports retain build commands, source and binary hashes, every sample,
shuffle order, startup controls and load averages. They are measurement output
under ignored `bench/results`, not committed build artifacts.

## Whole-process wall time (ms)

Both round medians are shown.

| Case | Before 1 / 2 | After 1 / 2 | C 1 / 2 | Rust 1 / 2 |
| --- | ---: | ---: | ---: | ---: |
| records-1000 | 2.775 / 2.644 | 2.367 / 2.467 | 2.595 / 2.428 | 2.589 / 2.823 |
| integers-1000 | 2.438 / 2.333 | 2.293 / 2.492 | 2.286 / 2.267 | 2.697 / 2.515 |
| records-50000 | 14.138 / 16.057 | 7.679 / 8.463 | 4.582 / 5.285 | 3.789 / 4.122 |
| integers-50000 | 7.871 / 8.050 | 6.215 / 6.043 | 4.395 / 4.878 | 3.936 / 4.001 |
| records-250000 | 61.849 / 62.820 | 28.834 / 30.908 | 12.402 / 14.163 | 9.447 / 9.762 |
| integers-250000 | 31.157 / 31.673 | 19.286 / 18.678 | 10.836 / 10.842 | 7.568 / 7.878 |
| long-integers-1000 | 2.705 / 2.907 | 2.545 / 2.575 | 2.415 / 2.413 | 2.619 / 2.690 |
| long-integers-50000 | 21.944 / 23.182 | 14.658 / 14.418 | 5.605 / 5.851 | 4.315 / 4.996 |
| long-integers-250000 | 123.918 / 120.840 | 58.251 / 61.885 | 17.748 / 17.886 | 12.390 / 12.804 |
| pieces-8192-1 | 2.587 / 2.581 | 2.545 / 2.412 | 2.553 / 2.133 | 2.631 / 2.628 |
| pieces-8192-5 | 2.467 / 2.587 | 2.275 / 2.415 | 2.261 / 2.363 | 2.259 / 2.510 |
| pieces-8192-7 | 2.687 / 2.644 | 2.491 / 2.587 | 2.390 / 2.271 | 2.888 / 2.677 |
| pieces-8192-8 | 2.381 / 2.391 | 2.426 / 2.599 | 2.467 / 2.335 | 2.689 / 2.490 |
| pieces-8192-9 | 2.667 / 2.684 | 2.475 / 2.347 | 2.462 / 2.398 | 2.608 / 2.823 |
| pieces-8192-64 | 2.427 / 2.489 | 2.346 / 2.415 | 2.410 / 2.343 | 2.548 / 2.509 |
| pieces-8192-4096 | 2.600 / 2.444 | 2.427 / 2.276 | 2.328 / 2.217 | 2.506 / 2.542 |
| pieces-524288-1 | 12.957 / 11.562 | 8.594 / 8.412 | 3.301 / 3.197 | 3.859 / 3.539 |
| pieces-524288-5 | 11.886 / 12.958 | 5.970 / 6.056 | 2.837 / 3.374 | 3.211 / 3.514 |
| pieces-524288-7 | 11.056 / 11.662 | 5.305 / 5.767 | 3.094 / 3.138 | 3.243 / 3.254 |
| pieces-524288-8 | 12.552 / 12.533 | 5.514 / 5.578 | 3.200 / 2.987 | 3.368 / 3.396 |
| pieces-524288-9 | 11.517 / 11.516 | 5.600 / 5.647 | 3.211 / 3.104 | 3.270 / 3.365 |
| pieces-524288-64 | 10.368 / 10.093 | 5.124 / 5.153 | 3.128 / 2.868 | 3.430 / 3.309 |
| pieces-524288-4096 | 11.386 / 10.905 | 5.386 / 5.105 | 3.368 / 3.107 | 3.406 / 3.423 |
| pieces-4194304-1 | 81.259 / 77.315 | 44.983 / 46.781 | 8.611 / 8.261 | 9.433 / 9.389 |
| pieces-4194304-5 | 79.248 / 76.576 | 25.976 / 26.247 | 7.814 / 8.013 | 9.349 / 8.436 |
| pieces-4194304-7 | 74.003 / 74.024 | 22.193 / 24.109 | 8.145 / 8.105 | 8.382 / 7.974 |
| pieces-4194304-8 | 75.445 / 73.647 | 22.966 / 24.249 | 7.520 / 7.579 | 7.376 / 7.681 |
| pieces-4194304-9 | 69.689 / 67.985 | 22.573 / 24.076 | 7.787 / 7.764 | 8.524 / 8.083 |
| pieces-4194304-64 | 61.542 / 61.560 | 21.200 / 21.153 | 7.749 / 7.438 | 8.007 / 7.775 |
| pieces-4194304-4096 | 64.949 / 61.627 | 20.961 / 20.205 | 7.684 / 7.255 | 8.138 / 7.926 |
| original-control | 16.060 / 15.032 | 8.044 / 7.983 | 4.332 / 4.492 | 4.024 / 3.697 |

## Whole-process peak RSS (KiB)

Both round medians are shown.

| Case | Before 1 / 2 | After 1 / 2 | C 1 / 2 | Rust 1 / 2 |
| --- | ---: | ---: | ---: | ---: |
| records-1000 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| integers-1000 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| records-50000 | 9896 / 9896 | 2732 / 2732 | 2856 / 2856 | 3416 / 3416 |
| integers-50000 | 3752 / 3752 | 2088 / 2088 | 2216 / 2216 | 2776 / 2776 |
| records-250000 | 34472 / 34472 | 7340 / 7340 | 7464 / 7464 | 8024 / 8024 |
| integers-250000 | 18088 / 18088 | 4392 / 4392 | 4520 / 4520 | 5080 / 5080 |
| long-integers-1000 | 1964 / 1964 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| long-integers-50000 | 10280 / 10280 | 3624 / 3624 | 3752 / 3752 | 4312 / 4312 |
| long-integers-250000 | 67240 / 67240 | 11432 / 11432 | 11560 / 11560 | 12120 / 12120 |
| pieces-8192-1 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-5 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-7 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-8 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-9 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-64 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-8192-4096 | 1928 / 1928 | 1928 / 1928 | 1928 / 1928 | 2264 / 2264 |
| pieces-524288-1 | 6184 / 6184 | 2728 / 2728 | 2724 / 2724 | 3416 / 3416 |
| pieces-524288-5 | 6056 / 6056 | 2728 / 2728 | 2724 / 2724 | 3416 / 3416 |
| pieces-524288-7 | 6056 / 6056 | 2728 / 2728 | 2724 / 2724 | 3416 / 3416 |
| pieces-524288-8 | 6184 / 6184 | 2728 / 2728 | 2724 / 2724 | 3416 / 3416 |
| pieces-524288-9 | 6056 / 6056 | 2728 / 2728 | 2724 / 2724 | 3288 / 3288 |
| pieces-524288-64 | 6184 / 6184 | 2728 / 2728 | 2724 / 2724 | 3416 / 3416 |
| pieces-524288-4096 | 6192 / 6192 | 2736 / 2736 | 2728 / 2728 | 3416 / 3416 |
| pieces-4194304-1 | 38440 / 38440 | 9896 / 9896 | 9892 / 9892 | 10584 / 10584 |
| pieces-4194304-5 | 38312 / 38312 | 9896 / 9896 | 9892 / 9892 | 10584 / 10584 |
| pieces-4194304-7 | 38312 / 38312 | 9896 / 9896 | 9892 / 9892 | 10584 / 10584 |
| pieces-4194304-8 | 38440 / 38440 | 9896 / 9896 | 9892 / 9892 | 10584 / 10584 |
| pieces-4194304-9 | 38312 / 38312 | 9896 / 9896 | 9892 / 9892 | 10456 / 10456 |
| pieces-4194304-64 | 38440 / 38440 | 9896 / 9896 | 9892 / 9892 | 10584 / 10584 |
| pieces-4194304-4096 | 38448 / 38448 | 9904 / 9904 | 9896 / 9896 | 10584 / 10584 |
| original-control | 9756 / 9756 | 2588 / 2588 | 2340 / 2340 | 2768 / 2768 |

## Append interval (ms)

Both round medians are shown.

| Case | Before 1 / 2 | After 1 / 2 | C 1 / 2 | Rust 1 / 2 |
| --- | ---: | ---: | ---: | ---: |
| records-1000 | 0.2287 / 0.2174 | 0.0929 / 0.1283 | 0.0686 / 0.0880 | 0.0351 / 0.0338 |
| integers-1000 | 0.0682 / 0.0615 | 0.0386 / 0.0380 | 0.0625 / 0.0478 | 0.0298 / 0.0251 |
| records-50000 | 10.3174 / 11.5377 | 4.1349 / 4.5008 | 1.8154 / 2.1130 | 0.9476 / 1.0969 |
| integers-50000 | 4.7111 / 4.9965 | 2.7526 / 2.7990 | 1.8546 / 2.0453 | 1.0698 / 1.2020 |
| records-250000 | 49.5140 / 49.9516 | 19.3643 / 21.3925 | 7.8009 / 9.4836 | 4.4741 / 4.7539 |
| integers-250000 | 23.7048 / 24.5923 | 13.1273 / 12.8136 | 7.2366 / 7.2165 | 4.0794 / 4.2262 |
| long-integers-1000 | 0.4359 / 0.5188 | 0.2486 / 0.2456 | 0.0582 / 0.0751 | 0.0323 / 0.0320 |
| long-integers-50000 | 16.3133 / 17.4919 | 9.6084 / 9.2618 | 2.3516 / 2.5975 | 1.1521 / 1.3767 |
| long-integers-250000 | 102.0653 / 100.6715 | 42.7137 / 47.4826 | 10.6818 / 11.7231 | 5.3250 / 6.2742 |
| pieces-8192-1 | 0.1459 / 0.1189 | 0.0819 / 0.0950 | 0.0099 / 0.0114 | 0.0082 / 0.0102 |
| pieces-8192-5 | 0.1166 / 0.2158 | 0.0301 / 0.0451 | 0.0063 / 0.0105 | 0.0052 / 0.0074 |
| pieces-8192-7 | 0.1544 / 0.1659 | 0.0399 / 0.0345 | 0.0099 / 0.0086 | 0.0051 / 0.0064 |
| pieces-8192-8 | 0.1536 / 0.1225 | 0.0291 / 0.0329 | 0.0097 / 0.0082 | 0.0045 / 0.0038 |
| pieces-8192-9 | 0.1115 / 0.1229 | 0.0323 / 0.0324 | 0.0097 / 0.0078 | 0.0064 / 0.0061 |
| pieces-8192-64 | 0.1048 / 0.1218 | 0.0231 / 0.0253 | 0.0079 / 0.0085 | 0.0041 / 0.0046 |
| pieces-8192-4096 | 0.1085 / 0.1063 | 0.0202 / 0.0194 | 0.0079 / 0.0072 | 0.0036 / 0.0034 |
| pieces-524288-1 | 8.8216 / 7.5540 | 5.1625 / 4.6963 | 0.4812 / 0.4624 | 0.6524 / 0.5168 |
| pieces-524288-5 | 8.2778 / 9.2726 | 1.8180 / 2.0164 | 0.3420 / 0.3987 | 0.4023 / 0.4325 |
| pieces-524288-7 | 6.8726 / 7.8912 | 1.4820 / 1.6508 | 0.3329 / 0.3425 | 0.3282 / 0.3431 |
| pieces-524288-8 | 8.9199 / 8.6151 | 1.5432 / 1.5624 | 0.3320 / 0.3224 | 0.3017 / 0.3453 |
| pieces-524288-9 | 7.4486 / 7.4892 | 1.5316 / 1.6618 | 0.3396 / 0.3405 | 0.3492 / 0.3439 |
| pieces-524288-64 | 6.1755 / 6.2938 | 1.2729 / 1.1928 | 0.3025 / 0.3134 | 0.3325 / 0.3239 |
| pieces-524288-4096 | 6.7820 / 6.9425 | 1.1785 / 1.0781 | 0.3049 / 0.2861 | 0.3529 / 0.3517 |
| pieces-4194304-1 | 62.7640 / 59.2123 | 32.3801 / 33.7543 | 2.9081 / 2.8917 | 3.2476 / 3.5368 |
| pieces-4194304-5 | 59.3626 / 59.5377 | 13.5680 / 13.2865 | 2.2756 / 2.3453 | 2.7005 / 2.2288 |
| pieces-4194304-7 | 55.5016 / 56.9675 | 10.3246 / 11.3019 | 2.3938 / 2.1833 | 2.1258 / 2.1280 |
| pieces-4194304-8 | 57.7249 / 55.2577 | 10.6192 / 10.6340 | 1.8504 / 1.9180 | 1.5999 / 1.7221 |
| pieces-4194304-9 | 50.2910 / 50.6287 | 10.1527 / 10.7714 | 2.0332 / 2.1082 | 2.0981 / 1.9977 |
| pieces-4194304-64 | 43.8993 / 44.0468 | 8.2571 / 8.2292 | 1.7938 / 1.8890 | 1.9223 / 1.9082 |
| pieces-4194304-4096 | 46.8874 / 43.8522 | 9.0952 / 7.4642 | 2.0494 / 1.7196 | 2.1607 / 1.9860 |

## Owned finalization interval (ms)

Both round medians are shown.

| Case | Before 1 / 2 | After 1 / 2 | C 1 / 2 | Rust 1 / 2 |
| --- | ---: | ---: | ---: | ---: |
| records-1000 | 0.0122 / 0.0141 | 0.0009 / 0.0012 | 0.0050 / 0.0061 | 0.0081 / 0.0090 |
| integers-1000 | 0.0019 / 0.0021 | 0.0011 / 0.0010 | 0.0007 / 0.0005 | 0.0031 / 0.0026 |
| records-50000 | 0.7978 / 0.6517 | 0.1947 / 0.1944 | 0.3216 / 0.3362 | 0.2760 / 0.3011 |
| integers-50000 | 0.2649 / 0.2140 | 0.1393 / 0.1334 | 0.1738 / 0.1569 | 0.1681 / 0.1810 |
| records-250000 | 5.1050 / 4.5627 | 1.3427 / 1.2572 | 1.7246 / 1.6942 | 1.7026 / 1.5832 |
| integers-250000 | 2.2736 / 1.9591 | 0.6117 / 0.5646 | 0.9090 / 0.8384 | 0.8261 / 0.7621 |
| long-integers-1000 | 0.0140 / 0.0149 | 0.0015 / 0.0017 | 0.0097 / 0.0109 | 0.0109 / 0.0107 |
| long-integers-50000 | 1.5891 / 1.2703 | 0.5070 / 0.4390 | 0.5341 / 0.5512 | 0.5461 / 0.6110 |
| long-integers-250000 | 9.9346 / 7.7700 | 2.4557 / 2.2878 | 3.3152 / 2.8903 | 3.0987 / 3.0539 |
| pieces-8192-1 | 0.0038 / 0.0032 | 0.0044 / 0.0045 | 0.0033 / 0.0035 | 0.0098 / 0.0101 |
| pieces-8192-5 | 0.0032 / 0.0057 | 0.0035 / 0.0065 | 0.0032 / 0.0045 | 0.0067 / 0.0100 |
| pieces-8192-7 | 0.0039 / 0.0042 | 0.0064 / 0.0050 | 0.0042 / 0.0039 | 0.0113 / 0.0111 |
| pieces-8192-8 | 0.0037 / 0.0035 | 0.0046 / 0.0048 | 0.0049 / 0.0036 | 0.0083 / 0.0067 |
| pieces-8192-9 | 0.0031 / 0.0041 | 0.0055 / 0.0050 | 0.0040 / 0.0036 | 0.0094 / 0.0086 |
| pieces-8192-64 | 0.0038 / 0.0048 | 0.0044 / 0.0048 | 0.0036 / 0.0039 | 0.0099 / 0.0084 |
| pieces-8192-4096 | 0.0062 / 0.0071 | 0.0036 / 0.0035 | 0.0040 / 0.0038 | 0.0081 / 0.0071 |
| pieces-524288-1 | 0.7110 / 0.5609 | 0.2862 / 0.2420 | 0.3446 / 0.3343 | 0.3391 / 0.2769 |
| pieces-524288-5 | 0.6699 / 0.5999 | 0.2719 / 0.2823 | 0.2916 / 0.3730 | 0.3222 / 0.3483 |
| pieces-524288-7 | 0.5985 / 0.5548 | 0.2358 / 0.2581 | 0.2972 / 0.2839 | 0.2916 / 0.3084 |
| pieces-524288-8 | 0.7372 / 0.6016 | 0.2793 / 0.2658 | 0.3436 / 0.3363 | 0.3388 / 0.3459 |
| pieces-524288-9 | 0.7009 / 0.6004 | 0.2790 / 0.3353 | 0.3170 / 0.3121 | 0.3671 / 0.3452 |
| pieces-524288-64 | 0.6797 / 0.6046 | 0.2789 / 0.2646 | 0.3313 / 0.3139 | 0.3502 / 0.3208 |
| pieces-524288-4096 | 0.6815 / 0.5833 | 0.2825 / 0.2496 | 0.3242 / 0.3032 | 0.3184 / 0.3267 |
| pieces-4194304-1 | 7.5714 / 6.7856 | 2.2415 / 1.9399 | 2.5958 / 2.5013 | 2.4575 / 2.3404 |
| pieces-4194304-5 | 8.1153 / 6.7574 | 2.3035 / 1.9163 | 2.6970 / 2.5714 | 2.9846 / 2.5295 |
| pieces-4194304-7 | 7.1797 / 6.8101 | 2.0836 / 1.9531 | 2.6134 / 2.5293 | 2.7384 / 2.4741 |
| pieces-4194304-8 | 7.9602 / 6.6113 | 2.1484 / 1.9545 | 2.4963 / 2.5082 | 2.5338 / 2.3570 |
| pieces-4194304-9 | 7.8880 / 6.6394 | 2.0763 / 1.9669 | 2.7346 / 2.4730 | 2.5866 / 2.4730 |
| pieces-4194304-64 | 7.4108 / 6.8560 | 2.1106 / 1.9177 | 2.5528 / 2.4984 | 2.5797 / 2.4483 |
| pieces-4194304-4096 | 7.5573 / 6.9117 | 2.2221 / 1.9336 | 2.8401 / 2.4807 | 2.7005 / 2.4106 |

## Checked String checksum interval (ms)

Both round medians are shown.

| Case | Before 1 / 2 | After 1 / 2 | C 1 / 2 | Rust 1 / 2 |
| --- | ---: | ---: | ---: | ---: |
| records-1000 | 0.0307 / 0.0268 | 0.0275 / 0.0425 | 0.0019 / 0.0024 | 0.0018 / 0.0018 |
| integers-1000 | 0.0092 / 0.0103 | 0.0110 / 0.0106 | 0.0007 / 0.0004 | 0.0008 / 0.0007 |
| records-50000 | 0.9266 / 1.0397 | 1.0127 / 1.1860 | 0.0567 / 0.0681 | 0.0638 / 0.0715 |
| integers-50000 | 0.5850 / 0.6342 | 0.6152 / 0.6514 | 0.0322 / 0.0340 | 0.0376 / 0.0414 |
| records-250000 | 4.9905 / 5.5350 | 5.0552 / 5.5089 | 0.3335 / 0.3554 | 0.3937 / 0.4123 |
| integers-250000 | 2.5441 / 2.6184 | 2.4763 / 2.5987 | 0.1584 / 0.1613 | 0.1873 / 0.1925 |
| long-integers-1000 | 0.0485 / 0.0522 | 0.0564 / 0.0578 | 0.0022 / 0.0030 | 0.0030 / 0.0032 |
| long-integers-50000 | 1.7757 / 1.9055 | 1.8233 / 1.9366 | 0.1031 / 0.1156 | 0.1271 / 0.1606 |
| long-integers-250000 | 8.6515 / 9.5124 | 8.7678 / 9.6452 | 0.6899 / 0.6261 | 0.7183 / 0.7143 |
| pieces-8192-1 | 0.0255 / 0.0215 | 0.0225 / 0.0245 | 0.0011 / 0.0011 | 0.0013 / 0.0015 |
| pieces-8192-5 | 0.0210 / 0.0375 | 0.0201 / 0.0324 | 0.0009 / 0.0017 | 0.0013 / 0.0020 |
| pieces-8192-7 | 0.0264 / 0.0322 | 0.0324 / 0.0294 | 0.0015 / 0.0014 | 0.0015 / 0.0016 |
| pieces-8192-8 | 0.0247 / 0.0227 | 0.0233 / 0.0286 | 0.0013 / 0.0013 | 0.0017 / 0.0015 |
| pieces-8192-9 | 0.0204 / 0.0275 | 0.0266 / 0.0276 | 0.0016 / 0.0012 | 0.0015 / 0.0016 |
| pieces-8192-64 | 0.0264 / 0.0333 | 0.0241 / 0.0306 | 0.0011 / 0.0015 | 0.0016 / 0.0014 |
| pieces-8192-4096 | 0.0253 / 0.0215 | 0.0246 / 0.0263 | 0.0013 / 0.0016 | 0.0016 / 0.0013 |
| pieces-524288-1 | 1.0281 / 1.0547 | 1.1585 / 1.1398 | 0.0717 / 0.0664 | 0.0874 / 0.0675 |
| pieces-524288-5 | 0.9459 / 1.0576 | 1.1558 / 1.2875 | 0.0579 / 0.0821 | 0.0658 / 0.0877 |
| pieces-524288-7 | 0.9343 / 1.0969 | 1.1290 / 1.3205 | 0.0611 / 0.0631 | 0.0629 / 0.0774 |
| pieces-524288-8 | 1.0644 / 1.0619 | 1.2410 / 1.3255 | 0.0780 / 0.0774 | 0.0813 / 0.0877 |
| pieces-524288-9 | 1.0660 / 1.1536 | 1.2365 / 1.4011 | 0.0593 / 0.0679 | 0.0789 / 0.0820 |
| pieces-524288-64 | 1.0592 / 1.1796 | 1.2229 / 1.2999 | 0.0729 / 0.0704 | 0.0672 / 0.0750 |
| pieces-524288-4096 | 1.1206 / 1.1117 | 1.3982 / 1.2309 | 0.0623 / 0.0731 | 0.0776 / 0.0774 |
| pieces-4194304-1 | 7.2127 / 7.9871 | 7.3330 / 8.1241 | 0.5807 / 0.5156 | 0.5933 / 0.5784 |
| pieces-4194304-5 | 7.7355 / 7.6795 | 7.5558 / 8.0896 | 0.5490 / 0.5693 | 0.6045 / 0.6527 |
| pieces-4194304-7 | 7.3431 / 7.8853 | 7.2183 / 8.1327 | 0.5729 / 0.5461 | 0.6577 / 0.5893 |
| pieces-4194304-8 | 7.5808 / 7.9892 | 7.3481 / 8.2027 | 0.5249 / 0.5709 | 0.6266 / 0.5968 |
| pieces-4194304-9 | 7.2964 / 8.0463 | 7.3589 / 8.0964 | 0.5285 / 0.5459 | 0.7368 / 0.6213 |
| pieces-4194304-64 | 7.1606 / 7.8678 | 7.2961 / 8.0363 | 0.5469 / 0.5631 | 0.6547 / 0.6395 |
| pieces-4194304-4096 | 7.4298 / 7.9560 | 7.4645 / 8.2025 | 0.5775 / 0.5569 | 0.6895 / 0.6023 |

## Small-case investigation

The same comparison runs with `--repeat-document 100`, selecting integers-1000
and all seven 8 KiB piece widths. All 960 measured processes return successfully
and print all 100 correct document groups. The first five intervals are discarded
inside each process; the remaining 95 are averaged before process medians.
The repeat changes startup/cache/allocator context and is a separate diagnostic.

| Case | Append change 1 / 2 | Finalize change 1 / 2 | Wall change 1 / 2 |
| --- | ---: | ---: | ---: |
| integers-1000 | -10.5% / +2.5% | -20.1% / -7.3% | -1.2% / +0.2% |
| pieces-8192-1 | -1.8% / +2.1% | -87.9% / -87.8% | -6.9% / -2.0% |
| pieces-8192-5 | -67.9% / -64.4% | -88.9% / -87.8% | -46.5% / -45.1% |
| pieces-8192-7 | -69.2% / -66.0% | -89.0% / -88.4% | -42.1% / -42.6% |
| pieces-8192-8 | -73.0% / -68.7% | -89.0% / -87.4% | -47.1% / -41.2% |
| pieces-8192-9 | -66.0% / -57.5% | -88.6% / -85.9% | -37.1% / -38.4% |
| pieces-8192-64 | -65.3% / -68.7% | -87.9% / -89.0% | -37.2% / -40.1% |
| pieces-8192-4096 | -71.9% / -65.4% | -89.2% / -87.8% | -41.9% / -36.4% |

## Ownership, ABI and verification

The full frozen `SLOPIUM_STRICT=1 scripts/verify.sh` passed without skips,
including Nix. The established private host namespace supplies fixture shell
paths and drops capabilities; Nix checks an identical tracked/new-source
snapshot with binary substitution disabled. No required check is omitted.

- Fourteen actual-owner units use independent String references and cover
  offsets, all byte values, UTF-8/NUL, mixed growth, integer limits and ordinary
  clone/drop even with forged logical metadata. Fifteen external tests preserve
  the qualified public API, float writer, independent consumed clones and word
  views. Both targets run dev/release; native memory tools pass.
- Every size/write/build operation refuses five malformed owner shapes. Ten
  helper failures per mode cover negative lengths and integer-width/sign
  overflow. Owner, word-view and consuming-build loans retain exactly one
  spanned `SL0300` each. Word growth and C borrowing never overlap.
- Direct finalizer checks cover 4098 lengths with exact-sized heap word inputs,
  every output byte, len/cap/NUL, untouched sources and result reads after input
  release. Both C targets pass `-O0`/`-O2`, native Memcheck and ASan/UBSan, plus
  seven guarded failures. The deliberate `-O0` harness row undefines Fortify
  after wrapper flags; production flags and assertions remain unchanged.
- Core-only linkage retains only the four allocator/failure hooks and exercises
  actual qualified float append, clone/build independence and byte semantics.
  Existing cross/object/editor/kernel/debug gates pass. Counter wrappers retain
  exactly 131072 pushes and 256 pack calls for the 1 MiB-piece scenario.
- A same-compiler warm-cache probe changes Builder-shaped fields while keeping
  the manifest, function signatures and clone consumer source unchanged. The
  consumer object stamp changes, correct output remains, and the next build is
  fresh. Actual bundled source changes regenerate only the consumer core digest.
- Old `:bytes` construction fails with existing `SL0200` in a real package;
  separate code/span/rendering snapshots lock the refusal without changing old
  expectations. Russian migration examples, ABI, architecture, `D-167` and the
  breaking fragment are updated. Earlier C symbols, versions and dependencies
  remain unchanged. This change introduces no new Rust unsafe or compiler opcode.

The comparison sampler previously accepted correct output after a failed
process. A controlled probe demonstrates that exit 1 and signal -11 were
accepted before the guard and are rejected now. Both the full matrix and warm
repeat use the corrected guard and retain binary identities.

## Remaining work and attribution

Memcheck of the unchanged original kernel, built without stripping for symbols,
reports zero errors and zero bytes/blocks at exit in both revisions. Heap usage
falls from 18,821,586 bytes over 100113 allocation calls to 4,141,538 bytes over
100110 calls (78.0% fewer allocated bytes, only three fewer calls at this step).
Against the initial 240111-call baseline, the allocation-call reduction is
58.3%; the earlier 60% goal is still missed. This does not become a claimed
success merely because the separate RSS goals are met.

The final original-kernel Callgrind trace records one list construction,
67361 pushes, 50000 borrowed pack calls, one owned word-to-String conversion,
538890 checked String byte calls and 50030 String constructions. The record
loop alone still materializes its 50000 `"item "` literals as owned Strings;
the trace does not imply that all remaining allocation calls belong to buffer
growth. The profile contains 127028620 instruction references. The checked
String reader has 16.1% inclusive instruction references; numeric emission,
owner checks and allocation also remain. Inclusive/recursive totals overlap
and are not native elapsed-time shares, so they must not be summed into a
new timing decomposition.

The buffer memory goals and original elapsed-time improvement goal are met;
C/Rust elapsed-time parity remains a gap. Indexed String reads still use the
existing checked path, and small cold finalization costs remain visible.
General narrow collections, one reserve per piece, owner-header FFI and zero-copy
String transfer remain outside `D-167`. Final landed-source reconciliation and
goal disposition belong to #161; no release/version operation follows from
these measurements.
