import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { setupApi } from "../services/setupApi";
import { OpenCodeKeySignIn } from "./AccessConnect";

vi.mock("../services/setupApi", () => ({
  setupApi: { opencodeKeySave: vi.fn() },
}));

afterEach(() => {
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it("posts key only on submit, disables duplicate submit, and completes connection", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let finish!: (value: { ok: true; accountId: number; signedIn: true }) => void;
  vi.mocked(setupApi.opencodeKeySave).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const done = vi.fn();
  const cancel = vi.fn();
  const client = new QueryClient();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <OpenCodeKeySignIn accountId={7} onDone={done} onCancel={cancel} />
        </QueryClientProvider>,
      ),
    );
    const input = container.querySelector<HTMLInputElement>('input[aria-label="OpenCode Go API key"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "synthetic-go-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () =>
      container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
    );
    expect(setupApi.opencodeKeySave).toHaveBeenCalledWith("synthetic-go-key", 7);
    await vi.waitFor(() =>
      expect(container.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true),
    );
    await act(async () =>
      container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
    );
    expect(setupApi.opencodeKeySave).toHaveBeenCalledOnce();
    await act(async () => finish({ ok: true, accountId: 7, signedIn: true }));
    await vi.waitFor(() => expect(done).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(input.value).toBe(""));
    expect(cancel).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});

it("retains key after validation failure without showing server error text", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(setupApi.opencodeKeySave).mockRejectedValue(new Error("unsafe diagnostic contains key=synthetic-go-key"));
  const client = new QueryClient();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <OpenCodeKeySignIn onDone={vi.fn()} onCancel={vi.fn()} />
        </QueryClientProvider>,
      ),
    );
    const input = container.querySelector<HTMLInputElement>('input[aria-label="OpenCode Go API key"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "synthetic-go-key");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () =>
      container.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })),
    );
    await vi.waitFor(() => expect(container.textContent).toContain("Could not save this API key"));
    expect(input.value).toBe("synthetic-go-key");
    expect(container.textContent).not.toContain("synthetic-go-key");
    expect(container.textContent).toContain("Could not save this API key");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    client.clear();
  }
});
