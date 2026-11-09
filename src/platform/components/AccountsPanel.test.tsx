import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setupApi } from "../../services/setupApi";
import type { AiAccount } from "../../types";
import {
  AccountsPanel,
  accountHealth,
  accountStatusText,
  nextAccountId,
  usageSummary,
  windowName,
} from "./AccountsPanel";

vi.mock("../../services/setupApi", () => ({
  setupApi: {
    listAccounts: vi.fn(),
    accountUsage: vi.fn(() => Promise.resolve([])),
    updateAccount: vi.fn(),
    reorderAccounts: vi.fn(),
    deleteAccount: vi.fn(),
    claudeAuthStart: vi.fn(),
    claudeAuthEvents: vi.fn(),
    claudeAuthInput: vi.fn(),
    claudeAuthCancel: vi.fn(),
    claudeTokenSave: vi.fn(),
    codexAuthStart: vi.fn(),
    codexAuthEvents: vi.fn(),
    codexAuthCancel: vi.fn(),
    opencodeKeySave: vi.fn(),
    opencodeAuthStart: vi.fn(),
    opencodeAuthEvents: vi.fn(),
    opencodeAuthCancel: vi.fn(),
  },
}));

const now = 1_800_000_000_000;
const account = (id: number, change: Partial<AiAccount> = {}): AiAccount => ({
  id,
  provider: "claude",
  label: `Claude ${id}`,
  priority: id,
  enabled: true,
  status: "ok",
  limitedUntil: null,
  lastError: null,
  lastUsedAt: null,
  createdAt: 0,
  signedIn: true,
  ...change,
});

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("describes each account by what reviews will do with it", () => {
  expect(accountHealth(account(1), now)).toBe("ready");
  expect(accountStatusText(account(1, { lastUsedAt: now - 2 * 60 * 60_000 }), now)).toBe("Ready · last used 2 h ago");
  const resting = account(2, { status: "limited", limitedUntil: now + 95 * 60_000 });
  expect(accountHealth(resting, now)).toBe("resting");
  expect(accountStatusText(resting, now)).toBe("Usage limit reached · resets in 1 h 35 min");
  // A rest that already elapsed is ready again even before the next review clears it.
  expect(accountHealth(account(3, { status: "limited", limitedUntil: now - 1 }), now)).toBe("ready");
  expect(accountStatusText(account(4, { status: "auth_failed" }), now)).toBe(
    "Claude rejected this sign-in · sign in again",
  );
  expect(accountHealth(account(5, { signedIn: false }), now)).toBe("signed-out");
  expect(accountStatusText(account(6, { enabled: false }), now)).toBe("Paused · reviews skip this account");
  expect(accountStatusText(account(8, { enabled: false, status: "limited", limitedUntil: now + 3_600_000 }), now)).toBe(
    "Usage limit reached · resets in 1 h",
  );
});

const menuItem = async (container: HTMLElement, accountLabel: string, item: string) => {
  await act(async () =>
    container.querySelector<HTMLButtonElement>(`button[aria-label="${accountLabel} actions"]`)!.click(),
  );
  const entry = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((candidate) =>
    candidate.textContent?.startsWith(item),
  );
  if (entry) await act(async () => entry.click());
  return entry;
};

it("reorders accounts and removes one only after confirmation", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1), account(2)]);
  vi.mocked(setupApi.reorderAccounts).mockResolvedValue([account(2), account(1)]);
  vi.mocked(setupApi.deleteAccount).mockResolvedValue([account(1)]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <AccountsPanel />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const rows = () => [...container.querySelectorAll<HTMLElement>(".account-row")];
    expect(rows().map((row) => row.dataset.account)).toEqual(["1", "2"]);
    expect(await menuItem(container, "Claude 2", "Move up")).toBeUndefined();
    await act(async () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await menuItem(container, "Claude 2", "Use for new reviews");
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1]);
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(rows().map((row) => row.dataset.account)).toEqual(["2", "1"]);

    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Claude 2 actions"]')!.click());
    await act(async () =>
      [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((item) => item.textContent?.startsWith("Remove account"))!
        .click(),
    );
    expect(setupApi.deleteAccount).not.toHaveBeenCalled();
    const confirm = container.querySelector('[role="alertdialog"]')!;
    expect(confirm.textContent).toContain("Remove “Claude 2”?");
    await act(async () =>
      [...confirm.querySelectorAll("button")].find((button) => button.textContent === "Remove")!.click(),
    );
    expect(setupApi.deleteAccount).toHaveBeenCalledWith(2);
    expect(rows().map((row) => row.dataset.account)).toEqual(["1"]);
  } finally {
    await act(async () => root.unmount());
    client.clear();
  }
});

async function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  // Forms only submit while attached to the document.
  document.body.append(container);
  const root = createRoot(container);
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <AccountsPanel />
      </QueryClientProvider>,
    ),
  );
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return {
    container,
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
      client.clear();
    },
  };
}

it("adds a ChatGPT account through the browser sign-in", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([]);
  vi.mocked(setupApi.codexAuthStart).mockResolvedValue(undefined);
  vi.mocked(setupApi.codexAuthEvents).mockResolvedValue({
    running: true,
    events: [{ kind: "prompt", url: "https://auth.openai.com/oauth/authorize?state=s" }],
  });
  const { container, unmount } = await renderPanel();
  try {
    await act(async () =>
      [...container.querySelectorAll("button")]
        .find((button) => button.textContent?.includes("Add ChatGPT account"))!
        .click(),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(setupApi.codexAuthStart).toHaveBeenCalledWith(undefined, "browser");
    await vi.waitFor(() =>
      expect(
        container.querySelector<HTMLAnchorElement>("a[href='https://auth.openai.com/oauth/authorize?state=s']")
          ?.textContent,
      ).toBe("Open ChatGPT sign-in"),
    );
    expect(container.textContent).toContain("Add a ChatGPT account");
  } finally {
    await unmount();
  }
});

it("adds a ChatGPT account with a one-time code from the add menu", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1)]);
  vi.mocked(setupApi.codexAuthStart).mockResolvedValue(undefined);
  vi.mocked(setupApi.codexAuthEvents).mockResolvedValue({
    running: true,
    events: [{ kind: "prompt", url: "https://auth.openai.com/codex/device", code: "7KQ2-9XWPL" }],
  });
  const { container, unmount } = await renderPanel();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Add account"]')!.click());
    await act(async () =>
      [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((item) => item.textContent?.startsWith("ChatGPT with a code"))!
        .click(),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(setupApi.codexAuthStart).toHaveBeenCalledWith(undefined, "device");
    await vi.waitFor(() =>
      expect(container.querySelector('[aria-label="Codex sign-in code"]')?.textContent).toBe("7KQ2-9XWPL"),
    );
  } finally {
    await unmount();
  }
});

it("signs in to OpenCode with a one-time code from the add menu", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1)]);
  vi.mocked(setupApi.opencodeAuthStart).mockResolvedValue(undefined);
  vi.mocked(setupApi.opencodeAuthEvents).mockResolvedValue({
    running: true,
    events: [
      {
        kind: "prompt",
        url: "https://opencode.ai/console/device?user_code=MCTD-DMGG&client_id=opencode-cli",
        code: "MCTD-DMGG",
      },
    ],
  });
  const { container, unmount } = await renderPanel();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Add account"]')!.click());
    await act(async () =>
      [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((entry) => entry.textContent?.startsWith("OpenCodeOpenCode Go plan"))!
        .click(),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(setupApi.opencodeAuthStart).toHaveBeenCalledWith(undefined);
    await vi.waitFor(() =>
      expect(container.querySelector('[aria-label="OpenCode sign-in code"]')?.textContent).toBe("MCTD-DMGG"),
    );
    expect(
      [...container.querySelectorAll("a")].find((link) => link.textContent?.includes("Open sign-in page"))?.href,
    ).toBe("https://opencode.ai/console/device?user_code=MCTD-DMGG&client_id=opencode-cli");
  } finally {
    await unmount();
  }
});

it("adds an OpenCode API key from the menu", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1)]);
  vi.mocked(setupApi.opencodeKeySave).mockResolvedValue({ ok: true });
  const { container, unmount } = await renderPanel();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Add account"]')!.click());
    await act(async () =>
      [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((entry) => entry.textContent?.startsWith("OpenCode with a key"))!
        .click(),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.textContent).toContain("Add an OpenCode API key");
    const key = container.querySelector<HTMLInputElement>('input[aria-label="OpenCode API key"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(key, "sk-opencode-fixture-key");
      key.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () =>
      [...container.querySelectorAll("button")].find((button) => button.textContent === "Save key")!.click(),
    );
    expect(setupApi.opencodeKeySave).toHaveBeenCalledWith("sk-opencode-fixture-key", undefined);
  } finally {
    await unmount();
  }
});

it("saves a pasted Claude token", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1)]);
  vi.mocked(setupApi.claudeTokenSave).mockResolvedValue({ ok: true });
  const { container, unmount } = await renderPanel();
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Add account"]')!.click());
    await act(async () =>
      [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
        .find((item) => item.textContent?.startsWith("Claude with a token"))!
        .click(),
    );
    const input = container.querySelector<HTMLInputElement>('input[aria-label="Claude token"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, " sk-ant-oat01-fixture ");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () =>
      [...container.querySelectorAll("button")].find((button) => button.textContent === "Save token")!.click(),
    );
    expect(setupApi.claudeTokenSave).toHaveBeenCalledWith("sk-ant-oat01-fixture", undefined);
  } finally {
    await unmount();
  }
});

it("labels each account with its provider", () => {
  expect(accountStatusText(account(7, { provider: "codex", status: "auth_failed" }), now)).toBe(
    "ChatGPT rejected this sign-in · sign in again",
  );
});

it("marks the account the next review starts on and summarizes what is left", () => {
  expect(
    nextAccountId(
      [
        account(1, { enabled: false }),
        account(2, { status: "limited", limitedUntil: now + 60_000 }),
        account(3),
        account(4),
      ],
      now,
    ),
  ).toBe(3);
  expect(nextAccountId([account(1, { signedIn: false })], now)).toBeUndefined();
  // A limit moves reviews to the first account's provider's other accounts before another provider's.
  expect(
    nextAccountId(
      [account(1, { status: "limited", limitedUntil: now + 60_000 }), account(2, { provider: "codex" }), account(3)],
      now,
    ),
  ).toBe(3);
  expect([windowName(300), windowName(10_080), windowName(1440), windowName(null)]).toEqual([
    "5-hour",
    "Weekly",
    "1-day",
    "Usage",
  ]);
  expect(
    usageSummary({
      accountId: 1,
      windows: [
        { windowMins: 300, usedPercent: 27.6, resetsAt: null },
        { windowMins: 10_080, usedPercent: 104, resetsAt: null },
      ],
    }),
  ).toBe("5-hour 72% left · Weekly 0% left");
});

it("shows the next-review badge and usage meters on account rows", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([
    account(1, { enabled: false }),
    account(2, { provider: "codex", label: "dev@example.com" }),
  ]);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([
    { accountId: 2, windows: [{ windowMins: 300, usedPercent: 25, resetsAt: null }] },
    { accountId: 1, windows: [], error: "Claude shares this sign-in's usage after its next review" },
  ]);
  const { container, unmount } = await renderPanel();
  try {
    const [paused, chatgpt] = [...container.querySelectorAll<HTMLElement>(".account-row")];
    expect(chatgpt.textContent).toContain("Next review");
    expect(paused.textContent).not.toContain("Next review");
    expect(chatgpt.querySelector('[role="meter"]')?.getAttribute("aria-valuenow")).toBe("75");
    expect(chatgpt.textContent).toContain("75% left");
    expect(paused.textContent).toContain("Paused");
    expect(paused.textContent).not.toContain("usage after its next review");
  } finally {
    await unmount();
  }
});

it("starts new reviews on the account picked, resuming it if it was paused", async () => {
  const codex = account(2, { provider: "codex", label: "ChatGPT", enabled: false });
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1), codex, account(3, { signedIn: false })]);
  vi.mocked(setupApi.updateAccount).mockResolvedValue([
    account(1),
    { ...codex, enabled: true },
    account(3, { signedIn: false }),
  ]);
  vi.mocked(setupApi.reorderAccounts).mockResolvedValue([
    { ...codex, enabled: true },
    account(1),
    account(3, { signedIn: false }),
  ]);
  const { container, unmount } = await renderPanel();
  try {
    expect(container.querySelector('[role="radiogroup"]')).toBeNull();
    expect(await menuItem(container, "Claude 1", "Use for new reviews")).toBeUndefined();
    await act(async () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await menuItem(container, "ChatGPT", "Use for new reviews");
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(setupApi.updateAccount).toHaveBeenCalledWith(2, { enabled: true });
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1, 3]);
    const first = container.querySelector<HTMLElement>(".account-row")!;
    expect(first.dataset.account).toBe("2");
    expect(first.textContent).toContain("Next review");
  } finally {
    await unmount();
  }
});
