#!/usr/bin/env python3
"""Synthetic non-executable Mach-O fixtures; no app or OS-key access."""
import importlib.util
import json
import os
from pathlib import Path
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('bundle', Path(__file__).with_name('package-macos-release.py'))
bundle = importlib.util.module_from_spec(spec); spec.loader.exec_module(bundle)


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix='ka-bundle-test-', dir='/private/tmp')
        self.root = Path(self.work.name); self.root.chmod(0o700)
        self.binary = self.root / 'fixture-binary'
        self.binary.write_bytes(struct.pack('<II', 0xfeedfacf, bundle.TARGETS['macos-arm64']) + b'not-executable-fixture')
        self.binary.chmod(0o700)
        self.frontend = self.root / 'frontend'; self.frontend.mkdir(mode=0o700)
        (self.frontend / 'index.html').write_text('<html>fixture</html>')
        (self.frontend / 'assets').mkdir(mode=0o700)
        (self.frontend / 'assets/app-abc.js').write_text('fixture')

    def tearDown(self): self.work.cleanup()
    def build(self, name='bundle', **kwargs):
        return bundle.build(self.binary, self.frontend, self.root / name, 'macos-arm64', 'a' * 40, **kwargs)

    def test_reproducible_build_verify_and_no_real_paths_in_manifest(self):
        a = self.build('first'); b = self.build('second')
        self.assertEqual(a['manifest_sha256'], b['manifest_sha256'])
        self.assertTrue(bundle.verify(self.root / 'first', a['manifest_sha256'])['complete'])
        self.assertNotIn(str(self.root), (self.root / 'first/manifest.json').read_text())
        self.assertEqual(stat.S_IMODE((self.root / 'first/bin/kaccounting-backend').stat().st_mode), 0o700)

    def test_existing_output_is_never_adopted_or_overwritten(self):
        self.build()
        before = (self.root / 'bundle/manifest.json').read_bytes()
        with self.assertRaises(FileExistsError): self.build()
        self.assertEqual((self.root / 'bundle/manifest.json').read_bytes(), before)

    def test_corruption_unknown_files_and_missing_marker_are_rejected(self):
        self.build('corrupt'); (self.root / 'corrupt/frontend/index.html').write_text('modified')
        with self.assertRaises(ValueError): bundle.verify(self.root / 'corrupt')
        self.build('unknown'); (self.root / 'unknown/financial.sqlite3').write_text('fixture only')
        with self.assertRaises(ValueError): bundle.verify(self.root / 'unknown')
        self.build('partial'); (self.root / 'partial/complete.json').unlink()
        with self.assertRaises(OSError): bundle.verify(self.root / 'partial')

    def test_input_symlink_hardlink_permissions_and_unknown_assets_fail_closed(self):
        alias = self.root / 'alias'; alias.symlink_to(self.binary)
        with self.assertRaises(ValueError): bundle.build(alias, self.frontend, self.root / 'bad', 'macos-arm64', 'a' * 40)
        hard = self.root / 'hard'; os.link(self.binary, hard)
        with self.assertRaises(ValueError): self.build()
        hard.unlink(); self.binary.chmod(0o722)
        with self.assertRaises(ValueError): self.build()
        self.binary.chmod(0o700); (self.frontend / 'secrets.json').write_text('{}')
        with self.assertRaises(ValueError): self.build()

    def test_output_symlink_hardlink_and_mode_changes_fail_verification(self):
        self.build('alias-output'); p = self.root / 'alias-output/frontend/index.html'
        p.unlink(); p.symlink_to(self.frontend / 'index.html')
        with self.assertRaises(ValueError): bundle.verify(self.root / 'alias-output')
        self.build('hard-output'); p = self.root / 'hard-output/frontend/index.html'; os.link(p, self.root / 'linked')
        with self.assertRaises(ValueError): bundle.verify(self.root / 'hard-output')
        self.build('mode-output'); (self.root / 'mode-output/frontend/index.html').chmod(0o644)
        with self.assertRaises(ValueError): bundle.verify(self.root / 'mode-output')

    def test_wrong_architecture_and_oversized_binary_are_rejected_before_copy(self):
        with self.assertRaises(ValueError): bundle.build(self.binary, self.frontend, self.root / 'badarch', 'macos-x86_64', 'a' * 40)
        with self.binary.open('r+b') as output: output.truncate(bundle.MAX_BINARY + 1)
        with self.assertRaises(ValueError): self.build()
        self.assertFalse((self.root / 'bundle').exists())

    def test_external_manifest_digest_rejects_rewritten_metadata(self):
        receipt = self.build(); manifest = self.root / 'bundle/manifest.json'
        value = json.loads(manifest.read_bytes()); value['source_revision'] = 'b' * 40
        raw = bundle.encoded(value); manifest.write_bytes(raw)
        marker = self.root / 'bundle/complete.json'; marker.write_bytes(bundle.encoded({'format': bundle.FORMAT, 'manifest_sha256': bundle.digest(raw)}))
        with self.assertRaises(ValueError): bundle.verify(self.root / 'bundle', receipt['manifest_sha256'])

    def test_interrupted_copy_preserves_partial_without_complete_marker(self):
        original = bundle.read_checked
        def fail_asset(path, maximum):
            if str(path).endswith('.js'): raise OSError('fixture interruption')
            return original(path, maximum)
        with patch.object(bundle, 'read_checked', fail_asset):
            with self.assertRaises(OSError): self.build('interrupted')
        self.assertTrue((self.root / 'interrupted/bin/kaccounting-backend').exists())
        self.assertFalse((self.root / 'interrupted/complete.json').exists())
        with self.assertRaises(OSError): bundle.verify(self.root / 'interrupted')

    def test_frontend_file_count_is_bounded_before_output_creation(self):
        for index in range(bundle.MAX_FILES):
            (self.frontend / 'assets' / ('fixture-' + str(index) + '.js')).write_text('x')
        with self.assertRaises(ValueError): self.build('too-many')
        self.assertFalse((self.root / 'too-many').exists())

    def test_source_checkout_output_and_relative_paths_are_refused(self):
        with self.assertRaises(ValueError): bundle.build(self.binary, self.frontend, bundle.SOURCE / 'never-created-bundle', 'macos-arm64', 'a' * 40)
        with self.assertRaises(ValueError): bundle.build(self.binary, self.frontend, Path('relative'), 'macos-arm64', 'a' * 40)


if __name__ == '__main__': unittest.main()
