import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setupApi } from "../../services/setupApi";
import type { AiAccount, ReviewEngine } from "../../types";
import { ProvidersPanel } from "./ProvidersPanel";

vi.mock("../../services/setupApi", () => ({
  setupApi: {
    listAccounts: vi.fn(),
    accountUsage: vi.fn(() => Promise.resolve([])),
    reviewEngines: vi.fn(),
    saveReviewEngine: vi.fn(),
    reorderAccounts: vi.fn(),
    updateAccount: vi.fn(),
    deleteAccount: vi.fn(),
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
const engine = (provider: AiAccount["provider"], name: string): ReviewEngine => ({
  provider,
  name,
  model: "m1",
  effort: "medium",
  label: provider === "opencode" ? "m1" : `${name} · Model 1 · medium`,
  models: [
    { id: "m1", label: "Model 1", efforts: ["low", "medium", "high"], defaultEffort: "medium" },
    { id: "m2", label: "Model 2", efforts: ["low", "medium"], defaultEffort: "low" },
  ],
  ...(provider === "opencode"
    ? { catalogue: { status: "ready" as const, id: "go-catalogue-1", error: null }, selectionValid: true }
    : {}),
});

beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

it("lists connected providers in review order and opens one to set its model, effort and accounts", async () => {
  const accounts = [account(1, "codex", "me@x.com"), account(2, "claude", "Work"), account(3, "codex", "alt@x.com")];
  const engines = [engine("claude", "Claude"), engine("codex", "ChatGPT")];
  vi.mocked(setupApi.listAccounts).mockResolvedValue(accounts);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue(engines);
  vi.mocked(setupApi.saveReviewEngine).mockResolvedValue(engines);
  vi.mocked(setupApi.reorderAccounts).mockResolvedValue(accounts);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const settle = () =>
    act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  const button = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent?.includes(text))!;
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await settle();
    const rows = [...container.querySelectorAll("button")].map((item) => item.textContent);
    expect(rows.findIndex((text) => text?.includes("ChatGPT"))).toBeLessThan(
      rows.findIndex((text) => text?.includes("Claude")),
    );
    expect(button("ChatGPT").textContent).toContain("Next review");
    expect(button("ChatGPT").textContent).toContain("2 accounts · Model 1 · medium");

    await act(async () => button("Claude").click());
    await settle();
    expect(container.textContent).toContain("Work");
    expect(container.textContent).not.toContain("me@x.com");
    await act(async () => button("high").click());
    expect(setupApi.saveReviewEngine).toHaveBeenCalledWith({ provider: "claude", model: "m1", effort: "high" });
    await act(async () => button("Use for new reviews").click());
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1, 3]);
    await act(async () => button("Providers").click());
    expect(container.textContent).toContain("Connected");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("lists Go from the API catalogue and shows its data-use notice", async () => {
  const accountGo = account(9, "opencode", "Go key");
  const goEngine = engine("opencode", "OpenCode Go");
  vi.mocked(setupApi.listAccounts).mockResolvedValue([accountGo]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([goEngine]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(container.textContent).toContain("OpenCode Go");
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes("OpenCode Go"))!
        .click(),
    );
    expect(container.textContent).toContain("Review code and findings are sent to OpenCode Go for review.");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("requires an explicit Settings choice for stale Go model and saves API model, effort, and catalogue ID", async () => {
  const stale = {
    ...engine("opencode", "OpenCode Go"),
    model: "retired-go-model",
    effort: "old-effort",
    catalogue: { status: "ready" as const, id: "catalogue-live", error: null },
    selectionValid: false,
    models: [
      { id: "live-model-x", label: "Live Model X", efforts: ["balanced", "deep"], defaultEffort: "balanced" },
      { id: "live-model-y", label: "Live Model Y", efforts: [], defaultEffort: "" },
    ],
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(20, "opencode", "Go key")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([stale]);
  vi.mocked(setupApi.saveReviewEngine).mockImplementation(async (choice) => [
    { ...stale, model: choice.model, effort: choice.effort, selectionValid: true },
  ]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const changeModel = async (value: string) => {
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="OpenCode Go model"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, value);
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  };
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("OpenCode Go"));
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("OpenCode Go"))!
        .click(),
    );
    await vi.waitFor(() => expect(container.querySelector('select[aria-label="OpenCode Go model"]')).not.toBeNull());
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="OpenCode Go model"]')!;
    expect(select.value).toBe("");
    expect(select.options[0].textContent).toBe("Choose a model…");
    const accountRow = container.querySelector<HTMLElement>(".account-row")!;
    expect(accountRow.textContent).toContain("Model required");
    expect(accountRow.textContent).not.toContain("Next review");
    expect(setupApi.saveReviewEngine).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Choose a model to enable reviews.");
    await changeModel("live-model-x");
    await vi.waitFor(() =>
      expect(setupApi.saveReviewEngine).toHaveBeenLastCalledWith({
        provider: "opencode",
        model: "live-model-x",
        effort: "balanced",
        catalogueId: "catalogue-live",
      }),
    );
    await vi.waitFor(() =>
      expect(
        [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "deep"),
      ).toBeDefined(),
    );
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "deep")!
        .click(),
    );
    await vi.waitFor(() =>
      expect(setupApi.saveReviewEngine).toHaveBeenLastCalledWith({
        provider: "opencode",
        model: "live-model-x",
        effort: "deep",
        catalogueId: "catalogue-live",
      }),
    );
    await changeModel("live-model-y");
    await vi.waitFor(() =>
      expect(setupApi.saveReviewEngine).toHaveBeenLastCalledWith({
        provider: "opencode",
        model: "live-model-y",
        effort: "",
        catalogueId: "catalogue-live",
      }),
    );
    await vi.waitFor(() => expect(container.querySelector('[aria-label="Effort"]')).toBeNull());
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("retries Go catalogue from provider settings and reloads catalogue", async () => {
  const unavailable = {
    ...engine("opencode", "OpenCode Go"),
    models: [],
    catalogue: { status: "error" as const, id: null, error: "Catalogue unavailable. Retry." },
    selectionValid: false,
  };
  const ready = {
    ...engine("opencode", "OpenCode Go"),
    catalogue: { status: "ready" as const, id: "catalogue-reloaded", error: null },
    selectionValid: true,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(21, "opencode", "Go key")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValueOnce([unavailable]).mockResolvedValueOnce([ready]);
  vi.mocked(setupApi.refreshOpenCodeCatalogue).mockResolvedValue({ ok: true });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("OpenCode Go"));
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("OpenCode Go"))!
        .click(),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("Catalogue unavailable. Retry."));
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "Retry")!
        .click(),
    );
    await vi.waitFor(() => expect(setupApi.refreshOpenCodeCatalogue).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(setupApi.reviewEngines).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(container.textContent).not.toContain("Catalogue unavailable. Retry."));
    expect(setupApi.saveReviewEngine).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("shows loading Go catalogue in provider settings without saving a stale model", async () => {
  const loading = {
    ...engine("opencode", "OpenCode Go"),
    model: "retired-go-model",
    models: [],
    catalogue: { status: "loading" as const, id: null, error: null },
    selectionValid: false,
  };
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(22, "opencode", "Go key")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([loading]);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("OpenCode Go"));
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent?.includes("OpenCode Go"))!
        .click(),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("Loading OpenCode Go models…"));
    expect(container.querySelector('select[aria-label="OpenCode Go model"]')).toBeNull();
    expect(setupApi.saveReviewEngine).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("keeps provider connection choices available when engine discovery fails", async () => {
  vi.mocked(setupApi.listAccounts).mockResolvedValue([]);
  vi.mocked(setupApi.reviewEngines).mockRejectedValue(new Error("catalogue unavailable"));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("OpenCode Go API key"));
    expect(container.textContent).toContain("Claude Pro, Max or Team plan");
    expect(container.textContent).toContain("ChatGPT Plus or Pro plan");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("shows rejected model save errors", async () => {
  const go = engine("opencode", "OpenCode Go");
  vi.mocked(setupApi.listAccounts).mockResolvedValue([account(23, "opencode", "Go key")]);
  vi.mocked(setupApi.reviewEngines).mockResolvedValue([go]);
  vi.mocked(setupApi.saveReviewEngine).mockRejectedValue(new Error("Catalogue changed; refresh and confirm."));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <ProvidersPanel />
        </QueryClientProvider>,
      ),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("OpenCode Go"));
    await act(async () =>
      [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find((item) => item.textContent?.includes("OpenCode Go"))!
        .click(),
    );
    await act(async () => {
      const select = container.querySelector<HTMLSelectElement>('select[aria-label="OpenCode Go model"]')!;
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "m2");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await vi.waitFor(() => expect(container.textContent).toContain("Catalogue changed; refresh and confirm."));
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});
