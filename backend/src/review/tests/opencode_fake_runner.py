"""Synthetic external runner IPC boundary. Never launches an AI CLI or opens network sockets."""
import json
import socket
import struct
import sys

path, log, outcome, first = sys.argv[1:]
first = int(first)
listener = socket.socket(socket.AF_UNIX)
listener.bind(path)
listener.listen()
print("ready", flush=True)


def receive(connection):
    def exact(size):
        data = b""
        while len(data) < size:
            part = connection.recv(size - len(data))
            if not part:
                raise EOFError()
            data += part
        return data
    return json.loads(exact(struct.unpack(">I", exact(4))[0]))


def send(connection, value):
    data = json.dumps(value).encode()
    connection.sendall(struct.pack(">I", len(data)) + data)


while True:
    connection, _ = listener.accept()
    with connection:
        try:
            while True:
                request = receive(connection)
                operation = request["operation"]
                if operation == "Catalog":
                    send(connection, {"event": "Catalog", "models": ["new-review-model", "claude-opus-5-5"], "version": "2.2.6"})
                    break
                if operation != "Run":
                    send(connection, {"event": "Ack"})
                    continue
                if request["preflight"]:
                    send(connection, {"event": "Ready"})
                    break
                account = request["account"]
                with open(log, "a", encoding="utf8") as output:
                    output.write(json.dumps({"account": account, "provider": request["provider"], "model": request["model"], "effort": request["effort"]}) + "\n")
                if outcome == "startup":
                    send(connection, {"event": "Error", "error": "OpenCode Go startup failed: synthetic fixture"})
                    break
                send(connection, {"event": "Started"})
                error = None
                if account == first and "failover" in outcome:
                    error = "usage_limit:0|Synthetic authenticated runner outcome"
                elif account == first and outcome in ["auth", "limit", "health"]:
                    error = {"auth": "auth_failed:0|Synthetic authenticated runner outcome", "limit": "usage_limit:0|Synthetic authenticated runner outcome", "health": "OpenCode Go guard health failed: synthetic fixture"}[outcome]
                tokens = {"input": 14, "output": 5, "reasoning": 2, "cache": {"read": 3, "write": 1}} if account == first else {"input": 20, "output": 8, "reasoning": 4, "cache": {"read": 6, "write": 2}}
                transcript = "\n".join(json.dumps(event) for event in [
                    {"type": "ocr.opencode.model", "model": request["model"]},
                    {"type": "step_finish", "sessionID": "synthetic", "part": {"id": "step", "tokens": tokens}},
                ])
                if request["provider"] == "claude":
                    transcript = json.dumps({"type": "result", "modelUsage": {request["model"]: {
                        "inputTokens": 140 if outcome == "claude-failover-go" else 14, "outputTokens": 7, "cacheReadInputTokens": 3, "cacheCreationInputTokens": 1,
                    }}})
                elif request["provider"] == "codex":
                    transcript = json.dumps({"type": "turn.completed", "model": request["model"], "usage": {
                        "input_tokens": 17, "cached_input_tokens": 3, "output_tokens": 7,
                    }})
                stage = next(json.loads(value) for key, value in request["files"].items() if key.endswith("/STAGE.json"))
                report = {"contractVersion": 2, "snapshotId": stage["snapshotId"], "reviewedFiles": stage["files"], "missingEvidence": [], "findings": []}
                send(connection, {"event": "Finished", "report": None if error else json.dumps(report), "decisions": None, "transcript": transcript, "error": error, "credential": None})
                break
        except (EOFError, BrokenPipeError):
            pass
