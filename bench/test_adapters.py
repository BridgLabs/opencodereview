"""Tests of the benchmark adapters' container commands, with a fake engine that records its arguments.

    python3 -B bench/test_adapters.py
"""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent / "adapters"))
import common  # noqa: E402

# Prints a container id for `run -d`, records every call, and fails nothing.
FAKE_ENGINE = """#!/bin/sh
printf '%s\\n' "$*" >> "$(dirname "$0")/calls.txt"
printf 'fake-id\\n'
"""


class EngineTests(unittest.TestCase):
    def setUp(self):
        self.root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(subprocess.run, ["rm", "-rf", str(self.root)])
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        self.engine = bin_dir / "podman"
        self.engine.write_text(FAKE_ENGINE)
        self.engine.chmod(0o755)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        git = ["git", "-C", str(self.repo)]
        subprocess.run([*git, "init", "-q", "-b", "main"], check=True)
        (self.repo / "a.txt").write_text("one\n")
        commit = [*git, "-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q"]
        subprocess.run([*git, "add", "."], check=True)
        subprocess.run([*commit, "-m", "base"], check=True)
        self.base = subprocess.run([*git, "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()
        (self.repo / "a.txt").write_text("two\n")
        subprocess.run([*commit, "-am", "head"], check=True)
        self.head = subprocess.run([*git, "rev-parse", "HEAD"], check=True, capture_output=True, text=True).stdout.strip()

    def calls(self):
        return [line.split(" ") for line in (self.engine.parent / "calls.txt").read_text().splitlines()]

    def review(self):
        with patch.object(common, "ENGINE", str(self.engine)), patch.dict("os.environ", {"CLAUDE_CODE_OAUTH_TOKEN": "x"}):
            return common.review("image:tag", self.repo, "o/r", 7, self.base, self.head, "t", "b", self.root / "run", "claude")

    def test_review_runs_the_configured_engine(self):
        status, _, _ = self.review()
        self.assertEqual(status, 0)
        [call] = self.calls()
        self.assertEqual(call[:2], ["run", "--rm"])
        self.assertEqual(call[-1], "image:tag")

    def test_bind_mounts_carry_the_shared_selinux_label_and_keep_read_only(self):
        self.review()
        [call] = self.calls()
        mounts = [call[index + 1] for index, arg in enumerate(call) if arg == "-v"]
        run = self.root / "run"
        self.assertEqual(mounts, [
            f"{self.repo}:/work/repo:z", f"{run / 'pr'}:/work/pr:ro,z", f"{run / 'out'}:/work/out:z",
        ])
        pr = json.loads((run / "pr" / "pr.json").read_text())
        self.assertEqual((pr["base"], pr["head"]), (self.base, self.head))

    def test_runner_pool_connects_the_egress_to_the_engine_default_network(self):
        for name, network in (("podman", "podman"), ("docker", "bridge")):
            with self.subTest(engine=name):
                engine = self.engine.with_name(name)
                if engine != self.engine:
                    engine.write_text(FAKE_ENGINE)
                    engine.chmod(0o755)
                (engine.parent / "calls.txt").unlink(missing_ok=True)
                with patch.object(common, "ENGINE", str(engine)):
                    with common.runner_pool("image:tag", 2) as args:
                        self.assertIn("--network", args)
                connects = [call for call in self.calls() if call[:2] == ["network", "connect"]]
                self.assertEqual(len(connects), 1)
                self.assertEqual(connects[0][2], network)
                self.assertTrue(any(call[:2] == ["rm", "-f"] for call in self.calls()))


if __name__ == "__main__":
    unittest.main()
