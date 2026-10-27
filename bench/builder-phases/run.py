#!/usr/bin/env python3
"""Measure builder stages without changing the end-to-end control kernel."""

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
HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PHASES = ("append_ns", "finalize_ns", "checksum_ns", "clock_pair_ns")


def run(command):
    return subprocess.check_output([str(x) for x in command], text=True).strip()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--slopic", type=Path, required=True)
    parser.add_argument("--compiler-root", type=Path, required=True,
                        help="source tree used to build slopic and its bundled library")
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--pin", type=int)
    parser.add_argument("--quick", action="store_true",
                        help="one record size and one document size, three samples")
    parser.add_argument("--outdir", type=Path)
    args = parser.parse_args()
    if args.quick:
        args.runs, args.warmup = 3, 1
    if args.runs < 1 or args.warmup < 0:
        parser.error("runs must be positive and warmup nonnegative")
    allowed = sorted(os.sched_getaffinity(0))
    cpu = args.pin if args.pin is not None else allowed[0]
    if cpu not in allowed:
        parser.error(f"CPU {cpu} is outside the allowed affinity {allowed}")
    compiler = args.slopic.resolve()
    compiler_root = args.compiler_root.resolve()
    target = {"x86_64": "x86_64-unknown-linux-gnu",
              "aarch64": "aarch64-unknown-linux-gnu"}.get(platform.machine())
    if target is None:
        parser.error("native probes require an x86-64 or AArch64 Linux host")
    out = (args.outdir or REPO / "bench/results/builder-phases" /
           time.strftime("%Y%m%d-%H%M%S")).resolve()
    out.mkdir(parents=True, exist_ok=False)

    # Reuse the suite's fork/exec wrapper, so Python's RSS is never the floor.
    spec = importlib.util.spec_from_file_location("bench_harness", HERE.parent / "run.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    harness = module.Harness.__new__(module.Harness)
    harness.args = argparse.Namespace(pin=cpu)
    harness.taskset = "taskset"
    harness.rssrun = str(out / "rssrun")
    subprocess.run(["cc", "-O2", str(HERE.parent / "rssrun.c"), "-o",
                    harness.rssrun], check=True)

    counts = [50000] if args.quick else [1000, 50000, 250000]
    sizes = [524288] if args.quick else [8192, 524288, 4194304]
    cases = []
    for count in counts:
        cases += [(f"records-{count}", 0, count, 5),
                  (f"integers-{count}", 2, count, 5)]
    for size in sizes:
        for width in [5, 64, 4096]:
            cases.append((f"pieces-{size}-{width}", 1, max(1, size // width), width))

    patch = subprocess.check_output(["git", "-C", str(compiler_root), "diff", "--binary", "HEAD"])
    (out / "compiler.patch").write_bytes(patch)
    library = hashlib.sha256()
    for directory in ("std", "runtime"):
        for path in sorted((compiler_root / directory).rglob("*")):
            if path.is_file():
                library.update(str(path.relative_to(compiler_root)).encode() + b"\0")
                library.update(path.read_bytes())
    report = {
        "complete": False,
        "metadata": {
            "utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "host": platform.node(), "platform": platform.platform(),
            "cpu": next((line.split(":", 1)[1].strip() for line in
                         Path("/proc/cpuinfo").read_text().splitlines()
                         if line.startswith("model name")), "unknown"),
            "affinity": allowed, "pin": cpu, "runs": args.runs, "warmup": args.warmup,
            "load_average_before": os.getloadavg(),
            "order_seed": 153,
            "profile": "release (--optimize --strip)",
            "target": target,
            "compiler_info": json.loads(run([compiler, "--info"])),
            "compiler_revision": run(["git", "-C", compiler_root, "rev-parse", "HEAD"]),
            "compiler_sha256": digest(compiler.read_bytes()),
            "compiler_patch_sha256": digest(patch),
            "library_runtime_sha256": library.hexdigest(),
            "probe_revision": run(["git", "-C", REPO, "rev-parse", "HEAD"]),
            "probe_sources": {p.name: digest(p.read_bytes()) for p in
                              sorted(HERE.iterdir()) if p.is_file()},
            "tools": {name: run(command).splitlines()[0] for name, command in {
                "slopic": [compiler, "--version"], "cc": ["cc", "--version"],
                "rustc": ["rustc", "--version"], "python": [sys.executable, "--version"]
            }.items()},
            "timing": "three contiguous internal intervals; no subtraction across unlike probes",
            "rss": "whole process peak KiB, including construction; not per-phase RSS",
            "profiling": "not collected by this runner; record tool/permission checks in findings",
        },
        "cases": {},
        "sample_order": [],
    }
    jobs = []
    for name, mode, count, width in cases:
        piece = "a" * width
        expected = ("".join(f"item {i}\n" for i in range(count)) if mode == 0 else
                    piece * count if mode == 1 else "".join(map(str, range(count))))
        correctness = [len(expected), sum(expected.encode())]
        report["cases"][name] = {"mode": mode, "count": count, "piece_bytes": width,
                                 "correctness": correctness, "languages": {}}
        for language, suffix in [("slopium", "slp"), ("c", "c"), ("rust", "rs")]:
            source = out / f"{name}.{suffix}"
            template = (HERE / f"probe.{suffix}.in").read_text()
            source.write_text(template.replace("@COUNT@", str(count)).replace(
                "@MODE@", str(mode)).replace("@PIECE@", piece))
            binary = out / f"{name}-{language}"
            command = ([compiler, source, "--target", target, "--optimize", "--strip", "-o", binary]
                       if language == "slopium" else
                       ["cc", "-O2", source, "-o", binary] if language == "c" else
                       ["rustc", "-O", "--crate-name", "builder_probe", source, "-o", binary])
            built = subprocess.run([str(x) for x in command], capture_output=True, text=True)
            if built.returncode:
                raise RuntimeError(f"build failed: {command}\n{built.stdout}\n{built.stderr}")
            report["cases"][name]["languages"][language] = {"samples": [], "build": list(map(str, command))}
            jobs.append((name, language, binary, correctness))

    # Run the untouched three-language control, not just the diagnostic copy.
    expected = "".join(f"item {i}\n" for i in range(50000))
    correctness = [len(expected), sum(expected.encode())]
    report["cases"]["original-control"] = {"correctness": correctness, "languages": {}}
    for language, source, command in [
        ("slopium", REPO / "bench/slopium/string-builder.slp", [compiler, "--target", target, "--optimize", "--strip"]),
        ("c", REPO / "bench/c/stringbuilder.c", ["cc", "-O2"]),
        ("rust", REPO / "bench/rust/src/bin/stringbuilder.rs", ["rustc", "-O"]),
    ]:
        binary = out / f"original-{language}"
        build = [*command, source, "-o", binary]
        subprocess.run(list(map(str, build)), check=True)
        report["cases"]["original-control"]["languages"][language] = {
            "samples": [], "source_sha256": digest(source.read_bytes()),
            "build": list(map(str, build))}
        jobs.append(("original-control", language, binary, correctness))

    startup_source = out / "startup.c"
    startup_source.write_text("int main(void) { return 0; }\n")
    startup = out / "startup"
    subprocess.run(["cc", "-O2", startup_source, "-o", startup], check=True)
    report["startup_samples"] = []

    def sample(job):
        name, language, binary, expected = job
        with tempfile.TemporaryFile() as stdout:
            _, rss, wall = harness.spawn([str(binary)], stdout.fileno())
            stdout.seek(0)
            values = [int(line) for line in stdout.read().splitlines()]
        size = 2 if name == "original-control" else 6
        if len(values) != size or values[:2] != expected or any(v < 0 for v in values[2:]):
            raise RuntimeError(f"invalid correctness/timing output: {name}/{language}: {values}")
        return {**dict(zip(PHASES, values[2:])), "rss_kib": rss, "wall_ms": wall}

    # Build and validate every control before collecting timings. Interleave
    # cases/languages with a recorded deterministic order to reduce drift.
    for job in jobs:
        sample(job)
    for _ in range(args.warmup):
        for job in jobs:
            sample(job)
    rng = random.Random(153)
    for repeat in range(args.runs):
        order = list(jobs)
        rng.shuffle(order)
        report["sample_order"].append([f"{name}/{language}" for name, language, _, _ in order])
        for job in order:
            name, language, _, _ = job
            report["cases"][name]["languages"][language]["samples"].append(sample(job))
        with tempfile.TemporaryFile() as stdout:
            _, rss, wall = harness.spawn([str(startup)], stdout.fileno())
        report["startup_samples"].append({"rss_kib": rss, "wall_ms": wall})
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"sample {repeat + 1}/{args.runs}", flush=True)
    if digest(compiler.read_bytes()) != report["metadata"]["compiler_sha256"]:
        raise RuntimeError("compiler changed during measurement; rerun on a stable binary")
    for case in report["cases"].values():
        for language in case["languages"].values():
            language["median"] = {key: statistics.median(s[key] for s in language["samples"])
                                  for key in language["samples"][0]}
    report["complete"] = True
    report["metadata"]["load_average_after"] = os.getloadavg()
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(out / "report.json")


if __name__ == "__main__":
    main()
