"""ReviewBench adapter (github.com/review-bench/ReviewBench).

The benchmark image already speaks the ReviewBench agent contract, so this only gates ReviewBench's
own `scripts/try-agent.sh` behind the paid-run authorization and passes the credential:

    python3 bench/adapters/reviewbench.py --reviewbench-dir ../ReviewBench --set test \\
        --provider claude --authorize-paid-run 25

try-agent.sh runs the image as `linux/amd64`, like the benchmark: build it with
`--platform linux/amd64`. `--native` runs the same loop here instead, on the host's architecture:
the same mounts, variables, mirror and findings checks, for local runs on Apple Silicon. Findings land in ./findings/; ReviewBench scores them in a portal test
run, or offline with its judge (docs/OFFLINE_EVAL.md in that repository).
"""

import argparse
import json
import os
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import common  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--reviewbench-dir", required=True, type=pathlib.Path)
    parser.add_argument("--set", choices=("test", "full"), default="test")
    parser.add_argument("--pr", type=int, help="one pull request by index")
    parser.add_argument("--provider", default="claude")
    parser.add_argument("--run-id", help="defaults to <set>-<provider>")
    parser.add_argument("--image", default=os.environ.get("OCR_BENCH_IMAGE", "opencodereview-bench:local"))
    parser.add_argument("--authorize-paid-run", type=int, metavar="REVIEWS")
    parser.add_argument("--native", action="store_true", help="run on the host architecture without try-agent.sh")
    parser.add_argument("--jobs", type=int, default=1, help="--native: pull requests reviewed at a time")
    args = parser.parse_args()
    common.check_provider(args.provider)

    manifest = args.reviewbench_dir / ("corpus/test/test.json" if args.set == "test" else "corpus/manifest.json")
    count = 1 if args.pr is not None else len(json.loads(manifest.read_text()))
    run_id = args.run_id or f"{args.set}-{args.provider}"
    run_dir = common.BENCH / "results" / "reviewbench" / run_id
    common.authorize(run_dir, count, args.authorize_paid_run, args.provider, "ReviewBench")
    if args.native:
        entries = json.loads(manifest.read_text())
        indices = [args.pr] if args.pr is not None else range(len(entries))
        sys.exit(native(args, [entries[index] for index in indices], run_dir))

    command = [str(args.reviewbench_dir / "scripts" / "try-agent.sh"), args.image, "--set", args.set]
    if args.pr is not None:
        command += ["--pr", str(args.pr)]
    command += ["-e", f"RB_CONFIG_PROVIDER={args.provider}"]
    for name in ("RB_CONFIG_MODEL", "RB_CONFIG_EFFORT", "RB_CONFIG_MAX_FILES", "RB_CONFIG_FOCUS_FILES", "RB_CONFIG_LENS_PASS", "RB_CONFIG_SUGGESTIONS", "RB_CONFIG_DIFF_RENAMES", "RB_CONFIG_DEADLINE_SECS"):
        if os.environ.get(name):
            command += ["-e", f"{name}={os.environ[name]}"]
    # try-agent.sh forwards variables, not files: a sign-in file travels as its contents.
    if os.environ.get("OCR_BENCH_CREDENTIAL_FILE"):
        os.environ["OCR_BENCH_CREDENTIAL"] = pathlib.Path(os.environ["OCR_BENCH_CREDENTIAL_FILE"]).read_text()
        command += ["-e", "OCR_BENCH_CREDENTIAL"]
    else:
        command += ["-e", "CLAUDE_CODE_OAUTH_TOKEN"]
    sys.exit(subprocess.run(command, cwd=run_dir).returncode)


def native(args, entries, run_dir):
    """try-agent.sh's loop on the host architecture. Returns the number of failed pull requests."""
    findings_dir = run_dir / "findings"
    findings_dir.mkdir(parents=True, exist_ok=True)

    def one(index, entry):
        nwo, number, base, head = entry["nwo"], int(entry["pr_number"]), entry["base"], entry["head"]
        key = f"{nwo.replace('/', '_')}_{number}-{head[:8]}"
        common.log(f"== [{index}/{len(entries)}] {nwo}#{number} ({key})")
        repo = None
        # The review-bench mirror is the snapshot the judge uses; upstream only when it lacks a commit.
        for url in (f"https://github.com/review-bench/{nwo.replace('/', '_')}.git", f"https://github.com/{nwo}.git"):
            try:
                repo = common.checkout(nwo, url, base, head, number)
                break
            except Exception as error:  # noqa: BLE001 - fall back to upstream
                common.log(f"   {key}: {url}: {error}")
                common.reset_cache(nwo)
        if repo is None:
            return False
        try:
            status, findings, export = common.review(
                args.image, repo, nwo, number, base, head, entry.get("title"), entry.get("body"), run_dir / "runs" / key, args.provider,
            )
        finally:
            common.release(repo)
        problem = check(findings, head, number) if status == 0 else f"exit code {status}"
        if problem:
            common.log(f"   {key}: FAIL: {problem}")
            return False
        (findings_dir / f"{key}.json").write_text(json.dumps(findings, indent=2))
        common.log(f"   {key}: ok: {len(findings['findings'])} finding(s), status {(export or {}).get('status')}")
        return True

    failed = common.run_all(entries, args.jobs, one).count(False)
    common.log(f"passed {len(entries) - failed}, failed {failed}; findings in {findings_dir}")
    return 1 if failed else 0


def check(findings, head, number):
    """The checks try-agent.sh applies to RB_OUT."""
    if not isinstance(findings, dict):
        return "no valid JSON at RB_OUT"
    if findings.get("pr", {}).get("head") != head:
        return "pr.head is not RB_HEAD"
    if findings["pr"].get("pr_number") != number:
        return "pr.pr_number is not RB_PR_NUMBER"
    if not isinstance(findings.get("findings"), list):
        return "findings is not an array"
    bad = [
        item for item in findings["findings"]
        if not isinstance(item.get("file"), str) or not isinstance(item.get("start_line"), int)
        or not isinstance(item.get("end_line"), int) or not isinstance(item.get("message"), str)
        or item["file"].startswith(("./", "/"))
    ]
    return f"{len(bad)} finding(s) miss file, start_line, end_line or message" if bad else ""


if __name__ == "__main__":
    main()
