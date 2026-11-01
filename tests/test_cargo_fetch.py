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
    def test_module_architectures_reenter_the_source_workspace_frontend(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            view = root / 'generated/aarch64'
            script = view / 'sdk/toolchain/tests/module.sh'
            script.parent.mkdir(parents=True)
            shutil.copyfile(Path(__file__).with_name('module.sh'), script)
            source = root / 'source workspace'
            source.mkdir()
            log = root / 'calls.jsonl'
            binary = root / 'python3'
            binary.write_text(f'#!{sys.executable}\nimport json,os,sys\n'
                              f'with open({str(log)!r},"a") as f: f.write(json.dumps('
                              '{"args":sys.argv[1:],"arch":os.getenv("ARCH")})+"\\n")\n')
            binary.chmod(0o755)
            subprocess.run(['sh', str(script)], check=True,
                           env={**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                                'HYPER_WORKSPACE': str(source), 'SDK_CI_ARCH': 'aarch64 riscv64'})
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            runs = [call for call in calls if call['args'][0] == str(source / 'project.py')]
            self.assertEqual([call['arch'] for call in runs], ['aarch64', 'riscv64'])
            for call in runs:
                self.assertEqual(call['args'][1:], ['--workspace', str(source), 'exec', '--',
                                                   'make', 'sdk-check', 'app-check', 'app-sdk-test'])

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
