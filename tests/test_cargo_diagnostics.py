# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

import importlib.util
import io
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).resolve().parents[1] / 'bin/hyper-cargo-diagnostics.py'
spec = importlib.util.spec_from_file_location('cargo_diagnostics', HELPER)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CargoDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='cargo-diagnostics-')
        self.addCleanup(self.temporary.cleanup)
        self.sdk = Path(self.temporary.name).resolve() / 'sdk with spaces'
        self.environment = {**os.environ, 'HYPER_SYSROOT': str(self.sdk)}
        self.environment.pop('HYPER_CARGO_PATCH_DIAGNOSTICS', None)

    def warning(self, name='hyper-os', path=None):
        if path is None:
            path = self.sdk / 'share/hyper/rust' / name
        return f'warning: patch `{name} v0.0.0 ({path})` was not used in the crate graph\n'.encode()

    def compact(self, data):
        result = io.BytesIO()
        module.compact(io.BytesIO(data), result, self.sdk)
        return result.getvalue()

    def test_repeated_known_sdk_warnings_have_one_informative_summary(self):
        data = self.warning() + self.warning('hyper-rt') + b'\n'.join(module.HELP) + b'\n'
        result = self.compact(data * 3)
        self.assertEqual(result.count(b'warning:'), 1)
        self.assertIn(b'hyper-os, hyper-rt', result)
        self.assertIn(b'versions/features and Cargo.lock', result)
        self.assertIn(b'HYPER_CARGO_PATCH_DIAGNOSTICS=1', result)

    def test_other_warnings_errors_and_machine_readable_records_are_unchanged(self):
        data = (self.warning('third-party') + self.warning(path='/different/sdk/hyper-os')
                + b'warning: unused variable\nerror: type mismatch\n{"reason":"compiler-message"}\n')
        self.assertEqual(self.compact(data), data)

    def test_mixed_patch_help_and_partial_unknown_help_are_preserved(self):
        for extra in (self.warning('external') + self.warning(),
                      self.warning() + self.warning('external')):
            help_text = b'\n'.join(module.HELP) + b'\n'
            result = self.compact(extra + help_text)
            self.assertIn(help_text, result)
            self.assertIn(self.warning('external'), result)
        partial = module.HELP[0] + b'\n' + b'help: new Cargo explanation\n'
        self.assertIn(partial, self.compact(self.warning() + partial))

    def test_colored_known_diagnostics_do_not_rewrite_other_ansi_text(self):
        warning = b'\x1b[33m' + self.warning().rstrip(b'\n') + b'\x1b[0m\n'
        other = b'\x1b[31merror: actual failure\x1b[0m\n'
        result = self.compact(warning + other)
        self.assertIn(other, result)
        self.assertEqual(result.count(b'warning:'), 1)

    def run_driver(self, script, **environment):
        return subprocess.run([sys.executable, str(HELPER), sys.executable, '-c', script],
                              env={**self.environment, **environment}, capture_output=True)

    def test_stdout_and_nonzero_exit_status_are_preserved(self):
        result = self.run_driver('import sys; print("JSON stdout"); '
                                 'print("error: compiler failure", file=sys.stderr); sys.exit(37)')
        self.assertEqual(result.returncode, 37)
        self.assertEqual(result.stdout, b'JSON stdout\n')
        self.assertEqual(result.stderr, b'error: compiler failure\n')

    def test_verbose_mode_preserves_original_warning_bytes(self):
        warning = self.warning()
        result = self.run_driver(f'import sys; sys.stderr.buffer.write({warning!r})',
                                 HYPER_CARGO_PATCH_DIAGNOSTICS='1')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, warning)

    def test_child_signal_exit_is_preserved(self):
        result = self.run_driver('import os,signal; os.kill(os.getpid(), signal.SIGTERM)')
        self.assertEqual(result.returncode, -signal.SIGTERM)

    def test_parent_signal_is_forwarded_and_child_reaped(self):
        process = subprocess.Popen(
            [sys.executable, str(HELPER), sys.executable, '-c',
             'import os,time; print(os.getpid(), flush=True); time.sleep(60)'],
            env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True)
        try:
            child = int(process.stdout.readline())
            process.send_signal(signal.SIGTERM)
            process.communicate(timeout=5)
            self.assertEqual(process.returncode, -signal.SIGTERM)
            with self.assertRaises(ProcessLookupError):
                os.kill(child, 0)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate(timeout=5)


if __name__ == '__main__':
    unittest.main()
