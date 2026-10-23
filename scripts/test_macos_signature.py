"""Read-only signature qualification with synthetic files and mocked codesign."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('signature', Path(__file__).with_name('check-macos-signature.py'))
signature = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signature)
SIGNED = ('Identifier=local.kAccounting-Extra-01.backend\n'
          'CodeDirectory v=20500 size=123 flags=0x10000(runtime) hashes=1+7 location=embedded\n'
          'Authority=Developer ID Application: Synthetic Fixture (AAAAAAAAAA)\n'
          'TeamIdentifier=AAAAAAAAAA\n')


class SignatureTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(dir='/private/tmp', prefix='ka-signature-')
        self.path = Path(self.work.name) / 'synthetic-not-executable'
        self.path.write_bytes(b'synthetic fixture, never executed')
        self.path.chmod(0o700)
        self.calls = []

    def tearDown(self):
        self.work.cleanup()

    def runner(self, command, **kwargs):
        self.calls.append(command)
        return subprocess.CompletedProcess(command, 0, '', SIGNED if '--display' in command else '')

    def inspect(self, **kwargs):
        return signature.inspect(self.path, 'local.kAccounting-Extra-01.backend', 'AAAAAAAAAA', **kwargs)

    def test_qualified_signature_does_not_claim_os_or_deployment_acceptance(self):
        result = self.inspect(runner=self.runner)
        self.assertTrue(result['developer_id_qualified'])
        for field in ['notarization_checked', 'production_keychain_qualified', 'login_reboot_qualified', 'private_proxy_qualified', 'installed', 'started']:
            self.assertFalse(result[field])
        self.assertEqual(len(self.calls), 3)
        self.assertTrue(all(command[0] == '/usr/bin/codesign' and '--sign' not in command for command in self.calls))
        requirement = self.calls[2][self.calls[2].index('-R') + 1]
        self.assertIn('anchor apple generic', requirement)
        self.assertIn('1.2.840.113635.100.6.1.13', requirement)

    def test_developer_id_labels_without_apple_certificate_requirement_never_qualify(self):
        def untrusted(command, **kwargs):
            return subprocess.CompletedProcess(command, 1 if '-R' in command else 0, '', SIGNED)
        result = self.inspect(runner=untrusted)
        self.assertTrue(result['signature_verified'])
        self.assertFalse(result['developer_id_qualified'])
        self.assertIn('developer_id_requirement_failed', result['reasons'])

    def test_ad_hoc_signature_is_valid_but_not_developer_id_qualified(self):
        def adhoc(command, **kwargs):
            return subprocess.CompletedProcess(command, 0, '', SIGNED.replace('Authority=Developer ID Application: Synthetic Fixture (AAAAAAAAAA)', 'Signature=adhoc').replace('TeamIdentifier=AAAAAAAAAA', 'TeamIdentifier=not set'))
        result = self.inspect(runner=adhoc)
        self.assertTrue(result['signature_verified'])
        self.assertFalse(result['developer_id_qualified'])
        self.assertIn('developer_id_application_required', result['reasons'])

    def test_wrong_identifier_team_runtime_or_failed_signature_remains_unqualified(self):
        for text in [SIGNED.replace('AAAAAAAAAA', 'BBBBBBBBBB'), SIGNED.replace('local.kAccounting-Extra-01.backend', 'local.other'), SIGNED.replace('0x10000', '0x0')]:
            def runner(command, **kwargs):
                return subprocess.CompletedProcess(command, 0, '', text)
            self.assertFalse(self.inspect(runner=runner)['developer_id_qualified'])
        def failed(command, **kwargs):
            return subprocess.CompletedProcess(command, 1, '', 'raw paths and diagnostics are never printed')
        result = self.inspect(runner=failed)
        self.assertFalse(result['signature_verified'])
        self.assertNotIn('raw paths', str(result))

    def test_no_expected_team_never_qualifies_a_certificate(self):
        result = signature.inspect(self.path, 'local.kAccounting-Extra-01.backend', runner=self.runner)
        self.assertIn('expected_team_id_required', result['reasons'])

    def test_duplicate_metadata_identity_and_oversized_output_fail_closed(self):
        with self.assertRaises(ValueError): signature.metadata(SIGNED + 'TeamIdentifier=BBBBBBBBBB\n')
        with self.assertRaises(ValueError): signature.metadata('x' * 65537)

    def test_changed_binary_symlink_and_invalid_expectations_are_refused(self):
        def changed(command, **kwargs):
            self.path.write_bytes(b'changed fixture')
            return self.runner(command, **kwargs)
        with self.assertRaises(ValueError): self.inspect(runner=changed)
        alias = self.path.parent / 'alias'; alias.symlink_to(self.path)
        with self.assertRaises(ValueError): signature.inspect(alias, 'local.valid', 'AAAAAAAAAA', runner=self.runner)
        with self.assertRaises(ValueError): signature.inspect(self.path, 'bad identifier', runner=self.runner)

    def test_timeout_is_not_a_signature_pass(self):
        def timeout(command, **kwargs):
            raise subprocess.TimeoutExpired(command, 15)
        with self.assertRaises(subprocess.TimeoutExpired): self.inspect(runner=timeout)


if __name__ == '__main__': unittest.main()
