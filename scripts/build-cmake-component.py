#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Cache an independently built CMake component before SDK publication."""

import argparse
import hashlib
import importlib.util
import json
import os
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


def mirror(source, destination):
    """Keep compiler-visible SDK paths and unchanged dependency mtimes stable."""
    expected = set()
    if source.exists():
        for base, directories, files in os.walk(source, followlinks=True):
            directories[:] = [name for name in directories if name not in state.EXCLUDED]
            for name in files:
                if name in state.EXCLUDED:
                    continue
                path = Path(base) / name
                relative = path.relative_to(source)
                expected.add(relative)
                target = destination / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                if not target.is_file() or path.read_bytes() != target.read_bytes():
                    shutil.copyfile(path, target)
                target.chmod(path.stat().st_mode & 0o777)
    if destination.exists():
        for path in destination.rglob('*'):
            if path.is_file() and path.relative_to(destination) not in expected:
                path.unlink()


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
    configuration = {**requested, 'source': requested['source'][0]}
    configuration.pop('dependencies')
    configuration_key = hashlib.sha256(state.encode(configuration)).hexdigest()
    persistent = args.cache / 'work' / configuration_key
    build_directory = persistent / 'build'
    work_record = persistent / 'work.json'
    names = [sorted(requested['source'][1]), *[sorted(tree) for tree in requested['dependencies']]]
    try:
        previous = json.loads(work_record.read_bytes())
        reusable = previous['names'] == names and previous['outputs'] == state.tree(build_directory)
    except (OSError, ValueError, KeyError):
        reusable = False
    if not reusable and build_directory.exists():
        # New search-path candidates can shadow headers that an old compiler
        # depfile never named. Also reject corrupted retained build products.
        shutil.rmtree(build_directory)
    sdk = persistent / 'sysroot'
    source = persistent / 'source'
    mirror(args.source, source)
    replacements = [(str(args.output), str(sdk)), (str(args.source), str(source))]
    for index, dependency in enumerate(args.dependency):
        try:
            relative = dependency.relative_to(args.output)
        except ValueError:
            target = persistent / 'dependencies' / str(index)
            mirror(dependency, target)
            replacements.append((str(dependency), str(target)))
            continue
        mirror(dependency, sdk / relative)
    configure = list(args.configure)
    for original, replacement in sorted(replacements, key=lambda pair: len(pair[0]), reverse=True):
        configure = [value.replace(original, replacement) for value in configure]
    with tempfile.TemporaryDirectory(prefix='.build-', dir=args.cache) as temporary:
        root = Path(temporary)
        subprocess.run(['cmake', '-S', str(source), '-B', str(persistent / 'build'),
                        *configure], check=True)
        subprocess.run(['cmake', '--build', str(persistent / 'build')], check=True)
        staged = root / 'result'
        subprocess.run(['cmake', '--install', str(persistent / 'build'), '--prefix',
                        str(staged / 'install')], check=True)
        if identity(args) != requested:
            shutil.rmtree(persistent)
            raise RuntimeError('component inputs changed during the build; retry')
        work_record.write_bytes(state.encode({'names': names, 'outputs': state.tree(build_directory)}))
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
