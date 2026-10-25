import { type ChildProcess, spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

type Event = { kind: string; args?: string[]; pid?: number };
const script = resolve("scripts/dev-web-docker.mjs");
const fixtures: Array<{ directory: string; child: ChildProcess; done: Promise<unknown>; events: () => Event[] }> = [];
const prefix = `#!/usr/bin/env node
const fs = require("node:fs");
const record = (event) => fs.appendFileSync(process.env.OCR_DEBUG_TEST_LOG, JSON.stringify(event) + "\\n");
`;

function start(options: Record<string, string> = {}) {
  const directory = mkdtempSync(join(tmpdir(), "ocr-debug-script-"));
  const log = join(directory, "events.jsonl");
  writeFileSync(
    join(directory, "docker"),
    prefix +
      `
const args = process.argv.slice(2);
record({ kind: "docker", args });
const command = args.join(" ");
if (command === "compose ps --status running -q web" && process.env.TEST_WEB_RUNNING !== "0") console.log("test-web");
if (command === "compose up -d server" && process.env.TEST_UP_FAIL === "1") { console.error("Backend unavailable"); process.exit(1); }
if (command === "compose exec -T server cat /data/api-token" && process.env.TEST_TOKEN_READY === "1") console.log("a".repeat(64));
`,
    { mode: 0o755 },
  );
  writeFileSync(
    join(directory, "pnpm"),
    prefix +
      `
record({ kind: "vite-ready", args: process.argv.slice(2), pid: process.pid });
process.once("exit", () => record({ kind: "vite-exit", pid: process.pid }));
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => { record({ kind: signal }); process.exit(0); });
if (process.env.TEST_VITE_EXIT !== undefined) setTimeout(() => process.exit(Number(process.env.TEST_VITE_EXIT)), 30);
else setInterval(() => {}, 1000);
`,
    { mode: 0o755 },
  );
  const child = spawn(process.execPath, [script], {
    env: {
      ...process.env,
      ...options,
      PATH: `${directory}:${dirname(process.execPath)}:${process.env.PATH ?? ""}`,
      OCR_DEBUG_TEST_LOG: log,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout!.on("data", (chunk) => {
    output += chunk;
  });
  child.stderr!.on("data", (chunk) => {
    output += chunk;
  });
  const done = new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (code, signal) => resolve({ code, signal }));
  });
  const events = (): Event[] =>
    existsSync(log)
      ? readFileSync(log, "utf8")
          .trim()
          .split("\n")
          .filter(Boolean)
          .map((line) => JSON.parse(line))
      : [];
  const commands = () =>
    events()
      .filter((event) => event.kind === "docker")
      .map((event) => event.args!.join(" "));
  fixtures.push({ directory, child, done, events });
  return { child, done, events, commands, output: () => output };
}

async function waitForEvent(run: ReturnType<typeof start>, predicate: (event: Event) => boolean) {
  await vi.waitFor(() => expect(run.events().some(predicate)).toBe(true), { timeout: 5000 });
}

async function waitForExit(run: ReturnType<typeof start>) {
  await vi.waitFor(() => expect(run.child.exitCode !== null || run.child.signalCode !== null).toBe(true), {
    timeout: 5000,
  });
  return run.done;
}

afterEach(async () => {
  for (const fixture of fixtures.splice(0)) {
    const events = fixture.events();
    for (const event of events.filter((event) => event.kind === "vite-ready")) {
      if (!events.some((exit) => exit.kind === "vite-exit" && exit.pid === event.pid)) {
        try {
          process.kill(event.pid!, "SIGKILL");
        } catch {}
      }
    }
    if (fixture.child.exitCode === null && fixture.child.signalCode === null) fixture.child.kill("SIGKILL");
    await fixture.done.catch(() => undefined);
    rmSync(fixture.directory, { recursive: true, force: true });
  }
});

describe.skipIf(process.platform === "win32")("Docker UI debug lifecycle with fake commands", () => {
  it.each(["SIGINT", "SIGTERM"] as const)("restores web when %s interrupts the token wait", async (signal) => {
    const run = start();
    await waitForEvent(run, (event) => event.args?.includes("/data/api-token") ?? false);
    run.child.kill(signal);
    const exit = await waitForExit(run);
    expect(run.commands().filter((command) => command === "compose start web")).toHaveLength(1);
    expect(run.events().some((event) => event.kind === "vite-ready")).toBe(false);
    expect(exit).toEqual({ code: signal === "SIGINT" ? 130 : 143, signal: null });
  });

  it("does not start web if it was already stopped", async () => {
    const run = start({ TEST_WEB_RUNNING: "0" });
    await waitForEvent(run, (event) => event.args?.includes("/data/api-token") ?? false);
    run.child.kill("SIGTERM");
    await waitForExit(run);
    expect(run.commands()).not.toContain("compose stop web");
    expect(run.commands()).not.toContain("compose start web");
  });

  it("restores web when starting the backend fails", async () => {
    const run = start({ TEST_UP_FAIL: "1" });
    expect((await waitForExit(run)).code).toBe(1);
    expect(run.commands().filter((command) => command === "compose start web")).toHaveLength(1);
    expect(run.events().some((event) => event.kind === "vite-ready")).toBe(false);
  });

  it.each(["SIGINT", "SIGTERM"] as const)("forwards %s to Vite and restores web after child exit", async (signal) => {
    const run = start({ TEST_TOKEN_READY: "1" });
    await waitForEvent(run, (event) => event.kind === "vite-ready");
    run.child.kill(signal);
    await waitForExit(run);
    expect(run.events().some((event) => event.kind === signal)).toBe(true);
    expect(run.commands().filter((command) => command === "compose start web")).toHaveLength(1);
    expect(run.events().findIndex((event) => event.args?.join(" ") === "compose start web")).toBeGreaterThan(
      run.events().findIndex((event) => event.kind === "vite-exit"),
    );
    expect(run.output()).not.toContain("a".repeat(64));
  });

  it("restores web after a normal Vite exit and preserves its status", async () => {
    const run = start({ TEST_TOKEN_READY: "1", TEST_VITE_EXIT: "7" });
    expect((await waitForExit(run)).code).toBe(7);
    expect(run.commands().filter((command) => command === "compose start web")).toHaveLength(1);
    expect(run.output()).not.toContain("a".repeat(64));
  });
});
