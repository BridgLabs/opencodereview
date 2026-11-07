"""Behavioral regression for the web container's same-origin /api nginx proxy.

Runs the repository's docker/web-nginx.conf inside a real nginx container on a
disposable container network, against a disposable mock upstream registered under
the Compose service name `server`. Verifies the /api prefix strip, forwarding of
the headers the backend trust checks rely on (Authorization, Referer,
X-Flow-Secret), the nginx Origin mapping (explicit nonempty Origin wins, a
missing or empty Origin falls back to the Referer origin, and unparseable or
absent sources produce no Origin upstream), POST body/query preservation, the
SPA fallback, and prompt SSE delivery with proxy buffering disabled (streams
mirror the real backend headers, plus a variant without x-accel-buffering so
the nginx config itself is proven). Trusted
origin acceptance and rejection itself stays with the backend guard and its
Rust tests; this checks the exact Origin value nginx presents to it.

Creates and removes only its own uniquely named containers and network. Runs on
macOS Docker Desktop, Linux Docker and rootless Linux Podman (OCR_CONTAINER_ENGINE=podman).
"""

import http.client
import json
import os
import pathlib
import subprocess
import tempfile
import time
import uuid

PREFIX = "ocr-web-proxy-test-" + uuid.uuid4().hex[:10]
ENGINE = os.environ.get("OCR_CONTAINER_ENGINE", "docker")
NGINX_IMAGE = "nginx:1.30-alpine"
MOCK_IMAGE = "python:3.13-alpine"
ROOT = pathlib.Path(__file__).resolve().parent.parent
containers = []
networks = []

MOCK = r'''
import json
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def echo(self):
        length = int(self.headers.get("content-length") or 0)
        body = self.rfile.read(length).decode("utf-8", "replace") if length else ""
        payload = json.dumps({
            "method": self.command,
            "path": self.path,
            "body": body,
            "headers": {key.lower(): value for key, value in self.headers.items()},
        }).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def events(self, accel):
        self.send_response(200)
        self.send_header("content-type", "text/event-stream; charset=utf-8")
        self.send_header("cache-control", "no-cache, no-store")
        if accel:
            self.send_header("x-accel-buffering", "no")
            self.send_header("connection", "keep-alive")
        else:
            self.send_header("connection", "close")
        self.end_headers()
        self.close_connection = True
        for index in range(3):
            self.wfile.write(f"event: tick\ndata: {index}\n\n".encode())
            self.wfile.flush()
            time.sleep(1.5)

    def route(self):
        if self.path.startswith("/events-raw"):
            self.events(accel=False)
        elif self.path.startswith("/events"):
            self.events(accel=True)
        else:
            self.echo()

    do_GET = route
    do_POST = route


ThreadingHTTPServer(("0.0.0.0", 8787), Handler).serve_forever()
'''


def docker(*args, check=True, timeout=300):
    result = subprocess.run([ENGINE, *args], text=True, capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{ENGINE} {args[0]} failed: {result.stderr.strip()}")
    return result


def cleanup():
    for name in containers:
        docker("rm", "-f", name, check=False)
    for name in networks:
        docker("network", "rm", name, check=False)


def request(port, method, path, headers=None, body=None):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    connection.request(method, path, body=body, headers=headers or {})
    return connection, connection.getresponse()


def expect(condition, label, detail=""):
    if not condition:
        raise AssertionError(f"{label}: {detail}")
    print(f"PASS {label}")


def main():
    work = tempfile.TemporaryDirectory(prefix=PREFIX + "-")
    try:
        run_checks(work)
    finally:
        cleanup()
        work.cleanup()


def run_checks(work):
    network = f"{PREFIX}-net"
    upstream = f"{PREFIX}-upstream"
    web = f"{PREFIX}-web"
    docker("network", "create", network)
    networks.append(network)
    mock_path = pathlib.Path(work.name) / "mock_upstream.py"
    mock_path.write_text(MOCK)
    docker(
        "run", "-d", "--name", upstream,
        "--network", network, "--network-alias", "server",
        "-v", f"{mock_path}:/mock_upstream.py:ro,z",
        MOCK_IMAGE, "python", "-u", "/mock_upstream.py",
    )
    containers.append(upstream)
    docker(
        "run", "-d", "--name", web,
        "--network", network,
        "-p", "127.0.0.1::1421",
        "-v", f"{ROOT}/docker/web-nginx.conf:/etc/nginx/conf.d/default.conf:ro,z",
        NGINX_IMAGE,
    )
    containers.append(web)

    published = docker("port", web, "1421/tcp").stdout.strip().splitlines()[0]
    port = int(published.rsplit(":", 1)[1])

    deadline = time.time() + 60
    while True:
        try:
            connection, response = request(port, "GET", "/api/health")
            response.read()
            connection.close()
            if response.status == 200:
                break
        except OSError:
            pass
        if time.time() > deadline:
            raise RuntimeError("nginx test container did not reach its upstream within 60s")
        time.sleep(0.5)

    connection, response = request(port, "GET", "/platform/reviews")
    body = response.read()
    connection.close()
    expect(response.status == 200 and "html" in response.getheader("content-type", ""),
           "SPA fallback serves index.html", f"status={response.status}")

    connection, response = request(port, "GET", "/api/health")
    echo = json.loads(response.read())
    connection.close()
    expect(response.status == 200 and echo["method"] == "GET" and echo["path"] == "/health",
           "GET /api/health reaches upstream as /health", json.dumps(echo))

    connection, response = request(
        port, "GET", "/api/settings?tab=reviews&scope=all",
        headers={
            "Authorization": "Bearer proxy-test-token",
            "Origin": "https://hostile.example",
            "Referer": "http://app.example:1421/platform/settings",
            "X-Flow-Secret": "flow-secret-123",
        },
    )
    echo = json.loads(response.read())
    connection.close()
    headers = echo["headers"]
    expect(echo["path"] == "/settings?tab=reviews&scope=all",
           "query string survives the prefix strip", echo["path"])
    expect(headers.get("authorization") == "Bearer proxy-test-token",
           "Authorization header forwarded verbatim", repr(headers.get("authorization")))
    expect(headers.get("origin") == "https://hostile.example",
           "explicit Origin always wins over a trusted Referer", repr(headers.get("origin")))
    expect(headers.get("referer") == "http://app.example:1421/platform/settings",
           "Referer forwarded verbatim", repr(headers.get("referer")))
    expect(headers.get("x-flow-secret") == "flow-secret-123",
           "X-Flow-Secret forwarded verbatim", repr(headers.get("x-flow-secret")))
    expect(headers.get("host") == "127.0.0.1",
           "Host header forwards the client-facing host name ($host drops the port)",
           repr(headers.get("host")))
    expect(bool(headers.get("x-forwarded-for")),
           "X-Forwarded-For records the client address", repr(headers.get("x-forwarded-for")))

    def upstream_headers(extra):
        connection, response = request(port, "GET", "/api/echo", headers=extra)
        echo = json.loads(response.read())
        connection.close()
        return echo["headers"]

    headers = upstream_headers({"Referer": "http://app.example:1421/platform/indexing"})
    expect(headers.get("origin") == "http://app.example:1421",
           "missing Origin falls back to the Referer origin", repr(headers.get("origin")))

    headers = upstream_headers({"Referer": "http://app.example:1421"})
    expect(headers.get("origin") == "http://app.example:1421",
           "path-less Referer still maps to its origin", repr(headers.get("origin")))

    headers = upstream_headers({"Origin": "null", "Referer": "http://app.example:1421/platform"})
    expect(headers.get("origin") == "null",
           "explicit null Origin wins over a trusted Referer", repr(headers.get("origin")))

    headers = upstream_headers({"Origin": "", "Referer": "http://app.example:1421/platform"})
    expect(headers.get("origin") == "http://app.example:1421",
           "empty Origin counts as missing and falls back to Referer", repr(headers.get("origin")))

    headers = upstream_headers({"Referer": "garbage"})
    expect("origin" not in headers,
           "unparseable Referer produces no Origin upstream", repr(headers.get("origin")))

    headers = upstream_headers({})
    expect("origin" not in headers,
           "missing Origin and Referer produces no Origin upstream", repr(headers.get("origin")))

    headers = upstream_headers({"Referer": "http://user@app.example:1421/platform"})
    expect(headers.get("origin") == "http://user@app.example:1421",
           "credentialed Referer origin reaches the backend for rejection", repr(headers.get("origin")))

    connection, response = request(
        port, "POST", "/api/settings",
        headers={"content-type": "application/json", "Authorization": "Bearer proxy-test-token"},
        body=json.dumps({"key": "theme", "value": "dark"}),
    )
    echo = json.loads(response.read())
    connection.close()
    expect(echo["method"] == "POST" and echo["path"] == "/settings"
           and json.loads(echo["body"]) == {"key": "theme", "value": "dark"}
           and echo["headers"].get("content-type") == "application/json",
           "POST body and content-type reach the upstream intact", json.dumps(echo))

    def check_stream(path, label):
        connection = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
        started = time.time()
        connection.request("GET", path, headers={"Accept": "text/event-stream"})
        response = connection.getresponse()
        expect(response.status == 200 and "text/event-stream" in response.getheader("content-type", ""),
               f"{label}: responds with text/event-stream", f"status={response.status}")
        first = response.read(1)
        first_byte_seconds = time.time() - started
        rest = first + response.read()
        connection.close()
        events = [line for line in rest.decode().splitlines() if line.startswith("data:")]
        expect(first_byte_seconds < 3.0,
               f"{label}: first byte arrives before a buffered proxy could flush",
               f"{first_byte_seconds:.2f}s (upstream streams for ~4.5s)")
        expect(events == ["data: 0", "data: 1", "data: 2"],
               f"{label}: all SSE events arrive in order", repr(events))

    check_stream("/api/events", "SSE with backend-mirrored headers")
    check_stream("/api/events-raw", "SSE without x-accel-buffering proves the nginx config streams")

    print("web /api proxy regression passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"FAIL {error}")
        raise SystemExit(1)
    finally:
        cleanup()
