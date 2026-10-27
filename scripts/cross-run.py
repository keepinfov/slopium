#!/usr/bin/env python3
"""Capture one cross-check execution without confusing a signal with an exit."""

import argparse
import errno
import math
import os
from pathlib import Path
import resource
import signal
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("a command and a positive finite timeout are required")

    # Fault injection must not leave core dumps in the shared source tree.
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    output = args.output
    with output.open("wb") as stdout, Path(str(output) + ".stderr").open("wb") as stderr:
        try:
            child = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                     start_new_session=True)
        except OSError as error:
            status = 127 if error.errno == errno.ENOENT else 126
            outcome = f"could not start: {error}"
            stderr.write((str(error) + "\n").encode())
        else:
            try:
                status = child.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                # The process may have children; stop its entire private group.
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
                status = 124
                outcome = f"timed out after {args.timeout:g} seconds"
            else:
                outcome = "exit"
                if status < 0:
                    number = -status
                    try:
                        name = signal.Signals(number).name
                    except ValueError:
                        # Intermediate realtime signals have no enum member.
                        name = str(number)
                    outcome = f"signal {name} ({number})"
                    status = 128 + number
        stdout.write(f"exit status: {status}\n".encode())
    Path(str(output) + ".status").write_text(f"{status}\n")
    Path(str(output) + ".outcome").write_text(outcome + "\n")


if __name__ == "__main__":
    main()
