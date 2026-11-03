#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Compact only Cargo's repeated installed-SDK unused-patch diagnostics."""

import os
from pathlib import Path
import re
import signal
import subprocess
import sys


ANSI = re.compile(rb'\x1b\[[0-9;]*m')
PATCH = re.compile(rb'^warning: patch `(hyper-(?:abi|os|filesystem|rt|sys)) v[^ ]+ \((.*)\)` was not used in the crate graph$')
HELP = (
    b'help: Check that the patched package version and available features are compatible',
    b'      with the dependency requirements. If the patch has a different version from',
    b'      what is locked in the Cargo.lock file, run `cargo update` to use the new',
    b'      version. This may also occur with an optional dependency that is not enabled.',
)


def compact(source, destination, sdk):
    paths = {b'hyper-abi': sdk / 'share/hyper/abi'}
    paths.update({name.encode(): sdk / 'share/hyper/rust' / name
                  for name in ('hyper-os', 'hyper-filesystem', 'hyper-rt', 'hyper-sys')})
    paths = {name: os.fsencode(path.resolve()) for name, path in paths.items()}
    omitted = set()
    pending = []
    eligible_help = False
    mixed_patches = False
    for original in source:
        line = ANSI.sub(b'', original).rstrip(b'\r\n')
        match = PATCH.fullmatch(line)
        if not pending and match and paths.get(match[1]) == match[2]:
            omitted.add(match[1])
            eligible_help = not mixed_patches
            continue
        if eligible_help and line == HELP[len(pending)]:
            pending.append(original)
            if len(pending) == len(HELP):
                pending.clear()
                eligible_help = False
            continue
        # Unknown Cargo text and partial help blocks remain byte-for-byte visible.
        for buffered in pending:
            destination.write(buffered)
        pending.clear()
        eligible_help = False
        if line.startswith(b"warning: patch `"):
            mixed_patches = True
        else:
            mixed_patches = False
        destination.write(original)
        destination.flush()
    for buffered in pending:
        destination.write(buffered)
    if omitted:
        destination.write(b'warning: unused SDK patches: ' + b', '.join(sorted(omitted))
                          + b'; check requested versions/features and Cargo.lock'
                          + b' (HYPER_CARGO_PATCH_DIAGNOSTICS=1 shows original warnings)\n')
    destination.flush()


def main():
    command = sys.argv[1:]
    if not command:
        raise SystemExit('usage: hyper-cargo-diagnostics.py CARGO [ARGUMENT ...]')
    if os.environ.get('HYPER_CARGO_PATCH_DIAGNOSTICS') == '1':
        os.execvp(command[0], command)
    environment = dict(os.environ)
    if sys.stderr.isatty() and 'CARGO_TERM_COLOR' not in environment:
        environment['CARGO_TERM_COLOR'] = 'always'
    process = None
    pending_signals = set()
    previous = {}

    def forward(signum, _frame):
        if process is None:
            pending_signals.add(signum)
            return
        try:
            process.send_signal(signum)
        except ProcessLookupError:
            pass

    # Install before spawning so an interruption cannot orphan a newly started
    # Cargo process during the parent/child setup window.
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        previous[signum] = signal.signal(signum, forward)
    try:
        process = subprocess.Popen(command, stderr=subprocess.PIPE, env=environment)
        for signum in pending_signals:
            forward(signum, None)
        with process.stderr:
            compact(process.stderr, sys.stderr.buffer, Path(os.environ['HYPER_SYSROOT']))
        status = process.wait()
    except BaseException:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        raise
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    if status < 0:
        signum = -status
        signal.signal(signum, signal.SIG_DFL)
        os.kill(os.getpid(), signum)
        return 128 + signum
    return status


if __name__ == '__main__':
    try:
        sys.exit(main())
    except OSError as error:
        print(f'hyper-cargo diagnostics: {error}', file=sys.stderr)
        sys.exit(127)
