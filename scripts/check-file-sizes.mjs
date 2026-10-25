import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname } from "node:path";

const limits = [
  { name: "Rust source", roots: ["backend/src/"], extensions: new Set([".rs"]), max: 2500 },
  { name: "CSS source", roots: ["src/"], extensions: new Set([".css"]), max: 3500 },
  { name: "Frontend source", roots: ["src/"], extensions: new Set([".js", ".jsx", ".mjs", ".ts", ".tsx"]), max: 800 },
  { name: "Check scripts", roots: ["scripts/"], extensions: new Set([".js", ".mjs", ".ts"]), max: 300 },
];

const result = spawnSync("git", ["ls-files", "-z", "--", "backend/src", "src", "scripts"], { encoding: "utf8" });
if (result.status !== 0) {
  process.stderr.write(result.stderr || "Could not list tracked source files.\n");
  process.exit(result.status || 1);
}

const violations = [];
let checked = 0;
for (const file of result.stdout.split("\0").filter(Boolean)) {
  const extension = extname(file);
  const rule = limits.find(
    ({ roots, extensions }) => roots.some((root) => file.startsWith(root)) && extensions.has(extension),
  );
  // Tracked files deleted in the working tree have nothing to measure.
  if (!rule || !existsSync(file)) continue;
  checked += 1;
  const source = readFileSync(file, "utf8");
  const lines = source.length === 0 ? 0 : source.split(/\r?\n/).length - (source.endsWith("\n") ? 1 : 0);
  if (lines > rule.max) violations.push({ file, lines, max: rule.max, name: rule.name });
}

if (violations.length > 0) {
  for (const violation of violations.sort((a, b) => b.lines - a.lines || a.file.localeCompare(b.file))) {
    console.error(`${violation.file}: ${violation.lines} lines exceeds ${violation.name} limit of ${violation.max}`);
  }
  process.exit(1);
}

console.log(`File-size check passed: ${checked} tracked source files checked.`);
