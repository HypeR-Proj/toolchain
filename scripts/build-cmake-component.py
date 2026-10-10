#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Cache an independently built CMake component before SDK publication."""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


spec = importlib.util.spec_from_file_location('sysroot_state', Path(__file__).with_name('sysroot-state.py'))
state = importlib.util.module_from_spec(spec)
spec.loader.exec_module(state)


def identity(args):
    inputs = json.loads(args.inputs.read_bytes())
    return {
        'builder': state.digest(__file__),
        'tools': inputs['tools'],
        'platform': inputs['platform'],
        'environment': {key: value for key, value in inputs['environment'].items()
                        if key not in ('HYPER_SDK_VERSION', 'HYPER_SDK_SOURCE_REVISION')},
        'source': [str(args.source.resolve()), state.tree(args.source)],
        'dependencies': [state.tree(path) for path in args.dependency],
        # The temporary publication directory is an input location, not a
        # component identity. Its relevant contents are captured above.
        'configure': [value.replace(str(args.output), '{sysroot}') for value in args.configure],
    }


def restore(directory, output):
    for source in sorted(directory.rglob('*')):
        if source.is_file():
            destination = output / source.relative_to(directory)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)


def build(args):
    requested = identity(args)
    key = hashlib.sha256(state.encode(requested)).hexdigest()
    cached = args.cache / key
    try:
        record = json.loads((cached / 'record.json').read_bytes())
        if record['inputs'] == requested and record['outputs'] == state.tree(cached / 'install'):
            restore(cached / 'install', args.output)
            print(f'Native component is up to date: {args.source}')
            return
    except (OSError, ValueError, KeyError):
        pass
    # A corrupt cache is never used as a successful build. The enclosing SDK
    # publisher owns the lock; no installed SDK is modified until it commits.
    if cached.exists():
        shutil.rmtree(cached)
    args.cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.build-', dir=args.cache) as temporary:
        root = Path(temporary)
        subprocess.run(['cmake', '-S', str(args.source), '-B', str(root / 'build'),
                        *args.configure], check=True)
        subprocess.run(['cmake', '--build', str(root / 'build')], check=True)
        staged = root / 'result'
        subprocess.run(['cmake', '--install', str(root / 'build'), '--prefix',
                        str(staged / 'install')], check=True)
        if identity(args) != requested:
            raise RuntimeError('component inputs changed during the build; retry')
        (staged / 'record.json').write_bytes(state.encode({
            'inputs': requested, 'outputs': state.tree(staged / 'install')}))
        staged.rename(cached)
    restore(cached / 'install', args.output)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'cache', 'output', 'inputs'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--dependency', type=Path, action='append', default=[])
    parser.add_argument('configure', nargs=argparse.REMAINDER)
    arguments = parser.parse_args()
    if arguments.configure[:1] == ['--']:
        arguments.configure.pop(0)
    build(arguments)
