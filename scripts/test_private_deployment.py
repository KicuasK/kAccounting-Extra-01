"""Synthetic deployment planning tests; never launch a process or access real keys."""
import hashlib
import contextlib
import io
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import stat
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('private_deployment', Path(__file__).with_name('prepare-private-deployment.py'))
plan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plan)


class PrivateDeploymentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ka-deploy-', dir='/private/tmp')
        self.root = Path(self.temp.name); self.root.chmod(0o700)
        self.binary = self.fixture('backend')
        self.helper = self.fixture('StoragePicker')
        self.daemon = self.fixture('tailscaled')
        self.client = self.fixture('tailscale')
        assets = self.root / 'assets'; assets.mkdir(mode=0o700)
        (assets / 'index.html').write_text('<html>synthetic</html>')
        self.runtime = self.root / 'runtime'
        receipt = plan.bundle.build(self.binary, assets, self.runtime, 'macos-arm64', 'a' * 40, self.helper)
        self.books = self.root / 'books'; self.books.mkdir(mode=0o700)
        self.socket = self.root / 'socket'; self.socket.mkdir(mode=0o700)
        self.config = self.root / 'private.json'
        self.config.write_text(json.dumps({'hostname': 'book.example.ts.net', 'login': 'owner@example.com', 'socket': str(self.socket / 'a.sock')})); self.config.chmod(0o600)
        self.provenance = self.root / 'provenance.json'
        self.metadata = {'version': '1.104.2', 'source_url': 'https://github.com/tailscale/tailscale', 'source_revision': 'b' * 40, 'build_note': 'Synthetic non-executable fixture; no real source attestation', 'tailscaled_sha256': self.hash(self.daemon), 'tailscale_sha256': self.hash(self.client)}
        self.provenance.write_text(json.dumps(self.metadata)); self.provenance.chmod(0o600)
        self.args = {'runtime': self.runtime, 'expected_manifest': receipt['manifest_sha256'], 'tailscaled': self.daemon, 'tailscaled_hash': self.hash(self.daemon), 'tailscale': self.client, 'tailscale_hash': self.hash(self.client), 'provenance': self.provenance, 'books_root': self.books, 'private_config': self.config, 'output': self.root / 'review'}

    def fixture(self, name):
        path = self.root / name
        path.write_bytes(struct.pack('<II', 0xfeedfacf, plan.bundle.TARGETS['macos-arm64']) + b'not-executable-' + name.encode())
        path.chmod(0o700); return path

    def hash(self, path): return hashlib.sha256(path.read_bytes()).hexdigest()
    def tearDown(self): self.temp.cleanup()
    def prepare(self, **changes): return plan.prepare(**{**self.args, **changes})

    def test_plans_are_disabled_and_no_install_or_qualification_claim(self):
        with patch('subprocess.run', side_effect=AssertionError('must never execute')):
            result = self.prepare()
        output = self.args['output']
        self.assertEqual(set(path.name for path in output.iterdir()), {'disabled-root-launchdaemon.plist', 'disabled-user-launchagent.plist', 'review.json', 'complete.json'})
        daemon = plistlib.loads((output / 'disabled-root-launchdaemon.plist').read_bytes())
        app = plistlib.loads((output / 'disabled-user-launchagent.plist').read_bytes())
        report = json.loads((output / 'review.json').read_bytes())
        self.assertTrue(daemon['Disabled']); self.assertFalse(daemon['RunAtLoad']); self.assertFalse(daemon['KeepAlive'])
        self.assertEqual(daemon['UserName'], 'root'); self.assertTrue(app['Disabled'])
        self.assertEqual(app['ProgramArguments'][1], 'serve-private-native-chooser')
        self.assertIn('--socket=/private/var/run/kaccounting-private-transport/tailscaled.sock', daemon['ProgramArguments'])
        self.assertIn('--state=/Library/Application Support/kAccounting-Extra-01/private-transport/state/tailscaled.state', daemon['ProgramArguments'])
        self.assertIn('--statedir=/Library/Application Support/kAccounting-Extra-01/private-transport/state', daemon['ProgramArguments'])
        self.assertFalse(result['installed']); self.assertFalse(result['started']); self.assertFalse(result['qualified'])
        self.assertFalse(report['future_root_install']['adopt_existing_gui_state'])
        self.assertTrue(report['provenance_is_operator_assertion_not_build_attestation'])
        for path in output.iterdir(): self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)

    def test_wrong_digests_and_tampered_runtime_refused(self):
        for changes in [{'expected_manifest': 'c' * 64}, {'tailscaled_hash': 'c' * 64}, {'tailscale_hash': 'c' * 64}]:
            with self.assertRaises(ValueError): self.prepare(**changes)
        (self.runtime / 'frontend/index.html').write_text('tampered')
        with self.assertRaises(ValueError): self.prepare()
        self.assertFalse(self.args['output'].exists())

    def test_transport_symlink_hardlink_mode_and_privilege_refusal(self):
        alias = self.root / 'alias'; alias.symlink_to(self.daemon)
        with self.assertRaises(ValueError): self.prepare(tailscaled=alias)
        alias.unlink(); os.link(self.daemon, alias)
        with self.assertRaises(ValueError): self.prepare()
        alias.unlink()
        for mode in [0o722, 0o600]:
            self.daemon.chmod(mode)
            with self.assertRaises(ValueError): self.prepare()
        # Some macOS temporary filesystems silently strip setuid. Test the metadata guard directly.
        with patch.object(plan.bundle, 'regular', return_value=SimpleNamespace(st_mode=stat.S_IFREG | 0o4700)):
            with self.assertRaises(ValueError): plan.identity(self.daemon, 1024, True)
        self.assertFalse(self.args['output'].exists())

    def test_wrong_architecture_and_missing_native_helper_refused(self):
        self.daemon.write_bytes(struct.pack('<II', 0xfeedfacf, plan.bundle.TARGETS['macos-x86_64']) + b'wrong-arch')
        with self.assertRaises(ValueError): self.prepare(tailscaled_hash=self.hash(self.daemon))
        (self.runtime / 'bin/StoragePicker').unlink()
        with self.assertRaises((ValueError, OSError)): self.prepare()

    def test_existing_review_and_symlink_parent_preserved(self):
        self.prepare(); before = (self.args['output'] / 'review.json').read_bytes()
        with self.assertRaises(FileExistsError): self.prepare()
        self.assertEqual((self.args['output'] / 'review.json').read_bytes(), before)
        alias = self.root / 'alias'; alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(ValueError): self.prepare(output=alias / 'review-other')

    def test_launch_directory_and_unsafe_output_parent_refused(self):
        launch = self.root / 'LaunchDaemons'; launch.mkdir(mode=0o700)
        with self.assertRaises(ValueError): self.prepare(output=launch / 'review')
        unsafe = self.root / 'unsafe'; unsafe.mkdir(mode=0o755)
        with self.assertRaises(ValueError): self.prepare(output=unsafe / 'review')
        with self.assertRaises(ValueError): self.prepare(output=self.books / 'review')
        with self.assertRaises(ValueError): self.prepare(output=self.runtime / 'review')

    def test_private_config_books_and_existing_ingress_socket_refused(self):
        self.config.chmod(0o644)
        with self.assertRaises(ValueError): self.prepare()
        self.config.chmod(0o600); self.books.chmod(0o755)
        with self.assertRaises(ValueError): self.prepare()
        self.books.chmod(0o700); (self.socket / 'a.sock').write_bytes(b'preserve')
        with self.assertRaises(ValueError): self.prepare()
        self.assertEqual((self.socket / 'a.sock').read_bytes(), b'preserve')

    def test_metadata_duplicate_fields_mismatch_and_nonofficial_source_refused(self):
        for field, value in [('source_url', 'https://example.com/source'), ('source_revision', 'main'), ('version', 'unknown'), ('tailscaled_sha256', 'c' * 64)]:
            self.provenance.write_text(json.dumps({**self.metadata, field: value}))
            with self.assertRaises(ValueError): self.prepare()
        self.provenance.write_text('{"version":"1.104.2","version":"1.104.2"}')
        with self.assertRaises(ValueError): self.prepare()

    def test_mutated_transport_during_publication_preserves_partial_without_completion(self):
        original = plan.staged_binary
        calls = 0
        def changing(*args):
            nonlocal calls
            calls += 1
            if calls == 3: raise ValueError('synthetic identity replacement')
            return original(*args)
        with patch.object(plan, 'staged_binary', changing):
            with self.assertRaises(ValueError): self.prepare()
        self.assertTrue(self.args['output'].exists())
        self.assertFalse((self.args['output'] / 'complete.json').exists())

    def test_inputs_changed_after_first_plan_never_publish_completion(self):
        original = plan.plistlib.dumps
        first = True
        def mutate_after_first_plan(value):
            nonlocal first
            data = original(value)
            if first:
                first = False
                self.provenance.write_text(json.dumps({**self.metadata, 'build_note': 'changed during emission'}))
            return data
        with patch.object(plan.plistlib, 'dumps', mutate_after_first_plan):
            with self.assertRaises(ValueError): self.prepare()
        self.assertTrue((self.args['output'] / 'disabled-root-launchdaemon.plist').exists())
        self.assertTrue((self.args['output'] / 'review.json').exists())
        self.assertFalse((self.args['output'] / 'complete.json').exists())

    def test_offline_review_verifier_authenticates_plans_and_example_marker(self):
        receipt = self.prepare(example_only=True)
        with patch.object(plan.bundle, 'verify', side_effect=AssertionError('offline verifier must not inspect runtime')):
            result = plan.verify_review(self.args['output'], receipt['review_sha256'])
        self.assertTrue(result['artifact_integrity_verified']); self.assertTrue(result['example_only'])
        self.assertFalse(result['source_inputs_revalidated']); self.assertFalse(result['qualified'])
        for digest in ['a' * 64, 'bad']:
            with self.assertRaises(ValueError): plan.verify_review(self.args['output'], digest)
        file = self.args['output'] / 'disabled-root-launchdaemon.plist'
        original = file.read_bytes(); file.write_bytes(original.replace(b'<true/>', b'<false/>', 1))
        with self.assertRaises(ValueError): plan.verify_review(self.args['output'], receipt['review_sha256'])
        file.write_bytes(original)
        self.assertTrue(plan.verify_review(self.args['output'], receipt['review_sha256'])['artifact_integrity_verified'])

    def test_review_verifier_refuses_missing_marker_extra_files_permissions_and_links(self):
        receipt = self.prepare(); output = self.args['output']; expected = receipt['review_sha256']
        marker = output / 'complete.json'; raw = marker.read_bytes(); marker.unlink()
        with self.assertRaises(ValueError): plan.verify_review(output, expected)
        marker.write_bytes(raw); marker.chmod(0o600)
        extra = output / 'extra'; extra.write_text('unexpected')
        with self.assertRaises(ValueError): plan.verify_review(output, expected)
        extra.unlink(); marker.chmod(0o644)
        with self.assertRaises(ValueError): plan.verify_review(output, expected)
        marker.chmod(0o600); os.link(marker, extra)
        # Restore exact file set but retain an external hard link.
        extra.rename(self.root / 'external-marker-link')
        with self.assertRaises(ValueError): plan.verify_review(output, expected)

    def test_live_revalidation_is_read_only_and_never_executes(self):
        receipt = self.prepare()
        before = {str(p.relative_to(self.root)): (p.stat().st_ino, self.hash(p))
                  for p in self.root.rglob('*') if p.is_file()}
        original_open = os.open
        def read_only_open(path, flags, *args, **kwargs):
            if flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND):
                raise AssertionError('revalidation must not open writable descriptors')
            return original_open(path, flags, *args, **kwargs)
        with patch('subprocess.run', side_effect=AssertionError('never execute')), patch.object(os, 'open', read_only_open):
            result = plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])
        after = {str(p.relative_to(self.root)): (p.stat().st_ino, self.hash(p))
                 for p in self.root.rglob('*') if p.is_file()}
        self.assertEqual(before, after)
        self.assertTrue(result['source_inputs_revalidated'])
        for key in ['installed', 'started', 'qualified', 'deployment_ready', 'atomic_installation_boundary']:
            self.assertFalse(result[key])

    def test_live_revalidation_refuses_example_and_legacy_snapshot(self):
        receipt = self.prepare(example_only=True)
        with self.assertRaises(ValueError):
            plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])
        report_file = self.args['output'] / 'review.json'
        report = json.loads(report_file.read_bytes())
        report['example_only'] = False
        del report['input_snapshot']
        report_file.write_bytes(plan.bundle.encoded(report))
        digest = self.hash(report_file)
        marker = self.args['output'] / 'complete.json'
        marker.write_bytes(plan.bundle.encoded({'format': plan.FORMAT, 'review_sha256': digest,
            'installed': False, 'started': False, 'qualified': False}))
        self.assertTrue(plan.verify_review(self.args['output'], digest)['artifact_integrity_verified'])
        with self.assertRaises(ValueError): plan.revalidate_inputs(self.args['output'], digest)

    def test_live_revalidation_refuses_same_content_file_replacements(self):
        receipt = self.prepare()
        # Offline artifact verification still works after any source replacement.
        for path in [self.runtime / 'frontend/index.html', self.daemon, self.client,
                     self.provenance, self.config]:
            original = self.root / ('original-' + path.name)
            path.rename(original)
            path.write_bytes(original.read_bytes()); path.chmod(stat.S_IMODE(original.stat().st_mode))
            self.assertTrue(plan.verify_review(self.args['output'], receipt['review_sha256'])['artifact_integrity_verified'])
            if path == self.runtime / 'frontend/index.html':
                self.assertTrue(plan.bundle.verify(self.runtime, self.args['expected_manifest'])['complete'])
            with self.assertRaises(ValueError):
                plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])
            path.unlink(); original.rename(path)

    def test_live_revalidation_refuses_replaced_books_or_existing_socket(self):
        receipt = self.prepare()
        old = self.root / 'old-books'; self.books.rename(old); self.books.mkdir(mode=0o700)
        with self.assertRaises(ValueError): plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])
        self.books.rmdir(); old.rename(self.books)
        socket = self.socket / 'a.sock'; socket.write_bytes(b'preserve')
        with self.assertRaises(ValueError): plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])
        self.assertEqual(socket.read_bytes(), b'preserve')

    def test_live_revalidation_rechecks_input_changes_during_scan(self):
        receipt = self.prepare()
        original = plan.runtime_snapshot
        calls = 0
        def changing(*args):
            nonlocal calls
            result = original(*args); calls += 1
            if calls == 1:
                (self.runtime / 'frontend/index.html').write_text('changed during scan')
            return result
        with patch.object(plan, 'runtime_snapshot', changing):
            with self.assertRaises(ValueError): plan.revalidate_inputs(self.args['output'], receipt['review_sha256'])

    def test_live_revalidation_refuses_digest_tamper_modes_and_symlinks(self):
        receipt = self.prepare(); output = self.args['output']; expected = receipt['review_sha256']
        with self.assertRaises(ValueError): plan.revalidate_inputs(output, 'c' * 64)
        for path in [self.daemon, self.provenance, self.config]:
            mode = stat.S_IMODE(path.stat().st_mode); path.chmod(0o666)
            with self.assertRaises(ValueError): plan.revalidate_inputs(output, expected)
            path.chmod(mode)
        # A symlink to unchanged bytes cannot stand in for the pinned file.
        old = self.root / 'old-client'; self.client.rename(old); self.client.symlink_to(old)
        with self.assertRaises(ValueError): plan.revalidate_inputs(output, expected)
        self.client.unlink(); old.rename(self.client)
        (self.runtime / 'frontend/index.html').write_text('tampered')
        with self.assertRaises(ValueError): plan.revalidate_inputs(output, expected)

    def test_live_revalidation_cli_requires_verify_and_independent_digest(self):
        import sys
        for args in [['--revalidate-inputs'], ['--verify', str(self.args['output']), '--revalidate-inputs'],
                     ['--revalidate-inputs', '--expected-review-sha256', 'a' * 64]]:
            with patch.object(sys, 'argv', ['prepare-private-deployment.py', *args]), contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error: plan.main()
                self.assertEqual(error.exception.code, 2)


if __name__ == '__main__': unittest.main()
