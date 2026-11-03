#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 roolrz
# SPDX-License-Identifier: Apache-2.0

"""Real process locks, crash recovery and publisher/recoverer exclusion."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


HELPER = Path(__file__).resolve().parents[1] / 'scripts/sysroot-publication.py'


class PublicationOwnership(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='publication-ownership-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.output = self.root / 'sdk'
        self.lock = Path(str(self.output) + '.publish-lock')

    def run_helper(self, action, *arguments):
        return subprocess.run([sys.executable, str(HELPER), action, str(self.output),
                               *map(str, arguments)], capture_output=True, text=True)

    def owner(self, body):
        process = subprocess.Popen(
            [sys.executable, str(HELPER), 'run', str(self.output), 'sh', '-c', body,
             'publisher', sys.executable, str(HELPER), str(self.output), str(self.root)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            start_new_session=True)

        def cleanup():
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.communicate(timeout=5)

        self.addCleanup(cleanup)
        return process

    def wait_marker(self, name):
        for _ in range(500):
            if (self.root / name).exists():
                return
            time.sleep(0.01)
        self.fail('publisher did not reach the expected phase')

    def test_active_owner_is_inspectable_and_cannot_be_recovered(self):
        process = self.owner('"$1" "$2" verify "$3" && touch "$4/ready"; exec sleep 60')
        self.wait_marker('ready')
        result = self.run_helper('inspect')
        self.assertEqual(result.returncode, 0, result.stderr)
        state = json.loads(result.stdout)
        self.assertEqual(state['state'], 'active')
        self.assertEqual(state['owner']['pid'], process.pid)
        self.assertEqual(state['owner']['output'], str(self.output))
        self.assertNotEqual(self.run_helper('recover').returncode, 0)
        self.assertNotEqual(self.run_helper('run', 'true').returncode, 0)
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)
        self.assertEqual(json.loads(self.run_helper('inspect').stdout)['state'], 'stale')
        self.assertEqual(self.run_helper('recover').returncode, 0)
        self.assertFalse(self.lock.exists())

    def test_crash_between_renames_restores_previous_and_preserves_staging(self):
        process = self.owner('set -e; transaction=$(mktemp -d "$4/.hyper-sysroot.XXXXXX"); '
                             '"$1" "$2" record "$3" "$transaction"; '
                             'mkdir "$transaction/previous" "$transaction/sysroot"; '
                             'echo sentinel > "$transaction/previous/sentinel"; '
                             'touch "$4/ready"; exec sleep 60')
        self.wait_marker('ready')
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)
        result = self.run_helper('recover')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.output / 'sentinel').read_text().strip(), 'sentinel')
        staging = Path(json.loads(result.stdout)['preserved_transaction'])
        self.assertTrue((staging / 'sysroot').is_dir())
        self.assertFalse(self.lock.exists())

    def test_normal_release_leaves_stable_guard_inode(self):
        process = self.owner('set -e; "$1" "$2" verify "$3"; '
                             '"$1" "$2" release "$3"; :')
        _, stderr = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0, stderr)
        self.assertFalse(self.lock.exists())
        guard = Path(str(self.output) + '.publish-guard')
        inode = guard.stat().st_ino
        self.assertEqual(json.loads(self.run_helper('inspect').stdout)['state'], 'idle')
        self.assertEqual(guard.stat().st_ino, inode)

    def test_legacy_or_malformed_owner_is_never_deleted(self):
        self.lock.mkdir()
        for contents in (None, '{}', 'not JSON'):
            if contents is not None:
                (self.lock / 'owner.json').write_text(contents)
            result = self.run_helper('recover')
            self.assertNotEqual(result.returncode, 0)
            self.assertTrue(self.lock.is_dir())
            self.assertNotEqual(self.run_helper('run', 'true').returncode, 0)

    def test_nonowner_cannot_record_or_release(self):
        process = self.owner('touch "$4/ready"; exec sleep 60')
        self.wait_marker('ready')
        self.assertNotEqual(self.run_helper('release').returncode, 0)
        self.assertNotEqual(self.run_helper('record', self.root).returncode, 0)
        self.assertTrue(self.lock.exists())
        self.assertIsNone(process.poll())

    def test_failed_exec_does_not_leave_a_stale_record(self):
        result = self.run_helper('run', self.root / 'missing-program')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.lock.exists())

    def test_recovery_preserves_both_completed_output_and_backup(self):
        process = self.owner('set -e; transaction=$(mktemp -d "$4/.hyper-sysroot.XXXXXX"); '
                             '"$1" "$2" record "$3" "$transaction"; '
                             'mkdir "$transaction/previous" "$3"; '
                             'echo old > "$transaction/previous/identity"; '
                             'echo new > "$3/identity"; :')
        _, stderr = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0, stderr)
        result = self.run_helper('recover')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.output / 'identity').read_text().strip(), 'new')
        staging = Path(json.loads(result.stdout)['preserved_transaction'])
        self.assertEqual((staging / 'previous/identity').read_text().strip(), 'old')

    def test_cleanup_interruption_after_removing_staging_is_recoverable(self):
        process = self.owner('set -e; transaction=$(mktemp -d "$4/.hyper-sysroot.XXXXXX"); '
                             '"$1" "$2" record "$3" "$transaction"; '
                             'mkdir "$3"; rmdir "$transaction"; :')
        _, stderr = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0, stderr)
        self.assertEqual(self.run_helper('recover').returncode, 0)
        self.assertTrue(self.output.is_dir())
        self.assertFalse(self.lock.exists())

    def test_foreign_host_record_requires_manual_investigation(self):
        process = self.owner(':')
        process.communicate(timeout=5)
        record = self.lock / 'owner.json'
        owner = json.loads(record.read_text())
        owner['host'] = 'different-build-host'
        record.write_text(json.dumps(owner))
        self.assertNotEqual(self.run_helper('recover').returncode, 0)
        self.assertTrue(self.lock.exists())


if __name__ == '__main__':
    unittest.main()
