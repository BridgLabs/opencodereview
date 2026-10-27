import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { cx } from "../../components/DesignSystem";
import { setupApi } from "../../services/setupApi";
import type { AccountUsage, AiAccount } from "../../types";
import {
  ACCOUNTS_QUERY_KEY,
  accountHealth,
  accountStatusText,
  ProviderGlyph,
  percentLeft,
  resetText,
  usageQuery,
  windowName,
} from "./AccountsPanel";

const RADIUS = 17;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** What is left of the tightest window, or null while the provider hasn't shared any. */
export function usageLeft(account: AiAccount, usage?: AccountUsage, now = Date.now()) {
  if (accountHealth(account, now) === "resting") return 0;
  if (!usage?.windows.length) return null;
  return Math.min(...usage.windows.map(percentLeft));
}

function UsageRing({ account, usage }: { account: AiAccount; usage?: AccountUsage }) {
  const left = usageLeft(account, usage);
  const lines = usage?.windows.length
    ? usage.windows.map((window) =>
        [`${windowName(window.windowMins)} ${percentLeft(window)}% left`, resetText(window.resetsAt)]
          .filter(Boolean)
          .join(" · "),
      )
    : [usage?.error ?? accountStatusText(account)];
  return (
    <Link
      to="/platform/settings"
      aria-label={`${account.label}: ${left === null ? "usage unknown" : `${left}% left`}`}
      className="group relative grid size-10 flex-none place-items-center rounded-xl no-underline transition-colors hover:bg-surface/80"
    >
      <svg viewBox="0 0 40 40" className="absolute inset-0 size-10 -rotate-90" aria-hidden="true">
        <circle cx="20" cy="20" r={RADIUS} fill="none" strokeWidth="2.5" className="stroke-line-strong" />
        {left !== null && left > 0 && (
          <circle
            cx="20"
            cy="20"
            r={RADIUS}
            fill="none"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - left / 100)}
            className={cx(left <= 10 ? "stroke-danger" : left <= 30 ? "stroke-warning" : "stroke-success")}
          />
        )}
      </svg>
      <ProviderGlyph provider={account.provider} />
      <span
        role="tooltip"
        className="pointer-events-none absolute bottom-0 left-full z-50 ml-2.5 rounded-lg border border-line-strong bg-surface-muted px-2.5 py-1.5 text-caption whitespace-nowrap text-fg-muted opacity-0 shadow-pop transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
      >
        <span className="block font-medium text-fg">{account.label}</span>
        {lines.map((line) => (
          <span key={line} className="block tabular-nums">
            {line}
          </span>
        ))}
      </span>
    </Link>
  );
}

/** One ring per account reviews can use, filled with how much of its plan is left. */
export function AccountUsageRail() {
  const accounts = useQuery({ queryKey: ACCOUNTS_QUERY_KEY, queryFn: setupApi.listAccounts, refetchInterval: 60_000 });
  const usage = useQuery(usageQuery);
  const usable = (accounts.data ?? []).filter((account) => account.signedIn && account.enabled);
  if (usable.length === 0) return null;
  return (
    <div className="flex flex-col items-center gap-1.5">
      {usable.map((account) => (
        <UsageRing
          key={account.id}
          account={account}
          usage={usage.data?.find((item) => item.accountId === account.id)}
        />
      ))}
    </div>
  );
}
