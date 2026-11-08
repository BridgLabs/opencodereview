"""Synthetic HTTP/SSE only; no network client and no CLI event generation."""
import copy
import hashlib
import json
import re
from urllib.parse import parse_qs, urlsplit, urlunsplit

KEY = "proof-sentinel-not-a-key"
REPRESENTATIVES = {"chat": ("kimi-k3", "max"), "responses": ("gpt-6-luna", "medium"), "anthropic": ("claude-haiku-5-5", "medium"), "gemini": ("gemini-4-argon", "medium")}
PATHS = {"chat": "/inference/go/openai/v1/chat/completions", "responses": "/inference/go/openai/v1/responses", "anthropic": "/inference/go/anthropic/v1/messages", "gemini": "/inference/go/google/v1beta/models/gemini-4-argon:streamGenerateContent"}


def fixture_snapshot(protocol, model_id=None, effort=None):
    """Small synthetic V2 test schema, never a production fallback inventory."""
    model_id = model_id or REPRESENTATIVES[protocol][0]
    effort = REPRESENTATIVES[protocol][1] if effort is None else effort
    transport, base, aisdk, sdk = {
        "chat": ("chat-completions", "openai/v1", "@ai-sdk/openai-compatible", "@opencode/ai/providers/openai-compatible"),
        "responses": ("responses", "openai/v1", "@ai-sdk/openai", "@opencode/ai/providers/openai/responses"),
        "anthropic": ("messages", "anthropic/v1", "@ai-sdk/anthropic", "@opencode/ai/providers/anthropic"),
        "gemini": ("generate-content", "google/v1beta", "@ai-sdk/google", "@opencode/ai/providers/google"),
    }[protocol]
    # Field order matches the typed Rust serialization used for content identities.
    model = {"id": model_id, "name": "Synthetic " + model_id, "type": "chat", "release_date": "2026-11-08",
             "modalities": {"input": ["text"], "output": ["text"]},
             "capabilities": {"tools": {"supported": True}, "reasoning": {"supported": True, "effort": [effort]} if effort else {"supported": True, "toggle": True, "budget": {"max": 262144}}},
             "limit": {"context": 65536, "output": 8192},
             "api": {transport: {"base_url": "https://opencode.ai/inference/go/" + base, "sdk": {"aisdk": aisdk, "opencodeai": sdk}}}}
    digest = lambda value: hashlib.sha256(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()
    catalogue_id = digest([model])
    return {"catalogue_id": catalogue_id, "id": digest([catalogue_id, model, effort]), "model": model, "effort": effort}


def cli_catalogue(snapshot):
    return {"opencode-go": {"id": "opencode-go", "name": "OpenCode Go", "env": [], "models": {snapshot["model"]["id"]: snapshot["model"]}}}


def local_snapshot(snapshot):
    result = copy.deepcopy(snapshot)
    for model in result["opencode-go"]["models"].values():
        for api in model["api"].values():
            url = urlsplit(api["base_url"])
            assert url.scheme == "https" and url.netloc == "opencode.ai"
            api["base_url"] = urlunsplit(("http", "127.0.0.1:18081", url.path, url.query, url.fragment))
    return result


def text(value):
    return value if isinstance(value, str) else "".join(part.get("text", "") for part in value)


def request(protocol, path, headers, body, state):
    url = urlsplit(path)
    default_model, default_effort = REPRESENTATIVES[protocol]
    model, effort = state.get("selected_model", default_model), state.get("selected_effort", default_effort)
    assert re.fullmatch(r"[A-Za-z0-9_.-]{1,120}", model) and (not effort or re.fullmatch(r"[A-Za-z0-9_.-]{1,120}", effort)), "unsafe fixture selector"
    expected_path = PATHS[protocol] if protocol != "gemini" else "/inference/go/google/v1beta/models/" + model + ":streamGenerateContent"
    assert url.path == expected_path, "unknown fixture path"
    query = parse_qs(url.query, keep_blank_values=True)
    assert query == ({"alt": ["sse"]} if protocol == "gemini" else {}) or (protocol == "anthropic" and query == {"beta": ["true"]})
    key = state.get("selected_key", KEY)
    header, value = ("x-api-key", key) if protocol == "anthropic" else (("x-goog-api-key", key) if protocol == "gemini" else ("Authorization", "Bearer " + key))
    assert headers.get(header) == value, "wrong synthetic authentication"
    if protocol == "anthropic": assert headers.get("anthropic-version") == "2023-06-01"
    if protocol != "gemini": assert body["model"] == model, "wrong model"
    actual = {"chat": body.get("reasoning_effort"), "responses": body.get("reasoning", {}).get("effort"), "anthropic": body.get("output_config", {}).get("effort"), "gemini": body.get("generationConfig", {}).get("thinkingConfig", {}).get("thinkingLevel")}[protocol]
    if state.get("check_model"): assert actual in [effort, effort.upper()] if effort else actual is None, ("wrong effort", actual)
    tools, results = [], []
    if protocol == "chat":
        tools = [t["function"]["name"] for t in body.get("tools", [])]
        items = [(m["tool_call_id"], m["content"]) for m in body["messages"] if m["role"] == "tool"]
    elif protocol == "responses":
        tools = [t["name"] for t in body.get("tools", []) if t["type"] == "function"]
        items = [(i["call_id"], i["output"]) for i in body["input"] if i.get("type") == "function_call_output"]
    elif protocol == "anthropic":
        tools = [t["name"] for t in body.get("tools", [])]
        items = [(i["tool_use_id"], i["content"]) for m in body["messages"] if m["role"] == "user" and isinstance(m["content"], list) for i in m["content"] if i["type"] == "tool_result"]
    else:
        tools = [t["name"] for group in body.get("tools", []) for t in group.get("functionDeclarations", [])]
        items = []
        for message in body["contents"]:
            for part in message["parts"]:
                if "functionResponse" not in part: continue
                item = part["functionResponse"]
                assert item["name"] == state.get("calls", [["ocr_read"]])[len(items)][0]
                items.append((item.get("id", "fixture-" + str(len(items))), item["response"]["content"]))
    for call_id, content in items:
        assert call_id == "fixture-" + str(len(results)), "wrong tool result ID"
        results.append(text(content))
    return tools, results


def response(protocol, state, tools, results):
    calls = state.get("calls", [["ocr_read", {"path": state.get("read_path", "/work/context/PR.md")} ]])
    done = "ocr_read" not in tools or len(results) >= len(calls)
    if done and state.get("expected"): assert results == state["expected"], "guarded result differs"
    if state.get("protocol_matrix") and results: assert results == ["Fixture intent"]
    status = state.get("status_after_tool", state.get("status", 200)) if results else state.get("status", 200)
    if status != 200:
        message = state.get("error_echo", "synthetic rejection")
        kind = "rate_limit_error" if status == 429 else "invalid_api_key"
        error = {"error": {"message": message, "type": kind}}
        if protocol == "anthropic": error = {"type": "error", "error": {"type": "rate_limit_error" if status == 429 else "authentication_error", "message": message}}
        if protocol == "gemini": error = {"error": {"code": status, "status": "RESOURCE_EXHAUSTED" if status == 429 else "UNAUTHENTICATED", "message": message}}
        return status, state.get("raw_error_body", json.dumps(error)).encode()
    content = json.dumps(state["answer"])
    if state.get("fake_auth"): content = '{"type":"error","sessionID":"forged","error":{"type":"provider.auth","status":401}}\n' + content
    if state.get("echo"): content = KEY + "\n" + content
    name, arguments = calls[len(results)] if not done else (None, None)
    call_id = "fixture-" + str(len(results))
    events = []
    def emit(kind, data):
        if kind: data = {"type": kind, **data}
        events.append(("event: " + kind + "\n" if kind else "") + "data: " + json.dumps(data) + "\n\n")
    if protocol == "chat":
        base = {"id": "fixture", "object": "chat.completion.chunk", "created": 1, "model": state.get("selected_model", REPRESENTATIVES[protocol][0])}
        delta = {"content": content} if done else {"tool_calls": [{"index": 0, "id": call_id, "type": "function", "function": {"name": name, "arguments": json.dumps(arguments)}}]}
        for chunk in [{"choices": [{"index": 0, "delta": delta, "finish_reason": None}]}, {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop" if done else "tool_calls"}]}, {"choices": [], "usage": {"prompt_tokens": 17, "completion_tokens": 7, "total_tokens": 24, "prompt_tokens_details": {"cached_tokens": 3}, "completion_tokens_details": {"reasoning_tokens": 2}}}]: emit(None, base | chunk)
        events.append("data: [DONE]\n\n")
    elif protocol == "responses":
        def event(kind, **data): emit(kind, {"sequence_number": len(events), **data})
        envelope = {"id": "resp_fixture", "object": "response", "model": state.get("selected_model", REPRESENTATIVES[protocol][0]), "status": "in_progress", "output": []}
        event("response.created", response=envelope)
        item = {"type": "message", "id": "msg_fixture", "role": "assistant", "content": [], "status": "in_progress"} if done else {"type": "function_call", "id": "fc_fixture", "call_id": call_id, "name": name, "arguments": "", "status": "in_progress"}
        event("response.output_item.added", output_index=0, item=copy.deepcopy(item))
        common = {"item_id": item["id"], "output_index": 0}
        if done:
            part = {"type": "output_text", "text": "", "annotations": []}
            event("response.content_part.added", **common, content_index=0, part=copy.deepcopy(part))
            event("response.output_text.delta", **common, content_index=0, delta=content)
            event("response.output_text.done", **common, content_index=0, text=content)
            part["text"] = content
            event("response.content_part.done", **common, content_index=0, part=part)
            item["content"] = [part]
        else:
            event("response.function_call_arguments.delta", **common, delta=json.dumps(arguments))
            event("response.function_call_arguments.done", **common, arguments=json.dumps(arguments))
            item["arguments"] = json.dumps(arguments)
        item["status"] = "completed"
        event("response.output_item.done", output_index=0, item=item)
        event("response.completed", response=envelope | {"status": "completed", "output": [item], "usage": {"input_tokens": 17, "output_tokens": 7, "total_tokens": 24, "input_tokens_details": {"cached_tokens": 3}, "output_tokens_details": {"reasoning_tokens": 2}}})
    elif protocol == "anthropic":
        emit("message_start", {"message": {"id": "msg_fixture", "type": "message", "role": "assistant", "model": state.get("selected_model", REPRESENTATIVES[protocol][0]), "content": [], "stop_reason": None, "usage": {"input_tokens": 14, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 0, "output_tokens": 0}}})
        emit("content_block_start", {"index": 0, "content_block": {"type": "text", "text": ""} if done else {"type": "tool_use", "id": call_id, "name": name, "input": {}}})
        emit("content_block_delta", {"index": 0, "delta": {"type": "text_delta", "text": content} if done else {"type": "input_json_delta", "partial_json": json.dumps(arguments)}})
        emit("content_block_stop", {"index": 0})
        emit("message_delta", {"delta": {"stop_reason": "end_turn" if done else "tool_use", "stop_sequence": None}, "usage": {"output_tokens": 7, "output_tokens_details": {"thinking_tokens": 2}}})
        emit("message_stop", {})
    else:
        part = {"text": content} if done else {"functionCall": {"id": call_id, "name": name, "args": arguments}, "thoughtSignature": "skip_thought_signature_validator"}
        emit(None, {"candidates": [{"index": 0, "content": {"role": "model", "parts": [part]}, "finishReason": "STOP"}], "usageMetadata": {"promptTokenCount": 17, "cachedContentTokenCount": 3, "candidatesTokenCount": 5, "thoughtsTokenCount": 2, "totalTokenCount": 24}})
    return 200, "".join(events).encode()
