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


if __name__ == "__main__":
    unittest.main()
