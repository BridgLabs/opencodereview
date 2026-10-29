"""Comparison tables: OpenCodeReview next to the other tools, each benchmark scored by its own judge.

    python3 bench/report.py \\
        --martian-evaluations ../code-review-benchmark/offline/results/<judge>/evaluations.json \\
        --reviewbench-leaderboard https://review-bench.ai/api/leaderboard \\
        --reviewbench-ours bench/results/reviewbench/test-claude/scores.json \\
        --aacr-metrics opencodereview-claude=<metrics json> --aacr-metrics claude-code=<metrics json>

Prints Markdown. Every number comes from a benchmark's own output; nothing is typed in by hand.
"""

import argparse
import json
import pathlib
import urllib.request


def percent(value):
    return "–" if value is None else f"{value:.1f}"


def f1(precision, recall):
    return 2 * precision * recall / (precision + recall) if precision + recall else 0.0


def table(title, columns, rows, note):
    lines = [f"### {title}", "", "| " + " | ".join(columns) + " |", "|" + "---|" * len(columns)]
    lines += ["| " + " | ".join(str(cell) for cell in row) + " |" for row in rows]
    return "\n".join(lines + ["", note, ""])


def martian(path):
    """Precision, recall and F1 per tool from Martian's judge output (all categories, one judge)."""
    evaluations = json.loads(pathlib.Path(path).read_text())
    totals = {}
    for tools in evaluations.values():
        for tool, result in tools.items():
            if result.get("skipped"):
                continue
            tp, fp, fn = (len(result.get(key) or []) for key in ("true_positives", "false_positives", "false_negatives"))
            entry = totals.setdefault(tool, [0, 0, 0, 0])
            entry[0] += tp
            entry[1] += fp
            entry[2] += fn
            entry[3] += 1
    rows = []
    for tool, (tp, fp, fn, prs) in totals.items():
        precision = 100 * tp / (tp + fp) if tp + fp else 0.0
        recall = 100 * tp / (tp + fn) if tp + fn else 0.0
        rows.append((tool, precision, recall, f1(precision, recall), prs))
    rows.sort(key=lambda row: -row[3])
    marked = [
        (f"**{tool}**" if tool.startswith("opencodereview") else tool, percent(p), percent(r), percent(score), prs)
        for tool, p, r, score, prs in rows
    ]
    return table(
        "Martian Code Review Bench (offline, 50 PRs)",
        ["Tool", "Precision", "Recall", "F1", "PRs judged"],
        marked,
        f"Recomputed from `{path}`: all golden categories, judge as named in the path. "
        "Martian's `summary_table` adds the Strict/Core profiles and deduplication.",
    )


def reviewbench(source, ours):
    raw = urllib.request.urlopen(source).read() if source.startswith("http") else pathlib.Path(source).read_bytes()
    rows = [
        (row.get("name"), row.get("groundedPrecision"), row.get("groundedRecall"), row.get("augmentedF1"), row.get("snapshotDate"))
        for row in json.loads(raw)["reviewerMetrics"]
    ]
    for path in ours or []:
        score = json.loads(pathlib.Path(path).read_text())
        rows.append((f"**{score.get('name', 'opencodereview')}**", score.get("groundedPrecision"), score.get("groundedRecall"), score.get("augmentedF1"), score.get("snapshotDate", "this run")))
    rows.sort(key=lambda row: -(row[3] or 0))
    return table(
        "ReviewBench",
        ["Agent", "Grounded precision", "Grounded recall", "Augmented F1", "Snapshot"],
        [(name, percent(p), percent(r), percent(score), date) for name, p, r, score, date in rows],
        f"Published leaderboard from `{source}`; OpenCodeReview rows from the files given. "
        "Compare test-set runs with care: the leaderboard is the full set over 3 rounds.",
    )


def find(value, key):
    if isinstance(value, dict):
        if key in value and isinstance(value[key], (int, float)):
            return value[key]
        for child in value.values():
            found = find(child, key)
            if found is not None:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find(child, key)
            if found is not None:
                return found
    return None


def aacr(entries):
    rows = []
    for entry in entries:
        name, _, path = entry.partition("=")
        metrics = json.loads(pathlib.Path(path).read_text())
        values = [find(metrics, key) for key in ("file_semantic_match_rate", "file_semantic_recall_rate", "balanced_score")]
        scaled = [None if value is None else (100 * value if value <= 1 else value) for value in values]
        rows.append((f"**{name}**" if name.startswith("opencodereview") else name, *scaled))
    rows.sort(key=lambda row: -(row[3] or 0))
    return table(
        "AACR-Bench",
        ["Reviewer", "Precision", "Recall", "Balanced score"],
        [(name, percent(p), percent(r), percent(score)) for name, p, r, score in rows],
        "AACR-Bench's file-level semantic metrics, one judge for every reviewer listed.",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--martian-evaluations")
    parser.add_argument("--reviewbench-leaderboard", help="URL or saved JSON of review-bench.ai/api/leaderboard")
    parser.add_argument("--reviewbench-ours", action="append", help="JSON with name, groundedPrecision, groundedRecall, augmentedF1")
    parser.add_argument("--aacr-metrics", action="append", metavar="NAME=PATH")
    args = parser.parse_args()
    sections = []
    if args.reviewbench_leaderboard:
        sections.append(reviewbench(args.reviewbench_leaderboard, args.reviewbench_ours))
    if args.martian_evaluations:
        sections.append(martian(args.martian_evaluations))
    if args.aacr_metrics:
        sections.append(aacr(args.aacr_metrics))
    if not sections:
        parser.error("name at least one benchmark's results")
    print("\n".join(sections))


if __name__ == "__main__":
    main()
