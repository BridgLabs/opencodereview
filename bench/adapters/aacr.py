"""AACR-Bench adapter (github.com/alibaba/aacr-bench).

Reviews each instance of an AACR standard-format dataset with the benchmark image and writes the
results where AACR's evaluator reads them, in the format of its built-in `ocr` reviewer (which is
Alibaba's open-code-review, a different tool). Score with AACR's own judge:

    python3 bench/adapters/aacr.py --aacr-dir ../aacr-bench/evaluation \\
        --dataset data/aacr_bench.jsonl --limit 50 --provider claude --authorize-paid-run 50
    cd ../aacr-bench/evaluation && python -m pipeline run --stage eval --reviewer ocr \\
        --dataset data/aacr_bench.jsonl --run-id opencodereview-claude

The result files say `"reviewer": "opencodereview"`; `--reviewer ocr` only selects the reader.
"""

import argparse
import json
import os
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import common  # noqa: E402


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--aacr-dir", required=True, type=pathlib.Path, help="aacr-bench/evaluation")
    parser.add_argument("--dataset", default="data/aacr_bench.jsonl", help="relative to --aacr-dir")
    parser.add_argument("--provider", default="claude")
    parser.add_argument("--run-id", help="defaults to opencodereview-<provider>")
    parser.add_argument("--limit", type=int)
    parser.add_argument("--image", default=os.environ.get("OCR_BENCH_IMAGE", "opencodereview-bench:local"))
    parser.add_argument("--authorize-paid-run", type=int, metavar="REVIEWS")
    parser.add_argument("--jobs", type=int, default=1, help="instances reviewed at a time")
    args = parser.parse_args()
    common.check_provider(args.provider)

    dataset = args.aacr_dir / args.dataset
    instances = [json.loads(line) for line in dataset.read_text().splitlines() if line.strip()]
    instances = instances[: args.limit] if args.limit else instances
    run_id = args.run_id or f"opencodereview-{args.provider}"
    results = args.aacr_dir / "results" / dataset.stem / "ocr" / run_id
    pending = [item for item in instances if not (results / result_name(item)).exists()]
    common.authorize(common.BENCH / "results" / "aacr" / run_id, len(pending), args.authorize_paid_run, args.provider, "AACR-Bench")

    def one(index, item):
        nwo = item["repo"]
        common.log(f"== [{index}/{len(pending)}] {item['instance_id']}")
        try:
            repo = common.checkout(nwo, item.get("clone_url") or f"https://github.com/{nwo}.git", item["base_commit"], item["head_commit"])
        except Exception as error:  # noqa: BLE001 - one bad sample must not stop the run
            common.log(f"   {item['instance_id']}: skipped: {error}")
            return {"instance_id": item["instance_id"], "status": "checkout_failed", "error": str(error)}
        work = common.BENCH / "results" / "aacr" / run_id / "runs" / item["instance_id"].replace("/", "__")
        try:
            status, findings, export = common.review(
                args.image, repo, nwo, 1, item["base_commit"], item["head_commit"], "", "", work, args.provider,
            )
        finally:
            common.release(repo)
        if status != 0 or findings is None:
            return {"instance_id": item["instance_id"], "status": "failed", "exit": status}
        usage = (export or {}).get("usage") or {}
        payload = {
            "instance_id": item["instance_id"],
            "repo": nwo,
            "base_commit": item["base_commit"],
            "head_commit": item["head_commit"],
            "reviewer": "opencodereview",
            "engine": (export or {}).get("engine"),
            "status": (export or {}).get("status"),
            "duration_seconds": round(((export or {}).get("durationMs") or 0) / 1000, 2),
            "review": {
                "comments": [
                    {"path": finding["file"], "start_line": finding["start_line"], "end_line": finding["end_line"], "content": finding["message"]}
                    for finding in findings["findings"]
                ],
                "summary": {"input_tokens": usage.get("inputTokens", 0), "output_tokens": usage.get("outputTokens", 0)},
            },
        }
        results.mkdir(parents=True, exist_ok=True)
        (results / result_name(item)).write_text(json.dumps(payload, ensure_ascii=False, indent=2))
        return {"instance_id": item["instance_id"], "status": payload["status"], "findings": len(findings["findings"])}

    summary = common.run_all(pending, args.jobs, one)
    (common.BENCH / "results" / "aacr" / run_id / "summary.json").write_text(json.dumps(summary, indent=2))
    common.log(f"done: results in {results}")


def result_name(item):
    return item["instance_id"].replace("/", "__") + ".json"


if __name__ == "__main__":
    main()
