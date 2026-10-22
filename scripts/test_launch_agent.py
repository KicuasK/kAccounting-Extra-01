"""Validate launcher preparation with disposable, invented paths only."""
import hashlib
import json
import os
from unittest import mock
import importlib.util
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

MODULE_PATH = Path(__file__).with_name("make-launch-agent.py")
SPEC = importlib.util.spec_from_file_location("launcher", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ka-launcher-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.binary = self.root / "test binary"
        self.binary.write_text("#!/bin/sh\nexit 0\n")
        self.binary.chmod(0o700)
        self.book = self.root / "test book"
        self.book.mkdir(mode=0o700)
        for filename in ["book.sqlite3", "device-key.json"]:
            (self.book / filename).write_text("invented launcher fixture")
        self.assets = self.root / "frontend assets"
        self.assets.mkdir()
        (self.assets / "index.html").write_text("<!doctype html>")

    def test_arguments_preserve_spaces_without_a_shell(self):
        value = MODULE.configuration(self.binary, self.book)
        roundtrip = plistlib.loads(plistlib.dumps(value))
        self.assertEqual(roundtrip["ProgramArguments"], [str(self.binary), "books", str(self.book)])
        self.assertNotIn("serve", roundtrip["ProgramArguments"])
        self.assertEqual(roundtrip["Umask"], 0o077)
        self.assertFalse(roundtrip["KeepAlive"])
        self.assertTrue(roundtrip["Disabled"])

    def test_assets_enable_session_without_selecting_any_book(self):
        value = MODULE.configuration(self.binary, self.book, self.assets)
        roundtrip = plistlib.loads(plistlib.dumps(value))
        self.assertEqual(roundtrip["ProgramArguments"], [
            str(self.binary), "serve-chooser", str(self.book), str(self.assets),
        ])
        self.assertNotIn("serve", roundtrip["ProgramArguments"])
        self.assertFalse(roundtrip["KeepAlive"])
        self.assertTrue(roundtrip["Disabled"])
        self.assertTrue(roundtrip["RunAtLoad"])

    def test_missing_relative_or_symlink_assets_are_rejected(self):
        for assets in [Path("relative-assets"), self.root / "missing-assets", self.binary]:
            with self.assertRaises((ValueError, OSError)):
                MODULE.configuration(self.binary, self.book, assets)
        link = self.root / "asset-link"
        link.symlink_to(self.assets, target_is_directory=True)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book, link)
        (self.assets / "index.html").unlink()
        with self.assertRaises((ValueError, OSError)):
            MODULE.configuration(self.binary, self.book, self.assets)
        (self.assets / "index.html").symlink_to(self.binary)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book, self.assets)

    def test_cli_prepares_session_launcher_without_running_it(self):
        destination = self.root / "session.plist"
        result = subprocess.run([
            sys.executable, str(MODULE_PATH), "--binary", str(self.binary),
            "--books-root", str(self.book), "--assets", str(self.assets),
            "--output", str(destination),
        ], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        value = plistlib.loads(destination.read_bytes())
        self.assertEqual(value["ProgramArguments"][1], "serve-chooser")
        self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
        self.assertIn("nothing was installed or started", result.stdout)

    def test_public_book_and_symbolic_link_binary_are_rejected(self):
        self.book.chmod(0o755)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book)
        self.book.chmod(0o700)
        link = self.root / "binary-link"
        link.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            MODULE.configuration(link, self.book)

    def test_output_is_private_and_existing_launcher_is_preserved(self):
        destination = self.root / "prepared.plist"
        arguments = [sys.executable, str(MODULE_PATH), "--binary", str(self.binary), "--books-root", str(self.book), "--output", str(destination)]
        result = subprocess.run(arguments, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
        original = destination.read_bytes()
        result = subprocess.run(arguments, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(destination.read_bytes(), original)

    def run_preparer(self, destination, *extra):
        return subprocess.run([
            sys.executable, str(MODULE_PATH), "--binary", str(self.binary),
            "--books-root", str(self.book), "--output", str(destination), *extra,
        ], capture_output=True, text=True)

    def test_preflight_reports_identity_without_writing_or_executing(self):
        # An executable containing a marker write must never be executed.
        marker = self.root / "executed"
        self.binary.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
        destination = self.root / "preflight.plist"
        result = self.run_preparer(destination, "--preflight", "--assets", str(self.assets))
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["mode"], "serve-chooser")
        self.assertTrue(report["disabled"])
        self.assertFalse(report["installed"])
        self.assertFalse(report["started"])
        self.assertEqual(report["executable_identity"]["sha256"], hashlib.sha256(self.binary.read_bytes()).hexdigest())
        self.assertFalse(destination.exists())
        self.assertFalse(marker.exists())
        self.assertFalse((self.book / "selection.stdout.log").exists())

    def test_output_requires_private_parent_and_rejects_symlink_parents(self):
        public = self.root / "public"
        public.mkdir(mode=0o755)
        result = self.run_preparer(public / "plan.plist")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((public / "plan.plist").exists())
        link = self.root / "parent-link"
        link.symlink_to(self.root, target_is_directory=True)
        result = self.run_preparer(link / "plan.plist")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "plan.plist").exists())

    def test_output_cannot_install_or_use_source_tree(self):
        agents = self.root / "LaunchAgents"
        agents.mkdir(mode=0o700)
        self.assertNotEqual(self.run_preparer(agents / "plan.plist").returncode, 0)
        self.assertFalse((agents / "plan.plist").exists())
        source = self.root / "invented-source"
        source.mkdir(mode=0o700)
        with mock.patch.object(MODULE, "SOURCE_ROOT", source):
            with self.assertRaises(ValueError):
                MODULE.output_parent(source / "plan.plist")
            with self.assertRaises(ValueError):
                MODULE.configuration(self.binary, source)
        self.assertFalse((source / "plan.plist").exists())

    def test_log_symlinks_hardlinks_and_public_files_are_rejected(self):
        log = self.book / "selection.stdout.log"
        log.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book)
        log.unlink()
        os.link(self.binary, log)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book)
        log.unlink()
        log.write_text("invented log")
        log.chmod(0o644)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book)
        log.chmod(0o600)
        self.assertEqual(MODULE.configuration(self.binary, self.book)["Label"], "local.kAccounting-Extra-01")

    def test_output_symlink_and_traversal_do_not_replace_files(self):
        original = self.binary.read_bytes()
        output = self.root / "linked.plist"
        output.symlink_to(self.binary)
        self.assertNotEqual(self.run_preparer(output).returncode, 0)
        self.assertEqual(self.binary.read_bytes(), original)
        traversal = self.root / "missing" / ".." / "plan.plist"
        self.assertNotEqual(self.run_preparer(traversal).returncode, 0)
        self.assertFalse((self.root / "plan.plist").exists())

    def test_executable_identity_rejects_nonexecutable_or_publicly_writable_binary(self):
        for permissions in [0o600, 0o722, 0o777]:
            self.binary.chmod(permissions)
            with self.assertRaises(ValueError):
                MODULE.configuration(self.binary, self.book)
        self.binary.chmod(0o700)
        identity = MODULE.executable_identity(self.binary)
        self.assertEqual(identity["inode"], self.binary.stat().st_ino)

    def private_options(self):
        config = self.root / 'private-config.json'
        config.write_text(json.dumps({'hostname':'book.example.ts.net', 'login':'FakeOwner@github', 'socket':str(self.root / 'proxy.sock')}))
        config.chmod(0o600)
        helper = self.root / 'trusted native helper'
        helper.write_bytes(self.binary.read_bytes())
        helper.chmod(0o700)
        return config, helper

    def test_explicit_private_native_modes_preserve_exact_production_argument_order(self):
        config, helper = self.private_options()
        for private, native, mode, extra in [
            (config, None, 'serve-private-chooser', [str(config)]),
            (None, helper, 'serve-native-chooser', [str(helper)]),
            (config, helper, 'serve-private-native-chooser', [str(config), str(helper)]),
        ]:
            with self.subTest(mode=mode):
                value = MODULE.configuration(self.binary, self.book, self.assets, private, native)
                self.assertEqual(value['ProgramArguments'], [str(self.binary),mode,str(self.book),str(self.assets),*extra])
                self.assertTrue(value['Disabled'])
                self.assertFalse(value['KeepAlive'])
                self.assertNotIn('FakeOwner@github', plistlib.dumps(value).decode())
                self.assertNotIn('book.example.ts.net', plistlib.dumps(value).decode())

    def test_private_native_options_require_assets_and_cannot_write_on_rejection(self):
        config, helper = self.private_options()
        for options in [(config, None), (None, helper), (config, helper)]:
            with self.assertRaises(ValueError):
                MODULE.configuration(self.binary,self.book,None,*options)
        destination = self.root / 'rejected.plist'
        result = self.run_preparer(destination,'--private-config',str(config))
        self.assertNotEqual(result.returncode,0)
        self.assertFalse(destination.exists())

    def test_private_config_modes_links_source_and_size_fail_closed(self):
        config, _ = self.private_options()
        original = config.read_bytes()
        for mode in [0o644,0o640,0o400]:
            config.chmod(mode)
            with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        config.chmod(0o600)
        hardlink=self.root/'config-hardlink';os.link(config,hardlink)
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        hardlink.unlink()
        alias=self.root/'config-link';alias.symlink_to(config)
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(alias)
        parent_alias=self.root/'parent-alias';parent_alias.symlink_to(self.root,target_is_directory=True)
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(parent_alias/config.name)
        with mock.patch.object(MODULE,'SOURCE_ROOT',self.root):
            with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        config.write_bytes(b'x'*16385)
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        config.write_bytes(original)
        self.root.chmod(0o755)
        try:
            with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        finally:self.root.chmod(0o700)

    def test_private_config_parent_replacement_during_read_is_refused(self):
        parent = self.root / 'configuration-directory'
        parent.mkdir(mode=0o700)
        config = parent / 'private.json'
        config.write_text(json.dumps({'hostname':'book.example.ts.net','login':'owner@github','socket':str(self.root/'proxy.sock')}))
        config.chmod(0o600)
        original_read = os.read
        replaced = False
        def replace_after_read(descriptor, count):
            nonlocal replaced
            result = original_read(descriptor,count)
            if not replaced:
                replaced = True
                parent.rename(self.root/'original-configuration-directory')
                parent.mkdir(mode=0o700)
            return result
        with mock.patch.object(MODULE.os,'read',side_effect=replace_after_read):
            with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        self.assertEqual(list(parent.iterdir()),[])
        self.assertTrue((self.root/'original-configuration-directory/private.json').is_file())

    def test_private_schema_policy_and_socket_validation_are_strict(self):
        config, _ = self.private_options()
        original=json.loads(config.read_text())
        documents=[{},dict(original,extra=True),dict(original,hostname=True),dict(original,hostname='BOOK.example.ts.net'),dict(original,login='owner@localhost'),dict(original,login='owner@GITHUB'),dict(original,socket='relative.sock'),dict(original,socket=str(self.root/('x'*104)))]
        for document in documents:
            with self.subTest(document=document):
                config.write_text(json.dumps(document))
                with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        config.write_text('{"hostname":"book.example.ts.net","hostname":"other.example.ts.net","login":"owner@github","socket":"/private/tmp/x"}')
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        config.write_text(json.dumps(original))
        (self.root/'proxy.sock').write_text('preserve existing destination')
        with self.assertRaises(ValueError):MODULE.private_configuration_identity(config)
        self.assertEqual((self.root/'proxy.sock').read_text(),'preserve existing destination')

    def test_native_helper_identity_checks_owner_parent_source_permissions_and_bound(self):
        _, helper = self.private_options()
        for mode in [0o600,0o722,0o777]:
            helper.chmod(mode)
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
        helper.chmod(0o700)
        alias=self.root/'helper-link';alias.symlink_to(helper)
        with self.assertRaises(ValueError):MODULE.native_helper_identity(alias)
        with mock.patch.object(MODULE,'SOURCE_ROOT',self.root):
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
        with mock.patch.object(MODULE.os,'getuid',return_value=os.getuid()+1):
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
        self.root.chmod(0o777)
        try:
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
        finally:self.root.chmod(0o700)
        with helper.open('wb') as output:output.truncate(16*1024*1024+1)
        with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)

    def test_private_native_preflight_fingerprints_without_execution_or_identity_disclosure(self):
        config, helper = self.private_options()
        marker=self.root/'unexpected-execution'
        helper.write_text(f"#!/bin/sh\ntouch '{marker}'\n")
        destination=self.root/'private-native.plist'
        options=['--assets',str(self.assets),'--private-config',str(config),'--native-helper',str(helper)]
        result=self.run_preparer(destination,*options,'--preflight')
        self.assertEqual(result.returncode,0,result.stderr)
        value=json.loads(result.stdout)
        self.assertEqual(value['mode'],'serve-private-native-chooser')
        self.assertTrue(value['disabled'])
        self.assertFalse(value['installed'])
        self.assertFalse(value['started'])
        self.assertEqual(value['private_config_identity']['sha256'],hashlib.sha256(config.read_bytes()).hexdigest())
        self.assertEqual(value['native_helper_identity']['sha256'],hashlib.sha256(helper.read_bytes()).hexdigest())
        self.assertNotIn('FakeOwner@github',result.stdout)
        self.assertNotIn('book.example.ts.net',result.stdout)
        self.assertFalse(marker.exists())
        self.assertFalse(destination.exists())
        result=self.run_preparer(destination,*options)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertTrue(plistlib.loads(destination.read_bytes())['Disabled'])
        self.assertEqual(destination.stat().st_mode&0o777,0o600)
        self.assertFalse(marker.exists())

    def test_helper_growth_between_precheck_and_hash_is_refused_before_reading(self):
        _, helper = self.private_options()
        original_identity = MODULE.executable_identity
        def grow_before_hash(path, maximum_size=None):
            with helper.open('wb') as output:output.truncate(16*1024*1024+1)
            return original_identity(path,maximum_size=maximum_size)
        with mock.patch.object(MODULE,'executable_identity',side_effect=grow_before_hash), mock.patch.object(MODULE.os,'read',wraps=os.read) as read:
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
            read.assert_not_called()

    def test_helper_growth_during_hash_has_a_bounded_descriptor_read(self):
        _, helper = self.private_options()
        original_read = os.read
        grown = False
        consumed = 0
        def grow_while_reading(descriptor,count):
            nonlocal grown,consumed
            result = original_read(descriptor,count)
            consumed += len(result)
            if not grown:
                grown = True
                with helper.open('ab') as output:output.truncate(18*1024*1024)
            return result
        with mock.patch.object(MODULE.os,'read',side_effect=grow_while_reading):
            with self.assertRaises(ValueError):MODULE.native_helper_identity(helper)
        self.assertLessEqual(consumed,16*1024*1024+1)

    def test_entire_frontend_tree_rejects_public_directory_and_writable_html_js_css(self):
        nested=self.assets/'assets';nested.mkdir()
        script=nested/'app.js';script.write_text('console.log("invented")')
        stylesheet=nested/'app.css';stylesheet.write_text('body{}')
        self.assertEqual(MODULE.frontend_tree(self.assets),self.assets)
        for directory in [self.assets,nested]:
            directory.chmod(0o777)
            with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
            directory.chmod(0o755)
        for file in [self.assets/'index.html',script,stylesheet]:
            file.chmod(0o666)
            with self.subTest(file=file),self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
            file.chmod(0o644)
        self.assertEqual(MODULE.frontend_tree(self.assets),self.assets)

    def test_frontend_tree_rejects_symlinks_hardlinks_special_files_and_wrong_owner(self):
        nested=self.assets/'assets';nested.mkdir()
        script=nested/'app.js';script.write_text('invented')
        alias=nested/'alias.js';alias.symlink_to(script)
        with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
        alias.unlink()
        os.link(script,alias)
        with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
        alias.unlink()
        alias.symlink_to(nested,target_is_directory=True)
        with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
        alias.unlink()
        os.mkfifo(alias)
        with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)
        alias.unlink()
        with mock.patch.object(MODULE.os,'getuid',return_value=os.getuid()+1):
            with self.assertRaises(ValueError):MODULE.frontend_tree(self.assets)

    def test_legacy_preselected_book_arguments_cannot_create_a_launcher(self):
        destination = self.root / "not-created.plist"
        result = subprocess.run([
            sys.executable, str(MODULE_PATH), "--binary", str(self.binary),
            "--book", str(self.book), "--assets", str(self.assets), "--output", str(destination),
        ], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
