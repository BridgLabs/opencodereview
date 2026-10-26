import { cx } from "../../../components/DesignSystem";
import type { UsageStripRun } from "../../../types";
import { compactTokens, outcomeMeta } from "./format";

/** One segment per run, oldest first, as wide as its share of the PR's tokens and colored by outcome. */
export function RunStrip({ runs, className }: { runs: UsageStripRun[]; className?: string }) {
  const total = runs.reduce((sum, run) => sum + run.tokens, 0);
  // Runs that used (almost) nothing keep a sliver so every run stays visible.
  const floor = Math.max(total * 0.04, 1);
  const counts = Object.entries(
    runs.reduce<Record<string, number>>((all, run) => {
      const label = outcomeMeta[run.outcome].label.toLowerCase();
      all[label] = (all[label] ?? 0) + 1;
      return all;
    }, {}),
  ).map(([label, count]) => `${count} ${label}`);
  return (
    <span
      role="img"
      aria-label={`${runs.length} ${runs.length === 1 ? "run" : "runs"}: ${counts.join(", ")}`}
      className={cx("flex h-2 gap-px overflow-hidden rounded-sm", className)}
    >
      {runs.map((run) => (
        <span
          key={run.runId}
          className={cx("min-w-0.5", outcomeMeta[run.outcome].segment)}
          style={{ flexGrow: Math.max(run.tokens, floor), flexBasis: 0 }}
          title={`${outcomeMeta[run.outcome].label} · ${compactTokens(run.tokens)}`}
        />
      ))}
    </span>
  );
}
