"""Validate launcher preparation with disposable, invented paths only."""
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
        value = MODULE.configuration(self.binary, self.book, self.assets)
        roundtrip = plistlib.loads(plistlib.dumps(value))
        self.assertEqual(roundtrip["ProgramArguments"], [str(self.binary), "serve", str(self.book), str(self.assets)])
        self.assertEqual(roundtrip["Umask"], 0o077)
        self.assertFalse(roundtrip["KeepAlive"]["SuccessfulExit"])

    def test_public_book_and_symbolic_link_binary_are_rejected(self):
        self.book.chmod(0o755)
        with self.assertRaises(ValueError):
            MODULE.configuration(self.binary, self.book, self.assets)
        self.book.chmod(0o700)
        link = self.root / "binary-link"
        link.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            MODULE.configuration(link, self.book, self.assets)

    def test_output_is_private_and_existing_launcher_is_preserved(self):
        destination = self.root / "prepared.plist"
        arguments = [sys.executable, str(MODULE_PATH), "--binary", str(self.binary), "--book", str(self.book), "--assets", str(self.assets), "--output", str(destination)]
        result = subprocess.run(arguments, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
        original = destination.read_bytes()
        result = subprocess.run(arguments, capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(destination.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
