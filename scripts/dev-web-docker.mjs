import { spawn, spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sleep = (milliseconds) => new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds));

function compose(args, { capture = false } = {}) {
  const result = spawnSync("docker", ["compose", ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });
  if (result.status !== 0) {
    const detail = capture ? result.stderr?.trim() : "";
    throw new Error(detail || `docker compose ${args.join(" ")} failed`);
  }
  return capture ? result.stdout.trim() : "";
}

async function readApiToken() {
  for (let attempt = 0; attempt < 30; attempt += 1) {
    try {
      const token = compose(["exec", "-T", "server", "cat", "/data/api-token"], { capture: true });
      if (/^[a-f0-9]{64}$/i.test(token)) return token;
    } catch {}
    await sleep(500);
  }
  throw new Error("Could not read the Docker backend API token.");
}

async function main() {
  const webWasRunning = Boolean(compose(["ps", "--status", "running", "-q", "web"], { capture: true }));
  let vite;
  let restored = false;
  const restoreWeb = () => {
    if (!webWasRunning || restored) return;
    restored = true;
    try {
      compose(["start", "web"]);
    } catch (error) {
      console.error(error instanceof Error ? error.message : "Could not restore the Docker web service.");
    }
  };
  const finish = (code) => {
    restoreWeb();
    process.exit(code);
  };
  const shutdown = (signal) => {
    if (!vite) finish(signal === "SIGINT" ? 130 : 143);
    else if (!vite.killed) vite.kill(signal);
  };
  process.on("SIGINT", () => shutdown("SIGINT"));
  process.on("SIGTERM", () => shutdown("SIGTERM"));

  try {
    if (webWasRunning) compose(["stop", "web"]);
    compose(["up", "-d", "server"]);
    const token = await readApiToken();
    vite = spawn("pnpm", ["dev:web"], {
      cwd: repoRoot,
      env: { ...process.env, VITE_OCR_API_TOKEN: token },
      stdio: "inherit",
    });
    vite.once("error", (error) => {
      console.error(error.message);
      finish(1);
    });
    vite.once("exit", (code, signal) => finish(code ?? (signal ? 1 : 0)));
  } catch (error) {
    restoreWeb();
    throw error;
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : "Could not start the UI debug server.");
  process.exit(1);
});
