#!/usr/bin/env python3
"""Measure actual private packed writers; research finalization stays separate."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import random
import statistics
import subprocess
import sys
import tempfile
import time
sys.dont_write_bytecode = True


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(argv):
    return subprocess.check_output(list(map(str, argv)), text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--baseline-compiler', type=Path, required=True)
    parser.add_argument('--baseline-root', type=Path, required=True)
    parser.add_argument('--outdir', type=Path, required=True)
    parser.add_argument('--pin', type=int, required=True)
    parser.add_argument('--quick', action='store_true', help='smoke only')
    args = parser.parse_args()
    if args.pin not in os.sched_getaffinity(0):
        parser.error('pin outside permitted affinity')
    root = Path(__file__).resolve().parents[2]
    here = Path(__file__).resolve().parent
    out = args.outdir.resolve()
    out.mkdir(parents=True, exist_ok=False)
    compiler, baseline = args.compiler.resolve(), args.baseline_compiler.resolve()
    baseline_root = args.baseline_root.resolve()

    def identity():
        files = [p for directory in ('std', 'runtime')
                 for p in sorted((root / directory).rglob('*')) if p.is_file()]
        files += [here / 'append.py', here / 'packed.c',
                  root / 'bench/builder-phases/probe.slp.in',
                  root / 'bench/run.py', root / 'bench/rssrun.c',
                  root / 'bench/slopium/string-builder.slp']
        return {'compiler': sha(compiler), 'baseline_compiler': sha(baseline),
                'sources': {str(p.relative_to(root)): sha(p) for p in files},
                'baseline_sources': {str(p.relative_to(baseline_root)): sha(p)
                    for directory in ('std', 'runtime')
                    for p in sorted((baseline_root / directory).rglob('*')) if p.is_file()}}

    frozen = identity()
    spec = importlib.util.spec_from_file_location('harness', root / 'bench/run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    harness = module.Harness.__new__(module.Harness)
    harness.args = argparse.Namespace(pin=args.pin)
    harness.taskset = 'taskset'
    harness.rssrun = str(out / 'rssrun')
    subprocess.run(['cc', '-O2', root / 'bench/rssrun.c', '-o', harness.rssrun], check=True)
    source = out / 'startup.c'
    source.write_text('int main(void) { return 0; }\n')
    subprocess.run(['cc', '-O2', source, '-o', out / 'startup'], check=True)
    report = {'complete': False, 'metadata': {
        'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'platform': platform.platform(), 'pin': args.pin,
        'cpu': next(x.split(':', 1)[1].strip() for x in Path('/proc/cpuinfo').read_text().splitlines() if x.startswith('model name')),
        'affinity': sorted(os.sched_getaffinity(0)), 'load_before': os.getloadavg(),
        'heads': {str(p): run(['git', '-C', p, 'rev-parse', 'HEAD']) for p in [root, baseline_root]},
        'patch': run(['git', '-C', root, 'diff', 'HEAD']), 'identity': frozen,
        'tools': {name: run([name, '--version']).splitlines()[0] for name in ['cc', 'rustc', 'python3']},
        'quick': args.quick, 'samples': 3 if args.quick else 15,
        'warmups': 1 if args.quick else 3,
        'seeds': [159] if args.quick else [159, 160]}, 'cases': {}, 'order': [], 'startup': []}
    cases = [(f'pieces-{size}-{width}', 1, size // width, width)
             for size in [8192, 524288, 4194304] for width in [1, 5, 7, 8, 9, 64, 4096]]
    cases += [('records-50000', 0, 50000, 5), ('integers-50000', 2, 50000, 5),
              ('long-integers-50000', 3, 50000, 5)]
    if args.quick:
        cases = [c for c in cases if c[0] in ['pieces-8192-1', 'pieces-524288-5', 'long-integers-50000']]
    binaries = {}
    adapter = '''
(export storage-new storage-write-str storage-write-byte storage-write-i64
        storage-write-u64 storage-study-build)
(extern "study_finish" (storage-study-finish (words &(Slice u64)) (tail u64) (used i64)) -> String)
(fn storage-study-build ((storage PackedStorage)) -> String
  (storage-check &storage)
  (match storage ((PackedStorage :words words :tail tail :used used)
    (let view (slice &words 0 (len &words)))
    (storage-study-finish &view tail used))))
'''
    for name, mode, count, width in cases:
        expected = (''.join(f'item {i}\n' for i in range(count)) if mode == 0 else
                    'a' * (count * width) if mode == 1 else
                    ''.join(str(2**64 - 1 - i) for i in range(count)) if mode == 3 else
                    ''.join(map(str, range(count))))
        case = report['cases'][name] = {'expected': [len(expected), sum(expected.encode())], 'variants': {}}
        for variant in ['public', 'private-packed']:
            package = out / f'{name}-{variant}-src'
            package.mkdir()
            (package / 'owner.slp').write_text((root / 'std/core/builder.slp').read_text() + adapter)
            imports = '(take owner new write-str write-i64 write-u64 write-byte build)'
            if variant == 'private-packed':
                imports = '(take owner (storage-new :as new) (storage-write-str :as write-str) (storage-write-i64 :as write-i64) (storage-write-u64 :as write-u64) (storage-write-byte :as write-byte) (storage-study-build :as build))'
            text = (root / 'bench/builder-phases/probe.slp.in').read_text()
            text = text.replace('(take std:builder new write-str write-i64 write-u64 write-byte build)', imports)
            for old, new in {'@COUNT@': str(count), '@MODE@': str(mode), '@PIECE@': 'a' * width}.items():
                text = text.replace(old, new)
            source = package / 'main.slp'
            source.write_text(text)
            binary = out / f'{name}-{variant}'
            command = [compiler, '--optimize', '--strip', '--source-root', package,
                       '--toolchain-dependency', 'std', source, '-o', binary]
            for runtime in [root / 'runtime/slop_rt_core.c', root / 'runtime/slop_rt_hosted.c', here / 'packed.c']:
                command += ['--runtime', runtime]
            subprocess.run(list(map(str, command)), check=True)
            binaries[name, variant] = binary
            case['variants'][variant] = {'build': list(map(str, command)), 'binary_sha256': sha(binary),
                'main_sha256': sha(source), 'owner_sha256': sha(package / 'owner.slp'), 'rounds': []}
    case = report['cases']['original-control'] = {'expected': [538890, 36116720], 'variants': {}}
    package = out / 'original-src'
    package.mkdir()
    source = package / 'main.slp'
    source.write_bytes((root / 'bench/slopium/string-builder.slp').read_bytes())
    for variant, compiler_path in [('before', baseline), ('after', compiler)]:
        binary = out / f'original-{variant}'
        command = [compiler_path, '--optimize', '--strip', '--source-root', package,
                   '--toolchain-dependency', 'std', source, '-o', binary]
        subprocess.run(list(map(str, command)), check=True)
        binaries['original-control', variant] = binary
        case['variants'][variant] = {'build': list(map(str, command)), 'binary_sha256': sha(binary),
                                    'source_sha256': sha(source), 'rounds': []}

    def sample(name, variant):
        with tempfile.TemporaryFile() as f:
            code, rss, wall = harness.spawn([str(binaries[name, variant])], f.fileno())
            f.seek(0)
            values = list(map(int, f.read().splitlines()))
        original = name == 'original-control'
        assert code == 0 and len(values) == (2 if original else 6), (name, variant, code, values)
        assert values[:2] == report['cases'][name]['expected'] and all(v >= 0 for v in values), (name, variant, values)
        return dict(zip(['wall_ms', 'rss_kib', 'append_ns', 'research_finalize_ns',
                         'checksum_ns', 'clock_pair_ns'], [wall, rss, *values[2:]]))

    for index, seed in enumerate(report['metadata']['seeds']):
        rng = random.Random(seed)
        pairs = list(binaries)
        for name, variant in pairs:
            report['cases'][name]['variants'][variant]['rounds'].append({'samples': []})
        for _ in range(report['metadata']['warmups']):
            for name, variant in pairs:
                sample(name, variant)
        for repeat in range(report['metadata']['samples']):
            rng.shuffle(pairs)
            report['order'].append([list(p) for p in pairs])
            for name, variant in pairs:
                report['cases'][name]['variants'][variant]['rounds'][index]['samples'].append(sample(name, variant))
            with tempfile.TemporaryFile() as f:
                code, rss, wall = harness.spawn([str(out / 'startup')], f.fileno())
                f.seek(0)
                assert code == 0 and f.read() == b''
            report['startup'].append({'wall_ms': wall, 'rss_kib': rss})
            print(f'round {index+1}: {repeat+1}/{report["metadata"]["samples"]}', flush=True)
            (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    assert identity() == frozen, 'source/compiler changed during measurements'
    for case in report['cases'].values():
        for variant in case['variants'].values():
            for data in variant['rounds']:
                data['median'] = {k: statistics.median(s[k] for s in data['samples']) for k in data['samples'][0]}
    report['complete'] = True
    report['metadata']['load_after'] = os.getloadavg()
    (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(out / 'report.json')


if __name__ == '__main__':
    main()
