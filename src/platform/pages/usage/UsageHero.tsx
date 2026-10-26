import { Hint, Tip } from "../../../components/Tip";
import type { UsageReport } from "../../../types";
import { ProviderGlyph } from "../../components/AccountsPanel";
import { attributedRows, compactTokens, isProvider, pct } from "./format";

const plural = (count: number, word: string) => `${count.toLocaleString()} ${word}${count === 1 ? "" : "s"}`;

/** One part of the token total: label, amount, share and a bar of that share. */
function MixRow({
  label,
  tokens,
  total,
  bar,
  hint,
}: {
  label: string;
  tokens: number;
  total: number;
  bar: string;
  hint: string;
}) {
  return (
    <li className="grid grid-cols-[7rem_minmax(0,1fr)_4.5rem_2.75rem] items-center gap-3 text-small">
      <span className="inline-flex items-center gap-1.5 text-fg-muted">
        {label}
        <Hint text={hint} />
      </span>
      <span className="h-1.5 overflow-hidden rounded-full bg-surface-sunken">
        <span className={`block h-full rounded-full ${bar}`} style={{ width: pct(tokens, total) }} />
      </span>
      <span className="text-right font-mono text-fg tabular-nums">{compactTokens(tokens)}</span>
      <span className="text-right font-mono text-caption text-fg-subtle tabular-nums">{pct(tokens, total)}</span>
    </li>
  );
}

export function UsageHero({ report }: { report: UsageReport }) {
  const { totals } = report;
  const providers = attributedRows(report.byProvider);
  return (
    <div className="flex h-full min-w-0 flex-col gap-6">
      <div className="flex flex-col gap-2">
        <Tip content={`${totals.tokens.toLocaleString()} tokens`} className="w-fit">
          <p className="font-sans text-hero text-fg tabular-nums" data-usage-hero>
            {compactTokens(totals.tokens)}
          </p>
        </Tip>
        <p className="flex flex-wrap items-center gap-x-1.5 text-small text-fg-muted">
          <span>
            tokens · {plural(totals.runs, "run")} on {plural(totals.prs, "PR")}
          </span>
          {totals.runsWithoutUsage > 0 && (
            <span className="inline-flex items-center gap-1 text-fg-subtle">
              · {plural(totals.runsWithoutUsage, "run")} without reported usage
              <Hint text="These runs finished without the AI CLI reporting token usage, so they add nothing to the totals." />
            </span>
          )}
        </p>
      </div>
      {totals.tokens > 0 && (
        <ul className="flex flex-col gap-2.5" aria-label="What the tokens are">
          <MixRow
            label="Input"
            tokens={totals.inputTokens}
            total={totals.tokens}
            bar="bg-brand"
            hint="Prompt tokens the model processed: the diff, context and instructions."
          />
          <MixRow
            label="Output"
            tokens={totals.outputTokens}
            total={totals.tokens}
            bar="bg-brand/70"
            hint="Tokens the model wrote: findings, questions and its reasoning."
          />
          <MixRow
            label="Cache writes"
            tokens={totals.cacheCreationTokens}
            total={totals.tokens}
            bar="bg-brand/40"
            hint="Prompt tokens written to the provider's cache so later calls can reuse them."
          />
        </ul>
      )}
      <div className="mt-auto flex flex-col gap-2 border-t border-line pt-3">
        {totals.cacheReadTokens > 0 && (
          <p className="flex items-center justify-between gap-3 text-small">
            <span className="inline-flex items-center gap-1.5 text-fg-muted">
              Read from cache
              <Hint text="Context the provider re-read from its prompt cache instead of processing it again. Not counted in tokens." />
            </span>
            <span className="font-mono text-fg-muted tabular-nums">+ {compactTokens(totals.cacheReadTokens)}</span>
          </p>
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
    </div>
  );
}
