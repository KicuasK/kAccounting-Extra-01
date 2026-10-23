#!/usr/bin/env python3
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
import types
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('transport_guard', Path(__file__).with_name('private-transport-guard.py'))
g = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(g)


class GuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name).resolve()
        self.root = self.base / 'transport'
        self.root.mkdir(mode=0o700)
        (self.root / 'bin').mkdir(mode=0o700)
        (self.root / 'state').mkdir(mode=0o700)
        self.control = self.base / 'control'
        self.uid, self.gid = os.getuid(), os.getgid()
        self.receipt = {'format': 'kAccounting-Extra-01.private-transport-install.v1',
                        'destination': str(self.root), 'installed': True, 'transport_sha256': {}}
        for name in ('tailscaled', 'tailscale', 'private-transport-guard.py'):
            raw = ('fake-' + name).encode()
            self.file(self.root / 'bin' / name, raw, 0o700)
            digest = hashlib.sha256(raw).hexdigest()
            if name.endswith('.py'):
                self.receipt['guard_sha256'] = digest
            else:
                self.receipt['transport_sha256'][name] = digest
        self.save_receipt()
        # Tests exercise private fake files; production directory() additionally
        # enforces root:wheel on every system ancestor (tested separately).
        self.walk = patch.object(g, 'directory', side_effect=lambda path, *_: os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW))
        self.walk.start()
    def tearDown(self):
        self.walk.stop()
        self.temp.cleanup()
    def file(self, path, raw, mode):
        path.write_bytes(raw)
        path.chmod(mode)
    def save_receipt(self):
        self.file(self.root / 'install-receipt.json', json.dumps(self.receipt).encode(), 0o600)
    def prepare(self):
        return g.prepare(self.root, self.control, self.uid, self.gid)
    def test_creates_empty_control_and_fixed_arguments_without_execution(self):
        args = self.prepare()
        self.assertEqual(args[0], str(self.root / 'bin/tailscaled'))
        self.assertEqual(args[-1], '--port=0')
        self.assertIn('--tun=userspace-networking', args)
        self.assertEqual(list(self.control.iterdir()), [])
        self.assertEqual(self.control.stat().st_mode & 0o777, 0o700)
    def test_reboot_missing_control_recreated_without_changing_state(self):
        state = self.root / 'state/tailscaled.state'
        self.file(state, b'preserve fake state', 0o600)
        self.prepare()
        self.control.rmdir()
        self.prepare()
        self.assertEqual(state.read_bytes(), b'preserve fake state')
    def test_new_control_inherited_group_normalized(self):
        with patch.object(g.os, 'fchown', wraps=g.os.fchown) as ownership:
            self.prepare()
            ownership.assert_called_once()
            self.assertEqual(ownership.call_args.args[1:], (self.uid, self.gid))
    def test_existing_wrong_group_refused_without_chown(self):
        self.control.mkdir(mode=0o700)
        real_fstat = g.os.fstat
        inode = self.control.stat().st_ino
        def wrong_group(fd):
            value = real_fstat(fd)
            if value.st_ino == inode:
                fields = {name: getattr(value, name) for name in ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns', 'st_mode', 'st_uid', 'st_gid', 'st_nlink')}
                fields['st_gid'] += 1
                return types.SimpleNamespace(**fields)
            return value
        with patch.object(g.os, 'fstat', side_effect=wrong_group), patch.object(g.os, 'fchown') as ownership:
            with self.assertRaisesRegex(ValueError, 'root owned'):
                self.prepare()
            ownership.assert_not_called()
    def test_existing_empty_control_accepted(self):
        self.control.mkdir(mode=0o700)
        inode = self.control.stat().st_ino
        self.prepare()
        self.assertEqual(self.control.stat().st_ino, inode)
    def test_existing_control_file_preserved_and_refused(self):
        self.control.mkdir(mode=0o700)
        marker = self.control / 'tailscaled.sock'
        marker.write_bytes(b'stale fake socket marker')
        with self.assertRaisesRegex(ValueError, 'not empty'):
            self.prepare()
        self.assertEqual(marker.read_bytes(), b'stale fake socket marker')
    def test_symlink_control_refused(self):
        self.control.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(OSError):
            self.prepare()
    def test_wrong_control_mode_refused(self):
        self.control.mkdir(mode=0o755)
        with self.assertRaises(ValueError):
            self.prepare()
    def test_binary_change_refused_before_control_creation(self):
        self.file(self.root / 'bin/tailscaled', b'replaced fake', 0o700)
        with self.assertRaisesRegex(ValueError, 'digest'):
            self.prepare()
        self.assertFalse(self.control.exists())
    def test_hardlinked_binary_refused(self):
        os.link(self.root / 'bin/tailscaled', self.base / 'hardlink')
        with self.assertRaises(ValueError):
            self.prepare()
    def test_guard_change_refused(self):
        self.file(self.root / 'bin/private-transport-guard.py', b'replaced', 0o700)
        with self.assertRaisesRegex(ValueError, 'guard digest'):
            self.prepare()
    def test_wrong_receipt_destination_refused(self):
        self.receipt['destination'] = '/somewhere/else'
        self.save_receipt()
        with self.assertRaisesRegex(ValueError, 'receipt'):
            self.prepare()
    def test_duplicate_receipt_key_refused(self):
        self.file(self.root / 'install-receipt.json', b'{"format":1,"format":2}', 0o600)
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            self.prepare()
    def test_cli_refuses_arguments_without_exec(self):
        with patch.object(g.sys, 'argv', ['guard', '--root=/tmp']), patch.object(g.os, 'execve') as execute:
            with self.assertRaises(ValueError):
                g.main()
            execute.assert_not_called()
    def test_control_replacement_race_refused(self):
        real_stat = g.os.stat
        def replaced(path, *args, **kwargs):
            value = real_stat(path, *args, **kwargs)
            if path == self.control.name and kwargs.get('dir_fd') is not None:
                fields = {name: getattr(value, name) for name in ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns', 'st_mode', 'st_uid', 'st_gid', 'st_nlink')}
                fields['st_ino'] += 1
                return types.SimpleNamespace(**fields)
            return value
        with patch.object(g.os, 'stat', side_effect=replaced):
            with self.assertRaisesRegex(ValueError, 'Control parent changed'):
                self.prepare()
    def test_arbitrary_group_writable_ancestor_refused(self):
        self.walk.stop()
        # First ancestor is not the exact /private/var/run system exception.
        real_fstat = g.os.fstat
        def unsafe(fd):
            value = real_fstat(fd)
            return types.SimpleNamespace(st_uid=self.uid, st_gid=self.gid, st_mode=0o40775)
        with patch.object(g.os, 'fstat', side_effect=unsafe):
            with self.assertRaisesRegex(ValueError, 'ancestor'):
                g.directory(self.root, self.uid, self.gid)
        self.walk.start()
    def test_real_directory_walk_rejects_user_owned_ancestor(self):
        self.walk.stop()
        with self.assertRaises(ValueError):
            g.directory(self.root, self.uid, self.gid)
        self.walk.start()

    def test_version_pending_blocks_before_control_creation(self):
        pending = self.root / 'version-pending.json'
        pending.write_bytes(b'preserve incomplete publication')
        with self.assertRaisesRegex(ValueError, 'reconciliation'):
            self.prepare()
        self.assertFalse(self.control.exists())
        self.assertEqual(pending.read_bytes(), b'preserve incomplete publication')

    def test_pending_symlink_is_refused_without_following(self):
        pending = self.root / 'version-pending.json'
        pending.symlink_to(self.base / 'missing')
        with self.assertRaisesRegex(ValueError, 'reconciliation'):
            g.record_use(self.root, self.uid, self.gid)
        self.assertTrue(pending.is_symlink())

    def test_execution_marker_is_durable_idempotent_and_receipt_bound(self):
        first = g.record_use(self.root, self.uid, self.gid)
        marker = self.root / ('version-use-' + first + '.json')
        before = marker.read_bytes(), marker.stat().st_ino
        self.assertEqual(g.record_use(self.root, self.uid, self.gid), first)
        self.assertEqual((marker.read_bytes(), marker.stat().st_ino), before)
        self.assertEqual(marker.stat().st_mode & 0o777, 0o600)
        self.receipt['generation'] = 'another-generation'
        self.save_receipt()
        second = g.record_use(self.root, self.uid, self.gid)
        self.assertNotEqual(first, second)
        self.assertTrue(marker.exists())
        self.assertTrue((self.root / ('version-use-' + second + '.json')).exists())

    def test_corrupt_existing_execution_marker_is_preserved_and_refused(self):
        digest = g.record_use(self.root, self.uid, self.gid)
        marker = self.root / ('version-use-' + digest + '.json')
        self.file(marker, b'unknown result', 0o600)
        with self.assertRaisesRegex(ValueError, 'marker changed'):
            g.record_use(self.root, self.uid, self.gid)
        self.assertEqual(marker.read_bytes(), b'unknown result')


if __name__ == '__main__':
    unittest.main()
