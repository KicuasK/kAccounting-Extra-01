#!/usr/bin/env python3
"""Disposable filesystem only; no launchctl, root installation, login or UI."""
import importlib.util
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('version_tool', Path(__file__).with_name('update-private-transport.py'))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


class Versions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.root = self.base / 'installed'
        self.root.mkdir(mode=0o700)
        (self.root / 'bin').mkdir(mode=0o700)
        (self.root / 'state').mkdir(mode=0o700)
        self.old = {name: b'\xcf\xfa\xed\xfe\x0c\x00\x00\x01' + bytes(24) + b'old' for name in tool.NAMES[:2]}
        self.old[tool.NAMES[2]] = b"UPGRADE_PROTOCOL = 'kAccounting-Extra-01.transport-version-guard.v1'\n"
        for name, raw in self.old.items():
            self.file(self.root / 'bin' / name, raw, 0o700)
        self.receipt = {'format': 'kAccounting-Extra-01.private-transport-install.v1',
            'destination': str(self.root), 'installed': True,
            'guard_sha256': tool.digest(self.old[tool.NAMES[2]]),
            'transport_sha256': {name: tool.digest(self.old[name]) for name in tool.NAMES[:2]}}
        raw = tool.encoded(self.receipt)
        self.file(self.root / 'install-receipt.json', raw)
        self.expected = tool.digest(raw)
        self.review = self.base / 'review.json'
        transport = {}
        for name in tool.NAMES[:2]:
            path = self.base / name
            self.file(path, self.old[name] + b'new', 0o700)
            transport[name] = {'path': str(path), 'sha256': tool.digest(path.read_bytes())}
        self.file(self.review, tool.encoded({'format': tool.REVIEW_FORMAT,
            'provenance': {'version': '1.2.3', 'source_url': 'https://github.com/tailscale/tailscale', 'source_revision': 'a' * 40, 'build_note': 'disposable test build'}, 'transport': transport}))
        self.review_sha = tool.digest(self.review.read_bytes())
        self.patches = [patch.object(tool, 'ROOT', self.root), patch.object(tool, 'private_root'),
            patch.object(tool, 'directory', side_effect=lambda p: os.open(p, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)),
            patch.object(tool, 'private_child', side_effect=lambda fd, name: os.open(name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)),
            patch.object(tool, 'read', side_effect=self.read), patch.object(tool, 'write', side_effect=self.write),
            patch.object(tool, 'lock', side_effect=lambda fd: os.open('control.lock', os.O_RDWR | os.O_CREAT, 0o600, dir_fd=fd)),
            patch.object(tool, 'absent'), patch.object(tool.sys, 'platform', 'darwin'),
            patch.object(tool.sys, 'flags', SimpleNamespace(isolated=1, utf8_mode=0)),
            patch.object(tool.os, 'getuid', return_value=0), patch.object(tool.os, 'geteuid', return_value=0),
            patch.dict(os.environ, {'SUDO_UID': str(os.getuid())})]
        for value in self.patches:
            value.start()
            self.addCleanup(value.stop)

    def file(self, path, raw, mode=0o600):
        path.write_bytes(raw)
        path.chmod(mode)

    def read(self, fd, name, mode, maximum):
        child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
        try:
            value = os.fstat(child)
            import stat
            if not stat.S_ISREG(value.st_mode) or value.st_nlink != 1:
                raise ValueError('Unsafe file')
            return os.read(child, maximum + 1)
        finally:
            os.close(child)

    def write(self, fd, name, raw, mode=0o600):
        child = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd)
        try:
            os.write(child, raw)
        finally:
            os.close(child)

    def upgrade(self, apply=True):
        return tool.operate('upgrade', self.expected, apply, str(self.review), self.review_sha)

    def test_plan_does_not_write_or_query_job(self):
        before = list(self.root.iterdir())
        report = self.upgrade(False)
        self.assertEqual(report['operation'], 'plan')
        self.assertEqual(report['proposed_transport_sha256']['tailscale'], tool.digest((self.base / 'tailscale').read_bytes()))
        self.assertEqual(report['current_install_receipt_sha256'], self.expected)
        self.assertNotIn('state_sha256', report)
        self.assertFalse(report['stop_status_checked'])
        self.assertEqual(list(self.root.iterdir()), before)
        tool.absent.assert_not_called()

    def test_upgrade_retains_exact_old_pair_and_guard_state(self):
        self.file(self.root / 'state' / 'secret', b'fake state')
        state = (self.root / 'state' / 'secret').read_bytes()
        result = self.upgrade()
        for name in tool.NAMES:
            self.assertEqual((self.root / ('version-old-' + result['id']) / name).read_bytes(), self.old[name])
        self.assertEqual((self.root / 'bin' / tool.NAMES[2]).read_bytes(), self.old[tool.NAMES[2]])
        self.assertEqual((self.root / 'state' / 'secret').read_bytes(), state)
        self.assertFalse((self.root / 'version-pending.json').exists())
        self.assertFalse(result['started'])

    def test_rollback_before_use_restores_exact_receipt_and_pair(self):
        before = (self.root / 'install-receipt.json').read_bytes()
        update = self.upgrade()
        result = tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])
        self.assertEqual((self.root / 'install-receipt.json').read_bytes(), before)
        for name in tool.NAMES:
            self.assertEqual((self.root / 'bin' / name).read_bytes(), self.old[name])
        self.assertFalse(result['state_restored'])

    def test_rollback_refuses_even_failed_new_daemon_execution(self):
        update = self.upgrade()
        (self.root / ('version-use-' + update['install_receipt_sha256'] + '.json')).write_text('{}')
        with self.assertRaises(ValueError):
            tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])

    def test_rollback_refuses_changed_state(self):
        update = self.upgrade()
        self.file(self.root / 'state' / 'new', b'new state')
        with self.assertRaises(ValueError):
            tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])

    def test_pending_control_refuses_before_staging(self):
        self.file(self.root / 'control-pending.json', b'{}')
        with self.assertRaises(ValueError): self.upgrade()
        self.assertEqual(list(self.root.glob('version-new-*')), [])

    def test_loaded_or_unknown_job_refuses_before_staging(self):
        tool.absent.side_effect = ValueError('loaded')
        with self.assertRaises(ValueError): self.upgrade()
        self.assertFalse((self.root / 'version-pending.json').exists())

    def test_review_hash_and_changed_binary_refuse(self):
        self.file(self.base / 'tailscale', self.old['tailscale'] + b'other', 0o700)
        with self.assertRaises(ValueError): self.upgrade()
        self.assertFalse((self.root / 'version-pending.json').exists())

    def test_wrong_architecture_refuses(self):
        path = self.base / 'tailscale'
        self.file(path, b'ELF' + bytes(50), 0o700)
        review = json.loads(self.review.read_bytes())
        review['transport']['tailscale']['sha256'] = tool.digest(path.read_bytes())
        self.file(self.review, tool.encoded(review))
        self.review_sha = tool.digest(self.review.read_bytes())
        with self.assertRaises(ValueError): self.upgrade()

    def test_source_symlink_and_hardlink_refuse(self):
        path = self.base / 'tailscale'
        copy = self.base / 'copy'
        path.rename(copy)
        path.symlink_to(copy)
        with self.assertRaises(OSError): self.upgrade()
        path.unlink()
        os.link(copy, path)
        with self.assertRaises(ValueError): self.upgrade()

    def test_reconcile_after_old_bin_rename(self):
        real = tool.os.rename
        def crash(source, destination, **kwargs):
            if source.startswith('version-new-'):
                raise OSError('power loss')
            return real(source, destination, **kwargs)
        with patch.object(tool.os, 'rename', side_effect=crash):
            with self.assertRaises(OSError): self.upgrade()
        self.assertFalse((self.root / 'bin').exists())
        result = tool.operate('reconcile', self.expected, True)
        self.assertEqual(result['outcome'], 'confirmed')
        self.assertTrue((self.root / 'bin').is_dir())

    def test_reconcile_after_receipt_publication(self):
        real = tool.write
        def crash(fd, name, raw, mode=0o600):
            if name.startswith('version-outcome-'):
                raise OSError('power loss')
            return real(fd, name, raw, mode)
        with patch.object(tool, 'write', side_effect=crash):
            with self.assertRaises(OSError): self.upgrade()
        current = tool.digest((self.root / 'install-receipt.json').read_bytes())
        self.assertEqual(tool.operate('reconcile', current, True)['outcome'], 'confirmed')

    def test_reconcile_tampered_staged_bytes_preserves_pending(self):
        with patch.object(tool, 'complete', side_effect=OSError('power loss')):
            with self.assertRaises(OSError): self.upgrade()
        stage = next(self.root.glob('version-new-*'))
        self.file(stage / 'tailscale', b'tampered', 0o700)
        with self.assertRaises(ValueError): tool.operate('reconcile', self.expected, True)
        self.assertTrue((self.root / 'version-pending.json').exists())

    def test_state_symlink_refuses_without_follow(self):
        (self.root / 'state' / 'link').symlink_to(self.review)
        with self.assertRaises(OSError): self.upgrade()

    def test_review_duplicate_keys_refuses(self):
        self.file(self.review, b'{"format":1,"format":2}')
        self.review_sha = tool.digest(self.review.read_bytes())
        with self.assertRaises(ValueError): self.upgrade()


    def test_reconcile_refuses_modified_immutable_intent(self):
        with patch.object(tool, 'complete', side_effect=OSError('power loss')):
            with self.assertRaises(OSError): self.upgrade()
        intent = next(self.root.glob('version-intent-*'))
        data = json.loads(intent.read_bytes())
        data['action'] = 'rollback'
        self.file(intent, tool.encoded(data))
        with self.assertRaises(ValueError): tool.operate('reconcile', self.expected, True)
        self.assertTrue((self.root / 'version-pending.json').exists())

    def test_rollback_refuses_replaced_empty_state_directory(self):
        update = self.upgrade()
        (self.root / 'state').rename(self.root / 'preserved-state')
        (self.root / 'state').mkdir(mode=0o700)
        with self.assertRaises(ValueError):
            tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])

    def test_reconcile_refuses_state_change_after_pending(self):
        with patch.object(tool, 'complete', side_effect=OSError('power loss')):
            with self.assertRaises(OSError): self.upgrade()
        self.file(self.root / 'state' / 'unexpected', b'new data')
        with self.assertRaises(ValueError): tool.operate('reconcile', self.expected, True)
        self.assertEqual((self.root / 'bin' / 'tailscale').read_bytes(), self.old['tailscale'])

    def test_plan_refuses_nonprivate_source_parent(self):
        self.base.chmod(0o755)
        with self.assertRaises(ValueError): self.upgrade(False)

    def test_plan_refuses_nonprivate_review_mode(self):
        self.review.chmod(0o644)
        with self.assertRaises(ValueError): self.upgrade(False)

    def test_plan_refuses_unknown_provenance_source(self):
        review = json.loads(self.review.read_bytes())
        review['provenance']['source_url'] = 'https://invalid.example/binary'
        self.file(self.review, tool.encoded(review))
        self.review_sha = tool.digest(self.review.read_bytes())
        with self.assertRaises(ValueError): self.upgrade(False)

    def test_rollback_refuses_archive_extra_name(self):
        update = self.upgrade()
        self.file(self.root / ('version-old-' + update['id']) / 'extra', b'extra')
        with self.assertRaises(ValueError):
            tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])

    def test_upgrade_refuses_legacy_guard(self):
        guard = b'legacy guard'
        self.file(self.root / 'bin' / tool.NAMES[2], guard, 0o700)
        self.receipt['guard_sha256'] = tool.digest(guard)
        self.file(self.root / 'install-receipt.json', tool.encoded(self.receipt))
        self.expected = tool.digest(tool.encoded(self.receipt))
        with self.assertRaises(ValueError): self.upgrade(False)

    def test_plan_refuses_receipt_digest_mismatch(self):
        self.expected = '0' * 64
        with self.assertRaises(ValueError): self.upgrade(False)

    def test_state_special_file_refuses_without_blocking(self):
        os.mkfifo(self.root / 'state' / 'pipe')
        with self.assertRaises(ValueError): self.upgrade()


    def test_rollback_state_change_during_staging_never_promotes(self):
        update = self.upgrade()
        current = (self.root / 'bin' / 'tailscale').read_bytes()
        real = tool.write
        changed = [False]
        def mutate(fd, name, raw, mode=0o600):
            real(fd, name, raw, mode)
            if name == 'tailscale' and not changed[0]:
                changed[0] = True
                self.file(self.root / 'state' / 'race', b'changed during stage')
        with patch.object(tool, 'write', side_effect=mutate):
            with self.assertRaises(ValueError):
                tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])
        self.assertEqual((self.root / 'bin' / 'tailscale').read_bytes(), current)

    def test_state_change_after_bin_promotion_preserves_pending(self):
        real = tool.os.rename
        def mutate(source, destination, **kwargs):
            result = real(source, destination, **kwargs)
            if source.startswith('version-new-') and destination == 'bin':
                self.file(self.root / 'state' / 'race', b'changed after promotion')
            return result
        with patch.object(tool.os, 'rename', side_effect=mutate):
            with self.assertRaises(ValueError): self.upgrade()
        self.assertTrue((self.root / 'version-pending.json').exists())
        self.assertEqual(tool.digest((self.root / 'install-receipt.json').read_bytes()), self.expected)
        with self.assertRaises(ValueError): tool.operate('reconcile', self.expected, True)

    def test_rollback_refuses_history_receipt_not_linked_to_current(self):
        update = self.upgrade()
        path = self.root / ('version-outcome-' + update['id'] + '.json')
        history = tool.decode(path.read_bytes())
        wrong = tool.decode(bytes.fromhex(history['before_receipt']))
        wrong['unrelated_receipt'] = True
        history['before_receipt'] = tool.encoded(wrong).hex()
        self.file(path, tool.encoded(history))
        before = (self.root / 'install-receipt.json').read_bytes()
        with self.assertRaisesRegex(ValueError, 'receipt chain'):
            tool.operate('rollback', update['install_receipt_sha256'], True, rollback_id=update['id'])
        self.assertEqual((self.root / 'install-receipt.json').read_bytes(), before)
        self.assertFalse((self.root / 'version-pending.json').exists())

    def test_reconcile_refuses_upgrade_receipt_chain_mismatch(self):
        with patch.object(tool, 'complete', side_effect=OSError('interrupted before publication')):
            with self.assertRaises(OSError):
                self.upgrade()
        path = self.root / 'version-pending.json'
        pending = tool.decode(path.read_bytes())
        after = tool.decode(bytes.fromhex(pending['after_receipt']))
        after['previous_install_receipt_sha256'] = 'f' * 64
        pending['after_receipt'] = tool.encoded(after).hex()
        self.file(path, tool.encoded(pending))
        self.file(self.root / ('version-intent-' + pending['id'] + '.json'), tool.encoded(pending))
        with self.assertRaisesRegex(ValueError, 'receipt chain'):
            tool.operate('reconcile', self.expected, True)
        self.assertEqual((self.root / 'bin/tailscale').read_bytes(), self.old['tailscale'])
        self.assertTrue(path.exists())


if __name__ == '__main__':
    unittest.main()
