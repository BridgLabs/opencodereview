// Release notes kept as one file per pull request in changelog.d/, so pull requests never edit the
// same lines of CHANGELOG.md.
//
//   node scripts/changelog.mjs check [files...]   Validate fragments (all of them without files).
//   node scripts/changelog.mjs release <version>  Move every fragment and the Unreleased items into
//                                                a new version section, then delete the fragments.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";

const DIRECTORY = "changelog.d";
const CHANGELOG = "CHANGELOG.md";
const REPOSITORY = "https://github.com/BridgLabs/opencodereview";
const GROUPS = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"];

function fragments() {
  if (!existsSync(DIRECTORY)) return [];
  return readdirSync(DIRECTORY)
    .filter((name) => name.endsWith(".md") && name !== "README.md")
    .map((name) => join(DIRECTORY, name));
}

// Returns { groups: Map<group, items[]>, errors: string[] }. An item keeps its continuation lines.
function parse(text, source) {
  const groups = new Map();
  const errors = [];
  let group = null;
  let item = null;
  text.split(/\r?\n/).forEach((line, index) => {
    const where = `${source}:${index + 1}`;
    const heading = line.match(/^### (.+)$/);
    if (heading) {
      group = heading[1].trim();
      item = null;
      if (!GROUPS.includes(group)) errors.push(`${where}: use one of ${GROUPS.join(", ")}, not "${group}"`);
      if (!groups.has(group)) groups.set(group, []);
    } else if (line.startsWith("- ")) {
      if (!group) errors.push(`${where}: put the item under a ### group heading`);
      else if (line.trim() === "-") errors.push(`${where}: the item is empty`);
      else {
        item = [line];
        groups.get(group).push(item);
      }
    } else if (/^ {2}\S/.test(line) && item) {
      item.push(line);
    } else if (line.trim() !== "") {
      errors.push(`${where}: only ### group headings and "- " items belong in release notes`);
      item = null;
    } else {
      item = null;
    }
  });
  if (errors.length === 0 && ![...groups.values()].some((items) => items.length > 0)) {
    errors.push(`${source}: write at least one "- " item`);
  }
  return { groups, errors };
}

function check(files) {
  const errors = (files.length ? files : fragments())
    .filter((file) => basename(file) !== "README.md")
    .flatMap((file) => parse(readFileSync(file, "utf8"), file).errors);
  if (errors.length) {
    process.stderr.write(`${errors.join("\n")}\nSee ${DIRECTORY}/README.md.\n`);
    process.exit(1);
  }
  console.log("Release notes are valid.");
}

// The time each fragment was first committed, so items keep the order their pull requests merged.
function addedAt(file) {
  const log = spawnSync("git", ["log", "--diff-filter=A", "--format=%ct", "--", file], { encoding: "utf8" });
  const times = log.stdout.trim().split("\n").filter(Boolean);
  return times.length ? Number(times.at(-1)) : Number.MAX_SAFE_INTEGER;
}

function release(version) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
    process.stderr.write("Give the new version, for example: node scripts/changelog.mjs release 0.2.0\n");
    process.exit(1);
  }
  const changelog = readFileSync(CHANGELOG, "utf8");
  if (changelog.includes(`## [${version}]`)) {
    process.stderr.write(`${CHANGELOG} already has a ${version} section.\n`);
    process.exit(1);
  }
  const unreleased = changelog.match(/## \[Unreleased\]\n([\s\S]*?)(?=\n## \[|\n\[Unreleased\]:|$)/);
  if (!unreleased) {
    process.stderr.write(`${CHANGELOG} has no "## [Unreleased]" section.\n`);
    process.exit(1);
  }
  const files = fragments().sort((a, b) => addedAt(a) - addedAt(b) || a.localeCompare(b));
  const sources = [{ text: unreleased[1], source: `${CHANGELOG} Unreleased` }].concat(
    files.map((file) => ({ text: readFileSync(file, "utf8"), source: file })),
  );
  const merged = new Map(GROUPS.map((group) => [group, []]));
  for (const { text, source } of sources) {
    const { groups, errors } = parse(text, source);
    if (errors.length && source !== `${CHANGELOG} Unreleased`) {
      process.stderr.write(`${errors.join("\n")}\n`);
      process.exit(1);
    }
    for (const [group, items] of groups) merged.get(group)?.push(...items);
  }
  const section = (group) => {
    const items = merged.get(group).map((item) => item.join("\n"));
    return `### ${group}\n\n${items.join("\n")}\n`;
  };
  const body = GROUPS.filter((group) => merged.get(group).length)
    .map(section)
    .join("\n");
  if (!body) {
    process.stderr.write("There are no release notes to release.\n");
    process.exit(1);
  }
  const date = new Date().toISOString().slice(0, 10);
  const previous = changelog.match(/^## \[(\d[^\]]*)\]/m)?.[1];
  let next = changelog.replace(unreleased[0], `## [Unreleased]\n\n## [${version}] - ${date}\n\n${body}`);
  next = next.replace(/^\[Unreleased\]:.*$/m, `[Unreleased]: ${REPOSITORY}/compare/v${version}...HEAD`);
  const link = previous
    ? `[${version}]: ${REPOSITORY}/compare/v${previous}...v${version}`
    : `[${version}]: ${REPOSITORY}/releases/tag/v${version}`;
  next = next.replace(/^(\[Unreleased\]:.*)$/m, `$1\n${link}`);
  writeFileSync(CHANGELOG, next);
  for (const file of files) rmSync(file);
  console.log(`Moved ${files.length} release note file(s) into ${CHANGELOG} ${version}.`);
}

const [command, ...args] = process.argv.slice(2);
if (command === "check") check(args);
else if (command === "release") release(args[0]);
else {
  process.stderr.write("Usage: node scripts/changelog.mjs check [files...] | release <version>\n");
  process.exit(1);
}
