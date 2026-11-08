#!/usr/bin/env python3
"""Offline, stdlib protocol contracts; does not execute the runner or CLI."""
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("codecs", Path(__file__).with_name("opencode-protocol-fixtures.py"))
codecs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(codecs)


def rejected(action):
    try: action()
    except (AssertionError, KeyError): return
    raise AssertionError("invalid contract accepted")


snapshot = codecs.cli_catalogue(codecs.fixture_snapshot("chat"))
local = codecs.local_snapshot(snapshot)
restored = json.loads(json.dumps(local).replace("http://127.0.0.1:18081", "https://opencode.ai"))
assert restored == snapshot, "metadata/path changed"
for protocol, (model, effort) in codecs.REPRESENTATIVES.items():
    frozen = codecs.fixture_snapshot(protocol)
    assert frozen["model"]["id"] == model and frozen["effort"] == effort
    assert len(frozen["id"]) == 64 and codecs.cli_catalogue(frozen)["opencode-go"]["models"][model] == frozen["model"]
    state = {"answer": {"report": {}}, "selected_model": model, "selected_effort": effort, "check_model": True, "protocol_matrix": True}
    headers = {"Authorization": "Bearer " + codecs.KEY, "x-api-key": codecs.KEY, "x-goog-api-key": codecs.KEY, "anthropic-version": "2023-06-01"}
    body = {"model": model, "reasoning_effort": effort, "reasoning": {"effort": effort}, "output_config": {"effort": effort}, "generationConfig": {"thinkingConfig": {"thinkingLevel": effort.upper()}}, "messages": [], "input": [], "contents": []}
    body["tools"] = {"chat": [{"function": {"name": "ocr_read"}}], "responses": [{"type": "function", "name": "ocr_read"}], "anthropic": [{"name": "ocr_read", "input_schema": {}}], "gemini": [{"functionDeclarations": [{"name": "ocr_read"}]}]}[protocol]
    path = codecs.PATHS[protocol] + ("?alt=sse" if protocol == "gemini" else "")
    tools, results = codecs.request(protocol, path, headers, body, state)
    assert tools == ["ocr_read"] and results == []
    status, wire = codecs.response(protocol, state, tools, results)
    assert status == 200 and b"ocr_read" in wire and wire.endswith(b"\n\n")
    first = [json.loads(line[6:]) for line in wire.decode().splitlines() if line.startswith("data: ") and line != "data: [DONE]"]
    if protocol == "chat": body["messages"] = [{"role": "tool", "tool_call_id": "fixture-0", "content": "Fixture intent"}]
    elif protocol == "responses": body["input"] = [{"type": "function_call_output", "call_id": "fixture-0", "output": "Fixture intent"}]
    elif protocol == "anthropic": body["messages"] = [{"role": "user", "content": [{"type": "tool_result", "tool_use_id": "fixture-0", "content": [{"type": "text", "text": "Fixture intent"}]}]}]
    else: body["contents"] = [{"parts": [{"functionResponse": {"name": "ocr_read", "response": {"content": "Fixture intent"}}}]}]
    tools, results = codecs.request(protocol, path, headers, body, state)
    assert results == ["Fixture intent"]
    status, wire = codecs.response(protocol, state, tools, results)
    assert status == 200 and b"report" in wire
    second = [json.loads(line[6:]) for line in wire.decode().splitlines() if line.startswith("data: ") and line != "data: [DONE]"]
    totals = [0, 0, 0]
    for events in [first, second]:
        if protocol == "chat":
            u = events[-1]["usage"]
            tokens = (u["prompt_tokens"] - u["prompt_tokens_details"]["cached_tokens"], u["completion_tokens"], u["prompt_tokens_details"]["cached_tokens"])
            assert events[-2]["choices"][0]["finish_reason"] in ["stop", "tool_calls"]
        elif protocol == "responses":
            assert events[-1]["type"] == "response.completed"
            assert [e["sequence_number"] for e in events] == list(range(len(events)))
            u = events[-1]["response"]["usage"]
            tokens = (u["input_tokens"] - u["input_tokens_details"]["cached_tokens"], u["output_tokens"], u["input_tokens_details"]["cached_tokens"])
        elif protocol == "anthropic":
            assert events[-1]["type"] == "message_stop"
            u = events[0]["message"]["usage"]
            tokens = (u["input_tokens"], events[-2]["usage"]["output_tokens"], u["cache_read_input_tokens"])
        else:
            assert events[-1]["candidates"][0]["finishReason"] == "STOP"
            u = events[-1]["usageMetadata"]
            tokens = (u["promptTokenCount"] - u["cachedContentTokenCount"], u["candidatesTokenCount"] + u["thoughtsTokenCount"], u["cachedContentTokenCount"])
        totals = [a + b for a, b in zip(totals, tokens)]
    assert totals == [28, 14, 6], totals
    for status in [401, 429]:
        for late in [False, True]:
            actual, wire = codecs.response(protocol, state | {"status_after_tool" if late else "status": status}, tools, results if late else [])
            assert actual == status and "error" in json.loads(wire) and b"data:" not in wire
    rejected(lambda: codecs.request(protocol, "/unknown", headers, body, state))
    rejected(lambda: codecs.request(protocol, path, {}, body, state))
    rejected(lambda: codecs.request(protocol, path, headers, body, state | {"selected_model": "unknown"}))
    print("PASS", protocol, "tool loop, terminal SSE, usage 28/14/6, HTTP errors, rejection")

smoke = Path(__file__).with_name("opencode-runner-smoke.py").read_text()
assert 'protocols.fixture_snapshot' in smoke and '"baseURL": "http://127.0.0.1:18081/v1"' not in smoke
assert '"unexpected retry/request count"' in smoke and '"--protocol-errors"' in smoke
assert 'ThreadingHTTPServer(("127.0.0.1", 18081)' in smoke
assert 'if args.check_auth_failover:' in smoke and 'authenticated-channel nonce' in smoke
print("PASS static harness seams; not runner integration")
