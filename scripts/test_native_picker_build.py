"""No GUI: verify compile-only helper publication and preflight."""
import importlib.util
import os
from pathlib import Path
import plistlib
import struct
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_picker_build', Path(__file__).with_name('build-native-picker.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class NativeBuildTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ka-picker-build-test-')
        self.parent = Path(self.temporary.name).resolve()
        self.output = self.parent / 'StoragePicker'

    def tearDown(self):
        self.temporary.cleanup()

    @staticmethod
    def fake_compile(arguments, check):
        assert check
        Path(arguments[-1]).write_bytes(b'fake compiled executable')

    def test_private_output_is_exclusively_published(self):
        with patch.object(module.subprocess, 'run', side_effect=self.fake_compile):
            module.build(str(self.output))
        self.assertEqual(self.output.read_bytes(), b'fake compiled executable')
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.output.stat().st_nlink, 1)

    def test_complete_app_bundle_is_atomically_published(self):
        output = self.parent / 'StoragePicker.app'
        with patch.object(module.subprocess, 'run', side_effect=self.fake_compile):
            module.build(str(output), 'local.kAccounting-Extra-01.NativeTest')
        executable = output / 'Contents/MacOS/StoragePicker'
        self.assertEqual(executable.read_bytes(), b'fake compiled executable')
        self.assertEqual(executable.stat().st_mode & 0o777, 0o700)
        self.assertEqual(executable.stat().st_nlink, 1)

    def test_published_helpers_are_accepted_by_runtime_packager(self):
        spec = importlib.util.spec_from_file_location('package_test', Path(__file__).with_name('package-macos-release.py'))
        package = importlib.util.module_from_spec(spec); spec.loader.exec_module(package)
        def fake_macho(arguments, check):
            self.assertTrue(check)
            Path(arguments[-1]).write_bytes(struct.pack('<II', 0xfeedfacf, package.TARGETS['macos-arm64']) + b'synthetic fixture')
        for name in ['StoragePicker', 'StoragePicker.app']:
            output = self.parent / name
            with patch.object(module.subprocess, 'run', side_effect=fake_macho):
                module.build(str(output))
            executable = output / 'Contents/MacOS/StoragePicker' if output.suffix == '.app' else output
            package.regular(executable, package.MAX_HELPER, True)
            package.macho(executable, package.MAX_HELPER, 'macos-arm64')
            self.assertEqual(executable.stat().st_nlink, 1)

    def test_app_metadata_is_preserved(self):
        output = self.parent / 'StoragePicker.app'
        with patch.object(module.subprocess, 'run', side_effect=self.fake_compile):
            module.build(str(output), 'local.kAccounting-Extra-01.NativeTest')
        with (output / 'Contents/Info.plist').open('rb') as file:
            info = plistlib.load(file)
        self.assertEqual(info['CFBundleIdentifier'], 'local.kAccounting-Extra-01.NativeTest')
        self.assertEqual(info['CFBundleExecutable'], 'StoragePicker')
        self.assertIs(info['LSUIElement'], False)
        self.assertEqual(output.stat().st_mode & 0o777, 0o700)
        self.assertEqual((output / 'Contents/Info.plist').stat().st_mode & 0o777, 0o600)

    def test_bundle_appearing_during_compile_is_preserved(self):
        output = self.parent / 'StoragePicker.app'
        def compile_with_conflict(arguments, check):
            self.fake_compile(arguments, check)
            output.mkdir(mode=0o700)
            (output / 'sentinel').write_bytes(b'keep')
        with patch.object(module.subprocess, 'run', side_effect=compile_with_conflict):
            with self.assertRaises(FileExistsError):
                module.build(str(output))
        self.assertEqual((output / 'sentinel').read_bytes(), b'keep')

    def test_even_an_empty_bundle_appearing_during_compile_is_not_replaced(self):
        output = self.parent / 'StoragePicker.app'
        def compile_with_conflict(arguments, check):
            self.fake_compile(arguments, check)
            output.mkdir(mode=0o700)
        with patch.object(module.subprocess, 'run', side_effect=compile_with_conflict):
            with self.assertRaises(FileExistsError):
                module.build(str(output))
        self.assertEqual(list(output.iterdir()), [])

    def test_invalid_bundle_identifier_is_rejected(self):
        for identifier in ['../untrusted', 'no spaces.allowed', '']:
            with self.assertRaises(ValueError):
                module.build(str(self.parent / 'StoragePicker.app'), identifier)

    def test_compiler_symlink_cannot_modify_or_publish_target(self):
        target = self.parent / 'sentinel'
        target.write_bytes(b'keep')
        target.chmod(0o600)
        def symbolic_output(arguments, check):
            Path(arguments[-1]).symlink_to(target)
        with patch.object(module.subprocess, 'run', side_effect=symbolic_output):
            with self.assertRaises(ValueError):
                module.build(str(self.output))
        self.assertFalse(self.output.exists())
        self.assertEqual(target.read_bytes(), b'keep')
        self.assertEqual(target.stat().st_mode & 0o777, 0o600)

    def test_compiler_hardlink_cannot_change_or_publish_an_existing_file(self):
        target = self.parent / 'private-sentinel'
        target.write_bytes(b'keep')
        target.chmod(0o600)
        def hardlinked_output(arguments, check):
            os.link(target, arguments[-1])
        with patch.object(module.subprocess, 'run', side_effect=hardlinked_output):
            with self.assertRaises(ValueError):
                module.build(str(self.output))
        self.assertFalse(self.output.exists())
        self.assertEqual(target.read_bytes(), b'keep')
        self.assertEqual(target.stat().st_mode & 0o777, 0o600)

    def test_parent_permissions_changed_during_compile_prevent_publication(self):
        def public_parent(arguments, check):
            self.fake_compile(arguments, check)
            self.parent.chmod(0o755)
        try:
            with patch.object(module.subprocess, 'run', side_effect=public_parent):
                with self.assertRaises(ValueError):
                    module.build(str(self.output))
            self.assertFalse(self.output.exists())
        finally:
            self.parent.chmod(0o700)

    def test_existing_output_is_preserved(self):
        self.output.write_bytes(b'keep')
        with patch.object(module.subprocess, 'run') as compiler:
            with self.assertRaises(ValueError):
                module.build(str(self.output))
            compiler.assert_not_called()
        self.assertEqual(self.output.read_bytes(), b'keep')

    def test_output_created_during_compile_is_never_overwritten(self):
        def compile_with_conflict(arguments, check):
            self.fake_compile(arguments, check)
            self.output.write_bytes(b'appeared concurrently')
        with patch.object(module.subprocess, 'run', side_effect=compile_with_conflict):
            with self.assertRaises(FileExistsError):
                module.build(str(self.output))
        self.assertEqual(self.output.read_bytes(), b'appeared concurrently')

    def test_nonprivate_parent_is_rejected_before_compile(self):
        self.parent.chmod(0o755)
        try:
            with patch.object(module.subprocess, 'run') as compiler:
                with self.assertRaises(ValueError):
                    module.build(str(self.output))
                compiler.assert_not_called()
        finally:
            self.parent.chmod(0o700)

    def test_symlink_parent_is_rejected(self):
        link = self.parent / 'alias'
        target = self.parent / 'target'
        target.mkdir(mode=0o700)
        link.symlink_to(target, target_is_directory=True)
        with self.assertRaises(ValueError):
            module.build(str(link / 'StoragePicker'))

    def test_source_checkout_and_relative_outputs_are_rejected(self):
        for output in ['relative', str(Path(__file__).resolve().parent.parent / 'StoragePicker')]:
            with self.assertRaises(ValueError):
                module.build(output)


if __name__ == '__main__':
    unittest.main()
