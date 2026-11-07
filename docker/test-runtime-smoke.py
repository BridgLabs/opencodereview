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
    if "exec" in args:
        if "cat" in args:
            output = "fixture-token"
        elif "hostname" in args:
            output = "192.0.2.2"
elif args[:2] == ["image", "ls"]:
    output = state["images"].get(args[-1], "")
elif args[:2] == ["image", "rm"]:
    assert len(args) == 3 and not args[2].startswith("-")
    del state["images"][args[2]]
else:
    raise SystemExit("Unexpected external engine command")
path.write_text(json.dumps(state))
print(output)
sys.exit(status)
'''

NETWORK_RUNNER = r'''
import json, subprocess, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
class Handler(BaseHTTPRequestHandler):
    theme = "dark"
    def log_message(self, *args): pass
    def do_GET(self):
        if self.path == "/api/settings":
            assert self.headers["Authorization"] == "Bearer fixture-token"
            body = json.dumps({"theme": Handler.theme}).encode()
        elif self.path == "/api/health": body = b'{"ok":true}'
        else: body = b'<html>isolated fixture</html>'
        self.send_response(200)
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        assert self.path == "/api/settings"
        assert self.headers["Authorization"] == "Bearer fixture-token"
        data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        assert data == {"key":"theme", "value":"light"}
        Handler.theme = "light"
        self.send_response(200)
        self.end_headers()
server = ThreadingHTTPServer(("127.0.0.1", 1421), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
sys.exit(subprocess.run([sys.executable, "-B", sys.argv[1]]).returncode)
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

    def invoke(self, network=False, **state):
        self.state.write_text(json.dumps({"calls": [], "stopped": False, "fail": "build",
                                         "images": {"opencodereview-runner:local": "production-runner",
                                                    "foreign:keep": "foreign-image",
                                                    "foreign:shared": "smoke-build",
                                                    "postgres:17": "base-image"}, **state}))
        command = [sys.executable, "-B", str(ROOT / "docker/runtime-smoke.py")]
        if network:
            command = ["unshare", "--user", "--map-root-user", "--net", sys.executable,
                       "-c", NETWORK_RUNNER, str(ROOT / "docker/runtime-smoke.py")]
        result = subprocess.run(
            command,
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

    def test_removes_only_owned_images_on_success_and_failure(self):
        for failure in ("build", None):
            with self.subTest(failure=failure):
                result, state = self.invoke(network=failure is None, fail=failure)
                if failure is None:
                    self.assertEqual(result.returncode, 0, result.stderr)
                else:
                    self.assertNotEqual(result.returncode, 0)
                self.assertEqual(state["images"], {
                    "opencodereview-runner:local": "production-runner", "foreign:keep": "foreign-image",
                    "foreign:shared": "smoke-build", "postgres:17": "base-image",
                })


if __name__ == "__main__":
    unittest.main()
