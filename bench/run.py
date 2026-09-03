#!/usr/bin/env python3
"""The Slopium benchmark harness.

Builds the same seven kernels in Slopium, C, Rust, Python and Java, checks
that every language's output agrees, then measures wall time and peak RSS
per kernel and emits a JSON file and a Markdown table under `results/`.

Everything the harness produces goes to `bench/results/`, which is build
output and gitignored. Run it from the bench shell:

    nix develop .#bench -c python3 bench/run.py

Pass `--quick` for a short run while changing the kernels themselves; a
number worth quoting wants the default ten runs.
"""

import argparse
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time

BENCH = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(BENCH)

KERNELS = [
    "hello",
    "fib",
    "mandelbrot",
    "matmul",
    "binary-trees",
    "hash-map",
    "string-builder",
]

# The kernel sources use the same constants everywhere. Slopium names its
# files with dashes and the other languages cannot all use one, so `sl` is
# the Slopium file name and `file` the one everywhere else. `verify` says how
# two languages' outputs are compared: byte-for-byte, or as floats with a
# tolerance (`matmul`, whose sum goes through each language's own formatter).
KERNEL_INFO = {
    "hello": {"sl": "hello", "file": "hello", "verify": "exact"},
    "fib": {"sl": "fib", "file": "fib", "verify": "exact"},
    "mandelbrot": {"sl": "mandelbrot", "file": "mandelbrot", "verify": "exact"},
    "matmul": {"sl": "matmul", "file": "matmul", "verify": "float"},
    "binary-trees": {
        "sl": "binary-trees", "file": "binarytrees", "verify": "exact"
    },
    "hash-map": {"sl": "hash-map", "file": "hashmap", "verify": "exact"},
    "string-builder": {
        "sl": "string-builder", "file": "stringbuilder", "verify": "exact"
    },
}

LANGUAGES = ["slopium", "c", "rust", "python", "java"]


def log(message):
    print(message, flush=True)


def fail(message):
    sys.exit(f"bench: {message}")


def find_tool(name, alternative=None):
    found = shutil.which(name)
    if found:
        return found
    if alternative and os.path.exists(alternative):
        return alternative
    return None


class Harness:
    def __init__(self, args):
        self.args = args
        self.results = os.path.abspath(args.outdir)
        self.bin = os.path.join(self.results, "bin")
        self.java_classes = os.path.join(self.results, "java-classes")
        self.rust_target = os.path.join(self.results, "rust-target")
        os.makedirs(self.bin, exist_ok=True)

        # The subject under test is this tree, so the repository's own
        # release build wins over anything in PATH -- a machine running the
        # toolchain as a system package would otherwise benchmark that one.
        def compiler(name):
            local = os.path.join(REPO, "target", "release", name)
            return (
                args.__dict__[name]
                or (local if os.path.exists(local) else None)
                or os.environ.get(name.upper())
                or find_tool(name)
            )

        self.slopic = compiler("slopic")
        self.slopium = compiler("slopium")
        self.cc = find_tool("cc")
        self.cargo = find_tool("cargo")
        self.rustc = find_tool("rustc")
        self.python3 = find_tool("python3")
        self.javac = find_tool("javac")
        self.java = find_tool("java")
        self.taskset = None if args.no_pin else find_tool("taskset")

        missing = []
        for tool, wanted in [
            ("slopic", True),
            ("cc", "c" in args.languages),
            ("cargo", "rust" in args.languages),
            ("rustc", "rust" in args.languages),
            ("python3", "python" in args.languages),
            ("javac", "java" in args.languages),
            ("java", "java" in args.languages),
        ]:
            if wanted and not getattr(self, tool):
                missing.append(tool)
        if missing:
            fail(f"missing tools: {', '.join(missing)}")
        if "debug" in self.slopic:
            log(
                "warning: slopic is a debug build; compile times will not be"
                " representative"
            )

    # ---------------------------------------------------------------- build

    def timed(self, argv):
        started = time.perf_counter()
        done = subprocess.run(argv, capture_output=True, text=True)
        seconds = time.perf_counter() - started
        if done.returncode != 0:
            fail(f"build failed ({' '.join(argv)}):\n{done.stdout}\n{done.stderr}")
        return seconds

    def build(self):
        builds = {}
        log("building ...")
        # The measuring wrapper: a program small enough that its own footprint
        # is no floor under the RSS numbers, doing the fork/exec/wait the
        # harness cannot do without inheriting the interpreter's pages.
        self.rssrun = os.path.join(self.bin, "rssrun")
        self.timed([self.cc, "-O2", "-o", self.rssrun,
                    os.path.join(BENCH, "rssrun.c")])
        for kernel in self.args.kernels:
            info = KERNEL_INFO[kernel]
            source = os.path.join(BENCH, "slopium", info["sl"] + ".slp")
            if "slopium" in self.args.languages:
                out = os.path.join(self.bin, kernel + "-slopic")
                builds.setdefault("slopium", {})[kernel] = {
                    "release_seconds": self.timed(
                        [self.slopic, source, "--emit", "exe",
                         "--optimize", "--strip", "-o", out]
                    ),
                    "dev_seconds": self.timed(
                        [self.slopic, source, "--emit", "exe", "-o",
                         out + ".dev"]
                    ),
                    "binary_bytes": os.path.getsize(out),
                }
            if "c" in self.args.languages:
                out = os.path.join(self.bin, kernel + "-c")
                builds.setdefault("c", {})[kernel] = {
                    "seconds": self.timed(
                        [self.cc, "-O2", "-o", out,
                         os.path.join(BENCH, "c", info["file"] + ".c")]
                    ),
                    "binary_bytes": os.path.getsize(out),
                }
            if "rust" in self.args.languages:
                builds.setdefault("rust", {})[kernel] = {}
            if "java" in self.args.languages:
                builds.setdefault("java", {})[kernel] = {}
            if "python" in self.args.languages:
                builds.setdefault("python", {})[kernel] = {
                    # No build step; the size recorded is the shipped source.
                    "seconds": 0.0,
                    "binary_bytes": os.path.getsize(
                        os.path.join(BENCH, "python", info["file"] + ".py")
                    ),
                }

        if "rust" in self.args.languages:
            # A cold number needs a cold cache: cargo would otherwise report
            # the previous run's work as this run's compile time.
            shutil.rmtree(self.rust_target, ignore_errors=True)
            env = dict(os.environ, CARGO_TARGET_DIR=self.rust_target)
            started = time.perf_counter()
            done = subprocess.run(
                [self.cargo, "build", "--release", "--manifest-path",
                 os.path.join(BENCH, "rust", "Cargo.toml")],
                capture_output=True, text=True, env=env,
            )
            seconds = time.perf_counter() - started
            if done.returncode != 0:
                fail(f"build failed (cargo):\n{done.stdout}\n{done.stderr}")
            for kernel in self.args.kernels:
                binary = os.path.join(
                    self.rust_target, "release", KERNEL_INFO[kernel]["file"]
                )
                builds["rust"][kernel]["binary_bytes"] = os.path.getsize(binary)
            builds["rust"]["_total"] = {"seconds": seconds}

        if "java" in self.args.languages:
            sources = [
                os.path.join(BENCH, "java", name)
                for name in sorted(os.listdir(os.path.join(BENCH, "java")))
                if name.endswith(".java")
            ]
            shutil.rmtree(self.java_classes, ignore_errors=True)
            os.makedirs(self.java_classes, exist_ok=True)
            started = time.perf_counter()
            done = subprocess.run(
                [self.javac, "-d", self.java_classes] + sources,
                capture_output=True, text=True,
            )
            seconds = time.perf_counter() - started
            if done.returncode != 0:
                fail(f"build failed (javac):\n{done.stdout}\n{done.stderr}")
            for kernel in self.args.kernels:
                binary = os.path.join(
                    self.java_classes, KERNEL_INFO[kernel]["file"] + ".class"
                )
                builds["java"][kernel]["binary_bytes"] = os.path.getsize(binary)
            builds["java"]["_total"] = {"seconds": seconds}

        return builds

    def build_manager(self):
        """`slopium build --release` over the fib project, cold and warm."""
        project = os.path.join(BENCH, "slopium-project", "fib")
        manifest = os.path.join(project, "Slopium.toml")
        target = os.path.join(project, "target")
        lock = os.path.join(project, "Slopium.lock")
        if not self.slopium:
            return {"cold_seconds": None, "warm_seconds": None}
        shutil.rmtree(target, ignore_errors=True)
        if os.path.exists(lock):
            os.remove(lock)
        cold = self.timed(
            [self.slopium, "--manifest-path", manifest, "build", "--release"]
        )
        warm = self.timed(
            [self.slopium, "--manifest-path", manifest, "build", "--release"]
        )
        # The fixture is committed clean; what it builds is output.
        shutil.rmtree(target, ignore_errors=True)
        if os.path.exists(lock):
            os.remove(lock)
        return {"cold_seconds": cold, "warm_seconds": warm}

    # --------------------------------------------------------------- verify

    def run_argv(self, language, kernel):
        info = KERNEL_INFO[kernel]
        if language == "slopium":
            return [os.path.join(self.bin, kernel + "-slopic")]
        if language == "c":
            return [os.path.join(self.bin, kernel + "-c")]
        if language == "rust":
            return [os.path.join(self.rust_target, "release", info["file"])]
        if language == "python":
            return [self.python3, os.path.join(
                BENCH, "python", info["file"] + ".py")]
        if language == "java":
            return [self.java, "-cp", self.java_classes, info["file"]]
        fail(f"unknown language {language}")

    def spawn(self, argv, stdout_fd):
        """Runs one program under the measuring wrapper and answers exit
        code, wall time and peak RSS.

        The wrapper is what makes the RSS honest: every spawn the harness
        could do itself leaves the child inheriting the interpreter's
        resident pages before exec, and `ru_maxrss` keeps that peak. The
        wrapper reports on descriptor 3, because 1 and 2 belong to the
        program under test. """
        if self.taskset:
            argv = [self.taskset, "-c", str(self.args.pin)] + argv
        read_fd, write_fd = os.pipe()
        try:
            pid = os.posix_spawn(
                self.rssrun, [self.rssrun] + argv, os.environ,
                file_actions=[
                    (os.POSIX_SPAWN_DUP2, stdout_fd, 1),
                    (os.POSIX_SPAWN_DUP2, stdout_fd, 2),
                    (os.POSIX_SPAWN_DUP2, write_fd, 3),
                ],
            )
        finally:
            os.close(write_fd)
        with os.fdopen(read_fd, "rb") as pipe:
            report = pipe.read()
        _, status, _ = os.wait4(pid, 0)
        code = os.waitstatus_to_exitcode(status)
        if code != 0:
            fail(f"{' '.join(argv)} exited {code}")
        try:
            label, rss_kb, wall_ms = report.decode().split()
        except ValueError:
            fail(f"the measuring wrapper reported nothing for {argv[0]}")
        if label != "BENCH":
            fail(f"the measuring wrapper reported {report!r} for {argv[0]}")
        return code, float(rss_kb), float(wall_ms)

    def capture(self, argv):
        read_fd, write_fd = os.pipe()
        try:
            pid = os.posix_spawn(
                argv[0], argv, os.environ,
                file_actions=[
                    (os.POSIX_SPAWN_DUP2, write_fd, 1),
                    (os.POSIX_SPAWN_DUP2, write_fd, 2),
                ],
            )
        finally:
            os.close(write_fd)
        chunks = []
        with os.fdopen(read_fd, "rb") as pipe:
            for chunk in pipe:
                chunks.append(chunk)
        _, status, _ = os.wait4(pid, 0)
        code = os.waitstatus_to_exitcode(status)
        if code != 0:
            fail(f"{' '.join(argv)} exited {code}")
        return b"".join(chunks)

    @staticmethod
    def outputs_agree(kernel, reference, candidate):
        if reference == candidate:
            return True
        if KERNEL_INFO[kernel]["verify"] == "float":
            left = extract_float(reference)
            right = extract_float(candidate)
            if left is None or right is None:
                return False
            scale = max(abs(left), abs(right), 1e-300)
            return abs(left - right) / scale <= 1e-9
        return False

    def verify(self):
        log("verifying outputs agree ...")
        agreed = {}
        for kernel in self.args.kernels:
            reference = None
            reference_language = None
            for language in self.args.languages:
                output = self.capture(self.run_argv(language, kernel))
                if reference is None:
                    reference = output
                    reference_language = language
                elif not self.outputs_agree(kernel, reference, output):
                    fail(
                        f"{kernel}: {language} output disagrees with"
                        f" {reference_language}"
                    )
            agreed[kernel] = {
                "reference": reference_language,
                "mode": KERNEL_INFO[kernel]["verify"],
            }
            log(f"  {kernel}: agree ({reference_language} is the reference)")
        return agreed

    # --------------------------------------------------------------- measure

    def measure(self):
        log(
            f"measuring: {self.args.runs} runs, {self.args.warmup} warmup,"
            f" pin={'cpu' + str(self.args.pin) if self.taskset else 'off'}"
        )
        numbers = {}
        devnull = os.open(os.devnull, os.O_WRONLY)
        try:
            for kernel in self.args.kernels:
                numbers[kernel] = {}
                for language in self.args.languages:
                    argv = self.run_argv(language, kernel)
                    for _ in range(self.args.warmup):
                        self.spawn(argv, devnull)
                    seconds = []
                    peaks = []
                    for _ in range(self.args.runs):
                        code, rss_kb, wall_ms = self.spawn(argv, devnull)
                        seconds.append(wall_ms)
                        peaks.append(rss_kb)
                        if code != 0:
                            fail(f"{kernel}/{language} exited {code}")
                    numbers[kernel][language] = {
                        "median_ms": statistics.median(seconds),
                        "min_ms": min(seconds),
                        "stdev_ms": (
                            statistics.stdev(seconds)
                            if len(seconds) > 1 else 0.0
                        ),
                        "rss_kb_median": statistics.median(peaks),
                        "runs": len(seconds),
                    }
                    got = numbers[kernel][language]
                    log(
                        f"  {kernel:<14} {language:<8}"
                        f" {got['median_ms']:>10.1f} ms"
                        f"  rss {got['rss_kb_median'] / 1024:>8.1f} MB"
                    )
        finally:
            os.close(devnull)
        return numbers

    # ---------------------------------------------------------------- report

    def versions(self):
        def first_line(argv):
            try:
                done = subprocess.run(
                    argv, capture_output=True, text=True, timeout=30
                )
                return (done.stdout or done.stderr).strip().splitlines()[0]
            except Exception as error:  # metadata only; never fail the run
                return f"unavailable ({error})"

        versions = {}
        if self.slopic:
            versions["slopic"] = first_line([self.slopic, "--version"])
        if self.slopium:
            versions["slopium"] = first_line([self.slopium, "--version"])
        if self.cc:
            versions["cc"] = first_line([self.cc, "--version"])
        if self.rustc:
            versions["rustc"] = first_line([self.rustc, "--version"])
        if self.python3:
            versions["python"] = first_line([self.python3, "--version"])
        if self.javac:
            versions["javac"] = first_line([self.javac, "-version"])
        return versions

    def metadata(self):
        cpu = "unknown"
        with open("/proc/cpuinfo", encoding="utf-8") as info:
            for line in info:
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
        return {
            "date": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "host": platform.node(),
            "cpu": cpu,
            "cores": os.cpu_count(),
            "kernel": platform.release(),
            "pin": str(self.args.pin) if self.taskset else "off",
            "runs": self.args.runs,
            "warmup": self.args.warmup,
            "tools": self.versions(),
        }


def extract_float(output):
    for line in output.decode(errors="replace").splitlines():
        if "sum" in line:
            try:
                return float(line.split("=", 1)[1].strip())
            except (ValueError, IndexError):
                return None
    return None


def fmt(value, digits=1):
    if value is None:
        return "-"
    return f"{value:.{digits}f}"


def markdown(languages, builds, manager, numbers):
    lines = []
    lines.append("### Runtime (median of runs, ms)")
    lines.append("")
    lines.append("| kernel | " + " | ".join(languages) + " |")
    lines.append("| --- | " + " | ".join(["---:"] * len(languages)) + " |")
    for kernel in KERNELS:
        cells = [fmt(numbers[kernel][l]["median_ms"], 1) for l in languages]
        lines.append(f"| {kernel} | " + " | ".join(cells) + " |")
    lines.append("")
    lines.append("### Peak RSS (median, MB)")
    lines.append("")
    lines.append("| kernel | " + " | ".join(languages) + " |")
    lines.append("| --- | " + " | ".join(["---:"] * len(languages)) + " |")
    for kernel in KERNELS:
        cells = [fmt(numbers[kernel][l]["rss_kb_median"] / 1024.0, 1)
                 for l in languages]
        lines.append(f"| {kernel} | " + " | ".join(cells) + " |")
    lines.append("")
    lines.append("### Artifact size (KB; the binary, or the Python source)")
    lines.append("")
    lines.append("| kernel | " + " | ".join(languages) + " |")
    lines.append("| --- | " + " | ".join(["---:"] * len(languages)) + " |")
    for kernel in KERNELS:
        cells = []
        for l in languages:
            entry = builds.get(l, {}).get(kernel, {})
            size = entry.get("binary_bytes")
            cells.append(fmt(size / 1024.0, 1) if size else "-")
        lines.append(f"| {kernel} | " + " | ".join(cells) + " |")
    lines.append("")
    lines.append("### Compile time (s, cold)")
    lines.append("")
    lines.append("| kernel | " + " | ".join(languages) + " |")
    lines.append("| --- | " + " | ".join(["---:"] * len(languages)) + " |")
    for kernel in KERNELS:
        cells = []
        for l in languages:
            entry = builds.get(l, {}).get(kernel, {})
            seconds = entry.get("release_seconds", entry.get("seconds"))
            cells.append(fmt(seconds, 2) if seconds is not None else "-")
        lines.append(f"| {kernel} | " + " | ".join(cells) + " |")
    for l in languages:
        total = builds.get(l, {}).get("_total", {}).get("seconds")
        if total is not None:
            lines.append("")
            lines.append(
                f"All seven {l} kernels are built by one invocation, which"
                f" took {total:.2f} s in total."
            )
    if manager["cold_seconds"] is not None:
        lines.append("")
        lines.append(
            f"`slopium build --release` over `bench/slopium-project/fib`:"
            f" cold {manager['cold_seconds']:.2f} s,"
            f" warm {manager['warm_seconds']:.2f} s."
        )
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(
        description="Build, verify and measure the Slopium benchmark kernels."
    )
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--quick", action="store_true",
                        help="three runs instead of ten")
    parser.add_argument("--languages", default=",".join(LANGUAGES))
    parser.add_argument("--kernels", default=",".join(KERNELS))
    parser.add_argument("--pin", type=int, default=2,
                        help="CPU core to pin every run to")
    parser.add_argument("--no-pin", action="store_true")
    parser.add_argument("--outdir", default=os.path.join(BENCH, "results"))
    parser.add_argument("--slopic", help="path to the release slopic")
    parser.add_argument("--slopium", help="path to the release slopium")
    args = parser.parse_args()
    if args.quick:
        args.runs = min(args.runs, 3)
    args.languages = [l for l in args.languages.split(",") if l]
    args.kernels = [k for k in args.kernels.split(",") if k]
    unknown = set(args.kernels) - set(KERNELS)
    if unknown:
        fail(f"unknown kernels: {', '.join(sorted(unknown))}")
    unknown = set(args.languages) - set(LANGUAGES)
    if unknown:
        fail(f"unknown languages: {', '.join(sorted(unknown))}")
    if not args.no_pin:
        args.pin = min(args.pin, (os.cpu_count() or 1) - 1)

    harness = Harness(args)
    builds = harness.build()
    agreed = harness.verify()
    numbers = harness.measure()
    manager = harness.build_manager()

    document = {
        "meta": harness.metadata(),
        "build": builds,
        "manager": manager,
        "verify": agreed,
        "runs": numbers,
    }

    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    json_path = os.path.join(harness.results, stamp + ".json")
    with open(json_path, "w", encoding="utf-8") as out:
        json.dump(document, out, indent=2)
        out.write("\n")
    table = markdown(args.languages, builds, manager, numbers)
    md_path = os.path.join(harness.results, stamp + ".md")
    with open(md_path, "w", encoding="utf-8") as out:
        out.write(table)
    log(f"wrote {json_path}")
    log(f"wrote {md_path}")
    print(table, flush=True)


if __name__ == "__main__":
    main()
