#!/usr/bin/env python3
"""Pure probe parsing/config/cleanup tests; never runs Serve or a probe listener."""
import importlib.util
import contextlib
import io
from unittest import mock
import json
import os
from pathlib import Path
import socket
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('probe', Path(__file__).with_name('private-ingress-probe.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ProbeTests(unittest.TestCase):
    def test_reports_counts_and_matches_without_identity_values(self):
        raw = b'GET /probe-abc HTTP/1.1\r\nHost: localhost\r\nX-Forwarded-Host: book.example.ts.net\r\nX-Forwarded-Proto: https\r\nTailscale-User-Login: fake@example.com\r\n\r\n'
        target, report = probe.sanitized_headers(raw, {'hostname': 'book.example.ts.net', 'login': 'fake@example.com'})
        self.assertEqual(target, b'/probe-abc')
        self.assertTrue(report['expected_login_matches'])
        self.assertTrue(report['expected_forwarded_host_matches'])
        serialized = json.dumps(report)
        self.assertNotIn('fake@example.com', serialized)
        self.assertNotIn('book.example.ts.net', serialized)
        self.assertEqual(report['header_counts']['tailscale-user-login'], 1)

    def test_duplicate_identity_is_reported_as_duplicate_not_authenticated(self):
        _, report = probe.sanitized_headers(b'GET / HTTP/1.1\r\nHost: localhost\r\nTailscale-User-Login: a\r\nTailscale-User-Login: b\r\n\r\n', {'hostname': 'book.example.ts.net', 'login': 'a'})
        self.assertEqual(report['header_counts']['tailscale-user-login'], 2)
        self.assertFalse(report['expected_login_matches'])

    def test_bounded_headers_invalid_request_and_folding_are_refused(self):
        for raw in [b'x' * (probe.MAX_HEADERS + 1), b'GET / HTTP/1.1\r\n', b'POST / HTTP/1.1\r\n\r\n', b'GET / HTTP/1.1\r\n bad: x\r\n\r\n']:
            with self.assertRaises(ValueError):
                probe.sanitized_headers(raw)

    def test_private_config_modes_duplicates_and_symlink_fail_closed(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            path = Path(work) / 'config.json'
            path.write_text('{"hostname":"book.example.ts.net","login":"fake@example.com"}')
            path.chmod(0o600)
            self.assertEqual(probe.private_config(path)['login'], 'fake@example.com')
            path.chmod(0o644)
            with self.assertRaises(ValueError): probe.private_config(path)
            path.chmod(0o600)
            path.write_text('{"hostname":"a","hostname":"b","login":"fake@example.com"}')
            with self.assertRaises(ValueError): probe.private_config(path)
            alias = Path(work) / 'alias'
            alias.symlink_to(path)
            with self.assertRaises(ValueError): probe.private_config(alias)

    def test_cleanup_preserves_unknown_or_replaced_parent(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            parent = Path(work) / 'owned'
            parent.mkdir()
            metadata = parent.lstat()
            (parent / 'unknown').write_bytes(b'preserve')
            self.assertFalse(probe.cleanup_owned(parent, metadata, parent / 'p.sock', None))
            self.assertEqual((parent / 'unknown').read_bytes(), b'preserve')
            parent.rename(Path(work) / 'original')
            parent.mkdir()
            self.assertFalse(probe.cleanup_owned(parent, metadata, parent / 'p.sock', None))
            self.assertTrue(parent.exists())

    def test_explicit_parent_root_must_be_owned_private_and_canonical(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            root = Path(work).resolve()
            root.chmod(0o700)
            selected, identity = probe.checked_parent_root(root)
            self.assertEqual(selected, root)
            self.assertEqual((identity.st_dev, identity.st_ino), (root.stat().st_dev, root.stat().st_ino))
            self.assertEqual(list(root.iterdir()), [])
            root.chmod(0o755)
            with self.assertRaises(ValueError):
                probe.checked_parent_root(root)
            root.chmod(0o700)
            with mock.patch.object(probe.os, 'getuid', return_value=root.stat().st_uid + 1):
                with self.assertRaises(ValueError):
                    probe.checked_parent_root(root)

    def test_source_and_symbolic_link_parent_roots_are_refused(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            root = Path(work).resolve()
            source = root / 'invented-source'
            source.mkdir(mode=0o700)
            nested = source / 'nested'
            nested.mkdir(mode=0o700)
            with mock.patch.object(probe, 'SOURCE', source):
                for path in [source, nested]:
                    with self.subTest(path=path), self.assertRaises(ValueError):
                        probe.checked_parent_root(path)
            alias = root / 'alias'
            alias.symlink_to(nested, target_is_directory=True)
            with self.assertRaises(ValueError):
                probe.checked_parent_root(alias)
            self.assertTrue(alias.is_symlink())
            self.assertTrue(source.is_dir())

    def test_overlong_explicit_parent_fails_before_child_creation_or_socket(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            root = Path(work).resolve()
            parent = root / ('x' * 100)
            parent.mkdir(mode=0o700)
            with mock.patch.object(probe.socket, 'socket') as socket_factory:
                with self.assertRaises(ValueError):
                    probe.listen(1, parent_root=parent)
                socket_factory.assert_not_called()
            self.assertTrue(parent.is_dir())
            self.assertEqual(list(parent.iterdir()), [])

    def test_failed_fresh_probe_cleans_only_child_and_preserves_selected_parent(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            root = Path(work).resolve()
            selected = root / 'approved'
            selected.mkdir(mode=0o700)
            original = selected.stat()
            # Existing content belongs to the selected parent, never to this probe.
            sentinel = selected / 'operator-receipt.json'
            sentinel.write_bytes(b'preserve approved parent evidence')
            fake_socket = mock.Mock()
            fake_socket.bind.side_effect = OSError('controlled bind refusal; no listener created')
            output = io.StringIO()
            with mock.patch.object(probe.socket, 'socket', return_value=fake_socket), \
                 mock.patch.object(probe.secrets, 'token_hex', return_value='a' * 32), \
                 contextlib.redirect_stdout(output):
                result = probe.listen(1, parent_root=selected)
            self.assertEqual(result, 1)
            fake_socket.close.assert_called_once()
            report = json.loads(output.getvalue().strip())
            self.assertTrue(report['probe_incomplete'])
            self.assertTrue(report['owned_socket_cleanup_complete'])
            self.assertEqual(sentinel.read_bytes(), b'preserve approved parent evidence')
            self.assertEqual(set(item.name for item in selected.iterdir()), {'operator-receipt.json'})
            self.assertEqual((selected.stat().st_dev, selected.stat().st_ino), (original.st_dev, original.st_ino))

    def test_socket_constructor_failure_removes_fresh_child_and_retains_approved_parent(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as work:
            root = Path(work).resolve()
            selected = root / 'approved'
            selected.mkdir(mode=0o700)
            before = selected.stat()
            output = io.StringIO()
            with mock.patch.object(probe.socket, 'socket', side_effect=OSError('controlled descriptor exhaustion')), \
                 contextlib.redirect_stdout(output):
                self.assertEqual(probe.listen(1, parent_root=selected), 1)
            report = json.loads(output.getvalue().strip())
            self.assertTrue(report['probe_incomplete'])
            self.assertTrue(report['owned_socket_cleanup_complete'])
            self.assertFalse(report['financial_data_accessed'])
            self.assertEqual(list(selected.iterdir()), [])
            self.assertEqual((selected.stat().st_dev, selected.stat().st_ino), (before.st_dev, before.st_ino))

    @unittest.skipUnless(os.uname().sysname == 'Darwin', 'Darwin getpeereid only')
    def test_kernel_credentials_on_local_socket_pair_are_real_current_uid(self):
        left, right = socket.socketpair(socket.AF_UNIX, socket.SOCK_STREAM)
        with left, right:
            uid, gid = probe.peer_credentials(left)
            self.assertEqual(uid, os.getuid())
            self.assertEqual(gid, os.getgid())


if __name__ == '__main__':
    unittest.main()
