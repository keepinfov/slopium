# Packed storage foundation — 2026-10-28

Issue #158 implements the private storage/growth foundation selected by `D-167`.
The public `Builder` still has its existing `bytes: (List u8)` field and writers.
Append and public integration follow in #159–#160; no installed speedup is
claimed by this change.

## Correctness and ownership

An unexported `PackedStorage` holds complete `List u64` words, a scalar tail
and logical length. Validation checks nonnegative length, exact complete-word
count and zero unused tail bits. The length planner rejects negative additions
and signed overflow before allocation or mutation. Word count uses division by
eight, avoiding an overflowing round-up addition. Existing List reserve/growth
performs checked capacity multiplication and geometric doubling.

`storage-grow-zero` initializes appended logical bytes while retaining the
prefix and valid owner state. If a partial word completes, its old tail becomes
the first new list word, followed by zero words; any remaining tail stays
initialized. Structural clone/drop and owner-wide loans use existing compiler
rules. There is no new public type, compiler builtin, collection layout or
runtime ABI.

Eight owning-module tests cover empty/zero growth, partial tails, all eight
offsets and twenty addition lengths, 1100 incremental byte extensions, clone
independence, signed-limit planning and malformed metadata. The checker copies
the actual library module into a test-owning package so these private tests
need no exported test API; it verifies that dependency tests are excluded.

The dedicated strict checker passed native/AArch64 dev/release, expected panics
for invalid counts/tails/lengths, native Memcheck and ASan/UBSan, core-only
linkage with the four existing hooks, and exactly one `SL0300` owner-loan
refusal with a span. A new external fixture preserves current public
construction, clone and NUL/high-byte output until the approved later migration.

The final complete `SLOPIUM_STRICT=1 scripts/verify.sh` passed through Nix
without skips. Standard-library consumer checksum regeneration was serialized;
only its expected lockfile digest changed, and the gate validated it again.
The NixOS host uses a private mount namespace for fixture `/bin` programs, drops
capabilities and checks a clean nonignored-source snapshot, as in the earlier
study. Executable sources stayed frozen through this gate and timing; only
this report and its index link were added afterwards.

Review found that `nm` inside a process substitution could hide its exit code.
The first full gate was deliberately interrupted. The checker now runs that
audit as a checked pipeline; injecting an `nm` that exits 86 makes the checker
exit 86. The corrected dedicated checker and complete suite were rerun. An
earlier sandbox-only attempt lacked tools and hit LeakSanitizer’s ptrace
restriction; that attempt is not counted as validation.

Validation log SHA-256:
`d29cffccb1b4a15ddca7b7e249b49ee21233ab49c7369e8333926e0d386f2075`.
Dedicated corrected-check log SHA-256:
`65e0aefb698d198c57a7c6c8408f8b892017e4845eaa517034f7d0fa8e018db9`.

## Public benchmark boundary

The original kernel still returns 538890 bytes and checksum 36116720 on every
invocation. Baseline production sources match landed main `23a1780`; its reused
compiler was built in the clean study worktree at `94cc282`, whose `std/` and
`runtime/` are byte-identical to that merge. Candidate sources are that main
plus the captured #158 diff and per-file identities below.

Both compilers build the same isolated copy of the unchanged kernel with
`--optimize --strip`. Original C (`-O2`) and Rust (`-O`) controls are unchanged.
All processes are pinned to CPU 2 (allowed affinity 0–15), Intel Core Ultra 7
265H, Linux 7.1.4/glibc 2.42, GCC 15.3.0, Rust 1.97.0 and Python 3.14.6.
Two independently warmed rounds use three warmups and fifteen shuffled samples
per variant, seeds 158/159: 120 measured workloads, 24 warmups and 30 empty
process calibrations. There are no overlapping builds/checks or discarded
samples. Existing `bench/rssrun.c` and `Harness.spawn` measure child RSS and
wall time without inheriting Python’s peak.

Entries below are round 1 / round 2 median wall milliseconds and RSS KiB.

| Variant | Wall ms | RSS KiB |
| --- | --- | --- |
| before | 14.277 / 14.501 | 9756 / 9756 |
| after | 14.574 / 14.867 | 9756 / 9756 |
| c | 4.141 / 4.044 | 2340 / 2340 |
| rust | 3.647 / 3.991 | 2768 / 2768 |

The candidate is **2.1% / 2.5% slower** in this control; RSS is unchanged.
The wall differences stay below the earlier study’s 3% investigation threshold.
This is no throughput win. Symbolic function identity does not prove equal wall
time: linked addresses/layout and process variation remain different, and no
exact cache or instruction-share attribution is claimed. The useful compact
owner is not connected to the public builder yet.

Empty-process wall medians are 2.401 / 2.381 ms; no startup median is
subtracted from workload times. System load averages before/after are recorded
in the raw report; external host activity is not controlled.

## Code and data footprint

The symbolic assembly body of original `main` and all seven public builder
functions is exactly identical before and after. Compare each `.type`/`.size`
function block from `--optimize --emit asm`; do not compare relocated instruction
addresses as though they were unchanged symbols.

Full ELF files are **not** byte-identical. Existing emission retains unused
type helpers and the three new diagnostic literals. The linked `.text` grows
by 128 bytes and `.rodata` by 97 bytes. Both ELF files remain 22536 bytes because
the additions occupy alignment space. This compiler emission behavior is not
expanded into an optimizer change here.

| Function | Identical symbolic body SHA-256 |
| --- | --- |
| `main` | `d6cb84e320c9d221d36709f9f8b9d45535f7a831c74634bb76ae74b88685ae71` |
| `core:builder:new` | `0c1e1e41d5f85f613ffe1664670a1c570f75a27831bc3d24ca58a68bed9978e8` |
| `core:builder:size` | `d55da03246a7c6ab542a729152ff5bedc05afe556004ba70aa9dc5d9583c8e13` |
| `core:builder:write-byte` | `431a8a1460a719394fdcc31b654ae7c02525764a74af97a5977e187c48badf00` |
| `core:builder:write-str` | `1a1445d4d668b98237fb1a8195cbb0271ecb93e4f504d5b4a2490a4119f81266` |
| `core:builder:write-i64` | `cd433ad6b67e10e8500eae38fe5bd9a77570a9127ff07324efbc24d2e6ed5daf` |
| `core:builder:write-u64` | `6648171da8d510587c183c5565353b9171c2fb7d7999d68e101a9959d434cb28` |
| `core:builder:build` | `96f1bd0e3e09d2c34dd5122ffa57f45a39f5e33d527297ac51e5d2022cf8f4b7` |

## Identities and reproduction

Raw samples, order, commands and generated executables are preserved locally
under `bench/results/builder-storage/2026-10-28-foundation-control/`; assembly
and section evidence is in the neighboring `foundation-identity/` directory.
These are ignored outputs. The source manifest and raw report identify the
untracked checker as well as the tracked patch used at sampling.

Raw report SHA-256:
`3dce1e9a87591d59b293376f034fa881db2fda78723512d199e889bd0965f255`.

| Artifact | SHA-256 |
| --- | --- |
| before_compiler | `9ce7cdf3961f9f7904bb83e535aab49371b987805133a07423d30f6431a4f2ba` |
| after_compiler | `a84ab46049e0868076284567aed12fd7f2b915917e5bf7bc9a0ecce1c9877d68` |
| `std/core/builder.slp` | `6cc9dee6e4379e0aa25f8446ce19ed627ecbae9332701568b427a1971184776e` |
| `scripts/builder-storage-check.sh` | `c4a93b779e3429c2c2b478b3a533540d26565c99e9e52aa1fcbf4c1a8dfa50ad` |
| `scripts/core-check.sh` | `c5d15514164a633b03d720f9be601339416e42a57310e7c0d58b58d56763dc62` |
| `tests/projects/pass/string-builder/src/main.slp` | `d83f378725435fab4490cebf4a07764ae0eed4f6e569098eced173a0c35b6aaf` |
| before executable | `cb6d64dd267848cb629e5be3bdc17a6e9eb7b9ade17b3844b99e43c204b55b4f` |
| after executable | `e2e8c5a24791b36772635b7098544bf30a0021843c340c9038c8017db04f8eb5` |
| c executable | `953954a26bb214c82aab3204d84e6e953323d78ff44ea1f711c8bd0508de9541` |
| rust executable | `c8380942b72419b60043196672ccce537b85576ff76cfb3895166f39c83ea894` |

Build the baseline and candidate compilers in separate worktrees, then save the
following reference runner as a Python file and pass those roots. It follows
the executed procedure with configurable checkout paths; the raw report also
records the exact executed runner hash. Output directories must not exist.

```sh
nix develop --command python3 /path/to/paired.py /path/to/baseline /path/to/candidate
```

```python
import hashlib, importlib.util, json, os, platform, random, statistics, subprocess, sys, tempfile, time
from pathlib import Path
sys.dont_write_bytecode=True
before_root=Path(sys.argv[1]).resolve(); root=Path(sys.argv[2]).resolve()
out=root/'bench/results/builder-storage/2026-10-28-foundation-control'
out.mkdir(parents=True,exist_ok=False)
def sha(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def command(xs): return subprocess.check_output(list(map(str,xs)),text=True).strip()
def identity():
 return {'before_compiler':sha(before_root/'target/debug/slopic'),'after_compiler':sha(root/'target/debug/slopic'),
 'code':{p:sha(root/p) for p in ['std/core/builder.slp','scripts/builder-storage-check.sh','scripts/core-check.sh','tests/projects/pass/string-builder/src/main.slp']}}
frozen=identity()
spec=importlib.util.spec_from_file_location('harness',root/'bench/run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
h=m.Harness.__new__(m.Harness);h.args=type('Args',(),{'pin':2})();h.taskset='taskset';h.rssrun=str(out/'rssrun')
assert 2 in os.sched_getaffinity(0)
subprocess.run(['cc','-O2',root/'bench/rssrun.c','-o',h.rssrun],check=True)
startup=out/'startup.c';startup.write_text('int main(void) { return 0; }\n');subprocess.run(['cc','-O2',startup,'-o',out/'startup'],check=True)
package=out/'package';package.mkdir();source=package/'main.slp';source.write_bytes((root/'bench/slopium/string-builder.slp').read_bytes())
builds={};binaries={}
for name,compiler in [('before',before_root/'target/debug/slopic'),('after',root/'target/debug/slopic')]:
 binary=out/name;argv=[compiler,'--optimize','--strip','--source-root',package,'--toolchain-dependency','std',source,'-o',binary]
 subprocess.run(argv,check=True);binaries[name]=binary;builds[name]=list(map(str,argv))
for name,src,flags in [('c',root/'bench/c/stringbuilder.c',['cc','-O2']),('rust',root/'bench/rust/src/bin/stringbuilder.rs',['rustc','-O','--crate-name','storage_original'])]:
 binary=out/name;argv=[*flags,src,'-o',binary];subprocess.run(argv,check=True);binaries[name]=binary;builds[name]=list(map(str,argv))
def sample(binary, expected):
 with tempfile.TemporaryFile() as f:
  code,rss,wall=h.spawn([str(binary)],f.fileno());f.seek(0);values=list(map(int,f.read().splitlines()))
 assert code==0 and values==expected,(binary,code,values)
 return {'wall_ms':wall,'rss_kib':rss}
r={'complete':False,'metadata':{'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'platform':platform.platform(),'cpu':next(x.split(':',1)[1].strip() for x in Path('/proc/cpuinfo').read_text().splitlines() if x.startswith('model name')),'affinity':sorted(os.sched_getaffinity(0)),'pin':2,'load_before':os.getloadavg(),'heads':{str(p):command(['git','-C',p,'rev-parse','HEAD']) for p in [before_root,root]},'patch':command(['git','-C',root,'diff','HEAD']),'identity':frozen,'runner_sha256':sha(__file__),'tools':{x:command([x,'--version']).splitlines()[0] for x in ['cc','rustc','python3']},'builds':builds,'binary_sha256':{n:sha(p) for n,p in binaries.items()},'source_sha256':sha(source)},'rounds':[]}
for seed in [158,159]:
 rng=random.Random(seed);names=list(binaries);data={'seed':seed,'samples':{n:[] for n in names},'startup':[],'order':[]}
 for _ in range(3):
  for n in names:sample(binaries[n],[538890,36116720])
 for _ in range(15):
  rng.shuffle(names);data['order'].append(names.copy())
  for n in names:data['samples'][n].append(sample(binaries[n],[538890,36116720]))
  data['startup'].append(sample(out/'startup',[]))
 data['medians']={n:{k:statistics.median(x[k] for x in xs) for k in xs[0]} for n,xs in data['samples'].items()};r['rounds'].append(data)
 print(seed,data['medians'],flush=True)
assert identity()==frozen,'code/compiler changed'
r['metadata']['load_after']=os.getloadavg();r['complete']=True
(out/'report.json').write_text(json.dumps(r,indent=2)+'\n');print(out/'report.json')
```
