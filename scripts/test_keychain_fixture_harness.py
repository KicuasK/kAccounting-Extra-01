#!/usr/bin/env python3
"""Compile and test the argument safety gate; NEVER perform live Keychain calls."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[1] / 'native' / 'IsolatedKeychainAcceptance.swift'


class ArgumentBoundary(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.work = tempfile.TemporaryDirectory(prefix='ka-keychain-code-test-', dir='/private/tmp')
        cls.executable = Path(cls.work.name) / 'fixture'
        subprocess.run(['/usr/bin/swiftc', '-suppress-warnings', '-framework', 'Security',
                        '-module-cache-path', str(Path(cls.work.name) / 'cache'),
                        str(SOURCE), '-o', str(cls.executable)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.work.cleanup()

    def invoke(self, *args):
        return subprocess.run([str(self.executable), *args], capture_output=True, text=True,
                              timeout=10, check=False)

    def test_preflight_is_pure_and_reports_operational_permission_boundary(self):
        result = self.invoke('--preflight')
        self.assertEqual(result.returncode, 0)
        self.assertIn('os_operations_performed=false', result.stdout)
        self.assertNotIn('owned_fixture_path=', result.stdout)
        self.assertNotIn('preferences_before_sha256=', result.stdout)

    def test_fixture_credential_encoding_is_ascii_utf8_without_os_calls(self):
        result = self.invoke('--self-test-credential-format')
        self.assertEqual(result.returncode, 0)
        self.assertIn('fixture_credential_ascii_utf8_format_verified=true', result.stdout)
        self.assertIn('os_operations_performed=false', result.stdout)
        self.assertNotIn('preferences_before_sha256=', result.stdout)

    def test_absent_or_unknown_opt_in_is_rejected_before_operational_output(self):
        for args in [(), ('--run',), ('--allow-new-test-items',), ('--preflight', 'extra')]:
            with self.subTest(args=args):
                result = self.invoke(*args)
                self.assertEqual(result.returncode, 64)
                self.assertEqual(result.stdout, '')

    def test_operational_flag_with_extra_argument_is_not_authorization(self):
        result = self.invoke('--run-approved-isolated-keychain', 'extra')
        self.assertEqual(result.returncode, 64)
        self.assertEqual(result.stdout, '')


if __name__ == '__main__':
    unittest.main()
