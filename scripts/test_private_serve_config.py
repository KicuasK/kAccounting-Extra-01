#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("private_config", Path(__file__).with_name("make-private-serve-config.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PrivateServePlanTests(unittest.TestCase):
    def test_valid_plan_cannot_enable_network_or_funnel(self):
        p = module.make_policy("book.example.ts.net", "owner@example.com")
        self.assertFalse(p["deployment_enabled"])
        self.assertFalse(p["funnel_allowed"])
        self.assertIsNone(p["serve_config"])
        self.assertEqual(p["origin"], "https://book.example.ts.net")
        self.assertEqual(p["regular_local_origin"], "http://127.0.0.1:3000")

    def test_rejects_ambiguous_or_injected_hosts(self):
        for host in ["example.ts.net", "BOOK.example.ts.net", "book.example.ts.net.",
                     "book.example.ts.net:443", "https://book.example.ts.net", "book..example.ts.net",
                     "book.example.ts.net/evil", "book.example.ts.net\n", "-book.example.ts.net",
                     "book.example.ts.net.evil", "127.0.0.1"]:
            with self.subTest(host=host), self.assertRaises(ValueError):
                module.make_policy(host, "owner@example.com")

    def test_github_provider_identity_is_retained_exactly(self):
        p = module.make_policy("book.example.ts.net", "FakeOwner@github")
        self.assertEqual(p["allowed_login"], "FakeOwner@github")
        self.assertFalse(p["deployment_enabled"])
        for bad in ["owner@local", "owner@localhost", "owner@GITHUB", "owner@gitlab",
                    "owner@github.", "owner@@github", "owner@github\n", "owner@-github"]:
            with self.subTest(login=bad), self.assertRaises(ValueError):
                module.make_policy("book.example.ts.net", bad)

    def test_requires_explicit_single_identity(self):
        for login in ["", "*", "owner", "owner@", "@example.com", "owner@example.com\n",
                      "owner@example.com,other@example.com", "owner@EXAMPLE.com"]:
            with self.subTest(login=login), self.assertRaises(ValueError):
                module.make_policy("book.example.ts.net", login)


if __name__ == "__main__":
    unittest.main()
