# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

BUILDER = Path(__file__).resolve().parents[1] / 'scripts/build-cmake-component.py'


class CMakeIncrementalTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('cmake') and shutil.which('clang'), 'requires CMake and Clang')
    def test_relocation_precise_rebuild_header_shadowing_and_corruption(self):
        with tempfile.TemporaryDirectory(prefix='hyper cmake ') as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            (source / 'CMakeLists.txt').write_text('''cmake_minimum_required(VERSION 3.16)
project(cache_probe C)
add_library(probe STATIC alpha.c beta.c)
target_include_directories(probe PRIVATE "${DEPS}")
install(TARGETS probe ARCHIVE DESTINATION lib)
''')
            alpha = source / 'alpha.c'
            alpha.write_text('#include "value.h"\nint alpha(void) { return VALUE; }\n')
            (source / 'beta.c').write_text('int beta(void) { return 42; }\n')
            inputs = root / 'inputs.json'
            inputs.write_text(json.dumps({'tools': {}, 'platform': 'test', 'environment': {}}))
            cache = root / 'cache'

            def build(number):
                output = root / f'sdk-{number}'
                include = output / 'include'
                include.mkdir(parents=True)
                (include / 'value.h').write_text('#define VALUE 7\n')
                command = [sys.executable, str(BUILDER), '--source', str(source),
                           '--cache', str(cache), '--output', str(output), '--inputs', str(inputs),
                           '--dependency', str(include), '--', f'-DDEPS={include}',
                           '-DCMAKE_C_COMPILER=' + shutil.which('clang')]
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                objects = {path.name: path for path in (cache / 'work').rglob('*.c.o')}
                return objects, {name: path.stat().st_mtime_ns for name, path in objects.items()}

            objects, first = build(1)
            self.assertEqual(set(first), {'alpha.c.o', 'beta.c.o'})
            self.assertEqual(build(2)[1], first)
            timestamp = alpha.stat()
            alpha.write_text('#include "value.h"\nint alpha(void) { return VALUE + 1; }\n')
            os.utime(alpha, ns=(timestamp.st_atime_ns, timestamp.st_mtime_ns))
            _, second = build(3)
            self.assertNotEqual(first['alpha.c.o'], second['alpha.c.o'])
            self.assertEqual(first['beta.c.o'], second['beta.c.o'])
            # A new local header shadows an SDK header absent from old depfiles.
            (source / 'value.h').write_text('#define VALUE 99\n')
            objects, third = build(4)
            self.assertNotEqual(second['beta.c.o'], third['beta.c.o'])
            objects['beta.c.o'].write_bytes(b'corrupt object')
            alpha.write_text('#include "value.h"\nint alpha(void) { return VALUE + 2; }\n')
            objects, _ = build(5)
            self.assertNotEqual(objects['beta.c.o'].read_bytes(), b'corrupt object')


if __name__ == '__main__':
    unittest.main()
