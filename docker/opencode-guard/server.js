import { spawn } from "node:child_process";
import { createHealth } from "./health.js";
import { catalogueGuard } from "./catalogue.js";

const denied = () => new Error("OpenCode tool request denied");
const MAX_INPUT = 512 * 1024;
const MAX_OUTPUT = 300 * 1024;

function invoke(tool, input, context, health) {
  health.enter(context.sessionID);
  const search = tool === "ocr_grep" || tool === "ocr_glob";
  const bash = tool === "ocr_bash";
  const write = tool === "ocr_write";
  const keys = bash ? ["command", "description", "timeout", "run_in_background", "dangerouslyDisableSandbox"] : search ? ["path", "pattern"] : write ? ["path", "content"] : ["path", "offset", "limit"];
  if (
    !input || typeof input !== "object" || Array.isArray(input) ||
    Object.keys(input).some(key => !keys.includes(key)) ||
    (!bash && typeof input.path !== "string") ||
    (bash && typeof input.command !== "string") ||
    (write && (typeof input.content !== "string" || Buffer.byteLength(input.content) > 64 * 1024)) ||
    (search && (typeof input.pattern !== "string" || !input.pattern || Buffer.byteLength(input.pattern) > 256)) ||
    (input.offset !== undefined && (!Number.isInteger(input.offset) || input.offset < 1 || input.offset > 1000000)) ||
    (input.limit !== undefined && (!Number.isInteger(input.limit) || input.limit < 1 || input.limit > 200))
  ) throw denied();
  const request = JSON.stringify({ tool, input });
  if (Buffer.byteLength(request) > MAX_INPUT || context.signal.aborted) throw denied();
  return new Promise((resolve, reject) => {
    health.begin(context.sessionID);
    let child;
    try {
      child = spawn("/usr/local/bin/ocr-sandbox", ["opencode-tool"], {
        env: {}, cwd: "/output", shell: false, stdio: ["pipe", "pipe", "pipe"],
      });
    } catch { health.end(); health.fault("helper_spawn"); reject(denied()); return; }
    const chunks = [];
    let output = 0;
    let errors = 0;
    let failure;
    const fail = code => { failure ??= code; health.fault(code); child.kill("SIGKILL"); };
    const abort = () => fail("helper_signal");
    const timer = setTimeout(() => fail("helper_timeout"), 3000);
    context.signal.addEventListener("abort", abort, { once: true });
    child.stdout.on("data", chunk => {
      output += chunk.length;
      if (output > MAX_OUTPUT) fail("helper_output_limit");
      else chunks.push(chunk);
    });
    child.stderr.on("data", chunk => { errors += chunk.length; if (errors > 4096) fail("helper_output_limit"); });
    child.stdin.on("error", () => fail("helper_exit"));
    child.on("error", () => fail("helper_spawn"));
    child.on("close", code => {
      health.end();
      clearTimeout(timer);
      context.signal.removeEventListener("abort", abort);
      if (failure) return reject(denied());
      if (code === null) { health.fault("helper_signal"); return reject(denied()); }
      try {
        const value = JSON.parse(Buffer.concat(chunks).toString("utf8"));
        if (value.v !== 1) throw denied();
        if (code === 2 && value.status === "rejected" && value.code === "request" && Object.keys(value).length === 3) return reject(denied());
        if (code !== 0) { health.fault(code === null ? "helper_signal" : "helper_exit"); return reject(denied()); }
        if (Object.keys(value).length !== 3 || value.status !== "ok" || typeof value.content !== "string") throw denied();
        resolve({ content: value.content });
      } catch { health.fault("helper_protocol"); reject(denied()); }
    });
    child.stdin.end(request);
  });
}

export default {
  id: "ocr.guard",
  async setup(context) {
    const catalogue = await catalogueGuard(context);
    let health;
    let registration;
    const cleanup = async () => {
      try { await health?.cleanup(); }
      finally {
        try { await registration?.dispose(); }
        finally { await catalogue.dispose(); }
      }
    };
    try {
      health = createHealth(context, catalogue.check);
      registration = await context.tool.transform(editor => {
        editor.add({
          name: "ocr_read",
          description: "Read a UTF-8 frozen file under /work, /evidence/snapshot, or /output. Absolute path required. At most 200 lines and 256 KiB per file. Symlinks are forbidden.",
          input: {
            type: "object", additionalProperties: false, required: ["path"],
            properties: {
              path: { type: "string", maxLength: 4096 },
              offset: { type: "integer", minimum: 1, maximum: 1000000 },
              limit: { type: "integer", minimum: 1, maximum: 200 },
            },
          },
          options: { permission: "ocr_read", codemode: false },
          execute: (input, call) => invoke("ocr_read", input, call, health),
        });
        editor.add({
          name: "ocr_grep",
          description: "Recursively search frozen UTF-8 files for literal text. Absolute path required. Sorted output; at most 80 matching lines, 4096 entries, depth 32 and 4 MiB scanned. Symlinks and special entries are skipped.",
          input: {
            type: "object", additionalProperties: false, required: ["path", "pattern"],
            properties: { path: { type: "string", maxLength: 4096 }, pattern: { type: "string", minLength: 1, maxLength: 256 } },
          },
          options: { permission: "ocr_grep", codemode: false },
          execute: (input, call) => invoke("ocr_grep", input, call, health),
        });
        editor.add({
          name: "ocr_glob",
          description: "List sorted regular frozen file paths matching a relative glob. Supports *, ?, [], and ** as a complete directory segment. Absolute root required. At most 4096 entries and depth 32. Symlinks and special entries are skipped.",
          input: {
            type: "object", additionalProperties: false, required: ["path", "pattern"],
            properties: { path: { type: "string", maxLength: 4096 }, pattern: { type: "string", minLength: 1, maxLength: 256 } },
          },
          options: { permission: "ocr_glob", codemode: false },
          execute: (input, call) => invoke("ocr_glob", input, call, health),
        });
        editor.add({
          name: "ocr_bash",
          description: "Interpret only validated snapshot commands: /usr/bin/sed -n '1,120p' -- '/work/file'; /usr/bin/grep -r -n -F -- 'literal' '/work' | /usr/bin/head -n 80; /usr/bin/ls -1A -- '/work'. No shell, network, cwd/workdir overrides or background jobs. Fixed directory /output. Execution deadline 3 seconds.",
          input: {
            type: "object", additionalProperties: false, required: ["command"],
            properties: {
              command: { type: "string", maxLength: 8192 }, description: {},
              timeout: { anyOf: [{ type: "integer", minimum: 1, maximum: 30000 }, { type: "null" }] },
              run_in_background: { anyOf: [{ const: false }, { type: "null" }] },
              dangerouslyDisableSandbox: { anyOf: [{ const: false }, { type: "null" }] },
            },
          },
          options: { permission: "ocr_bash", codemode: false },
          execute: (input, call) => invoke("ocr_bash", input, call, health),
        });
        editor.add({
          name: "ocr_write",
          description: "Atomically replace or create only /output/report.json or /output/finding_decisions.json with at most 64 KiB UTF-8 content. Symlinks, hardlinks and special targets are denied. No arbitrary paths.",
          input: {
            type: "object", additionalProperties: false, required: ["path", "content"],
            properties: {
              path: { type: "string", enum: ["/output/report.json", "/output/finding_decisions.json"] },
              content: { type: "string", maxLength: 65536 },
            },
          },
          options: { permission: "ocr_write", codemode: false },
          execute: (input, call) => invoke("ocr_write", input, call, health),
        });
      });
      await health.start(() => invoke("ocr_read", { path: "/work/context/PR.md", limit: 1 }, { signal: new AbortController().signal }, health));
      return cleanup;
    } catch (error) {
      health?.fault("helper_probe");
      try { await cleanup(); } catch { /* All resources are attempted; preserve the setup failure. */ }
      throw error;
    }
  },
};
