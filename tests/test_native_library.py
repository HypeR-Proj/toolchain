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
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
from native_library import build_archive, digest


class LibraryCacheTests(unittest.TestCase):
    def test_staging_relocation_precise_rebuilds_corruption_and_failure(self):
        compiler = shutil.which('clang')
        archiver = shutil.which('llvm-ar') or shutil.which('ar')
        self.assertIsNotNone(compiler)
        self.assertIsNotNone(archiver)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source, output = root / 'source', root / 'objects'
            source.mkdir()
            a, b = source / 'a.c', source / 'b.c'
            a.write_text('#include "api.h"\nint a(void) { return VALUE; }\n')
            b.write_text('int b(void) { return 7; }\n')
            sdk = root / 'first-sdk'
            (sdk / 'include').mkdir(parents=True)
            (sdk / 'include/api.h').write_text('#define VALUE 1\n')
            log, wrapper = root / 'commands', root / 'compiler'
            wrapper.write_text(f'#!{sys.executable}\nimport os,sys\n'
                               f'if "-c" in sys.argv:\n    with open({str(log)!r}, "a") as stream: stream.write(sys.argv[sys.argv.index("-c")+1]+"\\n")\n'
                               f'os.execv({compiler!r}, [{compiler!r}, *sys.argv[1:]])\n')
            wrapper.chmod(0o755)

            def build(directory):
                with patch.dict(os.environ, {'LLVM_AR': archiver}):
                    return build_archive(source, directory, output, 'libsample.a',
                                         [str(wrapper), '-g', '-O1', '-I', str(directory / 'include')],
                                         [a, b], [directory / 'include'], jobs=2)

            archive = build(sdk)
            original_time = archive.stat().st_mtime_ns
            second = root / 'another-publication-sdk'
            shutil.copytree(sdk, second)
            build(second)
            self.assertEqual(len(log.read_text().splitlines()), 2)
            self.assertEqual(archive.stat().st_mtime_ns, original_time)
            timestamp = a.stat().st_mtime_ns
            a.write_text('#include "api.h"\nint a(void) { return VALUE + 1; }\n')
            os.utime(a, ns=(timestamp, timestamp))
            build(second)
            self.assertEqual(log.read_text().splitlines()[-1], str(a))
            self.assertEqual(len(log.read_text().splitlines()), 3)
            (second / 'include/api.h').write_text('#define VALUE 8\n')
            build(second)
            self.assertEqual(len(log.read_text().splitlines()), 4)
            self.assertEqual(log.read_text().splitlines()[-1], str(a))
            receipt = next(path for path in output.glob('*.json')
                           if json.loads(path.read_bytes()).get('recipe', {}).get('source') == '{source}/b.c')
            receipt.with_suffix('.o').write_bytes(b'corrupted object')
            build(second)
            self.assertEqual(len(log.read_text().splitlines()), 5)
            self.assertEqual(log.read_text().splitlines()[-1], str(b))
            expected = digest(archive)
            archive.write_bytes(b'corrupted archive')
            build(second)
            self.assertEqual(digest(archive), expected)
            self.assertEqual(len(log.read_text().splitlines()), 5)
            a.write_text('invalid C syntax!\n')
            with self.assertRaises(subprocess.CalledProcessError):
                build(second)
            self.assertEqual(digest(archive), expected)


if __name__ == '__main__':
    unittest.main()
