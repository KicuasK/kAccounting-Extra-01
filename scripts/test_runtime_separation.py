"""Verify source/runtime guard using isolated fake checkouts, never user books."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("verify-backend.sh")


class RuntimeSeparationTests(unittest.TestCase):
    def check_rejected(self, name, kind="file"):
        with tempfile.TemporaryDirectory(prefix="ka-source-guard-") as temporary:
            root = Path(temporary)
            (root / "scripts").mkdir()
            script = root / "scripts" / SCRIPT.name
            shutil.copyfile(SCRIPT, script)
            script.chmod(0o700)
            candidate = root / name
            if kind == "directory":
                candidate.mkdir()
            elif kind == "symlink":
                candidate.symlink_to(root / "missing-target")
            else:
                candidate.write_bytes(b"invented guard fixture; no financial data")
            result = subprocess.run(
                [str(script)], cwd=root, capture_output=True, text=True,
                env=os.environ.copy(), timeout=15,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Runtime databases, backups, transfer packages", result.stderr)
            self.assertIn(str(candidate), result.stderr)
            self.assertNotIn("Checking accounting money types", result.stdout)

    def test_encrypted_packages_and_bindings_are_rejected(self):
        for name in ["book.sqlite3", "receipt.kae", "move.kagextra", "device-key.json"]:
            with self.subTest(name=name):
                self.check_rejected(name)

    def test_interrupted_private_work_is_rejected(self):
        for name in [
            ".kAccounting-Extra-01-recipient-export-0123456789abcdef.work",
            ".kAccounting-Extra-01-recipient-import-0123456789abcdef.work",
            ".kAccounting-Extra-01-recipient-import-0123456789abcdef",
            ".kAccounting-Extra-01-container-import-0123456789abcdef",
            ".kAccounting-Extra-01-ciphertext-0123456789abcdef.partial",
            ".kAccounting-Extra-01-transfer-object-0123456789abcdef.work",
            ".migration-work-0123456789abcdef",
        ]:
            with self.subTest(name=name):
                self.check_rejected(name, "directory")

    def test_symlinks_to_missing_runtime_data_are_rejected(self):
        self.check_rejected("missing.kabackup", "symlink")


if __name__ == "__main__":
    unittest.main()
