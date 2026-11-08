import { lstatSync, readFileSync } from "node:fs";

const failed = () => new Error("OpenCode Go catalogue materialization failed");
const canonical = value => Array.isArray(value) ? value.map(canonical) : value && typeof value === "object"
  ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value;
const same = (a, b) => JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const object = value => value !== null && typeof value === "object" && !Array.isArray(value);
const controls = ["reasoning_effort", "reasoningEffort", "reasoning", "thinking", "thinkingConfig", "thinking_config",
  "generationConfig", "generation_config", "output_config", "outputConfig", "thinking_budget", "thinkingBudget",
  "thinking_level", "thinkingLevel", "reasoning_budget", "reasoningBudget", "enable_thinking", "enableThinking",
  "budget_tokens", "budgetTokens", "effort", "extra_body", "extraBody", "providerOptions", "provider_options"];
const rejectControls = (value, allowed = []) => {
  if (!object(value) || controls.some(key => Object.hasOwn(value, key) && !allowed.includes(key))) throw failed();
};

function immutable(path) {
  if (!path?.startsWith("/scratch/session/go-")) throw failed();
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== 0 || stat.nlink !== 1 || (stat.mode & 0o222) || stat.size > 2 * 1024 * 1024) throw failed();
  return JSON.parse(readFileSync(path, "utf8"));
}

async function boundedBody(request) {
  const reader = request.clone().body?.getReader();
  if (!reader) throw failed();
  const chunks = [];
  let bytes = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      if (!ArrayBuffer.isView(value) || value.BYTES_PER_ELEMENT !== 1 || chunks.length >= 65536) throw failed();
      bytes += value.byteLength;
      if (bytes > 8 * 1024 * 1024) throw failed();
      chunks.push(value);
    }
  } catch (error) {
    // A tee's cancel promise may wait for the untouched original branch. Never await it.
    void reader.cancel().catch(() => {});
    throw error;
  } finally { reader.releaseLock(); }
  const joined = new Uint8Array(bytes);
  let offset = 0;
  for (const chunk of chunks) { joined.set(chunk, offset); offset += chunk.byteLength; }
  const body = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(joined));
  let nodes = 0;
  const visit = (value, depth) => {
    if (++nodes > 100000 || depth > 32) throw failed();
    if (value && typeof value === "object") {
      const children = Object.values(value);
      if (children.length > 10000) throw failed();
      for (const child of children) visit(child, depth + 1);
    }
  };
  visit(body, 0);
  if (!object(body)) throw failed();
  return body;
}

/** Prune normalized Model.Info values, then attest final inventory and serialized HTTP. */
export async function catalogueGuard(context) {
  const contractPath = process.env.OCR_OPENCODE_CATALOGUE_CONTRACT;
  const modelsPath = process.env.OPENCODE_MODELS_PATH;
  const contract = immutable(contractPath);
  const raw = contract.model;
  const efforts = raw.capabilities.reasoning.effort ?? [];
  const [[protocol, api]] = Object.entries(raw.api);
  const compatibility = Object.fromEntries(Object.entries(api.compatibility ?? {}).map(([key, value]) => [
    ({ reasoning_field: "reasoningField", max_tokens_field: "maxTokensField", media_input: "mediaInput" })[key], value,
  ]));
  const registrations = [];
  let disposed = false;
  const dispose = async () => {
    if (disposed) return;
    disposed = true;
    let failure;
    for (const registration of registrations.reverse()) {
      try { await registration.dispose(); } catch (error) { failure ??= error; }
    }
    if (failure) throw failure;
  };
  const verifyFiles = () => {
    if (!same(immutable(contractPath), contract)) throw failed();
    const inventory = immutable(modelsPath);
    if (Object.keys(inventory).length !== 1 || !same(inventory["opencode-go"], {
      id: "opencode-go", name: "OpenCode Go", env: [], models: { [raw.id]: raw },
    })) throw failed();
  };
  let materialized = false;
  let faulted = false;
  let expected;
  let session;
  const verifyBase = model => {
    if (model?.providerID !== "opencode-go" || model.id !== raw.id || model.modelID !== raw.id ||
      model.name !== raw.name || model.type !== "chat" || model.capabilities?.tools !== true ||
      !same(model.limit, raw.limit) || model.family !== (raw.family || undefined) || model.status !== (raw.status ?? "active") ||
      !same(model.compatibility ?? {}, compatibility) || model.body || model.headers) throw failed();
  };
  const verifyRef = (event, http = false) => {
    const ref = event.model;
    // CLI 2.2.6 represents an omitted suffix as the non-selectable 'default' sentinel.
    // It is valid only with no advertised/pruned variants and no reasoning wire controls.
    const variant = !contract.effort && efforts.length === 0 && ref?.variant === "default" ? "" : (ref?.variant ?? "");
    if (typeof event.sessionID !== "string" || !event.sessionID || event.sessionID.length > 128 ||
      event.agent !== "ocr-review" || (http ? event.kind !== "primary" : event.kind !== undefined && event.kind !== "primary") ||
      ref?.id !== raw.id || ref.providerID !== "opencode-go" || variant !== contract.effort ||
      (session && session !== event.sessionID)) throw failed();
    session = event.sessionID;
  };
  const check = async () => {
    try {
      if (faulted || disposed) throw failed();
      verifyFiles();
      const inventory = await context.model.list();
      if (faulted || !materialized || !expected || !Array.isArray(inventory.data) || inventory.data.length !== 1) throw failed();
      const model = inventory.data[0];
      verifyBase(model);
      // Provider config adds the private credential after transforms. Attest it against the
      // root-projected environment separately; it is never catalogue data or diagnostics.
      const { apiKey, ...baseSettings } = model.settings ?? {};
      if (!process.env.OCR_OPENCODE_GO_KEY || apiKey !== process.env.OCR_OPENCODE_GO_KEY ||
        model.package !== api.sdk.opencodeai || !same(baseSettings, expected.settings) ||
        !same(model.capabilities, expected.capabilities) || !same(model.variants, expected.variants)) throw failed();
    } catch { faulted = true; throw failed(); }
  };
  const wire = async event => {
    await check();
    verifyRef(event, true);
    const request = event.request;
    const url = new URL(request.url);
    const base = new URL(api.base_url);
    const suffix = { "chat-completions": "/chat/completions", responses: "/responses", messages: "/messages",
      "generate-content": `/models/${raw.id}:streamGenerateContent` }[protocol];
    if (request.method !== "POST" || url.origin !== base.origin || url.username || url.password || url.hash ||
      url.pathname !== base.pathname + suffix || request.headers.get("content-type")?.split(";")[0].trim().toLowerCase() !== "application/json" ||
      (protocol === "messages" ? !["", "?beta=true"].includes(url.search) : url.search !== (protocol === "generate-content" ? "?alt=sse" : ""))) throw failed();
    const body = await boundedBody(request);
    if (protocol === "generate-content") {
      rejectControls(body, ["generationConfig"]);
      if (Object.hasOwn(body, "model")) throw failed();
      const generation = body.generationConfig ?? {};
      rejectControls(generation, ["thinkingConfig"]);
      if (!contract.effort) {
        if (Object.hasOwn(generation, "thinkingConfig")) throw failed();
      } else {
        const thinking = generation.thinkingConfig;
        if (!object(thinking) || thinking.includeThoughts !== true ||
          ![contract.effort, contract.effort.toUpperCase()].includes(thinking.thinkingLevel) ||
          Object.keys(thinking).some(key => !["thinkingLevel", "includeThoughts"].includes(key))) throw failed();
      }
      return;
    }
    if (body.model !== raw.id || body.stream !== true) throw failed();
    if (protocol === "chat-completions") {
      rejectControls(body, ["reasoning_effort"]);
      if (contract.effort ? body.reasoning_effort !== contract.effort : Object.hasOwn(body, "reasoning_effort")) throw failed();
    } else if (protocol === "responses") {
      rejectControls(body, ["reasoning"]);
      if (!contract.effort) {
        if (Object.hasOwn(body, "reasoning")) throw failed();
      } else if (!object(body.reasoning) || body.reasoning.effort !== contract.effort ||
        Object.keys(body.reasoning).some(key => !["effort", "summary"].includes(key))) throw failed();
    } else if (protocol === "messages") {
      rejectControls(body, ["thinking", "output_config"]);
      const output = body.output_config ?? {};
      rejectControls(output, ["effort"]);
      if (!contract.effort) {
        if (Object.hasOwn(body, "thinking") || Object.hasOwn(output, "effort")) throw failed();
      } else if (contract.effort === "none") {
        // The pinned normalizer represents an advertised 'none' as disabled thinking.
        if (!same(body.thinking, { type: "disabled" }) || Object.hasOwn(output, "effort")) throw failed();
      } else if (output.effort !== contract.effort || !object(body.thinking) || body.thinking.type !== "adaptive" ||
        Object.keys(body.thinking).some(key => !["type", "display", "block_binding"].includes(key)) ||
        (body.thinking.block_binding !== undefined && !same(body.thinking.block_binding, { prefix_mismatch_behavior: "drop_block" })) ||
        (body.thinking.display !== undefined && body.thinking.display !== "summarized")) throw failed();
    } else { throw failed(); }
  };
  try {
    verifyFiles();
    registrations.push(await context.provider.transform(editor => {
      try {
        const record = editor.get("opencode-go");
        const models = record ? [...record.models.values()] : [];
        if (models.length !== 1) throw failed();
        const model = models[0];
        verifyBase(model);
        if ((model.package !== undefined && model.package !== api.sdk.opencodeai) ||
          Object.keys(model.settings ?? {}).some(key => !["baseURL", "provider"].includes(key))) throw failed();
        const variants = efforts.map(id => {
          const matches = (model.variants ?? []).filter(variant => variant.id === id);
          if (matches.length !== 1) throw failed();
          return matches[0];
        });
        if (contract.effort ? !efforts.includes(contract.effort) : efforts.length) throw failed();
        expected = structuredClone({ ...model, package: api.sdk.opencodeai,
          settings: { ...model.settings, baseURL: api.base_url }, variants });
        editor.models.set("opencode-go", [structuredClone(expected)]);
        materialized = true;
      } catch { faulted = true; throw failed(); }
    }));
    registrations.push(await context.session.hook("model.request", async event => {
      try { await check(); verifyRef(event); } catch { faulted = true; throw failed(); }
    }));
    registrations.push(await context.session.hook("http.request", async event => {
      try { await wire(event); } catch { faulted = true; throw failed(); }
    }));
    return { check, dispose };
  } catch (error) {
    try { await dispose(); } catch { /* Attempt every registration before preserving the setup error. */ }
    throw error;
  }
}
