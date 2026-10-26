import type { UsagePrPage, UsageQuery, UsageReport, UsageRun } from "../types";
import { http } from "./setupApiTransport";

/** `days` and `tz` always, `repo` only when a repository filter is set. */
function usageParams({ days, repoId, tz }: UsageQuery, extra: Record<string, number> = {}) {
  const params = new URLSearchParams({ days: String(days), tz: String(tz) });
  if (repoId !== null) params.set("repo", String(repoId));
  for (const [key, value] of Object.entries(extra)) params.set(key, String(value));
  return params.toString();
}

export const setupApiUsage = {
  usageReport: (query: UsageQuery) => http<UsageReport>(`/usage?${usageParams(query)}`),
  usagePrs: (query: UsageQuery, offset: number, limit: number) =>
    http<UsagePrPage>(`/usage/prs?${usageParams(query, { offset, limit })}`),
  usagePrRuns: (prId: number, query: Omit<UsageQuery, "repoId">) =>
    http<UsageRun[]>(`/usage/prs/${prId}/runs?${usageParams({ ...query, repoId: null })}`),
};
