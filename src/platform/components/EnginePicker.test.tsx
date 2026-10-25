import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { setupApi } from "../../services/setupApi";
import type { AiAccount, ReviewEngine } from "../../types";
import { EnginePicker } from "./EnginePicker";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to?: string }) => <a href={to}>{children}</a>,
}));
vi.mock("../../services/setupApi", () => ({
  setupApi: {
    listAccounts: vi.fn(),
    reviewEngines: vi.fn(),
    saveReviewEngine: vi.fn(),
    reorderAccounts: vi.fn(),
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
    provider: "gemini",
    name: "Gemini",
    model: "auto",
    effort: "",
    label: "Gemini · Auto",
    models: [{ id: "auto", label: "Auto", efforts: [], defaultEffort: "" }],
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
    expect(container.textContent).toContain("Medium");
    await act(async () => container.querySelector<HTMLButtonElement>("button[aria-haspopup]")!.click());
    // The rail holds the signed-in accounts; the models are the next account's provider's.
    expect(container.querySelector('[aria-label="ChatGPT · me@x.com"]')).not.toBeNull();
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
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="ChatGPT · me@x.com"]')!.click());
    expect(setupApi.reorderAccounts).toHaveBeenCalledWith([2, 1]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});
