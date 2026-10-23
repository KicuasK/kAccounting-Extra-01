"""Plan and orchestration tests; launchctl and HTTP are mocked throughout."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

MODULE_PATH = Path(__file__).with_name("login-acceptance.py")
SPEC = importlib.util.spec_from_file_location("login_acceptance", MODULE_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class LoginAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ka-login-harness-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.binary = self.root / "login_acceptance"
        self.binary.write_text("#!/bin/sh\nexit 99\n")
        self.binary.chmod(0o700)
        self.assets = self.root / "dist"
        self.assets.mkdir(mode=0o700)
        (self.assets / "index.html").write_text("<!doctype html>invented frontend")

    def prepare(self):
        with mock.patch.object(MODULE, "launch") as launch:
            plan = MODULE.prepare(self.binary, self.assets, self.root)
            launch.assert_not_called()
        return plan

    def print_job(self, plan):
        job = MODULE.expected_job(Path(plan["root"]), Path(plan["binary"]), Path(plan["assets"]), plan["label"])
        return "\n".join([
            f"gui/{plan['uid']}/{plan['label']} = {{", "path = " + str(Path(plan["root"]) / "job.plist"),
            "program = " + plan["binary"], "arguments = {", *job["ProgramArguments"], "}", "}",
        ])

    def test_prepare_is_private_unique_and_does_not_register_any_job(self):
        first, second = self.prepare(), self.prepare()
        self.assertNotEqual(first, second)
        plan, job = MODULE.load_plan(first)
        self.assertEqual(first.stat().st_mode & 0o777, 0o600)
        self.assertEqual(first.parent.stat().st_mode & 0o777, 0o700)
        self.assertEqual(list((first.parent / "books").iterdir()), [])
        self.assertNotIn("LaunchAgents", str(first))
        self.assertFalse(job["KeepAlive"])
        self.assertEqual(job["ProgramArguments"][-1], plan["label"][len(MODULE.PREFIX):])
        self.assertTrue((first.parent / "served-assets/index.html").read_text().startswith("<!-- kAccounting-login-acceptance:"))
        self.assertEqual((self.assets / "index.html").read_text(), "<!doctype html>invented frontend")

    def test_default_cli_prepares_without_executing_binary_or_launchctl(self):
        result = subprocess.run([
            "python3", str(MODULE_PATH), "--binary", str(self.binary),
            "--assets", str(self.assets), "--parent", str(self.root),
        ], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertFalse(report["installed"])
        self.assertFalse(report["started"])
        MODULE.load_plan(Path(report["plan"]))

    def test_tampered_label_binary_or_plist_is_rejected_before_any_os_job(self):
        for field, replacement in [("label", "local.kAccounting-Extra-01"), ("uid", -1), ("root", "/")]:
            path = self.prepare()
            plan = json.loads(path.read_text())
            plan[field] = replacement
            path.write_text(json.dumps(plan))
            with self.assertRaises(ValueError):
                MODULE.load_plan(path)
        path = self.prepare()
        (path.parent / "job.plist").write_bytes(b"tampered")
        with self.assertRaises(ValueError):
            MODULE.load_plan(path)
        path = self.prepare()
        self.binary.write_text("#!/bin/sh\nexit 42\n")
        with self.assertRaises(ValueError):
            MODULE.load_plan(path)

    def test_nonempty_books_and_asset_marker_tampering_fail_closed(self):
        path = self.prepare()
        (path.parent / "books/invented").write_text("not empty")
        with self.assertRaises(ValueError):
            MODULE.load_plan(path)
        path = self.prepare()
        (path.parent / "served-assets/index.html").write_text("wrong instance")
        with self.assertRaises(ValueError):
            MODULE.load_plan(path)

    def test_source_symlinks_and_wrong_executable_are_rejected(self):
        link = self.assets / "linked.js"
        link.symlink_to(self.binary)
        with self.assertRaises(ValueError):
            MODULE.prepare(self.binary, self.assets, self.root)
        link.unlink()
        wrong = self.root / "production_binary"
        wrong.write_bytes(self.binary.read_bytes())
        wrong.chmod(0o700)
        with self.assertRaises(ValueError):
            MODULE.prepare(wrong, self.assets, self.root)

    def test_exact_job_identity_includes_label_program_arguments_and_plist(self):
        path = self.prepare()
        plan, _ = MODULE.load_plan(path)
        output = self.print_job(plan)
        self.assertTrue(MODULE.owns_job(output, plan))
        for changed in [output.replace(plan["label"], "another.job"),
                        output.replace(plan["binary"], "/another/binary"),
                        output.replace("job.plist", "other.plist"),
                        output.replace(str(path.parent / "books"), "/another/books")]:
            self.assertFalse(MODULE.owns_job(changed, plan))

    def execute_mocked(self, path, *, bootstrap_error=False, wrong_owner=False, http_error=False):
        plan, _ = MODULE.load_plan(path)
        active = False
        calls = []
        def launch(arguments):
            nonlocal active
            calls.append(arguments)
            if arguments[0] == "print":
                return subprocess.CompletedProcess(arguments, 0 if active else 113,
                    self.print_job(plan) if active and not wrong_owner else "foreign job", "")
            if arguments[0] == "bootstrap":
                active = True
                return subprocess.CompletedProcess(arguments, 5 if bootstrap_error else 0, "", "")
            if arguments[0] == "bootout":
                active = False
                token = plan["label"][len(MODULE.PREFIX):]
                (path.parent / "stdout.log").write_text("\n".join(json.dumps({"fixture_token": token, "phase": phase, "key_calls": 0}) for phase in ["startup", "shutdown"]))
                return subprocess.CompletedProcess(arguments, 0, "", "")
            self.fail("unexpected launchctl command")
        def read_json(route):
            return {"book_id": None, "selection_required": True, "selection_id": None} if route == "/api/session" else {"books": []}
        with mock.patch.object(MODULE, "launch", side_effect=launch), \
             mock.patch.object(MODULE, "port_available", return_value=True), \
             mock.patch.object(MODULE, "verify_http_owner", side_effect=ValueError("wrong fixture") if http_error else None), \
             mock.patch.object(MODULE, "read_json", side_effect=read_json):
            if bootstrap_error or wrong_owner or http_error:
                with self.assertRaises(RuntimeError):
                    MODULE.execute(path, timeout=1)
                return calls
            result = MODULE.execute(path, timeout=1)
            self.assertTrue(result["passed"])
            self.assertTrue(result["own_job_removed"])
            self.assertEqual(result["fake_key_calls"], 0)
            return calls

    def test_authorized_execution_bootstraps_and_cleans_only_exact_owned_job(self):
        path = self.prepare()
        plan, _ = MODULE.load_plan(path)
        calls = self.execute_mocked(path)
        self.assertIn(["bootstrap", f"gui/{plan['uid']}", str(path.parent / "job.plist")], calls)
        self.assertIn(["bootout", f"gui/{plan['uid']}/{plan['label']}"], calls)
        self.assertTrue(all(call[0] in ["print", "bootstrap", "bootout"] for call in calls))
        self.assertFalse(any("local.kAccounting-Extra-01" == arg for call in calls for arg in call))

    def test_bootstrap_partial_failure_cleans_only_verified_own_registration(self):
        self.assertTrue(any(call[0] == "bootout" for call in self.execute_mocked(self.prepare(), bootstrap_error=True)))
        calls = self.execute_mocked(self.prepare(), bootstrap_error=True, wrong_owner=True)
        self.assertFalse(any(call[0] == "bootout" for call in calls))

    def test_wrong_http_instance_never_passes_and_owned_job_is_removed(self):
        calls = self.execute_mocked(self.prepare(), http_error=True)
        self.assertTrue(any(call[0] == "bootout" for call in calls))

    def test_occupied_port_or_existing_label_never_bootstraps_or_stops_any_job(self):
        path = self.prepare()
        with mock.patch.object(MODULE, "port_available", return_value=False), mock.patch.object(MODULE, "launch") as launch:
            with self.assertRaises(ValueError):
                MODULE.execute(path)
            launch.assert_not_called()
        with mock.patch.object(MODULE, "port_available", return_value=True), mock.patch.object(MODULE, "launch", return_value=subprocess.CompletedProcess([], 0, "exists", "")) as launch:
            with self.assertRaises(ValueError):
                MODULE.execute(path)
            self.assertEqual(launch.call_count, 1)
            self.assertEqual(launch.call_args.args[0][0], "print")

    def test_redirects_are_never_followed(self):
        self.assertIsNone(MODULE.NoRedirect().redirect_request(None, None, 302, "Found", {}, "https://elsewhere.example"))

    def test_unverified_job_is_never_stopped_even_after_successful_bootstrap(self):
        calls = self.execute_mocked(self.prepare(), wrong_owner=True)
        self.assertFalse(any(call[0] == "bootout" for call in calls))

    def test_public_or_symlink_plan_files_are_rejected(self):
        path = self.prepare()
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            MODULE.load_plan(path)
        path.chmod(0o600)
        original = path.read_bytes()
        path.unlink()
        elsewhere = self.root / "other-plan.json"
        elsewhere.write_bytes(original)
        elsewhere.chmod(0o600)
        path.symlink_to(elsewhere)
        with self.assertRaises(OSError):
            MODULE.load_plan(path)

    def test_run_requires_reviewed_plan_and_rejects_inline_program_arguments(self):
        result = subprocess.run(["python3", str(MODULE_PATH), "--run", "--binary", str(self.binary)], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
