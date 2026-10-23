#!/usr/bin/env python3
"""Synthetic non-executable Mach-O fixtures; no app or OS-key access."""
import importlib.util
import json
import os
import plistlib
from pathlib import Path
import stat
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('bundle', Path(__file__).with_name('package-macos-release.py'))
bundle = importlib.util.module_from_spec(spec); spec.loader.exec_module(bundle)


class BundleTests(unittest.TestCase):
    def application(self, name='StoragePicker.app'):
        app = self.root / name
        for part in [app, app / 'Contents', app / 'Contents/MacOS']:
            part.mkdir(mode=0o700)
        executable = app / 'Contents/MacOS/StoragePicker'
        executable.write_bytes(self.binary.read_bytes()); executable.chmod(0o700)
        info = {'CFBundleIdentifier': 'local.kAccounting-Extra-01.StoragePicker',
                'CFBundleExecutable': 'StoragePicker', 'CFBundlePackageType': 'APPL',
                'NSPrincipalClass': 'NSApplication', 'LSUIElement': False}
        path = app / 'Contents/Info.plist'; path.write_bytes(plistlib.dumps(info)); path.chmod(0o600)
        return app
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
    def rewrite_startup(self, name, startup):
        root = self.root / name
        manifest = json.loads((root / 'manifest.json').read_bytes())
        manifest['startup'] = startup
        raw = bundle.encoded(manifest); (root / 'manifest.json').write_bytes(raw)
        expected = bundle.digest(raw)
        (root / 'complete.json').write_bytes(bundle.encoded({'format': bundle.FORMAT, 'manifest_sha256': expected}))
        return expected

    def test_reproducible_build_verify_and_no_real_paths_in_manifest(self):
        a = self.build('first'); b = self.build('second')
        self.assertEqual(a['manifest_sha256'], b['manifest_sha256'])
        self.assertTrue(bundle.verify(self.root / 'first', a['manifest_sha256'])['complete'])
        self.assertNotIn(str(self.root), (self.root / 'first/manifest.json').read_text())
        self.assertEqual(stat.S_IMODE((self.root / 'first/bin/kaccounting-backend').stat().st_mode), 0o700)
        startup = json.loads((self.root / 'first/manifest.json').read_bytes())['startup']
        self.assertEqual(startup['command'], 'serve-chooser')
        self.assertEqual(startup['arguments'], ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend'])
        self.assertIsNone(startup['native_helper'])

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

    def test_complete_native_application_preserves_identity_and_exact_child_path(self):
        app = self.application()
        with patch('subprocess.run', side_effect=AssertionError('packaging never executes')):
            receipt = self.build(helper_bundle=app)
        verified = bundle.verify(self.root / 'bundle', receipt['manifest_sha256'])
        self.assertEqual(verified['native_helper'], bundle.APP_EXECUTABLE)
        self.assertEqual((self.root / 'bundle' / bundle.APP_INFO).read_bytes(), (app / 'Contents/Info.plist').read_bytes())
        self.assertEqual((self.root / 'bundle' / bundle.APP_EXECUTABLE).read_bytes(), self.binary.read_bytes())
        self.assertFalse((self.root / 'bundle/bin/StoragePicker').exists())
        manifest = json.loads((self.root / 'bundle/manifest.json').read_bytes())
        self.assertEqual(manifest['startup']['native_helper'], bundle.APP_EXECUTABLE)
        self.assertEqual(manifest['startup']['command'], 'serve-native-chooser')
        self.assertEqual(manifest['startup']['arguments'], ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend', '<BUNDLE>/' + bundle.APP_EXECUTABLE])
        self.assertFalse(verified['legacy_native_startup'])

    def test_standalone_helper_uses_exact_native_startup_and_legacy_is_explicit(self):
        receipt = self.build(helper=self.binary)
        root = self.root / 'bundle'
        startup = json.loads((root / 'manifest.json').read_bytes())['startup']
        self.assertEqual(startup['command'], 'serve-native-chooser')
        self.assertEqual(startup['arguments'], ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend', '<BUNDLE>/bin/StoragePicker'])
        self.assertFalse(bundle.verify(root, receipt['manifest_sha256'])['legacy_native_startup'])
        legacy = {**startup, 'command': 'serve-chooser', 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend']}
        expected = self.rewrite_startup('bundle', legacy)
        self.assertTrue(bundle.verify(root, expected)['legacy_native_startup'])

    def test_startup_rejects_missing_helper_wrong_argument_order_and_traversal(self):
        self.build('no-helper')
        fake = {'command': 'serve-native-chooser', 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend', '<BUNDLE>/bin/StoragePicker'], 'requires_explicit_book_selection': True, 'existing_books_require_original_keychain_material': True, 'native_helper': 'bin/StoragePicker'}
        expected = self.rewrite_startup('no-helper', fake)
        with self.assertRaises(ValueError): bundle.verify(self.root / 'no-helper', expected)
        self.build('with-helper', helper=self.binary)
        for altered in [
            {**fake, 'native_helper': '../StoragePicker'},
            {**fake, 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend', '<BUNDLE>/bin/../StoragePicker']},
            {**fake, 'arguments': ['<BUNDLE>/frontend', '<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/bin/StoragePicker']},
            {**fake, 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend']},
            {**fake, 'command': 'serve-chooser'},
        ]:
            expected = self.rewrite_startup('with-helper', altered)
            with self.assertRaises(ValueError): bundle.verify(self.root / 'with-helper', expected)

    def test_legacy_complete_app_chooser_is_verified_but_native_gap_reported(self):
        self.build(helper_bundle=self.application())
        startup = json.loads((self.root / 'bundle/manifest.json').read_bytes())['startup']
        expected = self.rewrite_startup('bundle', {**startup, 'command': 'serve-chooser', 'arguments': ['<PRIVATE_BOOK_COLLECTION>', '<BUNDLE>/frontend']})
        verified = bundle.verify(self.root / 'bundle', expected)
        self.assertEqual(verified['native_helper'], bundle.APP_EXECUTABLE)
        self.assertTrue(verified['legacy_native_startup'])

    def test_incomplete_conflicting_unknown_or_unsafe_application_metadata_refused(self):
        app = self.application()
        with self.assertRaises(ValueError): self.build(helper=self.binary, helper_bundle=app)
        path = app / 'Contents/Info.plist'; original = path.read_bytes()
        info = plistlib.loads(original); info['CFBundleExecutable'] = '../other'
        path.write_bytes(plistlib.dumps(info))
        with self.assertRaises(ValueError): self.build(helper_bundle=app)
        path.write_bytes(original)
        extra = app / 'Contents/financial-data'; extra.write_bytes(b'no arbitrary bundle payload')
        with self.assertRaises(ValueError): self.build(helper_bundle=app)
        extra.unlink(); path.unlink()
        with self.assertRaises(ValueError): self.build(helper_bundle=app)
        self.assertFalse((self.root / 'bundle').exists())

    def test_application_links_permissions_and_metadata_tampering_fail_closed(self):
        app = self.application(); executable = app / 'Contents/MacOS/StoragePicker'
        executable.chmod(0o722)
        with self.assertRaises(ValueError): self.build(helper_bundle=app)
        executable.chmod(0o700)
        extra = self.root / 'hardlinked-executable'; os.link(executable, extra)
        with self.assertRaises(ValueError): self.build(helper_bundle=app)
        extra.unlink()
        link = self.root / 'Alias.app'; link.symlink_to(app, target_is_directory=True)
        with self.assertRaises(ValueError): self.build(helper_bundle=link)
        receipt = self.build(helper_bundle=app)
        (self.root / 'bundle' / bundle.APP_INFO).write_bytes(b'tampered identity')
        with self.assertRaises((ValueError, plistlib.InvalidFileException)): bundle.verify(self.root / 'bundle', receipt['manifest_sha256'])

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
