import { Info } from "lucide-react";
import type { UsageReport } from "../../../types";
import { ProviderGlyph } from "../../components/AccountsPanel";
import { attributedRows, compactTokens, isProvider, pct } from "./format";

const plural = (count: number, word: string) => `${count.toLocaleString()} ${word}${count === 1 ? "" : "s"}`;

export function UsageHero({ report }: { report: UsageReport }) {
  const { totals } = report;
  const providers = attributedRows(report.byProvider);
  const split = [
    { label: "Input", tokens: totals.inputTokens, className: "bg-brand" },
    { label: "Output", tokens: totals.outputTokens, className: "bg-brand/60" },
    { label: "Cache writes", tokens: totals.cacheCreationTokens, className: "bg-brand/30" },
  ];
  const splitText = split.map((part) => `${part.label} ${compactTokens(part.tokens)}`).join(" · ");
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <p
        className="font-sans text-hero text-fg tabular-nums"
        data-usage-hero
        title={`${totals.tokens.toLocaleString()} tokens`}
      >
        {compactTokens(totals.tokens)}
      </p>
      <p className="flex flex-wrap items-center gap-x-1.5 text-small text-fg-muted">
        <span>
          tokens · {plural(totals.runs, "run")} on {plural(totals.prs, "PR")}
        </span>
        {totals.runsWithoutUsage > 0 && (
          <span
            className="inline-flex items-center gap-1 text-fg-subtle"
            title="These runs finished without the AI CLI reporting token usage, so they add nothing to the totals."
          >
            <Info className="size-3.5" aria-hidden="true" />
            {plural(totals.runsWithoutUsage, "run")} without reported usage
          </span>
        )}
      </p>
      {totals.cacheReadTokens > 0 && (
        <p
          className="inline-flex w-fit items-center gap-1 text-small text-fg-subtle tabular-nums"
          title="Prompt cache reads reported by the provider. They are not counted in tokens."
        >
          + {compactTokens(totals.cacheReadTokens)} read from cache
          <Info className="size-3.5" aria-hidden="true" />
        </p>
      )}
      {totals.tokens > 0 && (
        <div className="flex h-1.5 w-full max-w-72 gap-px overflow-hidden rounded-full" title={splitText}>
          {split.map(
            (part) =>
              part.tokens > 0 && <span key={part.label} className={part.className} style={{ flexGrow: part.tokens }} />,
          )}
          <span className="sr-only">{splitText}</span>
        </div>
      )}
      {providers.length >= 2 && (
        <ul className="flex flex-wrap gap-x-4 gap-y-1 text-caption text-fg-muted" aria-label="Tokens by provider">
          {providers.map((row) => (
            <li key={row.key} className="inline-flex items-center gap-1.5 tabular-nums">
              {isProvider(row.provider) && <ProviderGlyph provider={row.provider} />}
              <span>{row.label}</span>
              <span className="text-fg-subtle">{pct(row.tokens, totals.tokens)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
