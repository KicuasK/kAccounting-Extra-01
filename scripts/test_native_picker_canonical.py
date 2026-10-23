"""Compile the actual helper canonicalizer without launching AppKit or a panel."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class CanonicalNativeSelectionTests(unittest.TestCase):
    def test_trusted_initial_directory_is_canonical_existing_directory_for_book_destination_only(self):
        source = (Path(__file__).resolve().parent.parent / 'native/StoragePicker.swift').read_text()
        functions = []
        for name in ['posixCanonicalPath', 'validatedInitialDirectoryPath']:
            start = source.index('func ' + name + '(')
            end = source.index('\n}', start) + 2
            functions.append(source[start:end])
        with tempfile.TemporaryDirectory(prefix='ka-native-initial-') as temporary:
            root = Path(temporary).resolve()
            alias = root / 'alias'; alias.symlink_to(root, target_is_directory=True)
            file = root / 'file'; file.write_text('fixture')
            checks = ['precondition(validatedInitialDirectoryPath(' + json.dumps(str(root)) + ', kind: "book_destination") == ' + json.dumps(str(root)) + ')']
            for path in [str(alias), str(file), str(root / 'missing'), 'relative', '/tmp', str(root) + '/.', str(root) + '/\n']:
                checks.append('precondition(validatedInitialDirectoryPath(' + json.dumps(path) + ', kind: "book_destination") == nil)')
            checks.append('precondition(validatedInitialDirectoryPath(' + json.dumps(str(root)) + ', kind: "request_destination") == nil)')
            checks.append('print("PASS: trusted initial directory validated without opening panel")')
            program = 'import Foundation\nimport Darwin\n' + '\n'.join(functions + checks)
            cache = os.environ.get('NATIVE_SWIFT_MODULE_CACHE', str(root / 'module-cache'))
            result = subprocess.run(['/usr/bin/swift', '-module-cache-path', cache, '-e', program], check=True, text=True, capture_output=True)
        self.assertIn('PASS:', result.stdout)
    def test_foundation_shortened_paths_match_posix_canonical_paths(self):
        source = (Path(__file__).resolve().parent.parent / 'native/StoragePicker.swift').read_text()
        start = source.index('func posixCanonicalPath(')
        end = source.index('\n}', start) + 2
        actual_function = source[start:end]
        samples = ['/private/tmp', tempfile.gettempdir()]
        cases = [(sample, os.path.realpath(sample)) for sample in samples]
        checks = '\n'.join(
            'precondition(posixCanonicalPath(URL(fileURLWithPath: ' + json.dumps(sample) + ').standardizedFileURL.path) == ' + json.dumps(expected) + ')'
            for sample, expected in cases
        )
        program = 'import Foundation\nimport Darwin\n' + actual_function + '\n' + checks + '\nprint("PASS: native canonical paths match POSIX after Foundation normalization")\n'
        with tempfile.TemporaryDirectory(prefix='ka-native-canonical-') as temporary:
            cache = os.environ.get('NATIVE_SWIFT_MODULE_CACHE', str(Path(temporary) / 'module-cache'))
            result = subprocess.run(['/usr/bin/swift', '-module-cache-path', cache, '-e', program], check=True, text=True, capture_output=True)
        self.assertIn('PASS:', result.stdout)


    def test_save_parent_allows_system_aliases_but_refuses_custom_symlinks(self):
        source = (Path(__file__).resolve().parent.parent / 'native/StoragePicker.swift').read_text()
        functions = []
        for name in ['posixCanonicalPath', 'nonSymlinkCanonicalPath']:
            start = source.index('func ' + name + '(')
            end = source.index('\n}', start) + 2
            functions.append(source[start:end])
        with tempfile.TemporaryDirectory(prefix='ka-native-save-parent-') as temporary:
            root = Path(temporary).resolve()
            alias = root / 'custom-link'
            alias.symlink_to(root, target_is_directory=True)
            canonical = str(root)
            shortened = canonical.replace('/private/var/', '/var/', 1).replace('/private/tmp/', '/tmp/', 1)
            checks = [
                'let destination = URL(fileURLWithPath: ' + json.dumps(str(root / 'new-request.json')) + ').standardizedFileURL',
                'let parent = destination.deletingLastPathComponent()',
                'precondition(nonSymlinkCanonicalPath(parent.path) == ' + json.dumps(canonical) + ')',
                'precondition(nonSymlinkCanonicalPath(' + json.dumps(shortened) + ') == ' + json.dumps(canonical) + ')',
                'precondition(nonSymlinkCanonicalPath(' + json.dumps(str(alias)) + ') == nil)',
                'precondition(nonSymlinkCanonicalPath(' + json.dumps(str(root / 'missing')) + ') == nil)',
                'precondition(!FileManager.default.fileExists(atPath: destination.path))',
                'print("PASS: save parent canonical aliases accepted; custom symlinks refused; destination not created")',
            ]
            program = 'import Foundation\nimport Darwin\n' + '\n'.join(functions + checks)
            cache = os.environ.get('NATIVE_SWIFT_MODULE_CACHE', str(root / 'module-cache'))
            result = subprocess.run(['/usr/bin/swift', '-module-cache-path', cache, '-e', program], check=True, text=True, capture_output=True)
        self.assertIn('PASS:', result.stdout)


if __name__ == '__main__':
    unittest.main()
