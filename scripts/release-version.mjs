// The next release version, from the Conventional Commits merged since the last `v*` tag.
//
//   node scripts/release-version.mjs         Print the next version, or nothing when no commit
//                                            since the last tag changes what users run.
//   node scripts/release-version.mjs notes   Also write changelog.d/release.md from those commits
//                                            when no pull request left release notes and
//                                            CHANGELOG.md has none under Unreleased.
//
// feat → minor, fix/perf/revert → patch, `!` or BREAKING CHANGE → major (minor before 1.0). Other
// types (build, chore, ci, docs, refactor, style, test) release nothing on their own, and neither
// do the `release:` commits this workflow makes. The first release is 0.1.0. RELEASE_BUMP=patch,
// minor or major raises the bump to at least that level, even with nothing releasable.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";

const git = (...args) => spawnSync("git", args, { encoding: "utf8" }).stdout.trim();

const last = git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*.[0-9]*.[0-9]*");
const range = last ? `${last}..HEAD` : "HEAD";
const commits = git("log", range, "--format=%s%x1f%b%x1e")
  .split("\x1e")
  .map((entry) => entry.trim())
  .filter(Boolean)
  .map((entry) => {
    const [subject, body = ""] = entry.split("\x1f");
    const match = subject.match(/^(\w+)(\([^)]*\))?(!)?: (.+)$/);
    return {
      type: match?.[1] ?? "",
      breaking: Boolean(match?.[3]) || /^\s*BREAKING[ -]CHANGE:/m.test(body),
      description: (match?.[4] ?? subject).replace(/ \(#\d+\)$/, ""),
    };
  })
  .filter((commit) => commit.type !== "release");

const fragments = existsSync("changelog.d")
  ? readdirSync("changelog.d").filter((name) => name.endsWith(".md") && name !== "README.md")
  : [];
const rank = (commit) =>
  commit.breaking ? 3 : commit.type === "feat" ? 2 : ["fix", "perf", "revert"].includes(commit.type) ? 1 : 0;
// Release notes written by a pull request mean users see a change, whatever the commit type.
const forced = { patch: 1, minor: 2, major: 3 }[process.env.RELEASE_BUMP] ?? 0;
const bump = Math.max(forced, fragments.length ? 1 : 0, ...commits.map(rank));
if (!bump) process.exit(0);

let version = "0.1.0";
if (last) {
  let [major, minor, patch] = last.slice(1).split(".").map(Number);
  // Before 1.0 a breaking change moves the minor version, as the project is not stable yet.
  const level = major === 0 && bump === 3 && forced < 3 ? 2 : bump;
  if (level === 3) [major, minor, patch] = [major + 1, 0, 0];
  else if (level === 2) [minor, patch] = [minor + 1, 0];
  else patch += 1;
  version = `${major}.${minor}.${patch}`;
}

const unreleased = existsSync("CHANGELOG.md")
  ? (readFileSync("CHANGELOG.md", "utf8").match(/## \[Unreleased\]\n([\s\S]*?)(?=\n## \[|\n\[Unreleased\]:|$)/)?.[1] ??
    "")
  : "";
const written = fragments.length > 0 || /^- /m.test(unreleased);
if (process.argv[2] === "notes" && !written) {
  const groups = { Added: [], Changed: [], Fixed: [] };
  for (const commit of commits) {
    if (commit.type === "feat") groups.Added.push(commit.description);
    else if (commit.type === "fix" || commit.type === "revert") groups.Fixed.push(commit.description);
    else if (commit.type === "perf" || commit.breaking) groups.Changed.push(commit.description);
  }
  const text = Object.entries(groups)
    .filter(([, items]) => items.length)
    .map(
      ([group, items]) =>
        `### ${group}\n\n${items.map((item) => `- ${item[0].toUpperCase()}${item.slice(1)}.`).join("\n")}\n`,
    )
    .join("\n");
  // A bump forced by hand can have no feat, fix or perf commit to describe.
  writeFileSync("changelog.d/release.md", text || "### Changed\n\n- Maintenance release.\n");
}
console.log(version);
