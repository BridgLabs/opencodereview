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
    images = {"server": "ocr-cli-fixture-server", "web": "ocr-cli-fixture-web",
              "review-runner": "opencodereview-runner:local", "review-egress": "opencodereview-runner:local"}
    for index, arg in enumerate(args):
        if arg == "-f" and args[index + 1].endswith(".json"):
            services = json.loads(pathlib.Path(args[index + 1]).read_text())["services"]
            images.update({name: value["image"] for name, value in services.items()})
    if "build" in args:
        state["built"] = images
        for image in images.values():
            state["images"][image] = "smoke-build"
        status = 7 if state["fail"] == "build" else 0
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
        self.state.write_text(json.dumps({"calls": [], "stopped": False, "fail": "build",
                                         "images": {"opencodereview-runner:local": "production-runner",
                                                    "foreign:keep": "foreign-image"}, **state}))
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

    def test_build_preserves_foreign_image_tags_on_failure(self):
        result, state = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(state["images"]["opencodereview-runner:local"], "production-runner")
        self.assertEqual(state["images"]["foreign:keep"], "foreign-image")
        self.assertEqual(state["built"]["review-runner"], state["built"]["review-egress"])


if __name__ == "__main__":
    unittest.main()
