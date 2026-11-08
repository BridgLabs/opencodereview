import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setupApi } from "../../services/setupApi";
import type { AiAccount, ReviewEngine } from "../../types";
import { engineCataloguePollInterval } from "./EngineAccounts";
import { EnginePicker } from "./EnginePicker";

vi.mock("../../services/setupApi", () => ({
  setupApi: {
    listAccounts: vi.fn(),
    reviewEngines: vi.fn(),
    saveReviewEngine: vi.fn(),
    reorderAccounts: vi.fn(),
    updateAccount: vi.fn(),
    accountUsage: vi.fn(),
    refreshOpenCodeCatalogue: vi.fn(),
  },
}));

const account = (id: number, provider: AiAccount["provider"], label: string): AiAccount => ({
  id,
  provider,
  label,
  priority: id,
  enabled: true,
  status: "ok",
  limitedUntil: null,
  lastError: null,
  lastUsedAt: null,
  createdAt: 0,
  signedIn: true,
});
const engines: ReviewEngine[] = [
  {
    provider: "claude",
    name: "Claude",
    model: "claude-opus-5-5",
    effort: "medium",
    label: "Claude · Opus 5.5 · medium",
    models: [
      { id: "claude-opus-5-5", label: "Opus 5.5", efforts: ["low", "medium", "high"], defaultEffort: "medium" },
      { id: "claude-sonnet-5-5", label: "Sonnet 5.5", efforts: ["low", "medium", "high"], defaultEffort: "medium" },
    ],
  },
  {
    provider: "codex",
    name: "ChatGPT",
    model: "default",
    effort: "medium",
    label: "ChatGPT · Codex default · medium",
    models: [{ id: "default", label: "Codex default", efforts: ["low", "medium"], defaultEffort: "medium" }],
  },
  {
    provider: "opencode",
    name: "OpenCode Go",
    model: "go-default",
    effort: "medium",
    label: "OpenCode Go · Go Default · medium",
    catalogue: { status: "ready", id: "go-catalogue-1", error: null },
    selectionValid: true,
    models: [
      {
        id: "go-default",
        label: "Go Default",
        efforts: ["low", "medium", "high", "xhigh", "max", "ultra"],
        defaultEffort: "medium",
      },
      { id: "go-no-effort", label: "Go Compact", efforts: [], defaultEffort: "" },
    ],
  },
];

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("shows the next review's engine and changes model, effort and provider", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1, "claude", "Claude"), account(2, "codex", "me@x.com")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.saveReviewEngine).mockResolvedValue(engines);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  vi.mocked(setupApi.reorderAccounts).mockResolvedValue([
    account(2, "codex", "me@x.com"),
    account(1, "claude", "Claude"),
  ]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const button = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes(text))!;
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.textContent).toContain("Opus 5.5");
    // Effort and account live in the trigger's label, not its text.
    expect(container.querySelector("button[aria-haspopup]")?.getAttribute("aria-label")).toBe(
      "Opus 5.5 · Medium · Claude",
    );
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    // The rail holds the providers; the menu shows the next account's provider first.
    expect(container.querySelector('button[aria-label="ChatGPT"]')).not.toBeNull();
    expect(container.textContent).toContain("default");
    await act(async () => button("Sonnet 5.5").click());
    expect(setupApi.saveReviewEngine).toHaveBeenCalledWith({
      provider: "claude",
      model: "claude-sonnet-5-5",
      effort: "medium",
    });
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === "High")!.click(),
    );
    expect(setupApi.saveReviewEngine).toHaveBeenLastCalledWith({
      provider: "claude",
      model: "claude-opus-5-5",
      effort: "high",
    });
    // Showing ChatGPT changes nothing until one of its accounts is chosen.
    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="ChatGPT"]')!.click());
    expect(setupApi.reorderAccounts).not.toHaveBeenCalled();
    expect(container.textContent).toContain("New reviews run on Claude · Claude");
    await act(async () => button("me@x.com").click());
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("shows Go retry backoff instead of quota reset in picker account options", async () => {
  const paused = {
    ...account(1, "opencode", "Go paused"),
    status: "limited" as const,
    limitedUntil: Date.now() + 3_600_000,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([paused, account(2, "opencode", "Go ready")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.querySelector("button[aria-haspopup]")).not.toBeNull());
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    let option!: HTMLButtonElement;
    await vi.waitFor(() => {
      option = [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Account"] [role="radio"]')].find(
        (item) => item.textContent?.includes("Go paused"),
      )!;
      expect(option).toBeTruthy();
    });
    await vi.waitFor(() => expect(option.textContent).toContain("Paused · retry in 1 h"));
    expect(option.title).toContain("Paused · retry in 1 h");
    expect(option.textContent).not.toContain("resets in");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("shows Go dynamically, preserves its valid effort, and hides effort controls for effortless models", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(3, "opencode", "Go account")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.saveReviewEngine).mockResolvedValue(engines);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    expect(container.querySelector('button[aria-label="OpenCode Go"]')).not.toBeNull();
    expect(container.textContent).toContain("Ultra");
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes("Go Compact"))!
        .click(),
    );
    expect(setupApi.saveReviewEngine).toHaveBeenCalledWith({
      provider: "opencode",
      model: "go-no-effort",
      effort: "",
      catalogueId: "go-catalogue-1",
    });
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("requires explicit selection for stale Go model and uses API catalogue defaults and id", async () => {
  const go = {
    ...engines.find((item) => item.provider === "opencode")!,
    model: "retired-model",
    effort: "retired-effort",
    label: "OpenCode Go · Retired model",
    catalogue: { status: "ready" as const, id: "catalogue-current", error: null },
    selectionValid: false,
    models: [
      { id: "discovered-model-91", label: "Adaptive 91", efforts: ["balanced", "deep"], defaultEffort: "balanced" },
      { id: "discovered-model-92", label: "Compact 92", efforts: [], defaultEffort: "" },
    ],
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(4, "opencode", "Go account")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([go]);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  vi.mocked(setupApi.saveReviewEngine).mockImplementation(async (choice) => [
    { ...go, model: choice.model, effort: choice.effort, selectionValid: true },
  ]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const modelButton = (label: string) =>
    [...container.querySelectorAll<HTMLButtonElement>('[role="radiogroup"][aria-label="Model"] [role="radio"]')].find(
      (item) => item.textContent?.includes(label),
    )!;
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.querySelector("button[aria-haspopup]")?.textContent).toContain("Choose"));
    expect(setupApi.saveReviewEngine).not.toHaveBeenCalled();
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    expect(modelButton("Adaptive 91").getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("Choose a model to enable reviews.");
    await act(async () => modelButton("Adaptive 91").click());
    await vi.waitFor(() =>
      expect(setupApi.saveReviewEngine).toHaveBeenCalledWith({
        provider: "opencode",
        model: "discovered-model-91",
        effort: "balanced",
        catalogueId: "catalogue-current",
      }),
    );
    await vi.waitFor(() =>
      expect(
        [...container.querySelectorAll<HTMLButtonElement>("button")].some((item) => item.textContent === "Deep"),
      ).toBe(true),
    );
    await act(async () => modelButton("Compact 92").click());
    await vi.waitFor(() =>
      expect(setupApi.saveReviewEngine).toHaveBeenLastCalledWith({
        provider: "opencode",
        model: "discovered-model-92",
        effort: "",
        catalogueId: "catalogue-current",
      }),
    );
    expect(container.querySelector('[aria-label="Effort"]')).toBeNull();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("polls only empty/loading Go catalogue states", () => {
  const go = engines.find((item) => item.provider === "opencode")!;
  expect(engineCataloguePollInterval([{ ...go, catalogue: { status: "empty", id: null, error: null } }])).toBe(1000);
  expect(engineCataloguePollInterval([{ ...go, catalogue: { status: "loading", id: null, error: null } }])).toBe(1000);
  expect(engineCataloguePollInterval([{ ...go, catalogue: { status: "ready", id: "catalogue", error: null } }])).toBe(
    false,
  );
  expect(engineCataloguePollInterval([{ ...go, catalogue: { status: "error", id: null, error: "Try again" } }])).toBe(
    false,
  );
});

it("shows Go catalogue loading without exposing stale models", async () => {
  const loadingGo = {
    ...engines.find((item) => item.provider === "opencode")!,
    model: "retired-model",
    models: [],
    catalogue: { status: "loading" as const, id: null, error: null },
    selectionValid: false,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(5, "opencode", "Go account")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([loadingGo]);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.querySelector("button[aria-haspopup]")).not.toBeNull());
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    await vi.waitFor(() => expect(container.textContent).toContain("Loading OpenCode Go models…"));
    expect(container.querySelectorAll('[role="radiogroup"][aria-label="Model"] [role="radio"]')).toHaveLength(0);
    expect(setupApi.saveReviewEngine).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("retries an errored Go catalogue and refreshes engine query", async () => {
  const errorGo = {
    ...engines.find((item) => item.provider === "opencode")!,
    models: [],
    catalogue: { status: "error" as const, id: null, error: "Catalogue unavailable. Retry." },
    selectionValid: false,
  };
  const readyGo = {
    ...engines.find((item) => item.provider === "opencode")!,
    catalogue: { status: "ready" as const, id: "catalogue-new", error: null },
    selectionValid: true,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(4, "opencode", "Go account")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValueOnce([errorGo]).mockResolvedValueOnce([readyGo]);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([]);
  vi.mocked(setupApi.refreshOpenCodeCatalogue).mockResolvedValue({ ok: true });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.querySelector("button[aria-haspopup]")).not.toBeNull());
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    await vi.waitFor(() => expect(container.textContent).toContain("Catalogue unavailable. Retry."));
    await act(async () =>
      [...container.querySelectorAll("button")].find((item) => item.textContent === "Retry")!.click(),
    );
    await vi.waitFor(() => expect(setupApi.refreshOpenCodeCatalogue).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(setupApi.reviewEngines).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(container.textContent).not.toContain("Catalogue unavailable. Retry."));
    expect(container.textContent).toContain("Go Default");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("switches to another account of the provider, resuming a paused one and skipping one at its limit", async () => {
  const paused = { ...account(2, "claude", "Brito"), enabled: false };
  // Paused and still at its limit: resuming it would not start reviews on it, so it can't be picked.
  const limited = {
    ...account(3, "claude", "Bruna"),
    enabled: false,
    status: "limited" as const,
    limitedUntil: Date.now() + 3_600_000,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1, "claude", "Bruno"), paused, limited]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([
    { accountId: 1, windows: [{ windowMins: 300, usedPercent: 97, resetsAt: null }] },
    { accountId: 2, windows: [{ windowMins: 300, usedPercent: 0, resetsAt: null }] },
  ]);
  vi.mocked(setupApi.updateAccount).mockResolvedValue([]);
  vi.mocked(setupApi.reorderAccounts).mockResolvedValue([
    { ...paused, enabled: true },
    account(1, "claude", "Bruno"),
    limited,
  ]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const radio = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Account"] [role="radio"]')].find((item) =>
      item.textContent?.includes(text),
    )!;
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(radio("Bruno").getAttribute("aria-checked")).toBe("true");
    expect(radio("Bruno").textContent).toContain("3% left");
    expect(radio("Brito").textContent).toContain("Paused · 100% left");
    expect(radio("Bruna").disabled).toBe(true);
    expect(radio("Bruna").textContent).toContain("Limit reached · resets in 1 h");
    expect(radio("Bruna").title).toContain("Usage limit reached");
    await act(async () => radio("Brito").click());
    expect(setupApi.updateAccount).toHaveBeenCalledWith(2, { enabled: true });
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1, 3]);

    await act(async () => container.querySelector<HTMLButtonElement>('button[aria-label="Add an account"]')!.click());
    expect(container.textContent).toContain("Add an account");
    expect(container.textContent).toContain("ChatGPT");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("says when a used-up account resets, before any review has hit its limit", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(1, "claude", "Bruno"), account(2, "claude", "Brisa")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.accountUsage).mockResolvedValue([
    { accountId: 1, windows: [{ windowMins: 300, usedPercent: 100, resetsAt: null }] },
    {
      accountId: 2,
      windows: [
        { windowMins: 300, usedPercent: 100, resetsAt: new Date(Date.now() + 3_600_000).toISOString() },
        { windowMins: 10_080, usedPercent: 100, resetsAt: Date.now() + 7_200_000 },
      ],
    },
  ]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const radio = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>('[aria-label="Account"] [role="radio"]')].find((item) =>
      item.textContent?.includes(text),
    )!;
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <EnginePicker />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    // Without a reset time there is nothing better to say than how much is left.
    expect(radio("Bruno").textContent).toContain("0% left");
    // Both windows used up: the account is back only when the later one resets.
    expect(radio("Brisa").textContent).toContain("Resets in 2 h");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});
