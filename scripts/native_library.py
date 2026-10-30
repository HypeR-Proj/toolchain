# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Content-checked translation units, independent of SDK publication directories."""

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def write(path, content):
    if path.is_file() and path.read_bytes() == content:
        return
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        stream.write(content)
    try:
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def tool(name):
    path = Path(shutil.which(name) or name).resolve()
    return {'path': str(path), 'sha256': digest(path)}


def dependencies(path):
    body = path.read_text().replace('\\\n', '').split(':', 1)[1]
    return [Path(os.path.normpath(os.path.abspath(name.replace('$$', '$')))) for name in shlex.split(body)]


def snapshot(directories, sources):
    paths = set(sources)
    for directory in directories:
        paths.update(path for path in directory.rglob('*') if path.is_file())
    return {str(path.absolute()): digest(path) for path in sorted(paths)}


def build_archive(root, sdk, output, filename, flags, sources, includes, *, jobs=4, extra=None):
    root, sdk, output = root.absolute(), sdk.absolute(), output.absolute()
    output.mkdir(parents=True, exist_ok=True)
    substitutions = sorted([(str(sdk), '{sdk}'), (str(root), '{source}'), (str(output), '{output}')],
                           key=lambda pair: len(pair[0]), reverse=True)

    def normalize(value):
        for location, label in substitutions:
            value = value.replace(location, label)
        return value

    def current(value):
        for location, label in substitutions:
            value = value.replace(label, location)
        return Path(value)

    compiler = tool(flags[0])
    resource = Path(subprocess.check_output([flags[0], '-print-resource-dir'], text=True).strip()) / 'include'
    directories = sorted(set([*includes, *[path.parent for path in sources], resource]))
    before = snapshot(directories, sources)
    names = sorted(normalize(path) for path in before)
    search_identity = hashlib.sha256(encoded(names)).hexdigest()
    # Debug data and __FILE__ must not embed a new temporary SDK path each run.
    flags = [*flags, f'-ffile-prefix-map={sdk}=/hyper/sdk',
             f'-ffile-prefix-map={root}=/hyper/source/{root.name}',
             f'-ffile-prefix-map={output}=/hyper/build']
    recipe = {'compiler': compiler, 'flags': [normalize(arg) for arg in flags],
              'search': search_identity, 'builder': digest(__file__)}

    def compile_source(source):
        key = hashlib.sha256(normalize(str(source)).encode()).hexdigest()
        obj, record = output / (key + '.o'), output / (key + '.json')
        additional = extra(source) if extra else []
        requested = {**recipe, 'extra': additional, 'source': normalize(str(source))}
        try:
            saved = json.loads(record.read_bytes())
            if (saved['recipe'] == requested and digest(obj) == saved['object']
                    and all(before.get(str(current(path))) == identity
                            for path, identity in saved['dependencies'].items())):
                return obj
        except (OSError, ValueError, KeyError):
            pass
        with tempfile.TemporaryDirectory(prefix='.object-', dir=output) as temporary:
            temporary = Path(temporary)
            built, depfile = temporary / 'unit.o', temporary / 'unit.d'
            subprocess.run([*flags, *additional, '-MD', '-MF', str(depfile), '-MT', 'object',
                            '-c', str(source), '-o', str(built)], check=True)
            used = dependencies(depfile)
            # Unknown external includes are usable but are not cached. A change
            # to a captured input during compilation must never publish success.
            cacheable = all(str(path) in before for path in used)
            if any(str(path) in before and digest(path) != before[str(path)] for path in used):
                raise RuntimeError('native compilation inputs changed; retry')
            data = {'recipe': requested, 'object': digest(built),
                    'dependencies': {normalize(str(path)): digest(path) for path in used}}
            built.replace(obj)
            if cacheable:
                write(record, encoded(data))
            else:
                record.unlink(missing_ok=True)
        return obj

    with ThreadPoolExecutor(max_workers=jobs) as pool:
        objects = list(pool.map(compile_source, sources))
    if snapshot(directories, sources) != before:
        # Drop receipts for units compiled during a moving source snapshot.
        for record in output.glob('*.json'):
            record.unlink()
        raise RuntimeError('native library inputs changed during the build; retry')
    archiver = os.environ.get('LLVM_AR', str(Path(shutil.which(flags[0]) or flags[0]).with_name('llvm-ar')))
    archive, record = output / filename, output / (filename + '.json')
    requested = {'archiver': tool(archiver), 'objects': [(obj.name, digest(obj)) for obj in objects]}
    # JSON decodes tuples as arrays; keep the persisted comparison canonical.
    requested = json.loads(encoded(requested))
    try:
        saved = json.loads(record.read_bytes())
        if saved['inputs'] == requested and saved['archive'] == digest(archive):
            return archive
    except (OSError, ValueError, KeyError):
        pass
    with tempfile.TemporaryDirectory(prefix='.archive-', dir=output) as temporary:
        built = Path(temporary) / filename
        subprocess.run([archiver, 'rcs', str(built), *map(str, objects)], check=True,
                       env={**os.environ, 'ZERO_AR_DATE': '1'})
        write(archive, built.read_bytes())
    write(record, encoded({'inputs': requested, 'archive': digest(archive)}))
    return archive
