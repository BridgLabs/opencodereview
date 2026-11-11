"""Checks the newest release of each AI CLI before it is pinned in docker/Dockerfile.

  resolve          Print the Docker build arguments for the newest Claude Code, Codex and OpenCode
                   releases (linux/amd64) and write a version table to the step summary.
  flags IMAGE      Check that each CLI in IMAGE still accepts every flag its provider passes.

No step reads credentials or calls a model.
"""

import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLAUDE_RELEASES = "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"


def fetch(url, binary=False, data=None):
    request = urllib.request.Request(url, data=data, headers={"User-Agent": "opencodereview-provider-cli-canary"})
    if "api.github.com" in url and os.environ.get("GH_TOKEN"):
        request.add_header("Authorization", "Bearer " + os.environ["GH_TOKEN"])
    with urllib.request.urlopen(request, timeout=120) as response:
        body = response.read()
    return body if binary else body.decode()


def sha256(url):
    return hashlib.sha256(fetch(url, binary=True)).hexdigest()


def newest_stable(releases, prefix):
    # The highest X.Y.Z release, not the one GitHub marks latest: a project can mark an older
    # maintenance release latest after a newer major (OpenCode marked v1.19.5 after v2.2.6).
    pattern = re.compile(re.escape(prefix) + r"(\d+)\.(\d+)\.(\d+)")
    versions = [
        tuple(map(int, match.groups()))
        for release in releases
        if not release["draft"] and not release["prerelease"]
        and (match := pattern.fullmatch(release["tag_name"]))
    ]
    if not versions:
        sys.exit(f"No stable release tagged {prefix}X.Y.Z")
    return ".".join(map(str, max(versions)))


RELEASES_QUERY = """
query($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    releases(first: 100, after: $after, orderBy: {field: CREATED_AT, direction: DESC}) {
      nodes { tagName isDraft isPrerelease }
      pageInfo { hasNextPage endCursor }
    }
  }
}
"""


def latest_github(repository, prefix, pages=5):
    # GraphQL asks only for tags and flags: REST release pages carry every release body and time out.
    # Several pages, so a long prerelease series cannot push every stable release off the first.
    owner, name = repository.split("/")
    releases, after = [], None
    for _ in range(pages):
        body = json.dumps({"query": RELEASES_QUERY, "variables": {"owner": owner, "name": name, "after": after}})
        page = json.loads(fetch("https://api.github.com/graphql", data=body.encode()))["data"]["repository"]["releases"]
        releases += [
            {"tag_name": node["tagName"], "draft": node["isDraft"], "prerelease": node["isPrerelease"]}
            for node in page["nodes"]
        ]
        if not page["pageInfo"]["hasNextPage"]:
            break
        after = page["pageInfo"]["endCursor"]
    return newest_stable(releases, prefix)


def pinned():
    dockerfile = (ROOT / "docker/Dockerfile").read_text()
    return dict(re.findall(r"^ARG (\w+_CLI_VERSION)=(\S+)$", dockerfile, re.M))


def resolve():
    claude = fetch(f"{CLAUDE_RELEASES}/latest").strip()
    codex = latest_github("openai/codex", "rust-v")
    opencode = latest_github("anomalyco/opencode", "v")
    manifest = json.loads(fetch(f"{CLAUDE_RELEASES}/{claude}/manifest.json"))
    args = {
        "CLAUDE_CLI_VERSION": claude,
        "CLAUDE_CLI_SHA256_AMD64": manifest["platforms"]["linux-x64"]["checksum"],
        "CODEX_CLI_VERSION": codex,
        "CODEX_CLI_SHA256_AMD64": sha256(f"https://github.com/openai/codex/releases/download/rust-v{codex}/codex-x86_64-unknown-linux-musl.tar.gz"),
        "CODEX_CODE_MODE_HOST_SHA256_AMD64": sha256(f"https://github.com/openai/codex/releases/download/rust-v{codex}/codex-code-mode-host-x86_64-unknown-linux-musl.tar.gz"),
        "OPENCODE_CLI_VERSION": opencode,
        "OPENCODE_CLI_SHA256_AMD64": sha256(f"https://github.com/anomalyco/opencode/releases/download/v{opencode}/opencode-linux-x64.tar.gz"),
    }
    current = pinned()
    rows = ["| CLI | Pinned | Newest |", "| --- | --- | --- |"]
    for name in ["CLAUDE", "CODEX", "OPENCODE"]:
        key = name + "_CLI_VERSION"
        mark = "" if current.get(key) == args[key] else " (new)"
        rows.append(f"| {name.title()} | {current.get(key, '?')} | {args[key]}{mark} |")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as file:
            file.write("## Provider CLI releases\n\n" + "\n".join(rows) + "\n")
    print("\n".join(rows), file=sys.stderr)
    for key, value in args.items():
        print(f"{key}={value}")


def provider_flags(name):
    source = (ROOT / f"backend/src/providers/{name}.rs").read_text().split("#[cfg(test)]")[0]
    return sorted(set(re.findall(r'"(--[a-z][a-z-]*)"', source)))


def run(image, *command, entrypoint=None, env=(), stdin=None, timeout=60):
    args = ["docker", "run", "--rm", "-i", "-e", "HOME=/tmp/home", *[part for pair in env for part in ("-e", pair)]]
    if entrypoint:
        args += ["--entrypoint", entrypoint]
    result = subprocess.run([*args, image, *command], input=stdin, text=True, capture_output=True, timeout=timeout)
    return result.returncode, result.stdout + result.stderr


def flags(image):
    # The help screen of the subcommand each provider runs.
    help_commands = {
        "claude": ["claude", "--help"],
        "codex": ["codex", "exec", "--help"],
        "opencode": ["opencode", "run", "--help"],
    }
    failures = []
    for name, command in help_commands.items():
        _, output = run(image, "sh", "-c", 'mkdir -p "$HOME" && exec "$@"', "sh", *command, entrypoint="")
        expected = provider_flags(name)
        missing = [flag for flag in expected if not re.search(re.escape(flag) + r"(?![a-z-])", output)]
        print(f"{'FAIL' if missing else 'PASS'} {name}: {len(expected) - len(missing)}/{len(expected)} flags in {' '.join(command)}")
        if missing:
            failures.append(f"{name} no longer lists {', '.join(missing)}")
    if failures:
        sys.exit("\n".join(failures))


if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else ""
    if command == "resolve":
        resolve()
    elif command == "flags" and len(sys.argv) == 3:
        globals()[command](sys.argv[2])
    else:
        sys.exit(__doc__)
