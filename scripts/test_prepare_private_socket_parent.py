"""All administrator commands mocked; no real Tailscale paths are modified."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location('socket_parent', Path(__file__).with_name('prepare-private-socket-parent.py'))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
REAL_PARENT_IDENTITY = MODULE.parent_identity


class SocketParentTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='ka-parent-tool-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.parent = self.root / 'invented-Tailscale'
        self.parent.mkdir(mode=0o755)
        self.receipt = self.root / 'receipt.json'
        self.nonce = 'a' * 32
        self.patches = [mock.patch.object(MODULE, 'PARENT', self.parent),
                        mock.patch.object(MODULE, 'parent_identity', return_value=(1, 2))]
        for patch in self.patches:
            patch.start()
            self.addCleanup(patch.stop)

    def plan(self):
        return MODULE.plan(self.nonce, self.receipt)

    def test_default_plan_never_writes_or_runs_sudo(self):
        with mock.patch.object(MODULE.subprocess, 'run') as execute:
            value, identity = self.plan()
            execute.assert_not_called()
        self.assertEqual(identity, (1, 2))
        self.assertFalse(self.receipt.exists())
        self.assertFalse(Path(value['directory']).exists())
        self.assertEqual(value['commands'], [
            ['/usr/bin/sudo','/bin/mkdir','-m','700',value['directory']],
            ['/usr/bin/sudo','/usr/sbin/chown',f'{os.getuid()}:{os.getgid()}',value['directory']],
        ])

    def test_nonce_existing_directory_and_existing_receipt_refuse_adoption(self):
        for nonce in ['', '../escape', 'F'*32, 'a'*31]:
            with self.assertRaises(ValueError): MODULE.plan(nonce, self.receipt)
        value, _ = self.plan()
        Path(value['directory']).mkdir(mode=0o700)
        with self.assertRaises(ValueError): self.plan()
        Path(value['directory']).rmdir()
        self.receipt.write_text('keep')
        with self.assertRaises(ValueError): self.plan()
        self.assertEqual(self.receipt.read_text(), 'keep')

    def test_apply_without_terminal_never_creates_receipt_or_calls_sudo(self):
        value, identity = self.plan()
        with mock.patch.object(MODULE.sys.stdin,'isatty',return_value=False), mock.patch.object(MODULE.subprocess,'run') as execute:
            with self.assertRaises(ValueError): MODULE.apply(value,identity)
            execute.assert_not_called()
        self.assertFalse(self.receipt.exists())

    def apply(self, first_status=0, second_status=0, nonempty=False):
        value, identity = self.plan()
        calls = []
        def command(arguments,check):
            calls.append(arguments)
            path = Path(value['directory'])
            if len(calls)==1:
                if first_status==0: path.mkdir(mode=0o700)
                return subprocess.CompletedProcess(arguments,first_status)
            if nonempty: (path/'unexpected').write_text('invented')
            return subprocess.CompletedProcess(arguments,second_status)
        with mock.patch.object(MODULE.sys.stdin,'isatty',return_value=True), mock.patch.object(MODULE.sys.stderr,'isatty',return_value=True), mock.patch.object(MODULE,'new_directory',return_value=(7,8)), mock.patch.object(MODULE.subprocess,'run',side_effect=command):
            if first_status or second_status or nonempty:
                with self.assertRaises((RuntimeError,ValueError)): MODULE.apply(value,identity)
            else:
                self.assertEqual(MODULE.apply(value,identity)['status'],'new_empty_private_socket_parent_ready')
        return value,calls

    def test_apply_uses_only_two_exact_commands_and_writes_private_receipt(self):
        value,calls=self.apply()
        self.assertEqual(calls,value['commands'])
        self.assertEqual(self.receipt.stat().st_mode&0o777,0o600)
        self.assertEqual(json.loads(self.receipt.read_text())['status'],'new_empty_private_socket_parent_ready')

    def test_failed_mkdir_never_chowns_or_adopts_anything(self):
        _,calls=self.apply(first_status=1)
        self.assertEqual(len(calls),1)
        self.assertEqual(json.loads(self.receipt.read_text())['status'],'incomplete_preserve_for_inspection')

    def test_failed_chown_or_nonempty_result_preserves_new_directory(self):
        value,calls=self.apply(second_status=1)
        self.assertEqual(len(calls),2)
        self.assertTrue(Path(value['directory']).is_dir())

    def test_nonempty_result_is_not_removed_or_claimed_ready(self):
        value,_=self.apply(nonempty=True)
        self.assertTrue((Path(value['directory'])/'unexpected').exists())
        self.assertEqual(json.loads(self.receipt.read_text())['status'],'incomplete_preserve_for_inspection')

    def test_parent_identity_change_refuses_before_root_call(self):
        value,identity=self.plan()
        with mock.patch.object(MODULE.sys.stdin,'isatty',return_value=True), mock.patch.object(MODULE.sys.stderr,'isatty',return_value=True), mock.patch.object(MODULE,'parent_identity',return_value=(9,9)), mock.patch.object(MODULE.subprocess,'run') as execute:
            with self.assertRaises(ValueError): MODULE.apply(value,identity)
            execute.assert_not_called()

    def test_user_owned_or_symlink_parent_cannot_pass_root_parent_preflight(self):
        if os.getuid() != 0:
            with self.assertRaises(ValueError): REAL_PARENT_IDENTITY()
        link = self.root / 'parent-link'
        link.symlink_to(self.parent, target_is_directory=True)
        with mock.patch.object(MODULE, 'PARENT', link):
            with self.assertRaises(OSError): REAL_PARENT_IDENTITY()

    def test_private_receipt_parent_and_new_directory_identity_are_enforced(self):
        self.root.chmod(0o755)
        try:
            with self.assertRaises(ValueError): self.plan()
        finally: self.root.chmod(0o700)
        target=self.root/'new';target.mkdir(mode=0o700)
        self.assertEqual(MODULE.new_directory(target,os.getuid()),(target.stat().st_dev,target.stat().st_ino))
        if os.getuid() != 0:
            with self.assertRaises(ValueError): MODULE.new_directory(target,0)
        with self.assertRaises(ValueError): MODULE.new_directory(target,os.getuid(),(0,0))
        target.chmod(0o755)
        with self.assertRaises(ValueError): MODULE.new_directory(target,os.getuid())


if __name__=='__main__': unittest.main()
