import { createConnection } from "node:net";

const names = ["ocr_bash", "ocr_glob", "ocr_grep", "ocr_read", "ocr_write"];
const unavailable = () => new Error("OpenCode guard infrastructure failed");

export function createHealth(context, catalogueCheck) {
  if (typeof catalogueCheck !== "function") throw unavailable();
  let state = "active";
  let socket;
  let pending;
  let running = 0;
  let session;
  let ready = false;
  const registrations = [];
  const fault = code => {
    if (state === "failed") return;
    state = "failed";
    if (socket?.writable) socket.end(JSON.stringify({ v: 1, type: "fault", code }) + "\n");
    pending?.reject(unavailable());
  };
  const enter = id => {
    if (state !== "active") { fault("work_after_seal"); throw unavailable(); }
    if (id) {
      if (session && session !== id) { fault("channel"); throw unavailable(); }
      session = id;
    }
  };
  const registry = async () => {
    await catalogueCheck();
    const tools = await context.tool.list();
    if (tools.length !== names.length || tools.some(tool => !names.includes(tool.name) || tool.options?.permission !== tool.name || tool.options?.codemode !== false) || new Set(tools.map(tool => tool.name)).size !== names.length) {
      fault("registry_invalid"); throw unavailable();
    }
  };
  const send = (type, fields) => new Promise((resolve, reject) => {
    if (!socket?.writable || pending || state === "failed") return reject(unavailable());
    const timer = setTimeout(() => { fault("channel"); reject(unavailable()); }, 5000);
    pending = {
      type, resolve: () => { clearTimeout(timer); pending = undefined; resolve(); },
      reject: error => { clearTimeout(timer); pending = undefined; reject(error); },
    };
    socket.write(JSON.stringify({ v: 1, type, ...fields }) + "\n");
  });
  const seal = async (event, probe, type, fields = {}) => {
    enter(event.sessionID);
    if (!ready) { fault("registry_invalid"); throw unavailable(); }
    if (running) { fault("registry_invalid"); throw unavailable(); }
    await registry();
    try { await probe(); } catch { fault("helper_probe"); throw unavailable(); }
    await registry();
    enter(event.sessionID);
    if (running) { fault("registry_invalid"); throw unavailable(); }
    state = "sealed";
    await send(type, { sessionID: event.sessionID, ...fields });
  };
  return {
    enter, fault,
    begin(id) { enter(id); running++; },
    end() { running--; },
    async start(probe) {
      const path = process.env.OCR_OPENCODE_GUARD_SOCKET;
      const nonce = process.env.OCR_OPENCODE_GUARD_NONCE;
      if (!path?.startsWith("/") || path.length > 200 || !/^[a-f0-9]{64}$/.test(nonce ?? "")) throw unavailable();
      socket = createConnection(path);
      let buffer = Buffer.alloc(0);
      socket.on("error", () => fault("channel"));
      socket.on("close", () => { if (pending || state === "active") fault("channel"); });
      socket.on("data", chunk => {
        buffer = Buffer.concat([buffer, chunk]);
        if (buffer.length > 1024) return fault("channel");
        const end = buffer.indexOf(10);
        if (end < 0) return;
        try {
          const frame = JSON.parse(buffer.subarray(0, end).toString("utf8"));
          buffer = buffer.subarray(end + 1);
          if (Object.keys(frame).length !== 3 || frame.v !== 1 || frame.type !== "ack" || frame.for !== pending?.type || buffer.length) throw unavailable();
          pending.resolve();
        } catch { fault("channel"); }
      });
      await new Promise((resolve, reject) => {
        const timer = setTimeout(() => { fault("channel"); reject(unavailable()); }, 3000);
        socket.once("connect", () => { clearTimeout(timer); resolve(); });
        socket.once("error", () => { clearTimeout(timer); reject(unavailable()); });
      });
      // Provider transforms finalize only after plugin setup. Attest on the first resolved
      // model request, and block HTTP until the root acknowledges the validated readiness.
      let readiness;
      const ensureReady = () => readiness ??= (async () => {
        try {
          await registry();
          try { await probe(); } catch { fault("helper_probe"); throw unavailable(); }
          await registry();
          enter();
          await send("ready", { nonce });
          ready = true;
        } catch (error) { fault("registry_invalid"); throw error; }
      })();
      registrations.push(await context.session.hook("context", event => enter(event.sessionID)));
      for (const hook of ["model.request", "http.request"]) registrations.push(await context.session.hook(hook, async event => {
        enter(event.sessionID);
        await ensureReady();
      }));
      registrations.push(await context.session.hook("http.response", async event => {
        if (![401, 429].includes(event.response.status)) return;
        if (event.kind !== "primary" || event.agent !== "ocr-review" || event.model.providerID !== "opencode-go" || !session || session !== event.sessionID) {
          fault("channel"); throw unavailable();
        }
        await seal(event, probe, "provider_failed", { code: event.response.status === 401 ? "auth" : "usage_limit", status: event.response.status });
      }));
      registrations.push(await context.session.hook("experimental.idle", async event => {
        enter(event.sessionID);
        if (event.background.length || running) { fault("registry_invalid"); throw unavailable(); }
        await seal(event, probe, "complete");
      }));
    },
    async cleanup() {
      if (state === "active") fault("adapter_unloaded");
      let failure;
      for (const registration of registrations) {
        try { await registration.dispose(); } catch (error) { failure ??= error; }
      }
      socket?.end();
      if (failure) throw failure;
    },
  };
}
