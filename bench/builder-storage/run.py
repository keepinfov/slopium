#!/usr/bin/env python3
"""Compare actual Slopium word packing and explicitly separate native C models."""
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


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(argv):
    return subprocess.check_output(list(map(str, argv)), text=True).strip()


def capacity(length, packed):
    units = length // 8 if packed else length
    cap = 0 if units == 0 else max(4, 1 << (units - 1).bit_length())
    return cap * (8 if packed else 1) + (8 if packed else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, required=True)
    parser.add_argument('--compiler-root', type=Path, required=True)
    parser.add_argument('--outdir', type=Path, required=True)
    parser.add_argument('--pin', type=int, required=True)
    parser.add_argument('--quick', action='store_true', help='smoke only; three samples, one round')
    args = parser.parse_args()
    if args.pin not in os.sched_getaffinity(0):
        parser.error('pin outside permitted affinity')
    root = Path(__file__).resolve().parents[2]
    here = Path(__file__).resolve().parent
    compiler, compiler_root = args.compiler.resolve(), args.compiler_root.resolve()
    out = args.outdir.resolve()
    out.mkdir(parents=True, exist_ok=False)
    spec = importlib.util.spec_from_file_location('harness', root / 'bench/run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    harness = module.Harness.__new__(module.Harness)
    harness.args = argparse.Namespace(pin=args.pin)
    harness.taskset = 'taskset'
    harness.rssrun = str(out / 'rssrun')
    subprocess.run(['cc', '-O2', root / 'bench/rssrun.c', '-o', harness.rssrun], check=True)

    def identities():
        sources = {str(p.relative_to(root)): sha(p.read_bytes()) for p in sorted(here.iterdir()) if p.is_file()}
        library = {}
        for directory in ('std', 'runtime'):
            for p in sorted((compiler_root / directory).rglob('*')):
                if p.is_file():
                    other = root / p.relative_to(compiler_root)
                    if p.read_bytes() != other.read_bytes():
                        raise RuntimeError('compiler and study production sources differ')
                    library[str(p.relative_to(compiler_root))] = sha(p.read_bytes())
        return {'compiler_sha256': sha(compiler.read_bytes()), 'library_runtime': library, 'study_sources': sources}

    before = identities()
    count_samples = 3 if args.quick else 15
    warmup = 1 if args.quick else 3
    seeds = [157] if args.quick else [157, 158]
    report = {'complete': False, 'metadata': {
        'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'head': run(['git', '-C', root, 'rev-parse', 'HEAD']),
        'tracked_patch': run(['git', '-C', root, 'diff', 'HEAD']),
        'compiler_root_head': run(['git', '-C', compiler_root, 'rev-parse', 'HEAD']),
        'compiler_root_patch': run(['git', '-C', compiler_root, 'diff', 'HEAD']),
        'identity': before, 'platform': platform.platform(),
        'cpu': next(x.split(':', 1)[1].strip() for x in Path('/proc/cpuinfo').read_text().splitlines() if x.startswith('model name')),
        'affinity': sorted(os.sched_getaffinity(0)), 'pin': args.pin,
        'tools': {name: run(command).splitlines()[0] for name, command in {
            'cc': ['cc', '--version'], 'rustc': ['rustc', '--version'], 'python': [sys.executable, '--version']}.items()},
        'samples': count_samples, 'warmup': warmup, 'seeds': seeds,
        'quick': args.quick, 'load_before': os.getloadavg()}, 'cases': {}, 'order': [], 'startup': []}
    model_binaries = {}
    model_sources = [here / name for name in ['models.c', 'list-model.c', 'model-probe.c']]
    for name, storage in [('c-words-model', 0), ('c-owning-model', 1), ('c-narrow-model', 2)]:
        binary = out / name
        command = list(map(str, ['cc', '-std=c11', '-O2', '-Wall', '-Wextra', '-Werror', f'-DSTORAGE={storage}', *model_sources, '-o', binary]))
        subprocess.run(command, check=True)
        subprocess.run([binary, '--check'], check=True)
        model_binaries[name] = (binary, command)
    startup_source = out / 'startup.c'
    startup_source.write_text('int main(void) { return 0; }\n')
    startup = out / 'startup'
    subprocess.run(['cc', '-O2', startup_source, '-o', startup], check=True)
    cases = [(f'records-{n}', 0, n, 5) for n in [1000, 50000, 250000]]
    cases += [('integers-50000', 2, 50000, 5), ('long-integers-50000', 3, 50000, 5)]
    cases += [(f'pieces-{size}-{width}', 1, size // width, width)
              for size in [8192, 524288, 4194304] for width in [5, 64, 4096]]
    cases += [('original-control', 0, 50000, 5)]
    if args.quick:
        cases = [c for c in cases if c[0] in ['records-1000', 'long-integers-50000', 'pieces-8192-5', 'original-control']]
    binaries = {}
    for name, mode, count, width in cases:
        expected = (''.join(f'item {i}\n' for i in range(count)) if mode == 0 else
                    'a' * (count * width) if mode == 1 else
                    ''.join(str(2**64 - 1 - i) for i in range(count)) if mode == 3 else ''.join(map(str, range(count))))
        correct = [len(expected), sum(expected.encode())]
        case = report['cases'][name] = {'correctness': correct, 'mode': mode, 'count': count, 'piece_width': width, 'variants': {}}
        for variant in ['slopium-current', 'slopium-words']:
            package = out / f'{name}-{variant}-src'
            package.mkdir()
            if name == 'original-control':
                text = (root / 'bench/slopium/string-builder.slp').read_text()
                if variant == 'slopium-words':
                    text = text.replace('(take std:builder ', '(take packed ', 1)
            else:
                text = (here / 'probe.slp.in').read_text()
                imports = '(take std:builder Builder new write-str write-byte write-i64 write-u64 size build)'
                read = '(fn read-at ((out &Builder) (index i64)) -> i64 (match out ((Builder :bytes bytes) (as i64 (get bytes index)))))'
                if variant == 'slopium-words':
                    imports = '(take packed new write-str write-byte write-i64 write-u64 size read-at build)'
                    read = ''
                for old, new in {'@IMPORT@': imports, '@READ@': read, '@COUNT@': str(count), '@MODE@': str(mode), '@PIECE@': 'a' * width}.items():
                    text = text.replace(old, new)
            source = package / 'main.slp'
            source.write_text(text)
            if variant == 'slopium-words':
                (package / 'packed.slp').write_bytes((here / 'packed.slp').read_bytes())
            binary = out / f'{name}-{variant}'
            command = [compiler, '--optimize', '--strip', '--source-root', package, '--toolchain-dependency', 'std', source, '-o', binary]
            if variant == 'slopium-words':
                for runtime in [here / 'packed.c', root / 'runtime/slop_rt_core.c', root / 'runtime/slop_rt_hosted.c']:
                    command += ['--runtime', runtime]
            command = list(map(str, command))
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
            binaries[name, variant] = [str(binary)]
            logical = correct[0]
            cap = capacity(logical, True) if variant == 'slopium-words' else capacity(logical, False) * 8
            case['variants'][variant] = {'build': command, 'source_sha256': sha(source.read_bytes()),
                'binary_sha256': sha(binary.read_bytes()), 'capacity_bytes_from_runtime_growth_rule': cap, 'rounds': []}
        if name == 'original-control':
            for variant, source, command in [
                ('c-original', root / 'bench/c/stringbuilder.c', ['cc', '-O2']),
                ('rust-original', root / 'bench/rust/src/bin/stringbuilder.rs', ['rustc', '-O', '--crate-name', 'storage_original'])]:
                binary = out / variant
                command = list(map(str, [*command, source, '-o', binary]))
                subprocess.run(command, check=True)
                binaries[name, variant] = [str(binary)]
                case['variants'][variant] = {'build': command, 'source_sha256': sha(source.read_bytes()), 'binary_sha256': sha(binary.read_bytes()), 'rounds': []}
        else:
            for variant, (binary, command) in model_binaries.items():
                binaries[name, variant] = [str(binary), str(mode), str(count), str(width)]
                case['variants'][variant] = {'build': command, 'binary_sha256': sha(binary.read_bytes()), 'rounds': []}

    def sample(name, variant):
        with tempfile.TemporaryFile() as stdout:
            code, rss, wall = harness.spawn(binaries[name, variant], stdout.fileno())
            stdout.seek(0)
            values = list(map(int, stdout.read().splitlines()))
        original = name == 'original-control'
        is_model = variant.endswith('-model')
        expected_lines = 2 if original else 9 if is_model else 8
        expected = report['cases'][name]['correctness']
        if code or len(values) != expected_lines or values[:2] != expected or any(v < 0 for v in values) or (not original and values[2] != expected[1]):
            raise RuntimeError(f'incorrect workload: {name}/{variant}: {values}')
        sample_data = {'wall_ms': wall, 'rss_kib': rss}
        if not original:
            sample_data.update(zip(['append_ns', 'read_ns', 'finalize_ns', 'checksum_ns', 'clock_pair_ns'], values[3:8]))
            if is_model:
                cap = capacity(expected[0], variant == 'c-words-model')
                if values[8] != cap:
                    raise RuntimeError('native model capacity differs from its growth rule')
                sample_data['capacity_bytes'] = values[8]
        return sample_data

    for index, seed in enumerate(seeds):
        rng = random.Random(seed)
        pairs = [(name, variant) for name, case in report['cases'].items() for variant in case['variants']]
        for case in report['cases'].values():
            for variant in case['variants'].values():
                variant['rounds'].append({'samples': []})
        for _ in range(warmup):
            for name, variant in pairs:
                sample(name, variant)
        for repeat in range(count_samples):
            rng.shuffle(pairs)
            report['order'].append(list(map(list, pairs)))
            for name, variant in pairs:
                report['cases'][name]['variants'][variant]['rounds'][index]['samples'].append(sample(name, variant))
            with tempfile.TemporaryFile() as stdout:
                _, rss, wall = harness.spawn([str(startup)], stdout.fileno())
            report['startup'].append({'rss_kib': rss, 'wall_ms': wall})
            (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            print(f'round {index + 1}: {repeat + 1}/{count_samples}', flush=True)
    if identities() != before:
        raise RuntimeError('source or compiler changed during the study')
    for case in report['cases'].values():
        for variant in case['variants'].values():
            for data in variant['rounds']:
                data['median'] = {key: statistics.median(s[key] for s in data['samples']) for key in data['samples'][0]}
    report['metadata']['load_after'] = os.getloadavg()
    report['complete'] = True
    (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(out / 'report.json')


if __name__ == '__main__':
    main()
