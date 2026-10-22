import importlib.util
from pathlib import Path
import plistlib
import tempfile
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('media', Path(__file__).with_name('media-acceptance.py'))
media = importlib.util.module_from_spec(spec)
spec.loader.exec_module(media)

class MediaSafety(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kAccounting-Extra-01-media-test-')
        self.root = Path(self.temp.name).resolve()
    def tearDown(self):
        self.temp.cleanup()
    def test_rejects_relative_system_and_unmarked_names(self):
        for path in ['.', '/', '/tmp']:
            with self.assertRaises(ValueError): media.checked_root(path)
    def test_rejects_symlink_root(self):
        alias = self.root / 'kAccounting-Extra-01-media-test-alias'
        alias.symlink_to(self.root)
        with self.assertRaises(ValueError): media.checked_root(alias)
    def test_prepare_refuses_existing_contents(self):
        sentinel = self.root / 'keep'
        sentinel.write_text('original')
        with self.assertRaises(ValueError): media.prepare(self.root, 'exfat')
        self.assertEqual(sentinel.read_text(), 'original')
    def test_marker_token_identity_and_unexpected_content(self):
        with patch.object(media, 'filesystem', return_value='exfat'):
            token = media.prepare(self.root, 'exfat')
            self.assertEqual(media.verify(self.root, token, 'exfat'), self.root)
            with self.assertRaises(ValueError): media.verify(self.root, '0' * 64, 'exfat')
            (self.root / 'unexpected').write_text('keep')
            with self.assertRaises(ValueError): media.verify(self.root, token, 'exfat')
    def test_filesystem_parser_rejects_system_unmounted_unknown_and_false_mount(self):
        base = {'FilesystemType': 'ExFAT', 'MountPoint': str(self.root.parent), 'Mounted': True}
        def run(info):
            return lambda *a, **k: types.SimpleNamespace(stdout=plistlib.dumps(info))
        self.assertEqual(media.filesystem(self.root, run(base)), 'exfat')
        for change in [{'FilesystemType': 'hfs'}, {'Mounted': False}, {'MountPoint': '/'}, {'MountPoint': '/Volumes/other'}]:
            with self.assertRaises(ValueError): media.filesystem(self.root, run(dict(base, **change)))
    def test_symlink_marker_is_rejected(self):
        (self.root / media.MARKER).symlink_to('/etc/hosts')
        with self.assertRaises(ValueError): media.verify(self.root, '0' * 64, 'apfs')
    def test_selected_filesystem_must_match(self):
        with patch.object(media, 'filesystem', return_value='apfs'):
            with self.assertRaises(ValueError): media.prepare(self.root, 'exfat')
        self.assertEqual(list(self.root.iterdir()), [])

if __name__ == '__main__': unittest.main()
