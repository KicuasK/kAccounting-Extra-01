#!/usr/bin/env python3
"""Disposable publication tests; no root execution or service activation."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('transport_install', Path(__file__).with_name('install-private-transport.py'))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class TransportInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.dest = self.root / 'private-transport'
        self.report = {'transport': {}}
        for name in ('tailscaled', 'tailscale'):
            path = self.root / name
            path.write_bytes(b'fake-binary-' + name.encode())
            path.chmod(0o700)
            value = path.stat()
            self.report['transport'][name] = {'path': str(path), 'device': value.st_dev,
                'inode': value.st_ino, 'bytes': value.st_size, 'mode': '0o700',
                'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
        # Test-only fd seam, avoiding shared writable temp ancestors; production
        # always uses the fixed, ownership-validated /Library ancestor walker.
        self.patch = patch.object(installer, 'directory_fd', side_effect=lambda p, u: os.open(p, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW))
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def publish(self, validate=lambda: None):
        return installer.publish(self.report, 'a' * 64, self.dest, os.getuid(), validate)

    def test_new_publication_disabled_and_exact(self):
        result = self.publish()
        self.assertFalse(result['started'])
        self.assertFalse(result['launchdaemon_installed'])
        self.assertFalse(result['control_parent_created'])
        self.assertEqual(set(p.name for p in self.dest.iterdir()), {'bin', 'state', 'install-receipt.json', 'disabled-launchdaemon-review.plist'})
        for name in ('tailscaled', 'tailscale'):
            path = self.dest / 'bin' / name
            self.assertEqual(path.read_bytes(), (self.root / name).read_bytes())
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.dest / 'install-receipt.json').stat().st_mode), 0o600)

    def test_existing_directory_never_adopted(self):
        self.dest.mkdir()
        marker = self.dest / 'state'
        marker.write_text('preserve')
        with self.assertRaises(FileExistsError): self.publish()
        self.assertEqual(marker.read_text(), 'preserve')

    def test_existing_symlink_never_adopted(self):
        self.dest.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(FileExistsError): self.publish()
        self.assertTrue(self.dest.is_symlink())

    def test_existing_regular_file_never_replaced(self):
        self.dest.write_text('preserve')
        with self.assertRaises(FileExistsError): self.publish()
        self.assertEqual(self.dest.read_text(), 'preserve')

    def test_hardlinked_source_refused_before_writes(self):
        os.link(self.root / 'tailscaled', self.root / 'alias')
        with self.assertRaises(ValueError): self.publish()
        self.assertFalse(self.dest.exists())

    def test_symlink_source_refused_before_writes(self):
        path = self.root / 'tailscaled'
        path.rename(self.root / 'real')
        path.symlink_to(self.root / 'real')
        with self.assertRaises(OSError): self.publish()
        self.assertFalse(self.dest.exists())

    def test_changed_source_hash_refused(self):
        (self.root / 'tailscaled').write_bytes(b'fake-binary-CHANGED!!')
        with self.assertRaises(ValueError): self.publish()
        self.assertFalse(self.dest.exists())

    def test_final_validation_failure_preserves_partial_no_receipt(self):
        calls = []
        def validate():
            calls.append(True)
            if len(calls) == 2: raise ValueError('interrupted/stale')
        with self.assertRaises(ValueError): self.publish(validate)
        self.assertTrue((self.dest / 'bin/tailscaled').exists())
        self.assertFalse((self.dest / 'install-receipt.json').exists())
        with self.assertRaises(FileExistsError): self.publish()

    def test_same_bytes_replacement_inode_refused(self):
        path = self.root / 'tailscaled'
        contents = path.read_bytes()
        path.rename(self.root / 'original')
        path.write_bytes(contents)
        path.chmod(0o700)
        with self.assertRaises(ValueError): self.publish()
        self.assertFalse(self.dest.exists())

    def test_guard_published_private_and_pinned(self):
        guard = b'# immutable fake guard, never executed\n'
        result = installer.publish(self.report, 'a' * 64, self.dest, os.getuid(), lambda: None, guard)
        self.assertEqual(result['guard_sha256'], hashlib.sha256(guard).hexdigest())
        path = self.dest / 'bin/private-transport-guard.py'
        self.assertEqual(path.read_bytes(), guard)
        self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o700)
        import plistlib
        plan = plistlib.loads((self.dest / 'disabled-launchdaemon-review.plist').read_bytes())
        self.assertTrue(plan['Disabled'])
        self.assertFalse(plan['RunAtLoad'])
        self.assertEqual(plan['ProgramArguments'], ['/usr/bin/python3', '-I', '-S', str(installer.ROOT / 'bin/private-transport-guard.py')])

    def test_production_ancestor_walker_rejects_shared_writable_temp(self):
        self.patch.stop()
        try:
            with self.assertRaises((ValueError, OSError)): installer.directory_fd(self.root, 0)
        finally:
            self.patch.start()

    def test_plan_revalidates_but_never_publishes(self):
        with patch.object(installer, 'validated_report', return_value=({'installed': False}, self.report)) as validate, patch.object(installer, 'publish') as publish:
            result = installer.install('review', 'a' * 64)
        validate.assert_called_once()
        publish.assert_not_called()
        self.assertEqual(result['operation'], 'plan')

    def test_apply_requires_admin_on_macos(self):
        with patch.object(installer.sys, 'platform', 'linux'):
            with self.assertRaises(ValueError): installer.install('review', 'a' * 64, True)

    def test_root_must_not_load_external_review_module(self):
        with patch.object(installer.os, 'geteuid', return_value=0):
            with self.assertRaises(ValueError): installer.load_review()

    def test_root_format_gate_accepts_full_and_bootstrap_disabled_only(self):
        report = {'format': installer.BOOTSTRAP_FORMAT, 'example_only': False,
                  **{key: False for key in ('installed', 'started', 'qualified', 'funnel_enabled', 'deployment_ready', 'signed', 'serve_configured', 'login_performed')}}
        installer.check_report_status(report)
        installer.check_report_status({**report, 'format': 'kAccounting-Extra-01.private-deployment-review.v1'})
        for changes in ({'format': 'unknown'}, {'example_only': True}, {'started': True}, {'login_performed': True}, {'serve_configured': True}):
            with self.assertRaises(ValueError): installer.check_report_status({**report, **changes})

    def test_root_must_not_load_bootstrap_module(self):
        with patch.object(installer.os, 'geteuid', return_value=0):
            with self.assertRaises(ValueError): installer.load_review(True)


if __name__ == '__main__': unittest.main()
