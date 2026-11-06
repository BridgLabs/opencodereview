"""Build a disposable Compose stack, check HTTP and persistence, then remove it.

Uses localhost:1421 and 8787. No accounts, credentials or real inference are needed.
Set OCR_CONTAINER_ENGINE=podman and PODMAN_COMPOSE_PROVIDER for rootless Podman.
"""

import json
import os
import pathlib
import subprocess
import time
import urllib.request
import uuid

ENGINE = os.environ.get("OCR_CONTAINER_ENGINE", "docker")
PROJECT = os.environ.get("OCR_RUNTIME_PROJECT", "ocr-runtime-smoke-" + uuid.uuid4().hex[:10])
ROOT = pathlib.Path(__file__).resolve().parent.parent
ORIGIN = "http://localhost:1421"


def run(*args, timeout=120):
    return subprocess.run(args, cwd=ROOT, env={**os.environ, "OCR_REVIEW_RUNNERS": "1"},
                          text=True, capture_output=True, check=True, timeout=timeout).stdout.strip()


def compose(*args, timeout=120):
    return run(ENGINE, "compose", "-p", PROJECT, *args, timeout=timeout)


def request(path, token="", payload=None):
    headers = {"Origin": ORIGIN}
    if token:
        headers["Authorization"] = "Bearer " + token
    data = None
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    with urllib.request.urlopen(urllib.request.Request(ORIGIN + path, data=data, headers=headers), timeout=5) as response:
        return response.read()


def ready():
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        try:
            assert b"<html" in request("/").lower()
            assert json.loads(request("/api/health"))["ok"] is True
            return
        except (OSError, ValueError, AssertionError):
            time.sleep(1)
    raise RuntimeError("Compose stack did not serve UI and healthy API within 90s")


def main():
    # Refuse to adopt or delete an existing installation, including stopped containers.
    for label in ("com.docker.compose.project", "io.podman.compose.project"):
        for resource in ("container", "volume", "network"):
            if run(ENGINE, resource, "ls", "-q", "--filter", f"label={label}={PROJECT}"):
                raise RuntimeError("Smoke project already exists; choose another OCR_RUNTIME_PROJECT")
    try:
        compose("config")
        compose("build", timeout=1800)
        print("PASS Compose builds shared server and runner runtime")
        compose("up", "-d", "--no-build")
        ready()
        # Authenticate exactly as the web entrypoint does; never print the installation token.
        token = compose("exec", "-T", "server", "cat", "/data/api-token")
        assert token
        request("/api/settings", token, {"key": "theme", "value": "light"})
        assert json.loads(request("/api/settings", token))["theme"] == "light"
        print("PASS UI, API, Postgres settings write and installation authentication")
        database_ip = compose("exec", "-T", "postgres", "hostname", "-i").split()[0]
        compose("exec", "-T", "--user", "10001:10001", "review-runner", "perl", "-MIO::Socket::INET", "-e",
                "exit(IO::Socket::INET->new(PeerAddr=>$ARGV[0],PeerPort=>5432,Timeout=>2) ? 1 : 0)", database_ip)
        compose("exec", "-T", "--user", "10001:10001", "review-runner", "sh", "-c",
                "test ! -e /data/api-token && test ! -e /data/repos && test ! -e /run/ocr-db/app-url")
        print("PASS production runner cannot reach Postgres or private application files")
        compose("restart")
        ready()
        assert json.loads(request("/api/settings", token))["theme"] == "light"
        print("PASS settings and installation token survive Compose restart")
    finally:
        compose("down", "-v", "--remove-orphans")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        # Compose logs may contain private configuration. Keep captured output out of CI logs.
        command = error.cmd[4] if error.cmd[1] == "compose" else error.cmd[1]
        raise SystemExit(f"FAIL {ENGINE} {command} exited {error.returncode}") from None
    except subprocess.TimeoutExpired:
        raise SystemExit(f"FAIL {ENGINE} command exceeded its time limit") from None
