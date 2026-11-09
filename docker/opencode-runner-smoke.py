#!/usr/bin/env python3
"""Real runner/UID/IPC proof with pinned OpenCode and a non-forwarding fake API.

Namespace mode runs without a container engine. Container mode requires explicit,
already-local runner and Python fixture image names; it never builds or pulls.
"""
import argparse
import base64
import http.server
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid

KEY = "proof-sentinel-not-a-key"


spec = importlib.util.spec_from_file_location("protocol_fixtures", Path(__file__).with_name("opencode-protocol-fixtures.py"))
protocols = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protocols)


class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass

    def do_CONNECT(self):
        # Terminate synthetic TLS locally. Never open or forward an upstream connection.
        if self.path != "opencode.ai:443":
            self.send_error(400)
            return
        self.send_response(200, "Connection established")
        self.end_headers()
        self.connection = self.server.tls.wrap_socket(self.connection, server_side=True)
        self.rfile = self.connection.makefile("rb")
        self.wfile = self.connection.makefile("wb", buffering=0)
        self.close_connection = True
        self.handle_one_request()

    def do_POST(self):
        try:
            state = json.loads(self.server.state.read_text())
            log = self.server.state.with_suffix(".requests")
            with log.open("a") as output: output.write(json.dumps({"path": self.path}) + "\n")
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            protocol = state.get("protocol", "chat")
            tools, results = protocols.request(protocol, self.path, self.headers, body, state)
            with self.server.state.with_suffix(".request-details").open("a") as output:
                output.write(json.dumps({"path": self.path, "tools": tools, "results": len(results), "fields": sorted(body)}) + "\n")
            status, payload = protocols.response(protocol, state, tools, results)
        except (AssertionError, KeyError, ValueError, TypeError) as error:
            self.send_error(400, "fixture contract rejected")
            self.server.state.with_suffix(".failure").write_text(type(error).__name__)
            return
        self.send_response(status)
        self.send_header("Content-Type", "text/event-stream" if status == 200 else "application/json")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(payload)


def fake_tls(server, directory):
    server.tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server.tls.load_cert_chain(directory / "fixture-ca.pem", directory / "fixture-ca-key.pem")


def exact(stream, count):
    result = b""
    while len(result) < count:
        part = stream.recv(count - len(result))
        if not part: raise RuntimeError("truncated runner IPC")
        result += part
    return result


def decoded_contains(value, secret):
    if isinstance(value, dict): return any(decoded_contains(item, secret) for pair in value.items() for item in pair)
    if isinstance(value, list): return any(decoded_contains(item, secret) for item in value)
    if not isinstance(value, str): return False
    if secret in value: return True
    # Inspection only: plaintext need not itself be JSON to expose ASCII escapes.
    import re
    inspected = value
    for _ in range(64):
        normalized = re.sub(r"\\u00([0-7][0-9a-fA-F])", lambda match: chr(int(match[1], 16)), inspected)
        if secret in normalized: return True
        if normalized == inspected: break
        inspected = normalized
    for line in [value, *value.splitlines()]:
        try: decoded = json.loads(line)
        except (ValueError, TypeError): continue
        if decoded != value and decoded_contains(decoded, secret): return True
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/ocr-sandbox"))
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--protocol", choices=list(protocols.REPRESENTATIVES), default="chat")
    parser.add_argument("--protocol-matrix", action="store_true", help="Sequential four-protocol tool/usage/HTTP error proof")
    parser.add_argument("--diagnostics", action="store_true", help="Record only public guard structure and fault locations in the isolated fixture")
    parser.add_argument("--model", default=None, help="Explicit curated model exercised by this fixture")
    parser.add_argument("--effort", default=None, help="Explicit curated reasoning effort exercised by this fixture")
    parser.add_argument("--check-model", action="store_true", help="Check guarded stage and selected-model usage attribution only")
    parser.add_argument("--image", help="Explicit reconciled runner image, never built/pulled here")
    parser.add_argument("--provider-image", help="Already-local image containing python3, required with --image")
    parser.add_argument("--engine", choices=["docker", "podman"], default="docker")
    parser.add_argument("--inside", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--check-auth-failover", action="store_true", help="Exercise authenticated HTTP 401 termination and adversarial regressions")
    parser.add_argument("--check-429-fault", action="store_true", help="Require late queued guard faults to override trusted 429 classification")
    parser.add_argument("--check-key-escaping", action="store_true", help="Reject quoted/backslashed keys in decoded report and nested error transcript")
    parser.add_argument("--check-truncated-usage", action="store_true", help="Preserve completed usage when cancellation leaves a partial NDJSON record")
    parser.add_argument("--check-deep-key", action="store_true", help="Reject parser-budget failure with a deeply nested Unicode-escaped key")
    parser.add_argument("--check-evidence-identity", action="store_true", help="Preserve exact JSON evidence quote from the installed snapshot")
    parser.add_argument("--check-partial-utf8", action="store_true", help="Recover completed usage before a partial multibyte UTF-8 tail on cancellation")
    parser.add_argument("--check-plaintext-key", action="store_true", help="Detect recoverable ASCII Unicode key spellings inside non-JSON plaintext")
    parser.add_argument("--provider", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--protocol-errors", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.provider:
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 18081), Provider)
        server.state = args.provider
        fake_tls(server, args.provider.parent)
        server.serve_forever()
        return 0
    if args.protocol_matrix:
        forwarded = [value for value in sys.argv[1:] if value != "--protocol-matrix"]
        if any(value in forwarded for value in ["--protocol", "--model", "--effort"]): parser.error("matrix chooses representatives")
        for protocol in protocols.REPRESENTATIVES:
            subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), *forwarded, "--protocol", protocol, "--check-model", "--protocol-errors"], check=True)
        return 0
    model, effort = protocols.REPRESENTATIVES[args.protocol]
    args.model, args.effort = model if args.model is None else args.model, effort if args.effort is None else args.effort
    import re
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,120}", args.model) or args.effort and not re.fullmatch(r"[A-Za-z0-9_.-]{1,120}", args.effort):
        parser.error("unsafe synthetic protocol selector")
    if args.image and not args.provider_image: parser.error("--image requires --provider-image; no implicit downloads")
    if not args.image and not args.cli: parser.error("namespace mode requires --cli")
    if not args.image and not args.inside:
        command = ["unshare", "--user", "--map-auto", "--map-root-user", "--net", sys.executable, "-B", str(Path(__file__).resolve()), "--inside", "--binary", str(args.binary.resolve()), "--cli", str(args.cli.resolve())]
        command += ["--model", args.model, "--effort", args.effort, "--protocol", args.protocol]
        if args.diagnostics: command.append("--diagnostics")
        if args.protocol_errors: command.append("--protocol-errors")
        if args.check_model: command.append("--check-model")
        if args.check_auth_failover: command.append("--check-auth-failover")
        if args.check_429_fault: command.append("--check-429-fault")
        if args.check_key_escaping: command.append("--check-key-escaping")
        if args.check_truncated_usage: command.append("--check-truncated-usage")
        if args.check_deep_key: command.append("--check-deep-key")
        if args.check_evidence_identity: command.append("--check-evidence-identity")
        if args.check_partial_utf8: command.append("--check-partial-utf8")
        if args.check_plaintext_key: command.append("--check-plaintext-key")
        return subprocess.run(command, env={"PATH": "/usr/bin:/bin"}).returncode
    if args.inside: subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
    spec = importlib.util.spec_from_file_location("runner_fixture", Path(__file__).with_name("runner-smoke.py"))
    fixture = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture)
    prefix = "ocr-go-proof-" + uuid.uuid4().hex[:10]
    resources = []
    worker = None
    server = None
    def engine(*command, **kwargs):
        return subprocess.run([args.engine, *command], check=True, capture_output=True, text=True, timeout=60, **kwargs).stdout
    try:
        Path("/tmp/opencode").mkdir(mode=0o700, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix=prefix, dir="/tmp/opencode") as temporary:
            root = Path(temporary)
            for name in ["ipc", "work", "output", "scratch", "evidence", "tmp", "system", "provider", "proof", "auth/accounts/1/opencode"]:
                (root / name).mkdir(parents=True)
            (root / "tmp").chmod(0o1777)
            (root / "output").chmod(0o777)
            (root / "proof").chmod(0o777)
            (root / "auth").chmod(0o700)
            (root / "auth/accounts/1/opencode/go-api-key").write_text(KEY)
            (root / "auth/accounts/1/opencode/go-api-key").chmod(0o600)
            assets = root / "assets"
            shutil.copytree(Path(__file__).with_name("opencode-guard"), assets)
            if args.diagnostics:
                catalogue_path = assets / "catalogue.js"
                text = catalogue_path.read_text().replace('import { lstatSync, readFileSync }', 'import { lstatSync, readFileSync, appendFileSync }')
                text = text.replace('const failed = () => new Error("OpenCode Go catalogue materialization failed");', 'const failed = () => { appendFileSync("/proof/guard-diagnostics", JSON.stringify({phase:"failure",stack:new Error().stack})+"\\n"); return new Error("OpenCode Go catalogue materialization failed"); };')
                text = text.replace('const inventory = await context.model.list();', 'const inventory = await context.model.list(); appendFileSync("/proof/guard-diagnostics", JSON.stringify({phase:"inventory",models:inventory.data?.map(m=>({id:m.id,provider:m.providerID,tools:m.capabilities?.tools,package:m.package,settingKeys:Object.keys(m.settings??{}),variants:m.variants?.map(v=>v.id)}))})+"\\n");')
                text = text.replace('const body = await boundedBody(request);', 'const body = await boundedBody(request); appendFileSync("/proof/guard-diagnostics", JSON.stringify({phase:"wire",protocol,fields:Object.keys(body),model:body.model,effort:body.output_config?.effort,thinking:body.thinking&&{keys:Object.keys(body.thinking),type:body.thinking.type,display:body.thinking.display,block_binding:body.thinking.block_binding},google:body.generationConfig?.thinkingConfig})+"\\n");')
                text = text.replace('const ref = event.model;', 'const ref = event.model; appendFileSync("/proof/guard-diagnostics", JSON.stringify({phase:"reference",id:ref?.id,provider:ref?.providerID,variant:ref?.variant,agent:event.agent,kind:event.kind,session:event.sessionID})+"\\n");')
                catalogue_path.write_text(text)
                health_path = assets / "health.js"
                health_path.write_text('import { appendFileSync } from "node:fs";\n' + health_path.read_text().replace('const fault = code => {', 'const fault = code => { appendFileSync("/proof/guard-diagnostics", JSON.stringify({phase:"health",code})+"\\n");'))
            # Ephemeral test CA, not a provider credential. The wrapper trusts only this extra
            # local CA; production CLI, guard, catalogue URLs and SDK metadata stay unchanged.
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                            "-keyout", str(root / "provider/fixture-ca-key.pem"), "-out", str(root / "provider/fixture-ca.pem"),
                            "-subj", "/CN=opencode.ai", "-addext", "subjectAltName=DNS:opencode.ai"],
                           check=True, capture_output=True, timeout=30)
            shutil.copy2(root / "provider/fixture-ca.pem", assets / "fixture-ca.pem")
            config = json.loads((assets / "opencode.json").read_text())
            (root / "system/opencode.json").write_text(json.dumps(config))
            (root / "hostname").write_text(prefix)
            helper = root / "helper"
            helper.write_text('#!/usr/bin/sh\nexec /opt/ocr/root-sandbox "$@"\n')
            helper.chmod(0o755)
            cli_wrapper = root / "cli-wrapper"
            cli_wrapper.write_text('#!/usr/bin/sh\nexport NODE_EXTRA_CA_CERTS=/opt/ocr/opencode-guard/fixture-ca.pem\nexec /opt/opencode/bin/real-opencode "$@"\n')
            cli_wrapper.chmod(0o755)
            claude = root / "claude"
            claude.write_text('#!/usr/bin/sh\n[ "$1" = --version ] && printf "0.0.1\\n"\n')
            claude.chmod(0o755)
            state = root / "provider/state.json"
            state.write_text('{}')
            shutil.copy2(__file__, root / "provider/proof.py")
            shutil.copy2(Path(__file__).with_name("opencode-protocol-fixtures.py"), root / "provider/opencode-protocol-fixtures.py")
            if args.image:
                for image in [args.image, args.provider_image]: engine("image", "inspect", image)
                extraction = prefix + "-cli"
                resources.append(extraction)
                engine("create", "--name", extraction, "--pull=never", "--network=none", "--entrypoint=/bin/true", args.image)
                engine("cp", extraction + ":/opt/opencode/bin/opencode", str(root / "real-opencode"))
                fixture.ENGINE_VERSION = engine("run", "--rm", "--pull=never", "--network=none", args.image, "engine-version").strip()
                provider = prefix + "-api"
                resources.append(provider)
                volume_mode = "rw,Z" if args.engine == "podman" else "rw"
                provider_user = ["--user", f"{os.getuid()}:{os.getgid()}"] if args.engine == "docker" else []
                engine("run", "-d", "--name", provider, "--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges:true", *provider_user, "-v", f"{root / 'provider'}:/provider:{volume_mode}", "--entrypoint=python3", args.provider_image, "/provider/proof.py", "--provider", "/provider/state.json")
                runner = prefix
                resources.append(runner)
                mounts = []
                for source, target, mode in [("ipc", "/ipc", "rw"), ("auth", "/claude-auth/app-managed", "ro"), ("system", "/etc/opencode", "ro"), ("assets", "/opt/ocr/opencode-guard", "ro"), ("proof", "/proof", "rw"), ("cli-wrapper", "/opt/opencode/bin/opencode", "ro"), ("real-opencode", "/opt/opencode/bin/real-opencode", "ro")]:
                    if args.engine == "podman": mode += ",Z"
                    mounts += ["-v", f"{root / source}:{target}:{mode}"]
                engine("run", "-d", "--init", "--name", runner, "--hostname", prefix, "--pull=never", "--network=container:" + provider, "--read-only", "--cap-drop=ALL", "--cap-add=SETUID", "--cap-add=SETGID", "--cap-add=CHOWN", "--cap-add=DAC_OVERRIDE", "--cap-add=KILL", "--security-opt=no-new-privileges:true", "--pids-limit=64", "--memory=512m", "--memory-swap=512m", "--cpus=0.5", "--tmpfs", "/work:mode=0755", "--tmpfs", "/evidence:mode=0755", "--tmpfs", "/output:mode=0777", "--tmpfs", "/scratch:mode=0755", "--tmpfs", "/tmp:mode=1777", "-e", "OCR_RUNNER_PROXY=http://127.0.0.1:18081", *mounts, args.image, "runner")
            else:
                fixture.ENGINE_VERSION = subprocess.check_output([str(args.binary.resolve()), "engine-version"], text=True).strip()
                server = http.server.ThreadingHTTPServer(("127.0.0.1", 18081), Provider)
                server.state = state
                fake_tls(server, root / "provider")
                threading.Thread(target=server.serve_forever, daemon=True).start()
                command = ["bwrap", "--unshare-pid", "--die-with-parent", "--tmpfs", "/", "--ro-bind", "/usr", "/usr", "--ro-bind", "/lib", "/lib", "--ro-bind", "/lib64", "/lib64", "--ro-bind", "/bin", "/bin", "--tmpfs", "/usr/local", "--dev", "/dev", "--proc", "/proc", "--ro-bind", str(args.binary.resolve()), "/opt/ocr/root-sandbox", "--ro-bind", str(helper), "/usr/local/bin/ocr-sandbox", "--ro-bind", str(claude), "/opt/claude/bin/claude", "--ro-bind", str(args.cli.resolve()), "/opt/opencode/bin/opencode", "--ro-bind", str(root / "hostname"), "/etc/hostname"]
                command += ["--ro-bind", str(args.cli.resolve()), "/opt/opencode/bin/real-opencode", "--ro-bind", str(cli_wrapper), "/opt/opencode/bin/opencode"]
                command += ["--bind", str(root / "proof"), "/proof"]
                for source, target, flag in [("ipc", "/ipc", "--bind"), ("work", "/work", "--bind"), ("output", "/output", "--bind"), ("evidence", "/evidence", "--bind"), ("scratch", "/scratch", "--bind"), ("tmp", "/tmp", "--bind"), ("auth", "/claude-auth/app-managed", "--ro-bind"), ("system", "/etc/opencode", "--ro-bind"), ("assets", "/opt/ocr/opencode-guard", "--ro-bind")]:
                    command += [flag, str(root / source), target]
                for directory in ["/opt", "/opt/ocr", "/opt/claude", "/opt/claude/bin", "/opt/opencode", "/opt/opencode/bin", "/usr/local", "/usr/local/bin", "/etc"]:
                    command += ["--chmod", "0755", directory]
                command += ["--remount-ro", "/", "/opt/ocr/root-sandbox", "runner"]
                probe = subprocess.run(command[:-2] + ["/usr/bin/setpriv", "--reuid=10001", "--regid=10001", "--clear-groups", "/opt/claude/bin/claude", "--version"], capture_output=True, text=True)
                assert probe.returncode == 0 and probe.stdout.strip() == "0.0.1", probe
                worker = subprocess.Popen(command, env={"PATH": "/usr/bin:/bin", "OCR_RUNNER_PROXY": "http://127.0.0.1:18081"}, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, start_new_session=True)
            path = root / "ipc" / (prefix + ".sock")
            for _ in range(200):
                if path.exists(): break
                time.sleep(.05)
            else: raise AssertionError("runner did not expose private IPC")
            def rpc(packet, control=""):
                events = []
                with socket.socket(socket.AF_UNIX) as stream:
                    stream.settimeout(60)
                    stream.connect(str(path))
                    def send(frame):
                        data = json.dumps(frame).encode()
                        stream.sendall(struct.pack(">I", len(data)) + data)
                    def receive():
                        count = struct.unpack(">I", exact(stream, 4))[0]
                        assert 0 < count <= 48 * 1024 * 1024
                        return json.loads(exact(stream, count))
                    for frame in packet.get("frames", []):
                        send(frame)
                        assert receive()["event"] == "Ack"
                    send(packet["request"])
                    while True:
                        event = receive()
                        events.append(event)
                        if control == "cancel_partial" and event["event"] == "Pulse" and (root / "proof/partial-written").exists():
                            stream.sendall(b"\x01")
                        if event["event"] == "Started" and control and control != "cancel_partial":
                            if control == "disconnect": return events
                            stream.sendall(b"\x01")
                        if event["event"] in ["Finished", "Error", "Catalog", "Ready"]: return events
            catalog = rpc({"request": {"operation": "Catalog"}})[-1]
            assert catalog["event"] == "Catalog", catalog
            version = catalog["version"]
            def stage(control="", **mode):
                broken_health = None
                selected_key = mode.pop("test_key", KEY)
                legacy_only = mode.pop("legacy_only", False)
                key_path = root / "auth/accounts/1/opencode/go-api-key"
                key_path.write_text(selected_key)
                if legacy_only:
                    key_path.rename(key_path.with_name("held-key"))
                    key_path.with_name("auth.json").write_text(json.dumps({"apiKey": KEY}))
                if mode.pop("broken_output", False):
                    health_path = assets / "health.js"
                    broken_health = health_path.read_text()
                    health_path.write_text(broken_health.replace('state = "sealed";', 'const {mkdirSync} = await import("node:fs"); mkdirSync("/output/report.json"); state = "sealed";'))
                packet = fixture.frozen_packet(args.model, False, "intent")
                identity_quote = mode.pop("identity_quote", None)
                if identity_quote is not None:
                    install = packet["frames"][0]
                    manifest = json.loads(install["manifest"])
                    blobs = {frame["hash"]: base64.b64decode(frame["data"]) for frame in packet["frames"] if frame["operation"] == "Blob"}
                    offset, blob_frames, manifest["blobs"] = 0, [], []
                    for entry in manifest["entries"]:
                        content = (identity_quote + "\n").encode() if entry["source"]["path"] == "a.ts" else blobs[entry["contentHash"]]
                        digest = hashlib.sha256(content).hexdigest()
                        entry["source"].update(bytes=len(content), gitBlobId=hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest())
                        entry["contentHash"] = digest
                        manifest["blobs"].append({"hash": digest, "offset": offset, "bytes": len(content)})
                        blob_frames.append({"operation": "Blob", "hash": digest, "data": base64.b64encode(content).decode()})
                        offset += len(content)
                    encoded = json.dumps(manifest, separators=(",", ":"))
                    descriptor = install["descriptor"] | {"id": hashlib.sha256(encoded.encode()).hexdigest(), "manifestBytes": len(encoded.encode()), "blobBytes": offset, "expandedBytes": offset}
                    install.update(manifest=encoded, descriptor=descriptor)
                    packet["frames"] = [install, *blob_frames, {"operation": "Seal"}]
                    files = packet["request"]["files"]
                    files["context/FROZEN.json"] = json.dumps(descriptor)
                    files["context/changed/a.ts"] = identity_quote + "\n"
                    stage_info = json.loads(files["context/STAGE.json"])
                    stage_info.update(snapshotId=descriptor["id"], contextHashes={path: hashlib.sha256(files[path].encode()).hexdigest() for path in stage_info["contextHashes"]})
                    files["context/STAGE.json"] = json.dumps(stage_info)
                    packet["request"]["archive_id"] = descriptor["id"]
                packet["request"].update(provider="opencode", effort=args.effort, version=version)
                packet["request"]["go_snapshot"] = protocols.fixture_snapshot(args.protocol, args.model, args.effort)
                snapshot = packet["request"]["archive_id"]
                answer = {"report": {"contractVersion": 2, "snapshotId": snapshot, "reviewedFiles": ["a.ts"], "missingEvidence": [], "findings": []}}
                if identity_quote is not None:
                    answer["report"]["findings"] = [{"evidence": [{"path": "a.ts", "line": 1, "quote": identity_quote}]}]
                if mode.pop("report_echo", False): answer["report"]["summary"] = selected_key
                if "report_summary" in mode: answer["report"]["summary"] = mode.pop("report_summary")
                state.write_text(json.dumps({"answer": answer, "selected_key": selected_key, "selected_model": args.model, "selected_effort": args.effort, "check_model": args.check_model, "protocol": args.protocol, "protocol_matrix": args.check_model} | mode))
                request_log = state.with_suffix(".requests")
                failure = state.with_suffix(".failure")
                request_log.write_text("")
                state.with_suffix(".request-details").write_text("")
                failure.unlink(missing_ok=True)
                try: result = rpc(packet, control)[-1]
                finally:
                    if broken_health is not None: health_path.write_text(broken_health)
                    if legacy_only:
                        key_path.with_name("held-key").rename(key_path)
                        key_path.with_name("auth.json").unlink()
                    key_path.write_text(KEY)
                assert not failure.exists(), "fake provider rejected protocol contract"
                assert not decoded_contains(result, selected_key), "credential escaped decoded IPC/transcript"
                if args.check_model:
                    count = len(request_log.read_text().splitlines())
                    expected_count = 1 if mode.get("status") else 2
                    if count != expected_count:
                        stderr_path = root / "scratch/stderr"
                        stderr = stderr_path.read_text() if stderr_path.is_file() else "unavailable"
                        assert not decoded_contains(stderr, selected_key), "credential escaped diagnostic stderr"
                        print("SAFE fixture requests:", state.with_suffix(".request-details").read_text(), "CLI stderr:", stderr, file=sys.stderr)
                        diagnostic = root / "proof/guard-diagnostics"
                        if args.diagnostics and diagnostic.is_file():
                            details = diagnostic.read_text()
                            assert not decoded_contains(details, selected_key), "credential escaped guard diagnostics"
                            print("SAFE guard diagnostics:", details, file=sys.stderr)
                    assert count == expected_count, ("unexpected retry/request count", count, expected_count, result)
                return result
            result = stage()
            assert result["error"] is None and json.loads(result["report"])["reviewedFiles"] == ["a.ts"], result
            print("PASS actual runner UID, archive IPC, pinned inference, health seal and final report")
            if args.check_model:
                events = [json.loads(line) for line in result["transcript"].splitlines() if line.strip()]
                assert any(event.get("type") == "tool_use" for event in events), "real CLI never executed a tool"
                assert [event["model"] for event in events if event.get("type") == "ocr.opencode.model"] == [args.model]
                expected_catalogue = protocols.fixture_snapshot(args.protocol, args.model, args.effort)
                attribution = next(event for event in events if event.get("type") == "ocr.opencode.model")
                assert attribution["catalogueId"] == expected_catalogue["catalogue_id"]
                assert attribution["catalogueSnapshotId"] == expected_catalogue["id"]
                steps = {(event["sessionID"], event["part"]["id"]): event["part"]["tokens"] for event in events if event.get("type") == "step_finish"}
                assert len(steps) == 2, steps
                assert sum(tokens["input"] for tokens in steps.values()) == 28
                assert sum(tokens["output"] + tokens["reasoning"] for tokens in steps.values()) == 14
                assert sum(tokens["cache"]["read"] for tokens in steps.values()) == 6
                Path("/tmp/opencode/deepseek-proof").mkdir(exist_ok=True)
                Path("/tmp/opencode/deepseek-proof/selected-stage-" + args.model + "-" + args.effort + ".json").write_text(json.dumps(result))
                print("PASS exact selected model/effort reaches fake provider and both completed usage steps: " + args.model + ("#" + args.effort if args.effort else " (no effort; suffix omitted)"))
                if args.protocol_errors:
                    for status in [401, 429]:
                        for late in [False, True]:
                            result = stage(**{"status_after_tool" if late else "status": status})
                            assert result.get("report") is None and result["error"].startswith("auth_failed:" if status == 401 else "usage_limit:"), result
                    result = stage(fake_auth=True)
                    assert not (result["error"] or "").startswith("auth_failed:"), result
                    print("PASS initial/after-tool HTTP 401/429, no retries, forged text rejected: " + args.protocol)
                return 0
            result = stage(calls=[
                ["ocr_read", {"path": "/work/context/PR.md"}],
                ["ocr_grep", {"path": "/work/context", "pattern": "Fixture"}],
                ["ocr_glob", {"path": "/work/context", "pattern": "PR*.md"}],
                ["ocr_bash", {"command": "/usr/bin/sed -n '1,1p' -- '/work/context/PR.md'"}],
                ["ocr_write", {"path": "/output/report.json", "content": "{}"}],
            ], expected=["Fixture intent", "/work/context/PR.md:1:Fixture intent\n", "/work/context/PR.md\n", "Fixture intent", "Wrote /output/report.json\n"])
            assert result["error"] is None, result
            print("PASS all five guarded tools execute successfully as model UID 10001")
            result = stage(read_path="/work/context/missing.md")
            assert result["error"] is None, result
            print("PASS ordinary rejected model request still permits healthy attested completion")
            result = stage(legacy_only=True)
            assert result["event"] == "Error" and "projected credential" in result["error"], result
            print("PASS legacy auth.json is never reused and missing projection is non-replayable startup failure")
            result = stage(echo=True)
            assert result["report"] is None and "credential" in result["error"], result
            print("PASS credential echo rejected and transcript redacted")
            result = stage(echo=True, broken_output=True)
            assert result["report"] is None and result["error"], result
            print("PASS output-read failure cannot bypass credential redaction")
            result = stage(status=429)
            assert result["report"] is None and result["error"].startswith("usage_limit:"), result
            print("PASS trusted 429 stops retries and returns existing failover code")
            if args.check_plaintext_key:
                key = 'proof-"quoted\\key'
                escaped = "prefix " + "".join("\\u%04x" % ord(character) for character in key) + " suffix"
                assert decoded_contains(escaped, key)
                safe = r'const label = "\u0061"; // \u263A'
                result = stage(test_key=key, report_summary=safe)
                assert result["error"] is None and json.loads(result["report"])["summary"] == safe, result
                print("PASS safe Unicode/code plaintext remains byte-identical")
                nested = escaped
                for _ in range(3): nested = nested.replace("\\", r"\u005c")
                for label, options in [
                    ("model report plaintext", {"report_summary": escaped}),
                    ("nested model report plaintext", {"report_summary": nested}),
                    ("HTTP 401 plaintext message", {"status": 401, "error_echo": escaped}),
                    ("HTTP 401 plaintext body", {"status": 401, "raw_error_body": escaped}),
                ]:
                    result = stage(test_key=key, **options)
                    assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                    print("PASS recoverable plaintext Unicode key rejected in " + label)
                malformed = r"prefix \u00xz suffix"
                result = stage(test_key=key, report_summary=malformed)
                assert result["error"] is None and json.loads(result["report"])["summary"] == malformed, result
                print("PASS malformed non-secret Unicode remains byte-identical")
                result = stage(test_key=key, report_summary=malformed + nested)
                assert result.get("report") is None and result["error"], result
                print("PASS malformed Unicode cannot hide an encoded credential")
                exhausted = r"prefix \u005c" + "u005c" * 65 + "u0070 suffix"
                result = stage(test_key=key, report_summary=exhausted)
                assert result.get("report") is None and result["error"], result
                print("PASS Unicode inspection budget fails closed")
                return 0
            if args.check_evidence_identity:
                quote = '{"enabled": true}'
                result = stage(identity_quote=quote, calls=[["ocr_read", {"path": "/evidence/snapshot/trees/" + "a" * 40 + "/a.ts"}]], expected=[quote + "\n"])
                assert result["error"] is None, result
                report = json.loads(result["report"])
                actual_quote = report["findings"][0]["evidence"][0]["quote"]
                assert actual_quote.encode() == quote.encode(), (actual_quote, quote)
                print("PASS exact JSON evidence quote survives actual frozen snapshot read and runner IPC")
                return 0
            if args.check_deep_key:
                key = 'proof-"quoted\\key'
                nested = '{"nested":' * 140 + '"proof-\\u0022quoted\\u005ckey"' + '}' * 140
                body = '{"error":{"message":"synthetic rejection","type":"invalid_api_key","detail":' + nested + '}}'
                assert decoded_contains(json.loads(body), key), "fixture does not recover the synthetic key"
                result = stage(test_key=key, status=401, raw_error_body=body)
                assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                assert result.get("transcript") in [None, ""], "uninspectable transcript was retained"
                print("PASS deeply nested Unicode-escaped key fails closed without transcript or auth replay")
                return 0
            if args.check_truncated_usage or args.check_partial_utf8:
                if args.image: parser.error("--check-truncated-usage currently requires the namespace fixture")
                original_wrapper = cli_wrapper.read_text()
                partial = (r"printf '\344\275'" + "\n") if args.check_partial_utf8 else ""
                cli_wrapper.write_text('#!/usr/bin/sh\nexport NODE_EXTRA_CA_CERTS=/opt/ocr/opencode-guard/fixture-ca.pem\nif [ "$1" = --version ]; then exec /opt/opencode/bin/real-opencode "$@"; fi\n/opt/opencode/bin/real-opencode "$@"\nprintf \'{"type":"text","part":{"text":"unfinished\'\n' + partial + 'printf ready > /proof/partial-written\nexec /usr/bin/sleep 30\n')
                try: result = stage(control="cancel_partial")
                finally: cli_wrapper.write_text(original_wrapper)
                assert result.get("report") is None and "cancel" in result["error"].lower(), result
                lines = (result["transcript"] or "").splitlines()
                complete = []
                for line in lines:
                    if not line.strip(): continue
                    try: complete.append(json.loads(line))
                    except ValueError: pass
                assert len([event for event in complete if event.get("type") == "step_finish"]) == 2, "fixture never completed both billable steps"
                events = [json.loads(line) for line in lines if line.strip()]
                parts = {(event["sessionID"], event["part"]["id"]): event["part"]["tokens"] for event in events if event.get("type") == "step_finish"}
                assert len(parts) == 2 and sum(part["input"] for part in parts.values()) > 0
                assert [event["model"] for event in events if event.get("type") == "ocr.opencode.model"] == [args.model]
                print("PASS cancelled " + ("partial UTF-8" if args.check_partial_utf8 else "partial ASCII") + " record preserves both completed usage parts once and model attribution")
                if args.check_partial_utf8:
                    cli_wrapper.write_text('#!/usr/bin/sh\nexport NODE_EXTRA_CA_CERTS=/opt/ocr/opencode-guard/fixture-ca.pem\nif [ "$1" = --version ]; then exec /opt/opencode/bin/real-opencode "$@"; fi\n/opt/opencode/bin/real-opencode "$@"\n' + partial + 'exit 0\n')
                    try: result = stage()
                    finally: cli_wrapper.write_text(original_wrapper)
                    assert result.get("report") is None and result["error"] == "Runner output is not UTF8", result
                    print("PASS successful process with incomplete UTF-8 remains strictly rejected")
                return 0
            if args.check_key_escaping:
                quoted_key = 'proof-"quoted\\key'
                result = stage(test_key=quoted_key)
                assert result["error"] is None and result["report"], result
                print("PASS quoted/backslashed key remains accepted without an echo")
                for label, options in [
                    ("structured report", {"report_echo": True}),
                    ("HTTP 401 nested JSON body", {"status": 401, "error_echo": quoted_key}),
                ]:
                    result = stage(test_key=quoted_key, **options)
                    assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                    print("PASS quoted/backslashed key cannot escape via " + label)
                return 0
            if args.check_429_fault:
                health_path = assets / "health.js"
                original_health = health_path.read_text()
                before = 'if (state === "active") fault("adapter_unloaded");'
                assert before in original_health
                # Real CLI teardown queues the actual authenticated channel fault after
                # root has selected the streaming retry. No runner output is fabricated.
                health_path.write_text(original_health.replace(before, '{ const {writeFileSync} = await import("node:fs"); writeFileSync("/proof/late-429-fault", "helper_timeout"); fault("helper_timeout"); }'))
                try: result = stage(status=429)
                finally: health_path.write_text(original_health)
                assert (root / "proof/late-429-fault").read_text() == "helper_timeout", "late guard fault fixture did not execute"
                assert result.get("report") is None and result["error"] and not result["error"].startswith("usage_limit:"), result
                print("PASS late queued actual guard fault prevents 429 account replay")
                return 0
            result = stage(control="cancel")
            assert result["report"] is None and "cancel" in result["error"].lower(), result
            print("PASS root IPC cancellation is not account failover")
            assert stage(control="disconnect")["event"] == "Started"
            result = stage()
            assert result["error"] is None, result
            print("PASS disconnect cleanup permits a fresh guarded stage without replay")
            if not args.image:
                original = helper.read_text()
                for label, script in [
                    ("spawn", "#!/missing-interpreter\n"),
                    ("protocol", "#!/usr/bin/sh\nprintf malformed\n"),
                    ("probe rejection", "#!/usr/bin/sh\nprintf '{\"v\":1,\"status\":\"rejected\",\"code\":\"request\"}\\n'\nexit 2\n"),
                    ("signal", "#!/usr/bin/sh\nkill -KILL $$\n"),
                    ("overflow", "#!/usr/bin/sh\nexec /usr/bin/head -c 400000 /dev/zero\n"),
                    ("timeout", "#!/usr/bin/sh\nexec /usr/bin/sleep 30\n"),
                ]:
                    helper.write_text(script)
                    result = stage()
                    assert result["report"] is None and "guard" in result["error"].lower(), (label, result)
                    print("PASS root rejects helper " + label)
                helper.write_text(original)
                assert stage()["error"] is None
                print("PASS only an explicit fresh stage recovers after helper infrastructure failure")
                helper.write_text('''#!/usr/bin/sh
n=0
if [ -f /tmp/ocr-helper-count ]; then read -r n < /tmp/ocr-helper-count; fi
n=$((n + 1))
printf '%s\\n' "$n" > /tmp/ocr-helper-count
if [ "$n" -eq 3 ]; then exec /usr/bin/sleep 30; fi
exec /opt/ocr/root-sandbox "$@"
''')
                result = stage()
                assert result["report"] is None and "guard" in result["error"].lower(), result
                assert "contractVersion" in (result["transcript"] or ""), "fixture never produced its schema-valid final text"
                helper.write_text(original)
                print("PASS idle probe failure rejects an already emitted schema-valid report")
            if args.check_auth_failover:
                result = stage(status=401)
                assert result["report"] is None and result["error"].startswith("auth_failed:"), result
                print("PASS trusted 401 returns existing authentication failover code")
                for label, options in [
                    ("earlier read", {}),
                    ("earlier prohibited request", {"read_path": "/etc/shadow"}),
                ]:
                    result = stage(status_after_tool=401, **options)
                    assert result["report"] is None and result["error"].startswith("auth_failed:"), result
                    assert '"type":"tool_use"' in result["transcript"], result
                    assert json.dumps({"model": args.model}, separators=(",", ":"))[1:-1] in result["transcript"], result
                    print("PASS " + label + " preserves authenticated 401 failover")
                result = stage(fake_auth=True)
                assert not (result["error"] or "").startswith("auth_failed:"), result
                print("PASS HTTP 200 model text cannot forge authentication failure")
                for label, options in [
                    ("credential echo", {"status_after_tool": 401, "calls": [["ocr_write", {"path": "/output/report.json", "content": KEY}]]}),
                    ("cancellation", {"control": "cancel"}),
                ]:
                    result = stage(status=401 if "status_after_tool" not in options else 200, **options)
                    assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                    print("PASS " + label + " overrides authentication")
                if not args.image:
                    original_helper = helper.read_text()
                    for fail_at, label, options in [
                        (2, "helper fault during HTTP 401 probe", {"status": 401}),
                        (2, "earlier helper fault before later HTTP 401", {"status_after_tool": 401}),
                    ]:
                        helper.write_text('#!/usr/bin/sh\nn=0\nif [ -f /tmp/ocr-auth-helper-count ]; then read -r n < /tmp/ocr-auth-helper-count; fi\nn=$((n + 1))\nprintf "%s\\n" "$n" > /tmp/ocr-auth-helper-count\nif [ "$n" -eq ' + str(fail_at) + ' ]; then printf malformed; exit 0; fi\nexec /opt/ocr/root-sandbox "$@"\n')
                        try: result = stage(**options)
                        finally: helper.write_text(original_helper)
                        assert result.get("report") is None and "guard" in result["error"].lower(), result
                        print("PASS " + label + " prevents authentication failover")
                    original_wrapper = cli_wrapper.read_text()
                    for label, ending in [("success exit", "exit 0"), ("unexpected exit", "exit 7"), ("signal exit", "kill -TERM $$")]:
                        cli_wrapper.write_text('#!/usr/bin/sh\nexport NODE_EXTRA_CA_CERTS=/opt/ocr/opencode-guard/fixture-ca.pem\nif [ "$1" = --version ]; then exec /opt/opencode/bin/real-opencode "$@"; fi\n/opt/opencode/bin/real-opencode "$@"\n' + ending + '\n')
                        try: result = stage(status=401)
                        finally: cli_wrapper.write_text(original_wrapper)
                        assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                        print("PASS " + label + " cannot authorize authentication failover")
                    for label, output in [
                        ("receipt without auth event", '/usr/bin/grep -v \'"type":"error"\' /tmp/auth-events'),
                        ("mismatched transcript session", '/usr/bin/sed \'s/"sessionID":"[^"]*"/"sessionID":"wrong"/g\' /tmp/auth-events'),
                        ("duplicate terminal auth event", '/usr/bin/cat /tmp/auth-events /tmp/auth-events'),
                    ]:
                        cli_wrapper.write_text('#!/usr/bin/sh\nexport NODE_EXTRA_CA_CERTS=/opt/ocr/opencode-guard/fixture-ca.pem\nif [ "$1" = --version ]; then exec /opt/opencode/bin/real-opencode "$@"; fi\n/opt/opencode/bin/real-opencode "$@" > /tmp/auth-events\n' + output + '\nexit 1\n')
                        try: result = stage(status=401)
                        finally: cli_wrapper.write_text(original_wrapper)
                        assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), result
                        print("PASS " + label + " cannot authorize authentication failover")
                health_file = assets / "health.js"
                original_health = health_file.read_text()
                for label, before, after in [
                    ("registry loss at HTTP 401", 'const tools = await context.tool.list();', 'const tools = await context.tool.list(); if (session) tools.pop();'),
                    ("output-read failure", 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'const {mkdirSync} = await import("node:fs"); mkdirSync("/output/report.json"); await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });'),
                    ("missing receipt", 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'state = "sealed";'),
                    ("wrong receipt session", 'sessionID: event.sessionID, ...fields', 'sessionID: "wrong", ...fields'),
                    ("wrong receipt status", 'status: event.response.status', 'status: 403'),
                    ("duplicate terminal", 'await send(type, { sessionID: event.sessionID, ...fields });', 'await send(type, { sessionID: event.sessionID, ...fields }); await send(type, { sessionID: event.sessionID, ...fields });'),
                    ("fault after seal", 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status }); fault("helper_probe");'),
                    ("HTTP work after seal", 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status }); enter(event.sessionID);'),
                    ("helper fault before seal", 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'fault("helper_timeout"); await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });'),
                    ("unload before response", 'if (![401, 429].includes(event.response.status)) return;', 'if (![401, 429].includes(event.response.status)) return; await api.cleanup();'),
                    ("unload during probe", 'try { await probe(); } catch { fault("helper_probe"); throw unavailable(); }', 'try { await Promise.all([probe(), api.cleanup()]); } catch { fault("helper_probe"); throw unavailable(); }'),
                    ("wrong provider", 'event.model.providerID !== "opencode-go"', 'event.model.providerID !== "unrelated"'),
                    ("wrong request kind", 'event.kind !== "primary"', 'event.kind !== "title"'),
                ]:
                    assert before in original_health, label
                    injected = original_health.replace(before, after, 1)
                    if label == "registry loss at HTTP 401":
                        # Readiness now runs after session resolution. Mutate only after the
                        # real HTTP failure, not merely because a session already exists.
                        injected = injected.replace("let session;", "let session; let providerFailure = false;")
                        injected = injected.replace("if (session) tools.pop();", "if (providerFailure) tools.pop();")
                        injected = injected.replace('if (![401, 429].includes(event.response.status)) return;', 'if (![401, 429].includes(event.response.status)) return; providerFailure = true;')
                    if label.startswith("unload"):
                        injected = injected.replace("return {\n    enter, fault,", "const api = {\n    enter, fault,").replace("  };\n}", "  }; return api;\n}")
                    health_file.write_text(injected)
                    try: result = stage(status=401)
                    finally: health_file.write_text(original_health)
                    assert result.get("report") is None and result["error"] and not result["error"].startswith("auth_failed:"), (label, result)
                    print("PASS " + label + " cannot authorize authentication failover")
            health_file = assets / "health.js"
            original = health_file.read_text()
            health_file.write_text(original.replace("process.env.OCR_OPENCODE_GUARD_NONCE", '"0".repeat(64)'))
            result = stage()
            assert result.get("report") is None and "guard" in result["error"].lower(), result
            health_file.write_text(original)
            print("PASS root rejects a wrong authenticated-channel nonce")
            original_server = (assets / "server.js").read_text()
            (assets / "server.js").write_text('export default {id:"ocr.guard",setup(){throw new Error("fixture")}};')
            result = stage()
            assert result["report"] is None and "guard" in result["error"].lower(), result
            print("PASS failed adapter cannot accept a schema-valid model report")
            if args.check_auth_failover:
                (assets / "server.js").write_text(original_server)
                health_file.write_text(original.replace('await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });', 'await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status }); const {spawn} = await import("node:child_process"); spawn("/usr/bin/sleep", ["30"], {detached:true,stdio:"ignore"}).unref();'))
                result = stage(status=401)
                assert result.get("report") is None and "survived cleanup" in result["error"], result
                print("PASS model-UID survivor overrides an otherwise valid authentication failure")
    finally:
        if worker:
            try: os.killpg(worker.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            _, errors = worker.communicate()
            if errors: print(errors, file=sys.stderr)
        if server: server.shutdown()
        for name in reversed(resources):
            subprocess.run([args.engine, "rm", "-f", name], capture_output=True, timeout=30)
    return 0


if __name__ == "__main__":
    sys.exit(main())
