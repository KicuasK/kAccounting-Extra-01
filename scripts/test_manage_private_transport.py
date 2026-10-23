#!/usr/bin/env python3
"""Mock system boundary tests: never register, start or stop a real job."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
from types import SimpleNamespace
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('transport_manager', Path(__file__).with_name('manage-private-transport.py'))
manager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(manager)


class TransportManagerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / 'installed'
        self.daemons = self.base / 'LaunchDaemons'
        self.root.mkdir(mode=0o700)
        self.daemons.mkdir(mode=0o700)
        self.plist = self.daemons / (manager.LABEL + '.plist')
        self.patches = [patch.object(manager, 'ROOT', self.root), patch.object(manager, 'DAEMONS', self.daemons),
            patch.object(manager, 'PLIST', self.plist), patch.object(manager, 'validate', return_value={}),
            patch.object(manager, 'directory', side_effect=lambda p: os.open(p, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)),
            patch.object(manager, 'lock', side_effect=lambda fd: os.open('test-control.lock', os.O_RDWR | os.O_CREAT, 0o600, dir_fd=fd)),
            patch.object(manager, 'read', side_effect=self.read), patch.object(manager, 'write', side_effect=self.write),
            patch.object(manager.sys, 'platform', 'darwin'), patch.object(manager.sys, 'flags', SimpleNamespace(isolated=1, utf8_mode=0)),
            patch.object(manager.os, 'getuid', return_value=0), patch.object(manager.os, 'geteuid', return_value=0)]
        for value in self.patches:
            value.start()
            self.addCleanup(value.stop)
        self.state = 'absent'
        self.calls = []
        self.command_patch = patch.object(manager, 'command', side_effect=self.command)
        self.command_patch.start()
        self.addCleanup(self.command_patch.stop)

    def read(self, fd, name, mode, maximum):
        child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=fd)
        try: return os.read(child, maximum + 1)
        finally: os.close(child)

    def write(self, fd, name, raw, mode=0o600):
        child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
        try: os.write(child, raw)
        finally: os.close(child)

    def loaded(self):
        return manager.JOB + ' = {\n\tpath = ' + str(self.plist) + '\n\tprogram = ' + manager.ARGV[0] + '\n\targuments = {\n' + ''.join('\t\t' + arg + '\n' for arg in manager.ARGV) + '\t}\n}\n'

    def command(self, args):
        self.calls.append(args)
        if args[0] == 'print':
            if self.state == 'absent':
                return SimpleNamespace(returncode=113, stdout='', stderr='Bad request.\nCould not find service "' + manager.LABEL + '" in domain for system\n')
            return SimpleNamespace(returncode=0, stdout=self.loaded(), stderr='')
        if args[0] == 'bootstrap': self.state = 'loaded'
        if args[0] == 'bootout': self.state = 'absent'
        return SimpleNamespace(returncode=0, stdout='', stderr='')

    def apply(self, action): return manager.operate(action, 'a' * 64, True)
    def register(self): self.apply('register')

    def test_default_plan_never_calls_launchctl_or_writes(self):
        result = manager.operate('start', 'a' * 64)
        self.assertEqual(result['operation'], 'plan')
        self.assertEqual(self.calls, [])
        self.assertEqual(list(self.root.iterdir()), [])

    def test_register_disabled_exclusively(self):
        self.register()
        import plistlib
        plan = plistlib.loads(self.plist.read_bytes())
        self.assertTrue(plan['Disabled'])
        self.assertFalse(plan['RunAtLoad'])
        self.assertFalse((self.root / 'control-pending.json').exists())
        self.assertEqual(len(list(self.root.glob('control-intent-*.json'))), 1)
        self.assertTrue(all(call[0] == 'print' for call in self.calls))

    def test_register_preserves_existing_file(self):
        self.plist.write_bytes(b'existing GUI/other job')
        with self.assertRaises(FileExistsError): self.register()
        self.assertEqual(self.plist.read_bytes(), b'existing GUI/other job')
        self.assertTrue((self.root / 'control-pending.json').exists())

    def test_register_refuses_existing_loaded_job(self):
        self.state = 'loaded'
        with self.assertRaises(ValueError): self.register()
        self.assertFalse(self.plist.exists())

    def test_start_and_stop_use_only_exact_fixed_job(self):
        self.register()
        self.apply('start')
        self.assertIn(['enable', manager.JOB], self.calls)
        self.assertIn(['bootstrap', 'system', str(self.plist)], self.calls)
        self.assertIn(['kickstart', manager.JOB], self.calls)
        self.apply('stop')
        self.assertIn(['bootout', manager.JOB], self.calls)
        self.assertEqual(self.state, 'absent')
        self.assertTrue(self.plist.exists())

    def test_stop_absent_does_not_bootout(self):
        self.register()
        self.apply('stop')
        self.assertNotIn(['bootout', manager.JOB], self.calls)

    def test_timeout_preserves_pending_and_locks_mutations(self):
        self.register()
        original = manager.command
        def command(args):
            if args[0] == 'bootstrap': raise subprocess.TimeoutExpired('launchctl', 15)
            return self.command(args)
        with patch.object(manager, 'command', side_effect=command):
            with self.assertRaises(subprocess.TimeoutExpired): self.apply('start')
        pending = json.loads((self.root / 'control-pending.json').read_bytes())
        self.assertTrue(pending['enable_override_may_have_changed'])
        with self.assertRaises(ValueError): self.apply('stop')
        self.assertEqual(self.apply('reconcile')['outcome'], 'unknown')

    def test_reconcile_confirms_exact_loaded_job_without_retrying_start(self):
        self.register()
        def command(args):
            if args[0] == 'bootstrap':
                self.state = 'loaded'
                raise subprocess.TimeoutExpired('launchctl', 15)
            return self.command(args)
        with patch.object(manager, 'command', side_effect=command):
            with self.assertRaises(subprocess.TimeoutExpired): self.apply('start')
        self.calls = []
        self.assertEqual(self.apply('reconcile')['outcome'], 'confirmed')
        self.assertTrue(all(args[0] == 'print' for args in self.calls))
        self.assertFalse((self.root / 'control-pending.json').exists())

    def test_acknowledge_known_aborted_start_preserves_enable_uncertainty(self):
        self.register()
        def command(args):
            if args[0] == 'bootstrap': raise subprocess.TimeoutExpired('launchctl', 15)
            return self.command(args)
        with patch.object(manager, 'command', side_effect=command):
            with self.assertRaises(subprocess.TimeoutExpired): self.apply('start')
        result = manager.operate('reconcile', 'a' * 64, True, 'absent')
        self.assertEqual(result['outcome'], 'acknowledged-abort')
        self.assertTrue(result['enable_override_may_have_changed'])
        self.assertFalse((self.root / 'control-pending.json').exists())
        self.assertNotIn(['disable', manager.JOB], self.calls)
        self.apply('start')
        self.assertEqual(self.state, 'loaded')

    def test_acknowledgment_requires_matching_observation(self):
        self.register()
        def command(args):
            if args[0] == 'bootstrap': raise subprocess.TimeoutExpired('launchctl', 15)
            return self.command(args)
        with patch.object(manager, 'command', side_effect=command):
            with self.assertRaises(subprocess.TimeoutExpired): self.apply('start')
        with self.assertRaises(ValueError): manager.operate('reconcile', 'a' * 64, True, 'loaded')
        self.assertTrue((self.root / 'control-pending.json').exists())

    def test_changed_pending_cannot_be_unlinked(self):
        intent = {'id': 'a' * 32, 'format': manager.FORMAT, 'action': 'start',
                  'receipt_sha256': 'a' * 64, 'job': manager.JOB, 'outcome': 'pending'}
        other = {**intent, 'id': 'b' * 32}
        (self.root / 'control-pending.json').write_text(json.dumps(other))
        fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(ValueError): manager.finish(fd, intent)
        finally: os.close(fd)
        self.assertEqual(json.loads((self.root / 'control-pending.json').read_text()), other)

    def test_operation_lock_failure_prevents_mutation(self):
        with patch.object(manager, 'lock', side_effect=BlockingIOError('busy')):
            with self.assertRaises(BlockingIOError): self.register()
        self.assertFalse(self.plist.exists())
        self.assertEqual(self.calls, [])

    def test_mismatched_loaded_args_refuse_stop(self):
        self.register()
        self.state = 'loaded'
        wrong = self.loaded().replace('-S\n', '-E\n')
        with patch.object(manager, 'command', return_value=SimpleNamespace(returncode=0, stdout=wrong, stderr='')):
            with self.assertRaises(ValueError): self.apply('stop')
        self.assertTrue((self.root / 'control-pending.json').exists())

    def test_other_missing_job_message_is_unknown(self):
        with patch.object(manager, 'command', return_value=SimpleNamespace(returncode=113, stdout='', stderr='Could not find service "other" in domain for system\n')):
            with self.assertRaises(ValueError): manager.observe()

    def test_registration_symlink_preserved(self):
        external = self.base / 'external'
        external.write_bytes(b'preserve')
        self.plist.symlink_to(external)
        with self.assertRaises(FileExistsError): self.register()
        self.assertEqual(external.read_bytes(), b'preserve')

    def test_status_is_readonly(self):
        result = self.apply('status')
        self.assertFalse(result['registered'])
        self.assertEqual(result['job_state'], 'absent')
        self.assertEqual(list(self.root.iterdir()), [])

    def test_version_pending_blocks_after_operation_lock(self):
        pending = self.root / 'version-pending.json'
        pending.write_bytes(b'preserve publication journal')
        with self.assertRaisesRegex(ValueError, 'version publication'):
            self.apply('start')
        self.assertEqual(self.calls, [])
        self.assertFalse((self.root / 'control-pending.json').exists())
        self.assertEqual(pending.read_bytes(), b'preserve publication journal')

    def test_pending_created_at_lock_boundary_is_not_missed(self):
        def acquired(fd):
            (self.root / 'version-pending.json').write_bytes(b'boundary publication')
            return os.open('test-control.lock', os.O_RDWR | os.O_CREAT, 0o600, dir_fd=fd)
        with patch.object(manager, 'lock', side_effect=acquired):
            with self.assertRaisesRegex(ValueError, 'version publication'):
                self.apply('start')
        self.assertEqual(self.calls, [])

    def digest(self): return hashlib.sha256(self.plist.read_bytes()).hexdigest()
    def startup(self, action='startup-enable'):
        return manager.operate(action, 'a' * 64, True, expected_registered=self.digest())

    def test_startup_plan_no_write_or_launchctl(self):
        self.register(); self.calls = []; before = set(self.root.iterdir())
        result = manager.operate('startup-enable', 'a' * 64, expected_registered=self.digest())
        self.assertEqual(result['operation'], 'plan'); self.assertFalse(result['changed'])
        self.assertEqual(before, set(self.root.iterdir())); self.assertEqual(self.calls, [])
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes())

    def test_startup_explicit_digest_required(self):
        self.register()
        for digest in (None, 'b' * 64):
            with self.assertRaises(ValueError): manager.operate('startup-enable', 'a' * 64, True, expected_registered=digest)
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes())

    def test_enable_startup_preserves_live_service_and_backup(self):
        self.register(); self.apply('start'); self.calls = []
        before = self.plist.read_bytes(); result = self.startup()
        self.assertEqual(self.state, 'loaded'); self.assertTrue(all(call[0] == 'print' for call in self.calls))
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes(True))
        self.assertEqual(next(self.root.glob('startup-backup-*.plist')).read_bytes(), before)
        self.assertFalse(result['reload_performed']); self.assertFalse(result['reboot_start_qualified'])
        self.assertFalse((self.root / 'control-pending.json').exists())

    def test_startup_status_start_stop_require_new_hash(self):
        self.register(); self.startup()
        with self.assertRaises(ValueError): self.apply('status')
        result = manager.operate('status', 'a' * 64, True, expected_registered=self.digest())
        self.assertTrue(result['registered'])
        manager.operate('start', 'a' * 64, True, expected_registered=self.digest())
        manager.operate('stop', 'a' * 64, True, expected_registered=self.digest())
        self.assertEqual(self.state, 'absent')

    def test_disable_startup_roundtrip_keeps_live_job(self):
        self.register(); self.apply('start'); self.startup(); self.calls=[]
        self.startup('startup-disable')
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes())
        self.assertEqual(self.state, 'loaded'); self.assertTrue(all(call[0] == 'print' for call in self.calls))
        self.assertTrue(self.apply('status')['registered'])

    def test_startup_unknown_schema_refused(self):
        self.register(); self.plist.write_bytes(manager.plan_bytes().replace(b'<false/>', b'<true/>', 1))
        with self.assertRaises(ValueError): self.startup()

    def test_startup_pending_blocks_normal_mutation(self):
        self.register()
        with patch.object(manager.os, 'replace', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError): self.startup()
        self.assertTrue((self.root/'control-pending.json').exists())
        with self.assertRaises(ValueError): self.apply('start')
        with self.assertRaises(ValueError): self.startup()
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes())

    def test_startup_reconcile_aborted_preserves_backup_and_temp(self):
        self.register()
        with patch.object(manager.os, 'replace', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError): self.startup()
        before = list(self.daemons.glob('*.startup-*'))
        result = self.startup('startup-reconcile')
        self.assertEqual(result['outcome'], 'acknowledged-abort')
        self.assertEqual(before, list(self.daemons.glob('*.startup-*')))
        self.assertTrue(list(self.root.glob('startup-backup-*.plist')))
        self.assertFalse((self.root/'control-pending.json').exists())

    def test_startup_reconcile_after_replacement_without_retry(self):
        self.register()
        with patch.object(manager, 'finish', side_effect=OSError('crash after replace')):
            with self.assertRaises(OSError): self.startup()
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes(True)); self.calls=[]
        with patch.object(manager.os, 'replace', side_effect=AssertionError('must not replay')):
            result = self.startup('startup-reconcile')
        self.assertEqual(result['outcome'], 'confirmed'); self.assertEqual(self.calls, [])

    def test_startup_reconcile_tampered_backup_stays_locked(self):
        self.register()
        with patch.object(manager, 'finish', side_effect=OSError('crash')):
            with self.assertRaises(OSError): self.startup()
        next(self.root.glob('startup-backup-*.plist')).write_bytes(b'tamper')
        with self.assertRaises(ValueError): self.startup('startup-reconcile')
        self.assertTrue((self.root/'control-pending.json').exists())

    def test_startup_source_change_before_replace_stays_locked(self):
        self.register(); original = manager.write
        def mutate(fd, name, raw, mode=0o600):
            original(fd, name, raw, mode)
            if '.startup-' in name: self.plist.write_bytes(b'external edit')
        with patch.object(manager, 'write', side_effect=mutate):
            with self.assertRaises(ValueError): self.startup()
        self.assertEqual(self.plist.read_bytes(), b'external edit')
        self.assertTrue((self.root/'control-pending.json').exists())

    def test_startup_version_pending_refused(self):
        self.register(); (self.root/'version-pending.json').write_bytes(b'pending')
        with self.assertRaises(ValueError): self.startup()
        self.assertEqual(self.plist.read_bytes(), manager.plan_bytes())


if __name__ == '__main__': unittest.main()
