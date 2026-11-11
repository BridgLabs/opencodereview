import base64
import hashlib
import importlib.util
import io
import json
import os
import pathlib
import re
import sqlite3
import struct
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, pathlib.Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReachedIsolationChecks(Exception):
    pass


class PinnedVersionTests(unittest.TestCase):
    def test_exact_pin_with_or_without_build_suffix(self):
        smoke = load_script("runner-smoke")
        for version, expected, valid in [
            ("2.1.292 (Claude Code)", "2.1.292", True),
            ("2.1.292", "2.1.292", True),
            ("2.1.2920 (Claude Code)", "2.1.292", False),
            ("claude 2.1.292", "2.1.292", False),
            ("Claude Code", "2.1.292", False),
            ("", "2.1.292", False),
            ("2.1.292 (Claude Code)", "", False),
        ]:
            def docker(*args, **kwargs):
                if args[0] == "network":
                    raise ReachedIsolationChecks()
                output = expected if args[0] == "image" else version if args[-1] == "--version" else "fixture-engine"
                return subprocess.CompletedProcess(args, 0, stdout=output, stderr="")

            with self.subTest(version=version, expected=expected), patch.object(smoke, "docker", docker):
                with self.assertRaises(ReachedIsolationChecks if valid else AssertionError):
                    smoke.main()


class ExecutionBudgetTests(unittest.TestCase):
    def test_multibatch_final_uses_an_explicit_total_not_chunk_count(self):
        limit = load_script("run-real-review-diagnostic").execution_limit
        self.assertEqual(limit(True, False, 7), 7)
        self.assertEqual(limit(True, False, 1), 1)
        self.assertEqual(limit(False, True, None), 1)
        self.assertEqual(limit(True, True, 1), 1)
        self.assertIsNone(limit(False, False, None))
        for args in [(True, False, None), (True, False, 0), (True, False, -1), (False, True, 2), (False, False, 7)]:
            with self.subTest(args=args), self.assertRaises(ValueError):
                limit(*args)


class WebPortBindingTests(unittest.TestCase):
    """The resolved Compose file, not the YAML text, decides who can reach the web page."""

    @classmethod
    def setUpClass(cls):
        probe = subprocess.run(["docker", "compose", "version"], capture_output=True)
        if probe.returncode != 0:
            raise unittest.SkipTest("docker compose is not available")

    def published(self, bind=None):
        env = {key: value for key, value in os.environ.items() if key != "OCR_WEB_BIND"}
        if bind is not None:
            env["OCR_WEB_BIND"] = bind
        root = pathlib.Path(__file__).resolve().parent.parent
        out = subprocess.run(
            ["docker", "compose", "config", "--format", "json"],
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        return [port for port in json.loads(out)["services"]["web"]["ports"] if int(port["target"]) == 1421]

    def test_default_publishes_on_loopback_only(self):
        ports = self.published()
        self.assertEqual([port["host_ip"] for port in ports], ["127.0.0.1"])

    def test_override_opens_every_interface(self):
        ports = self.published("0.0.0.0")
        self.assertEqual([port["host_ip"] for port in ports], ["0.0.0.0"])


if __name__ == "__main__":
    unittest.main()
