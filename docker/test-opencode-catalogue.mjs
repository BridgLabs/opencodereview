// Pinned promise API shapes and real serialized Request objects. No CLI or network calls.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { EventEmitter } from "node:events";

const source = readFileSync(new URL("./opencode-guard/catalogue.js", import.meta.url), "utf8")
  .replace(/^import .*;\n/m, "").replace("export async function", "async function") + "\nglobalThis.guard = catalogueGuard;";
const routes = {
  "chat-completions": ["openai/v1", "@ai-sdk/openai-compatible", "@opencode/ai/providers/openai-compatible", "/chat/completions"],
  responses: ["openai/v1", "@ai-sdk/openai", "@opencode/ai/providers/openai/responses", "/responses"],
  messages: ["anthropic/v1", "@ai-sdk/anthropic", "@opencode/ai/providers/anthropic", "/messages?beta=true"],
  "generate-content": ["google/v1beta", "@ai-sdk/google", "@opencode/ai/providers/google", "/models/future-model:streamGenerateContent?alt=sse"],
};

async function fixture({ protocol = "chat-completions", noEffort = false, effort = "medium", missingFile = false,
  embedded = false, missingVariant = false, detachFailure = false, failHook, disposed = [], disposeThrows,
  credential = "synthetic-private-key" } = {}) {
  const [basePath, aisdk, opencodeai, suffix] = routes[protocol];
  const base = "https://opencode.ai/inference/go/" + basePath;
  const raw = {
    id: "future-model", name: "Future model", type: "chat", release_date: "2026-11-08",
    capabilities: { tools: { supported: true }, reasoning: { supported: true, ...(noEffort ? { toggle: true } : { effort: [effort] }) } },
    modalities: { input: ["text"], output: ["text"] }, limit: { context: 65536, output: 8192 },
    api: { [protocol]: { base_url: base, sdk: { aisdk, opencodeai } } },
  };
  const contract = { model: raw, effort: noEffort ? "" : effort };
  const files = new Map([
    ["/scratch/session/go-contract.json", JSON.stringify(contract)],
    ["/scratch/session/go-models.json", JSON.stringify({ "opencode-go": { id: "opencode-go", name: "OpenCode Go", env: [], models: { [raw.id]: raw } } })],
  ]);
  const settings = protocol === "messages" ? (effort === "none" ? { thinking: { type: "disabled" } }
    : { thinking: { type: "adaptive", display: "summarized" }, effort })
    : protocol === "generate-content" ? { thinkingConfig: { includeThoughts: true, thinkingLevel: effort } }
      : { reasoningEffort: effort };
  const normalized = {
    id: raw.id, modelID: raw.id, name: raw.name, providerID: "opencode-go", type: "chat", status: "active",
    capabilities: { tools: true, input: ["text"], output: ["text"] }, limit: raw.limit,
    variants: [...(!missingVariant && !noEffort ? [{ id: effort, settings }] : []), { id: "invented-budget", settings: { thinkingBudget: 1024 } }],
  };
  const records = new Map([["opencode-go", { models: new Map((embedded ? [normalized, { ...normalized, id: "embedded-other" }] : [normalized]).map(model => [model.id, model])) }]]);
  const hooks = new Map();
  let transform;
  let applied = false;
  const registration = tag => ({ async dispose() { disposed.push(tag); if (disposeThrows === tag) throw new Error("synthetic disposal failure"); } });
  const context = {
    provider: {
      async transform(callback) { transform = callback; return registration("provider"); },
      async list() { throw new Error("Use model.list().data for inventory, not provider.list()"); },
    },
    model: {
      async list() {
        if (!applied) {
          applied = true;
          try {
            transform({ get: id => records.get(id), models: {
              set(id, models) { records.set(id, { models: new Map(models.map(model => [model.id, model])) }); },
            } });
          } catch (error) { if (!detachFailure) throw error; }
        }
        return { location: "synthetic", data: [...records.values()].flatMap(record => [...record.models.values()])
          .map(model => ({ ...model, settings: { ...model.settings, apiKey: credential } })) };
      },
    },
    session: { async hook(name, callback) {
      if (failHook === name) throw new Error("synthetic registration failure");
      hooks.set(name, callback);
      return registration(name);
    } },
  };
  const sandbox = {
    process: { env: { OPENCODE_MODELS_PATH: "/scratch/session/go-models.json", OCR_OPENCODE_CATALOGUE_CONTRACT: "/scratch/session/go-contract.json", OCR_OPENCODE_GO_KEY: "synthetic-private-key" } },
    structuredClone, URL, TextDecoder,
    lstatSync(path) {
      if (missingFile || !files.has(path)) throw new Error("synthetic missing file");
      return { isFile: () => true, isSymbolicLink: () => false, uid: 0, nlink: 1, mode: 0o444, size: files.get(path).length };
    },
    readFileSync: path => files.get(path),
  };
  vm.runInNewContext(source, sandbox);
  const guard = await sandbox.guard(context);
  const ref = { id: raw.id, providerID: "opencode-go", variant: noEffort ? "default" : effort };
  const event = { sessionID: "fixture-session", agent: "ocr-review", model: ref };
  const body = protocol === "generate-content" ? { contents: [], generationConfig: noEffort ? {} : {
    thinkingConfig: { thinkingLevel: effort.toUpperCase(), includeThoughts: true },
  } } : { model: raw.id, stream: true,
    ...(noEffort ? {} : protocol === "chat-completions" ? { reasoning_effort: effort }
      : protocol === "responses" ? { reasoning: { effort, summary: "auto" } }
        : effort === "none" ? { thinking: { type: "disabled" } }
          : { thinking: { type: "adaptive", display: "summarized", block_binding: { prefix_mismatch_behavior: "drop_block" } }, output_config: { effort } }),
  };
  const request = (value = body, url = base + suffix) => new Request(url, {
    method: "POST", headers: { "content-type": "application/json; charset=utf-8" }, body: JSON.stringify(value),
  });
  const http = (value = body, url) => ({ ...event, kind: "primary", request: request(value, url) });
  return { guard, records, hooks, files, event, body, request, http, disposed, model: () => records.get("opencode-go").models.get(raw.id) };
}

async function rejectsHttp(options, change) {
  const fixtureValue = await fixture(options);
  await fixtureValue.guard.check();
  await fixtureValue.hooks.get("model.request")(fixtureValue.event);
  await assert.rejects(() => fixtureValue.hooks.get("http.request")(change(fixtureValue)));
  await assert.rejects(() => fixtureValue.guard.check(), "an attestation failure must remain fatal at seal");
  await fixtureValue.guard.dispose();
}

for (const protocol of Object.keys(routes)) {
  const valid = await fixture({ protocol });
  await valid.guard.check();
  assert.equal(valid.model().variants.length, 1);
  await valid.hooks.get("model.request")(valid.event); // Only Model.Ref, never Model.Info.
  const event = valid.http();
  await valid.hooks.get("http.request")(event);
  assert.deepEqual(await event.request.json(), valid.body, "attestation must not consume the original body");
  for (const change of [
    value => ({ ...value.http(), model: { ...value.event.model, variant: "wrong" } }),
    value => ({ ...value.http(), model: { ...value.event.model, id: "wrong-model" } }),
    value => ({ ...value.http(), sessionID: "different" }),
    value => ({ ...value.http(), kind: "title" }),
    value => ({ ...value.http(), agent: "untrusted" }),
    value => value.http(value.body, "https://evil.test/inference/go/openai/v1/chat/completions"),
    value => value.http({ ...value.body, reasoning_budget: 100 }),
  ]) await rejectsHttp({ protocol }, change);
  await rejectsHttp({ protocol }, value => {
    const wrong = structuredClone(value.body);
    if (protocol === "generate-content") wrong.generationConfig.thinkingConfig.thinkingBudget = 4096;
    else if (protocol === "messages") wrong.thinking = { type: "enabled", budget_tokens: 4096 };
    else if (protocol === "responses") wrong.reasoning.effort = "wrong";
    else wrong.reasoning_effort = "wrong";
    return value.http(wrong);
  });
  if (protocol === "generate-content") {
    await rejectsHttp({ protocol }, value => value.http(value.body, value.request().url.replace("future-model", "wrong-model")));
  } else {
    await rejectsHttp({ protocol }, value => value.http({ ...value.body, model: "wrong-model" }));
  }
  if (protocol === "messages") await rejectsHttp({ protocol }, value => value.http({ ...value.body,
    thinking: { ...value.body.thinking, block_binding: { prefix_mismatch_behavior: "untrusted" } },
  }));
  await rejectsHttp({ protocol }, value => value.http(value.body, value.request().url + "&extra=true"));
  const noEffort = await fixture({ protocol, noEffort: true });
  await noEffort.guard.check();
  assert.equal(noEffort.model().variants.length, 0);
  await noEffort.hooks.get("model.request")(noEffort.event);
  await noEffort.hooks.get("http.request")(noEffort.http());
  const invented = structuredClone(noEffort.body);
  if (protocol === "generate-content") invented.generationConfig.thinkingConfig = { thinkingLevel: "HIGH", includeThoughts: true };
  else if (protocol === "messages") invented.thinking = { type: "adaptive" };
  else if (protocol === "responses") invented.reasoning = { effort: "high" };
  else invented.reasoning_effort = "high";
  await assert.rejects(() => noEffort.hooks.get("http.request")(noEffort.http(invented)));
  await valid.guard.dispose();
  assert.equal(valid.disposed.length, 3);
}

const none = await fixture({ protocol: "messages", effort: "none" });
await none.guard.check();
await none.hooks.get("http.request")(none.http());
await assert.rejects(() => fixture({ missingFile: true }));
const wrongCredential = await fixture({ credential: "untrusted-key" });
await assert.rejects(() => wrongCredential.guard.check());
for (const options of [{ embedded: true }, { missingVariant: true }, { embedded: true, detachFailure: true }]) {
  const invalid = await fixture(options);
  await assert.rejects(() => invalid.guard.check());
}
const changed = await fixture();
await changed.guard.check();
changed.model().settings.baseURL = "https://evil.test/v1";
await assert.rejects(() => changed.guard.check());
const changedFile = await fixture();
await changedFile.guard.check();
changedFile.files.set("/scratch/session/go-models.json", "{}");
await assert.rejects(() => changedFile.guard.check());
const disposal = [];
await assert.rejects(() => fixture({ failHook: "http.request", disposed: disposal }));
assert.deepEqual(disposal, ["model.request", "provider"]);
const throwing = await fixture({ disposeThrows: "model.request" });
await assert.rejects(() => throwing.guard.dispose());
assert.deepEqual(throwing.disposed, ["http.request", "model.request", "provider"]);
const oversized = await fixture();
await oversized.guard.check();
await assert.rejects(() => oversized.hooks.get("http.request")(oversized.http({ ...oversized.body, input: "x".repeat(8 * 1024 * 1024) })));

// Setup failures after catalogue registration must dispose health, tools and catalogue,
// even if one of the disposers itself fails. No executor/tool process is started here.
const serverSource = readFileSync(new URL("./opencode-guard/server.js", import.meta.url), "utf8")
  .replace(/^import .*;\n/gm, "").replace("export default", "globalThis.plugin =");
for (const failure of ["health.create", "tools.register", "health.start", undefined]) {
  const disposed = [];
  const original = new Error("synthetic setup failure");
  const sandbox = {
    catalogueGuard: async () => ({ check: async () => {}, dispose: async () => { disposed.push("catalogue"); } }),
    createHealth: () => {
      if (failure === "health.create") throw original;
      return {
        fault() {}, cleanup: async () => { disposed.push("health"); },
        start: async () => { if (failure === "health.start") throw original; },
      };
    },
  };
  vm.runInNewContext(serverSource, sandbox);
  const setup = sandbox.plugin.setup({ tool: { async transform() {
    if (failure === "tools.register") throw original;
    return { async dispose() { disposed.push("tools"); if (failure === "health.start") throw new Error("synthetic cleanup failure"); } };
  } } });
  if (failure) await assert.rejects(setup, error => error === original);
  else await (await setup)();
  assert.deepEqual(disposed, failure === "health.create" ? ["catalogue"] : failure === "tools.register"
    ? ["health", "catalogue"] : ["health", "tools", "catalogue"]);
}
// Inventory is legitimately empty during plugin setup. Readiness must wait for the first
// resolved request and its root ACK, not attest an empty bootstrap inventory or remove checks.
const healthSource = readFileSync(new URL("./opencode-guard/health.js", import.meta.url), "utf8")
  .replace(/^import .*;\n/m, "").replace("export function", "function") + "\nglobalThis.makeHealth = createHealth;";
const frames = [];
const healthHooks = new Map();
let modelReady = false;
let checks = 0;
let probes = 0;
let disposedHealth = 0;
const healthSandbox = {
  process: { env: { OCR_OPENCODE_GUARD_SOCKET: "/scratch/session/guard.sock", OCR_OPENCODE_GUARD_NONCE: "a".repeat(64) } },
  Buffer, setTimeout, clearTimeout,
  createConnection() {
    const socket = new EventEmitter();
    socket.writable = true;
    socket.write = text => {
      const frame = JSON.parse(text);
      frames.push(frame);
      queueMicrotask(() => socket.emit("data", Buffer.from(JSON.stringify({ v: 1, type: "ack", for: frame.type }) + "\n")));
    };
    socket.end = text => {
      if (text) frames.push(JSON.parse(text));
      socket.writable = false;
      queueMicrotask(() => socket.emit("close"));
    };
    queueMicrotask(() => socket.emit("connect"));
    return socket;
  },
};
vm.runInNewContext(healthSource, healthSandbox);
const health = healthSandbox.makeHealth({
  tool: { async list() { return ["ocr_bash", "ocr_glob", "ocr_grep", "ocr_read", "ocr_write"]
    .map(name => ({ name, options: { permission: name, codemode: false } })); } },
  session: { async hook(name, callback) { healthHooks.set(name, callback); return { async dispose() { disposedHealth++; } }; } },
}, async () => { checks++; if (!modelReady) throw new Error("bootstrap inventory is not final"); });
await health.start(async () => { probes++; });
assert.equal(frames.length, 0);
assert.equal(checks, 0);
assert.equal(probes, 0);
modelReady = true;
await healthHooks.get("model.request")({ sessionID: "fixture-session" });
assert.equal(frames[0].type, "ready");
assert.equal(probes, 1);
assert.equal(checks, 2);
await healthHooks.get("http.request")({ sessionID: "fixture-session" });
assert.equal(frames.length, 1, "HTTP must await the same acknowledged readiness, never issue duplicate handshakes");
await healthHooks.get("experimental.idle")({ sessionID: "fixture-session", background: [] });
assert.equal(frames[1].type, "complete");
assert.equal(frames[1].sessionID, "fixture-session");
await assert.rejects(() => healthHooks.get("http.request")({ sessionID: "fixture-session" }));
assert.equal(frames[2].code, "work_after_seal");
await health.cleanup();
assert.equal(disposedHealth, 5);
console.log("PASS pinned APIs, deferred acknowledged readiness, four wire protocols, no-effort, tamper and cleanup");
