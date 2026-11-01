# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""An offline Native build needs both application and compiler-runtime inputs."""

import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class CargoFetchTests(unittest.TestCase):
    def test_module_consumers_use_the_selected_view_without_reentering_preparation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'calls.jsonl'
            for name in ('python3', 'make'):
                binary = root / name
                binary.write_text(f'#!{sys.executable}\nimport json,os,sys\n'
                                  f'with open({str(log)!r},"a") as f: f.write(json.dumps('
                                  '{"driver":os.path.basename(sys.argv[0]),"args":sys.argv[1:],'
                                  '"arch":os.getenv("ARCH"),"cwd":os.getcwd()})+"\\n")\n')
                binary.chmod(0o755)
            for arch in ('aarch64', 'riscv64'):
                with self.subTest(architecture=arch):
                    view = root / 'generated' / arch
                    script = view / 'sdk/toolchain/tests/module.sh'
                    script.parent.mkdir(parents=True)
                    shutil.copyfile(Path(__file__).with_name('module.sh'), script)
                    log.unlink(missing_ok=True)
                    subprocess.run(['sh', str(script)], check=True,
                                   env={**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                                        'HYPER_WORKSPACE': str(root / 'source'), 'ARCH': arch}, timeout=10)
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    runs = [call for call in calls if call['driver'] == 'make']
                    self.assertEqual(runs, [{'driver': 'make', 'arch': arch, 'cwd': str(view.resolve()),
                                            'args': ['sdk-check', 'app-check', 'app-sdk-test']}])
                    self.assertFalse(any('project.py' in arg for call in calls for arg in call['args']))

    def test_fetch_owns_runtime_lock_and_preserves_offline_requests(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = root / 'sdk with spaces'
            for relative in ('bin/hyper-clang', 'share/hyper/abi/Cargo.toml',
                             *('share/hyper/rust/' + name + '/Cargo.toml'
                               for name in ('hyper-os', 'hyper-filesystem', 'hyper-rt', 'hyper-sys'))):
                path = sdk / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('')
            (sdk / 'share/hyper/manifest').write_text('architecture=aarch64\n')
            log = root / 'calls.jsonl'
            cargo = root / 'cargo-driver'
            cargo.write_text(f'#!{sys.executable}\nimport json,os,sys\n'
                             f'with open({str(log)!r},"a") as f: f.write(json.dumps('
                             '{"args":sys.argv[1:],"offline":os.getenv("CARGO_NET_OFFLINE"),'
                             '"bootstrap":os.getenv("RUSTC_BOOTSTRAP")})+"\\n")\n')
            cargo.chmod(0o755)
            rustc = root / 'rustc'
            rustc.write_text(f'#!{sys.executable}\nprint({str(root / "compiler")!r})\n')
            rustc.chmod(0o755)
            driver = Path(__file__).resolve().parents[1] / 'bin/hyper-cargo'
            for runtime in ('0', '1'):
                for flag in (None, '--offline', '--frozen'):
                    log.unlink(missing_ok=True)
                    environment = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                                   'HYPER_SYSROOT': str(sdk), 'HYPER_CARGO_DRIVER': str(cargo),
                                   'HYPER_ARCH': 'aarch64', 'HYPER_RUST_STD': runtime, 'CARGO_NET_OFFLINE': 'false'}
                    command = [str(driver), 'fetch', '--manifest-path', str(root / 'app/Cargo.toml')]
                    if flag:
                        command.append(flag)
                    subprocess.run(command, env=environment, check=True)
                    calls = [json.loads(line) for line in log.read_text().splitlines()]
                    self.assertEqual(len(calls), 2)
                    self.assertIn(str(root / 'app/Cargo.toml'), calls[0]['args'])
                    manifest = (sdk / 'share/hyper/rust-src/library/Cargo.toml' if runtime == '1'
                                else root / 'compiler/lib/rustlib/src/rust/library/Cargo.toml')
                    self.assertEqual(calls[1]['args'], ['fetch', '--locked', '--manifest-path', str(manifest)])
                    self.assertEqual(calls[1]['offline'], 'true' if flag else 'false')
                    self.assertEqual(calls[1]['bootstrap'], '1')


if __name__ == '__main__':
    unittest.main()
