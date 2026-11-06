"""Public CLI regressions against a disposable external container-engine fixture."""

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
ENGINE = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
path = pathlib.Path(os.environ["ENGINE_STATE"])
state = json.loads(path.read_text())
args = sys.argv[1:]
state["calls"].append(args)
output, status = "", 0
if args[0] in ("container", "volume", "network"):
    if args[0] == "container" and state["stopped"] and "-a" in args:
        output = "existing-stopped-container"
elif args[0] == "compose":
    if "build" in args:
        status = 7
    if "down" in args:
        state["stopped"] = False
else:
    raise SystemExit("Unexpected external engine command")
path.write_text(json.dumps(state))
print(output)
sys.exit(status)
'''


class RuntimeSmokeTests(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="ocr-runtime-cli-")
        self.addCleanup(self.work.cleanup)
        root = pathlib.Path(self.work.name)
        self.state = root / "state.json"
        self.engine = root / "engine"
        self.engine.write_text(ENGINE)
        self.engine.chmod(0o755)

    def invoke(self, **state):
        self.state.write_text(json.dumps({"calls": [], "stopped": False, **state}))
        result = subprocess.run(
            [sys.executable, "-B", str(ROOT / "docker/runtime-smoke.py")],
            env={**os.environ, "OCR_CONTAINER_ENGINE": str(self.engine),
                 "OCR_RUNTIME_PROJECT": "ocr-cli-fixture", "ENGINE_STATE": str(self.state)},
            capture_output=True, text=True, timeout=20,
        )
        return result, json.loads(self.state.read_text())

    def test_refuses_stopped_project_without_adopting_it(self):
        result, state = self.invoke(stopped=True)
        self.assertIn("Smoke project already exists", result.stderr)
        self.assertTrue(state["stopped"])
        self.assertFalse(any(call[0] == "compose" for call in state["calls"]))


if __name__ == "__main__":
    unittest.main()
