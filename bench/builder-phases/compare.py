#!/usr/bin/env python3
"""Pair builder revisions with checked C and Rust controls on one pinned CPU."""

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


def run(command):
    return subprocess.check_output(list(map(str, command)), text=True).strip()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def identity(root):
    compiler = root / 'target/debug/slopic'
    digest = hashlib.sha256()
    for directory in ('std', 'runtime'):
        for path in sorted((root / directory).rglob('*')):
            if path.is_file():
                digest.update(str(path.relative_to(root)).encode() + b'\0')
                digest.update(path.read_bytes())
    return {'head': run(['git', '-C', root, 'rev-parse', 'HEAD']),
            'patch': run(['git', '-C', root, 'diff', '--binary', 'HEAD']),
            'compiler_sha256': sha(compiler.read_bytes()),
            'library_runtime_sha256': digest.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before-root', type=Path, required=True)
    parser.add_argument('--after-root', type=Path, required=True)
    parser.add_argument('--outdir', type=Path, required=True)
    parser.add_argument('--pin', type=int, required=True)
    parser.add_argument('--case', action='append', default=[], help='select a named workload; repeatable')
    parser.add_argument('--repeat-document', type=int, default=1,
                        help='repeat each diagnostic in one process; discard its first five intervals')
    args = parser.parse_args()
    if args.repeat_document != 1 and args.repeat_document <= 5:
        parser.error('repeat-document must be one or greater than five')
    before, after = args.before_root.resolve(), args.after_root.resolve()
    if args.pin not in os.sched_getaffinity(0):
        parser.error('pin is outside the allowed affinity')
    out = args.outdir.resolve()
    out.mkdir(parents=True, exist_ok=False)
    here = after / 'bench/builder-phases'
    spec = importlib.util.spec_from_file_location('harness', after / 'bench/run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    harness = module.Harness.__new__(module.Harness)
    harness.args = argparse.Namespace(pin=args.pin)
    harness.taskset = 'taskset'
    harness.rssrun = str(out / 'rssrun')
    subprocess.run(['cc', '-O2', after / 'bench/rssrun.c', '-o', harness.rssrun], check=True)
    report = {'complete': False, 'metadata': {
        'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'platform': platform.platform(), 'pin': args.pin,
        'cpu': next(line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),
        'affinity': sorted(os.sched_getaffinity(0)), 'load_before': os.getloadavg(),
        'runs': 15, 'warmup': 3, 'rounds': 2, 'seeds': [154, 155],
        'repeat_document': args.repeat_document,
        'before': identity(before), 'after': identity(after),
        'tools': {name: run(command).splitlines()[0] for name, command in {
            'cc': ['cc', '--version'], 'rustc': ['rustc', '--version'],
            'python': [sys.executable, '--version']}.items()},
        'script_sha256': sha(Path(__file__).read_bytes())}, 'cases': {}, 'order': [], 'startup_samples': []}
    startup_source = out / 'startup.c'
    startup_source.write_text('int main(void) { return 0; }\n')
    startup = out / 'startup'
    subprocess.run(['cc', '-O2', startup_source, '-o', startup], check=True)
    cases = [(f'{name}-{count}', mode, count, 5) for count in [1000, 50000, 250000]
             for name, mode in [('records', 0), ('integers', 2)]]
    cases += [(f'pieces-{size}-{width}', 1, size // width, width)
              for size in [8192, 524288, 4194304] for width in [5, 64, 4096]]
    cases += [('original-control', 0, 50000, 5)]
    if set(args.case) - {case[0] for case in cases}:
        parser.error('unknown workload name')
    if args.case:
        cases = [case for case in cases if case[0] in args.case]
    binaries = {}
    for name, mode, count, width in cases:
        expected = (''.join(f'item {i}\n' for i in range(count)) if mode == 0 else
                    'a' * (width * count) if mode == 1 else ''.join(map(str, range(count))))
        case = report['cases'][name] = {'correctness': [len(expected), sum(expected.encode())], 'languages': {}}
        for language, root, suffix in [('before', before, 'slp'), ('after', after, 'slp'),
                                       ('c', after, 'c'), ('rust', after, 'rs')]:
            if name == 'original-control':
                relative = {'slp': 'slopium/string-builder.slp', 'c': 'c/stringbuilder.c', 'rs': 'rust/src/bin/stringbuilder.rs'}[suffix]
                source = root / 'bench' / relative
                assert source.read_bytes() == (after / 'bench' / relative).read_bytes()
            else:
                source = out / f'{name}.{suffix}'
                template = (here / f'probe.{suffix}.in').read_text()
                source.write_text(template.replace('@COUNT@', str(count)).replace('@MODE@', str(mode)).replace('@PIECE@', 'a' * width))
                if args.repeat_document > 1:
                    count_documents = args.repeat_document
                    text = source.read_text()
                    if suffix == 'slp':
                        text = text.replace('(fn main ', '(fn measure ', 1)
                        text += f'\n(fn main () -> i32 (let mut document 0) (while (< document {count_documents}) (measure) (set document (+ document 1))) 0)\n'
                    elif suffix == 'c':
                        text = text.replace('int main(void)', 'static int measure(void)', 1)
                        text += f'\nint main(void) {{ for (int i = 0; i < {count_documents}; ++i) measure(); return 0; }}\n'
                    else:
                        text = text.replace('fn main()', 'fn measure()', 1)
                        text += f'\nfn main() {{ for _ in 0..{count_documents} {{ measure(); }} }}\n'
                    source.write_text(text)
            binary = out / f'{name}-{language}'
            command = ([root / 'target/debug/slopic', '--optimize', '--strip'] if suffix == 'slp' else
                       ['cc', '-O2'] if suffix == 'c' else ['rustc', '-O', '--crate-name', 'builder_probe'])
            command = list(map(str, [*command, source, '-o', binary]))
            subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
            binaries[name, language] = binary
            case['languages'][language] = {'build': command, 'source_sha256': sha(source.read_bytes()), 'rounds': []}

    def sample(name, language):
        with tempfile.TemporaryFile() as stdout:
            _, rss, wall = harness.spawn([str(binaries[name, language])], stdout.fileno())
            stdout.seek(0)
            values = list(map(int, stdout.read().splitlines()))
        width = 2 if name == 'original-control' else 6
        repetitions = 1 if name == 'original-control' else args.repeat_document
        groups = [values[i:i + width] for i in range(0, len(values), width)]
        if len(values) != width * repetitions or any(group[:2] != report['cases'][name]['correctness'] for group in groups) or any(v < 0 for v in values):
            raise RuntimeError(f'invalid output: {name}/{language}: {values}')
        retained = groups[5:] if repetitions > 1 else groups
        intervals = [statistics.mean(group[i] for group in retained) for i in range(2, width)]
        return {**dict(zip(['append_ns', 'finalize_ns', 'checksum_ns', 'clock_pair_ns'], intervals)), 'rss_kib': rss, 'wall_ms': wall}

    for round_index, seed in enumerate([154, 155]):
        rng = random.Random(seed)
        for case in report['cases'].values():
            for language in case['languages'].values():
                language['rounds'].append({'samples': []})
        for _ in range(3):
            for name in report['cases']:
                for language in ['before', 'after', 'c', 'rust']:
                    sample(name, language)
        for repeat in range(15):
            names = list(report['cases'])
            rng.shuffle(names)
            order = []
            for name in names:
                languages = ['before', 'after', 'c', 'rust']
                rng.shuffle(languages)
                for language in languages:
                    result = sample(name, language)
                    report['cases'][name]['languages'][language]['rounds'][round_index]['samples'].append(result)
                    order.append(f'{name}/{language}')
            report['order'].append(order)
            with tempfile.TemporaryFile() as stdout:
                _, rss, wall = harness.spawn([str(startup)], stdout.fileno())
            report['startup_samples'].append({'round': round_index + 1, 'rss_kib': rss, 'wall_ms': wall})
            (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            print(f'round {round_index + 1}: {repeat + 1}/15', flush=True)
    if identity(before) != report['metadata']['before'] or identity(after) != report['metadata']['after']:
        raise RuntimeError('source or compiler changed during measurement')
    for case in report['cases'].values():
        for language in case['languages'].values():
            for round_data in language['rounds']:
                samples = round_data['samples']
                round_data['median'] = {key: statistics.median(s[key] for s in samples) for key in samples[0]}
    report['metadata']['load_after'] = os.getloadavg()
    report['complete'] = True
    (out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(out / 'report.json')


if __name__ == '__main__':
    main()
