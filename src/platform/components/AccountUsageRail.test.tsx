import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vitest";
import type { AiAccount } from "../../types";
import { AccountUsageRail, usageLeft } from "./AccountUsageRail";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, ...props }: { children: ReactNode; to?: string }) => (
    <a href={to} {...props}>
      {children}
    </a>
  ),
}));
vi.mock("../../services/setupApi", () => ({ setupApi: {} }));

const account = (overrides: Partial<AiAccount>): AiAccount => ({
  id: 1,
  provider: "claude",
  label: "Work",
  priority: 1,
  enabled: true,
  status: "ok",
  limitedUntil: null,
  lastError: null,
  lastUsedAt: null,
  createdAt: 0,
  signedIn: true,
  ...overrides,
});

it("fills each ring with what is left of the tightest window", () => {
  const usage = {
    accountId: 1,
    windows: [
      { windowMins: 300, usedPercent: 20, resetsAt: null },
      { windowMins: 10_080, usedPercent: 65, resetsAt: null },
    ],
  };
  expect(usageLeft(account({}), usage)).toBe(35);
  expect(usageLeft(account({}), { accountId: 1, windows: [] })).toBeNull();
  expect(usageLeft(account({ status: "limited", limitedUntil: 2_000 }), usage, 1_000)).toBe(0);
});

it("shows only the accounts reviews can use", () => {
  const client = new QueryClient();
  client.setQueryData(
    ["accounts"],
    [
      account({ id: 1, label: "Work" }),
      account({ id: 2, label: "Paused", enabled: false }),
      account({ id: 3, label: "Signed out", provider: "codex", signedIn: false }),
    ],
  );
  client.setQueryData(
    ["accounts", "usage"],
    [{ accountId: 1, windows: [{ windowMins: 300, usedPercent: 92, resetsAt: null }] }],
  );
  try {
    const html = renderToStaticMarkup(
      <QueryClientProvider client={client}>
        <AccountUsageRail />
      </QueryClientProvider>,
    );
    expect(html).toContain('aria-label="Work: 8% left"');
    expect(html).toContain("stroke-danger");
    expect(html).toContain("5-hour 8% left");
    expect(html).not.toContain("Paused");
    expect(html).not.toContain("Signed out");
  } finally {
    client.clear();
  }
});
