"""Transport-only bootstrap tests using synthetic files, never service execution."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('transport_bootstrap', Path(__file__).with_name('prepare-private-transport.py'))
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ka-bootstrap-', dir='/private/tmp')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.root.chmod(0o700)
        self.args = {'target': 'macos-arm64', 'output': self.root / 'review'}
        metadata = {'version': '1.104.2', 'source_url': 'https://github.com/tailscale/tailscale', 'source_revision': 'a' * 40, 'build_note': 'Synthetic fixture only'}
        for name in ('tailscaled', 'tailscale'):
            path = self.root / name
            path.write_bytes(struct.pack('<II', 0xfeedfacf, bootstrap.bundle.TARGETS['macos-arm64']) + name.encode())
            path.chmod(0o700)
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            self.args[name] = path; self.args[name + '_hash'] = digest
            metadata[name + '_sha256'] = digest
        provenance = self.root / 'provenance.json'
        provenance.write_text(json.dumps(metadata)); provenance.chmod(0o600)
        self.args['provenance'] = provenance

    def prepare(self): return bootstrap.prepare(**self.args)
    def verify(self, receipt): return bootstrap.revalidate_inputs(self.args['output'], receipt['review_sha256'])

    def test_transport_only_no_runtime_or_identity_and_never_executes(self):
        with patch('subprocess.run', side_effect=AssertionError('must not execute')):
            receipt = self.prepare(); result = self.verify(receipt)
        report = json.loads((self.args['output'] / 'review.json').read_text())
        for field in ('runtime', 'private_config', 'hostname', 'login', 'books_root'):
            self.assertNotIn(field, report)
        self.assertTrue(result['source_inputs_revalidated'])
        self.assertFalse(result['started']); self.assertFalse(result['installed'])
        self.assertEqual(stat.S_IMODE(self.args['output'].stat().st_mode), 0o700)
        for path in self.args['output'].iterdir(): self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_installer_plan_dispatches_bootstrap_end_to_end(self):
        receipt = self.prepare()
        spec = importlib.util.spec_from_file_location('bootstrap_installer', Path(__file__).with_name('install-private-transport.py'))
        installer = importlib.util.module_from_spec(spec); spec.loader.exec_module(installer)
        with patch.object(installer, 'publish', side_effect=AssertionError('plan must not publish')):
            result = installer.install(str(self.args['output']), receipt['review_sha256'])
        self.assertEqual(result['operation'], 'plan')
        self.assertTrue(result['source_inputs_revalidated'])
        self.assertFalse(result['will_start'])

    def test_source_tamper(self):
        receipt = self.prepare(); self.args['tailscale'].write_bytes(b'changed')
        with self.assertRaises(ValueError): self.verify(receipt)

    def test_same_bytes_inode_replacement(self):
        receipt = self.prepare(); path = self.args['tailscale']; raw = path.read_bytes()
        path.rename(self.root / 'old'); path.write_bytes(raw); path.chmod(0o700)
        with self.assertRaises(ValueError): self.verify(receipt)

    def test_wrong_external_digest(self):
        self.prepare()
        with self.assertRaises(ValueError): bootstrap.revalidate_inputs(self.args['output'], 'b' * 64)

    def test_mode_refusal(self):
        self.args['tailscale'].chmod(0o755)
        with self.assertRaises(ValueError): self.prepare()

    def test_source_symlink_refusal(self):
        path = self.args['tailscale']; path.rename(self.root / 'real'); path.symlink_to(self.root / 'real')
        with self.assertRaises((ValueError, OSError)): self.prepare()

    def test_existing_output_not_overwritten(self):
        self.prepare(); before = (self.args['output'] / 'review.json').read_bytes()
        with self.assertRaises(FileExistsError): self.prepare()
        self.assertEqual(before, (self.args['output'] / 'review.json').read_bytes())

    def test_output_symlink_refusal(self):
        self.args['output'].symlink_to(self.root, target_is_directory=True)
        with self.assertRaises((ValueError, OSError)): self.prepare()

    def test_completion_tamper_and_extra_files(self):
        receipt = self.prepare(); marker = self.args['output'] / 'complete.json'; marker.write_text('{}')
        with self.assertRaises(ValueError): self.verify(receipt)

    def test_provenance_contract(self):
        path = self.args['provenance']; document = json.loads(path.read_text()); document['source_url'] = 'https://invalid.test/'
        path.write_text(json.dumps(document))
        with self.assertRaises(ValueError): self.prepare()

    def test_review_file_mode(self):
        receipt = self.prepare(); (self.args['output'] / 'review.json').chmod(0o644)
        with self.assertRaises(ValueError): self.verify(receipt)

    def test_report_status_even_with_rebound_digest(self):
        receipt = self.prepare(); path = self.args['output'] / 'review.json'; document = json.loads(path.read_text()); document['started'] = True
        path.write_text(json.dumps(document)); receipt['review_sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
        marker = self.args['output'] / 'complete.json'; complete = json.loads(marker.read_text()); complete['review_sha256'] = receipt['review_sha256']; marker.write_text(json.dumps(complete))
        with self.assertRaises(ValueError): self.verify(receipt)


if __name__ == '__main__': unittest.main()
