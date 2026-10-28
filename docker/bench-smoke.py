"""Smoke test of the benchmark image (docker/Dockerfile.bench) with a fake Claude CLI: no model call.

Builds a two-commit fixture repository, mounts it the ReviewBench way and checks that the image
reviews it end to end: the throwaway Postgres, the runner, the offline pipeline and the output
contract. The fake CLI reports one finding on the changed line.

    python3 docker/bench-smoke.py            # uses opencodereview-bench:local
    OCR_BENCH_IMAGE=... python3 docker/bench-smoke.py
    OCR_BENCH_RUNNERS=3 python3 docker/bench-smoke.py   # runners in containers of their own
"""

import json
import os
import pathlib
import platform
import subprocess
import sys
import tempfile
import uuid

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "bench" / "adapters"))
import common  # noqa: E402  - the adapters' runner pool, so OCR_BENCH_RUNNERS=N smokes it too

IMAGE = os.environ.get("OCR_BENCH_IMAGE", "opencodereview-bench:local")
FAKE_IMAGE = "ocr-bench-smoke-" + uuid.uuid4().hex[:10]

# Answers every stage with one finding on app.py's changed line, quoting the frozen head.
FAKE = r'''#!/bin/sh
set -eu
for arg in "$@"; do
  if [ "$arg" = --version ]; then printf '0.0.1 (Claude Code)\n'; exit 0; fi
done
model=claude-opus-5-5
while [ "$#" -gt 0 ]; do
  case "$1" in --model) model="$2"; shift ;; esac
  shift
done
cat > /dev/null
test "$(id -u)" = 10001
test ! -r /ocr
MOUNT_ISOLATION
perl -MJSON::PP -e '
  local $/;
  open my $f, "<", "/work/context/FROZEN.json" or die; my $id = decode_json(<$f>)->{id};
  open my $m, "<", "/evidence/snapshot/manifest.json" or die; my $head = decode_json(<$m>)->{scope}{headSha};
  open my $s, "<", "/work/context/STAGE.json" or die; my $files = decode_json(<$s>)->{files};
  my @findings;
  if (grep { $_ eq "app.py" } @$files) {
    push @findings, {
      kind => "finding", title => "add subtracts its arguments",
      detail => "add returns a - b, so every caller gets the difference instead of the sum.",
      action => "Return a + b.", path => "app.py", line => 2, side => "RIGHT",
      evidence => [{ path => "app.py", revision => $head, line => 2, quote => "    return a - b" }],
      counterEvidence => [{ path => "app.py", revision => $head, line => 1, quote => "def add(a, b):" }],
    };
  }
  my $report = { contractVersion => 2, snapshotId => $id, reviewedFiles => $files, missingEvidence => [], findings => \@findings };
  my $decisions = { contractVersion => 2, snapshotId => $id, decisions => [] };
  if (-e "/work/integration/chunk_findings.json") {
    open my $c, "<", "/work/integration/chunk_findings.json" or die;
    my $chunk = decode_json(<$c>);
    my $index = 0;
    for my $candidate (@{ $chunk->{comments} || [] }) {
      push @{ $decisions->{decisions} }, { candidateId => $candidate->{candidateId}, decision => "keep", commentIndex => $index, reason => "Verified finding" };
      $index++;
    }
  }
  my $answer = encode_json({ report => $report, findingDecisions => $decisions });
  print encode_json({ type => "result", subtype => "success", is_error => JSON::PP::false, result => $answer,
    modelUsage => { $ARGV[0] => { inputTokens => 10, outputTokens => 5 } } }) . "\n";
' "$model"
'''


def run(*args, cwd=None, check=True, timeout=600, env=None):
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, timeout=timeout, env=env)
    if check and result.returncode:
        raise RuntimeError(f"{' '.join(args[:3])} failed ({result.returncode}): {result.stderr.strip()[-4000:]}")
    return result


def git(repo, *args):
    return run("git", "-C", str(repo), *args).stdout.strip()


def fixture(root):
    repo = root / "repo"
    repo.mkdir()
    git(repo, "init", "-q", "-b", "main")
    git(repo, "config", "user.email", "smoke@example.invalid")
    git(repo, "config", "user.name", "Smoke")
    (repo / "app.py").write_text("def add(a, b):\n    return a + b\n")
    (repo / "README.md").write_text("Adds numbers.\n")
    git(repo, "add", ".")
    git(repo, "commit", "-q", "-m", "base")
    base = git(repo, "rev-parse", "HEAD")
    (repo / "app.py").write_text("def add(a, b):\n    return a - b\n")
    git(repo, "commit", "-q", "-am", "Simplify add")
    head = git(repo, "rev-parse", "HEAD")
    pr = root / "pr"
    pr.mkdir()
    (pr / "diff.patch").write_text(git(repo, "diff", f"{base}...{head}") + "\n")
    (pr / "pr.json").write_text(json.dumps({
        "repo": "https://github.com/smoke/fixture", "pr_number": 7, "base": base, "head": head,
        "nwo": "smoke/fixture", "title": "Simplify add", "body": "Small cleanup.",
    }))
    out = root / "out"
    out.mkdir()
    for path in (repo, pr, out):
        path.chmod(0o777)
    return repo, pr, out, base, head


def main():
    with tempfile.TemporaryDirectory() as build:
        build = pathlib.Path(build)
        # Docker Desktop's macOS file sharing ignores the mode the entry point sets on the mount.
        isolation = "test ! -r /work/repo/app.py" if platform.system() == "Linux" else ""
        (build / "claude").write_text(FAKE.replace("MOUNT_ISOLATION", isolation))
        (build / "Dockerfile").write_text(
            f"FROM {IMAGE}\nCOPY --chmod=0755 claude /opt/claude/bin/claude\n"
        )
        run("docker", "build", "-q", "-t", FAKE_IMAGE, str(build))
    try:
        with tempfile.TemporaryDirectory() as root:
            repo, pr, out, base, head = fixture(pathlib.Path(root))
            with common.runner_pool(FAKE_IMAGE, common.RUNNERS) as pool:
                result = run(
                    "docker", "run", "--rm", *pool,
                    "-v", f"{repo}:/work/repo", "-v", f"{pr}:/work/pr:ro", "-v", f"{out}:/work/out",
                    "-e", "RB_NWO=smoke/fixture", "-e", "RB_PR_NUMBER=7",
                    "-e", f"RB_BASE={base}", "-e", f"RB_HEAD={head}",
                    "-e", "RB_AGENT=opencodereview", "-e", "RB_DIFF=/work/pr/diff.patch",
                    "-e", "RB_PR_JSON=/work/pr/pr.json", "-e", "RB_OUT=/work/out/findings.json",
                    "-e", "RB_CONFIG_PROVIDER=claude", "-e", "CLAUDE_CODE_OAUTH_TOKEN=fixture-not-a-real-credential",
                    FAKE_IMAGE,
                    check=False, timeout=900,
                )
            print(result.stdout)
            print(result.stderr[-6000:], file=sys.stderr)
            assert result.returncode == 0, f"exit {result.returncode}"
            assert "provider=claude" in result.stdout
            findings = json.loads((out / "findings.json").read_text())
            assert findings["pr"] == {
                "repo": "https://github.com/smoke/fixture", "pr_number": 7, "base": base, "head": head,
            }, findings["pr"]
            assert findings["agent"] == "opencodereview"
            export = json.loads((out / "ocr-findings.json").read_text())
            assert export["schema"] == "ocr-bench/v1"
            print("status:", export["status"], export.get("error") or "")
            if export["status"] == "ok":
                assert len(findings["findings"]) == 1, findings
                finding = findings["findings"][0]
                assert (finding["file"], finding["start_line"], finding["end_line"]) == ("app.py", 2, 2), finding
                assert finding["message"].startswith("add subtracts its arguments."), finding
                assert finding["producer"] == "opencodereview"
            elif os.environ.get("OCR_BENCH_SMOKE_REQUIRE_OK") == "1":
                raise AssertionError(f"review did not complete: {export.get('error')}")
            else:
                assert findings["findings"] == []
            assert git(repo, "rev-parse", "HEAD") == head, "the mounted checkout changed"
            assert (repo / "app.py").read_text().endswith("return a - b\n")
    finally:
        run("docker", "image", "rm", "-f", FAKE_IMAGE, check=False)
    print("bench smoke ok")


if __name__ == "__main__":
    main()
