#!/usr/bin/env python3
import hashlib
import importlib.util
from pathlib import Path
import plistlib
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('manager', Path(__file__).with_name('manage-private-runtime.py'))
m = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(m)

class LifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.home = Path(self.temp.name).resolve()
        self.agents = self.home / 'Library/LaunchAgents'
        self.agents.mkdir(parents=True, mode=0o700)
        self.review = self.home / 'review'
        self.review.mkdir(mode=0o700)
        self.plan = {'Label': m.LABEL, 'Disabled': True, 'RunAtLoad': True,
                     'ProgramArguments': ['/private/runtime/bin/backend', 'serve-private-chooser']}
        self.prepared = self.review / 'disabled-user-launchagent.plist'
        self.prepared.write_bytes(plistlib.dumps(self.plan))
        self.prepared.chmod(0o600)
        self.target = self.agents / (m.LABEL + '.plist')
        self.dep = m.module()
        self.check = patch.object(self.dep, 'revalidate_inputs', return_value={})
        self.check.start()
        self.load = patch.object(m, 'module', return_value=self.dep)
        self.load.start()
        self.calls = []
    def tearDown(self):
        self.load.stop()
        self.check.stop()
        self.temp.cleanup()
    def absent(self, argv, **kwargs):
        self.calls.append(argv)
        return types.SimpleNamespace(returncode=113, stderr=f'Could not find service "{m.LABEL}" in domain for user gui: {m.os.getuid()}', stdout='')
    def installed(self):
        p = {**self.plan, 'Disabled': False, 'RunAtLoad': False}
        raw = plistlib.dumps(p)
        self.target.write_bytes(raw)
        self.target.chmod(0o600)
        return hashlib.sha256(raw).hexdigest()
    def test_plan_no_writes_or_commands(self):
        before = list(self.agents.iterdir())
        value = m.execute('install', self.home, self.review, 'a'*64, run=self.absent)
        self.assertEqual(value['phase'], 'read-only-plan')
        self.assertEqual(list(self.agents.iterdir()), before)
        self.assertFalse(self.calls)
    def test_install_unloaded_preserves_review(self):
        original = self.prepared.read_bytes()
        value = m.execute('install', self.home, self.review, 'a'*64, apply=True, run=self.absent)
        self.assertEqual(value['phase'], 'installed-unloaded')
        self.assertFalse(plistlib.loads(self.target.read_bytes())['RunAtLoad'])
        self.assertEqual(original, self.prepared.read_bytes())
        self.assertEqual(len(self.calls), 1)
    def test_install_existing_refused(self):
        sha = self.installed()
        with self.assertRaisesRegex(ValueError, 'preserves'):
            m.execute('install', self.home, self.review, 'a'*64, sha)
    def test_wrong_digest_preserves_installed(self):
        self.installed()
        raw = self.target.read_bytes()
        with self.assertRaisesRegex(ValueError, 'retained'):
            m.execute('update', self.home, self.review, 'a'*64, 'b'*64, True, self.absent)
        self.assertEqual(self.target.read_bytes(), raw)
    def test_unknown_print_refuses_mutation(self):
        def unknown(*args, **kwargs):
            return types.SimpleNamespace(returncode=5, stderr='service unavailable', stdout='')
        with self.assertRaisesRegex(ValueError, 'Unknown'):
            m.execute('install', self.home, self.review, 'a'*64, apply=True, run=unknown)
        self.assertFalse(self.target.exists())
    def test_wrong_loaded_program_refuses_stop(self):
        sha = self.installed()
        def wrong(argv, **kwargs):
            self.calls.append(argv)
            return types.SimpleNamespace(returncode=0, stderr='', stdout=f'path = {self.target}\nprogram = /other\n')
        with self.assertRaisesRegex(ValueError, 'does not match'):
            m.execute('stop', self.home, expected_installed=sha, apply=True, run=wrong)
        self.assertEqual(len(self.calls), 1)
    def test_stop_timeout_records_unknown_blocks_retry(self):
        import subprocess
        sha = self.installed()
        def timeout(argv, **kwargs):
            if argv[1] == 'print':
                return types.SimpleNamespace(returncode=0, stderr='', stdout=f'path = {self.target}\nprogram = /private/runtime/bin/backend\narguments = {{\n/private/runtime/bin/backend\nserve-private-chooser\n}}\n')
            raise subprocess.TimeoutExpired(argv, 15)
        with self.assertRaises(subprocess.TimeoutExpired):
            m.execute('stop', self.home, expected_installed=sha, apply=True, run=timeout)
        self.assertEqual(len(list(self.agents.glob('*.json'))), 1)
        with self.assertRaisesRegex(ValueError, 'reconciliation'):
            m.execute('stop', self.home, expected_installed=sha, apply=True, run=timeout)
    def test_update_keeps_old_bytes(self):
        sha = self.installed()
        old = self.target.read_bytes()
        self.plan['ProgramArguments'][0] = '/private/runtime2/bin/backend'
        self.prepared.write_bytes(plistlib.dumps(self.plan))
        value = m.execute('update', self.home, self.review, 'a'*64, sha, True, self.absent)
        self.assertTrue(value['applied'])
        backups = list(self.agents.glob('*.before-*.plist'))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), old)
    def test_symlink_parent_rejected(self):
        real = self.home / 'real'
        self.agents.rename(real)
        self.agents.symlink_to(real, target_is_directory=True)
        with self.assertRaises(OSError):
            m.execute('install', self.home, self.review, 'a'*64, apply=True, run=self.absent)
    def test_status_does_not_revalidate_or_write(self):
        sha = self.installed()
        self.dep.revalidate_inputs.reset_mock()
        value = m.execute('status', self.home, run=self.absent)
        self.assertEqual(value['installed_sha256'], sha)
        self.dep.revalidate_inputs.assert_not_called()
        self.assertEqual(list(self.agents.iterdir()), [self.target])

    def test_wrong_loaded_arguments_refuses_stop(self):
        sha = self.installed()
        def wrong(argv, **kwargs):
            return types.SimpleNamespace(returncode=0, stderr='', stdout=f'path = {self.target}\nprogram = /private/runtime/bin/backend\narguments = {{\n/private/runtime/bin/backend\nother\n}}\n')
        with self.assertRaisesRegex(ValueError, 'arguments'):
            m.execute('stop', self.home, expected_installed=sha, apply=True, run=wrong)
    def test_reconcile_absent_preserves_intent_audit(self):
        sha = self.installed()
        intent = self.agents / ('.' + m.LABEL + '.intent-fixture.json')
        import json
        intent.write_text(json.dumps({'domain': f'gui/{m.os.getuid()}/{m.LABEL}', 'installed_sha256': sha}))
        intent.chmod(0o600)
        value = m.execute('reconcile', self.home, expected_installed=sha, apply=True,
                          run=self.absent, expected_state='absent')
        self.assertEqual(value['phase'], 'operator-reconciled-absent')
        self.assertFalse(intent.exists())
        self.assertEqual(len(list(self.agents.glob('*.reconciled-*.json'))), 1)
    def test_reconcile_wrong_state_keeps_intent(self):
        sha = self.installed()
        intent = self.agents / ('.' + m.LABEL + '.intent-fixture.json')
        intent.write_text('{}')
        intent.chmod(0o600)
        with self.assertRaisesRegex(ValueError, 'differs'):
            m.execute('reconcile', self.home, expected_installed=sha, apply=True,
                      run=self.absent, expected_state='loaded')
        self.assertTrue(intent.exists())
    def test_install_singlelink_readable_afterwards(self):
        m.execute('install', self.home, self.review, 'a'*64, apply=True, run=self.absent)
        self.assertEqual(self.target.stat().st_nlink, 1)
        self.assertEqual(m.execute('status', self.home, run=self.absent)['state'], 'absent')
    def test_rollback_requires_actual_retained_version(self):
        sha = self.installed()
        with self.assertRaisesRegex(ValueError, 'retained prior'):
            m.execute('rollback', self.home, self.review, 'a'*64, sha, run=self.absent)
    def test_root_apply_refused(self):
        with patch.object(m.os, 'getuid', return_value=0):
            with self.assertRaisesRegex(ValueError, 'never root'):
                m.execute('install', self.home, self.review, 'a'*64, apply=True, run=self.absent)
    def test_revalidation_replacement_preserved(self):
        sha = self.installed()
        calls = 0
        replacement = {**self.plan, 'Disabled': False, 'unexpected': True}
        def change(*args):
            nonlocal calls
            calls += 1
            if calls == 2:
                self.target.write_bytes(plistlib.dumps(replacement))
        self.dep.revalidate_inputs.side_effect = change
        with self.assertRaisesRegex(ValueError, 'changed immediately'):
            m.execute('update', self.home, self.review, 'a'*64, sha, True, self.absent)
        self.assertEqual(plistlib.loads(self.target.read_bytes()), replacement)

    def test_bootstrap_timeout_intent_blocks_retry(self):
        import subprocess
        sha = self.installed()
        def timeout(argv, **kwargs):
            if argv[1] == 'print':
                return self.absent(argv, **kwargs)
            raise subprocess.TimeoutExpired(argv, 15)
        with self.assertRaises(subprocess.TimeoutExpired):
            m.execute('start', self.home, self.review, 'a'*64, sha, True, timeout)
        with self.assertRaisesRegex(ValueError, 'reconciliation'):
            m.execute('start', self.home, self.review, 'a'*64, sha, True, timeout)
    def test_kickstart_failure_retains_intent(self):
        sha = self.installed()
        def failure(argv, **kwargs):
            if argv[1] == 'print':
                return self.absent(argv, **kwargs)
            return types.SimpleNamespace(returncode=0 if argv[1] == 'bootstrap' else 5, stderr='', stdout='')
        with self.assertRaisesRegex(ValueError, 'reconciliation'):
            m.execute('start', self.home, self.review, 'a'*64, sha, True, failure)
        self.assertEqual(len(list(self.agents.glob('*.intent-*.json'))), 1)
    def test_same_bytes_inode_replacement_refused(self):
        sha = self.installed()
        raw = self.target.read_bytes()
        calls = 0
        def change(*args):
            nonlocal calls
            calls += 1
            if calls == 2:
                swapped = self.agents / 'swapped'
                swapped.write_bytes(raw)
                swapped.chmod(0o600)
                swapped.replace(self.target)
        self.dep.revalidate_inputs.side_effect = change
        with self.assertRaisesRegex(ValueError, 'changed immediately'):
            m.execute('update', self.home, self.review, 'a'*64, sha, True, self.absent)
        self.assertEqual(self.target.read_bytes(), raw)

    def test_explicit_login_install(self):
        value = m.execute('install', self.home, self.review, 'a'*64, apply=True,
                          run=self.absent, start_at_login=True)
        self.assertTrue(plistlib.loads(self.target.read_bytes())['RunAtLoad'])
        self.assertTrue(value['start_at_login'])
        self.assertFalse(value['login_reboot_qualified'])
        self.assertEqual(len(self.calls), 1)
    def test_start_preserves_installed_login_option(self):
        self.installed()
        installed = plistlib.loads(self.target.read_bytes())
        installed['RunAtLoad'] = True
        self.target.write_bytes(plistlib.dumps(installed))
        sha = m.digest(self.target.read_bytes())
        value = m.execute('start', self.home, self.review, 'a'*64, sha)
        self.assertTrue(value['start_at_login'])
        self.assertEqual(value['proposed_sha256'], sha)
    def test_start_login_option_does_not_ignore_payload_mismatch(self):
        self.installed()
        installed = plistlib.loads(self.target.read_bytes())
        installed.update(RunAtLoad=True, Unexpected=True)
        self.target.write_bytes(plistlib.dumps(installed))
        sha = m.digest(self.target.read_bytes())
        with self.assertRaisesRegex(ValueError, 'differs'):
            m.execute('start', self.home, self.review, 'a'*64, sha)
    def test_login_option_rejected_for_stop(self):
        with self.assertRaisesRegex(ValueError, 'only valid'):
            m.execute('stop', self.home, start_at_login=True)
    def test_login_update_keeps_manual_backup(self):
        sha = self.installed()
        original = self.target.read_bytes()
        value = m.execute('update', self.home, self.review, 'a'*64, sha, True,
                          self.absent, start_at_login=True)
        self.assertTrue(value['start_at_login'])
        self.assertEqual(next(self.agents.glob('*.before-*.plist')).read_bytes(), original)
        sha = m.digest(self.target.read_bytes())
        value = m.execute('rollback', self.home, self.review, 'a'*64, sha, True, self.absent)
        self.assertFalse(value['start_at_login'])
        self.assertEqual(self.target.read_bytes(), original)
    def test_login_start_bootstrap_only(self):
        self.installed()
        installed = plistlib.loads(self.target.read_bytes())
        installed['RunAtLoad'] = True
        self.target.write_bytes(plistlib.dumps(installed))
        sha = m.digest(self.target.read_bytes())
        printed = 0
        def runner(argv, **kwargs):
            nonlocal printed
            self.calls.append(argv)
            if argv[1] == 'print':
                printed += 1
                if printed == 1:
                    return types.SimpleNamespace(returncode=113, stderr=f'Could not find service "{m.LABEL}" in domain for user gui: {m.os.getuid()}', stdout='')
                return types.SimpleNamespace(returncode=0, stderr='', stdout=f'path = {self.target}\nprogram = /private/runtime/bin/backend\narguments = {{\n/private/runtime/bin/backend\nserve-private-chooser\n}}\n')
            return types.SimpleNamespace(returncode=0, stderr='', stdout='')
        value = m.execute('start', self.home, self.review, 'a'*64, sha, True, runner)
        self.assertEqual(value['phase'], 'loaded-not-qualified')
        self.assertEqual([item[1] for item in self.calls], ['print', 'bootstrap', 'print'])

if __name__ == '__main__':
    unittest.main()
