import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { setupApi } from "../../services/setupApi";
import type { ReviewProgressSnapshot, ReviewRun } from "../../types";
import { useReviewRunStream } from "./useReviewRunStream";

vi.mock("../../services/setupApi", () => ({ setupApi: { getReviewRun: vi.fn(), streamReviewProgress: vi.fn() } }));

function Monitor({ id = 1 }: { id?: number | null }) {
  const query = useReviewRunStream(id);
  return <p>{query.streamState}</p>;
}

const run = { id: 10, prId: 1, status: "running", headSha: "head" } as ReviewRun;

describe("review progress connection recovery", () => {
  let client: QueryClient;
  let container: HTMLDivElement;
  let root: ReturnType<typeof createRoot>;
  const signals: AbortSignal[] = [];
  const callbacks: Array<(snapshot: ReviewProgressSnapshot | null) => void> = [];
  const render = async (id: number | null = 1) => {
    await act(async () =>
      root.render(
        <QueryClientProvider client={client}>
          <Monitor id={id} />
        </QueryClientProvider>,
      ),
    );
  };
  const advance = async (ms: number) => {
    await act(async () => vi.advanceTimersByTimeAsync(ms));
  };
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    vi.resetAllMocks();
    signals.length = 0;
    callbacks.length = 0;
    vi.mocked(setupApi.getReviewRun).mockResolvedValue(run);
    vi.mocked(setupApi.streamReviewProgress).mockImplementation((_id, signal, callback) => {
      signals.push(signal);
      callbacks.push(callback);
      return new Promise<void>((resolve) => signal.addEventListener("abort", () => resolve(), { once: true }));
    });
    client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
    for (let id = 1; id <= 5; id++) client.setQueryData(["reviews", id, "run"], { ...run, id: id * 10, prId: id });
    container = document.createElement("div");
    root = createRoot(container);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    expect(signals.every((signal) => signal.aborted)).toBe(true);
    client.clear();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.resetAllMocks();
  });

  it.each([null, "preview", "posted", "approved", "failed", "cancelled", "superseded"])(
    "does not open a stream for recorded status %s",
    async (status) => {
      client.setQueryData(["reviews", 1, "run"], status ? { ...run, status } : null);
      await render();
      expect(setupApi.streamReviewProgress).not.toHaveBeenCalled();
      expect(container.textContent).toBe("closed");
    },
  );

  it("aborts previous subscriptions and ignores late snapshots while switching five PRs", async () => {
    for (let index = 0; index < 20; index++) {
      await render((index % 5) + 1);
      expect(signals.filter((signal) => !signal.aborted)).toHaveLength(1);
    }
    await act(async () =>
      callbacks[0]({ ...run, runId: 10, status: "failed", updatedAt: 2 } as ReviewProgressSnapshot),
    );
    expect(client.getQueryData<ReviewRun>(["reviews", 1, "run"])?.status).toBe("running");
    await render(null);
    expect(signals.every((signal) => signal.aborted)).toBe(true);
    expect(container.textContent).toBe("idle");
  });

  it("keeps fallback polling active while a reconnection is pending", async () => {
    vi.mocked(setupApi.streamReviewProgress).mockRejectedValueOnce(new Error("too many review progress streams"));
    await render();
    expect(container.textContent).toBe("fallback");
    await advance(2500);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(2);
    expect(container.textContent).toBe("fallback");
    const reads = vi.mocked(setupApi.getReviewRun).mock.calls.length;
    await advance(2000);
    expect(vi.mocked(setupApi.getReviewRun).mock.calls.length).toBeGreaterThan(reads);
    await act(async () => callbacks[0]({ ...run, runId: 10, updatedAt: 20 } as ReviewProgressSnapshot));
    expect(container.textContent).toBe("live");
  });

  it("uses fallback after unexpected EOF instead of leaving the canvas stale", async () => {
    vi.mocked(setupApi.streamReviewProgress).mockResolvedValueOnce(undefined);
    await render();
    expect(container.textContent).toBe("fallback");
    await advance(1600);
    expect(setupApi.getReviewRun).toHaveBeenCalled();
  });

  it("does not reconnect after an empty snapshot and discovers a later run by polling", async () => {
    vi.mocked(setupApi.getReviewRun).mockResolvedValue(null);
    vi.mocked(setupApi.streamReviewProgress).mockImplementationOnce(async (_id, _signal, callback) => callback(null));
    await render();
    await advance(4500);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe("closed");
    vi.mocked(setupApi.getReviewRun).mockResolvedValue({ ...run, id: 11 });
    await advance(6000);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(2);
  });

  it("closes on terminal events, fetches final details and ignores subsequent frames", async () => {
    vi.mocked(setupApi.getReviewRun).mockResolvedValue({ ...run, status: "preview", reviewBody: "Final details" });
    await render();
    await act(async () => {
      callbacks[0]({ ...run, runId: 10, status: "preview", updatedAt: 20 } as ReviewProgressSnapshot);
      callbacks[0]({ ...run, runId: 10, updatedAt: 10 } as ReviewProgressSnapshot);
    });
    await advance(1);
    expect(signals[0].aborted).toBe(true);
    expect(container.textContent).toBe("closed");
    expect(client.getQueryData<ReviewRun>(["reviews", 1, "run"])?.reviewBody).toBe("Final details");
    await advance(12000);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(1);
  });

  it("stops a pending reconnection when fallback polling observes completion", async () => {
    vi.mocked(setupApi.streamReviewProgress).mockRejectedValueOnce(new Error("HTTP 429"));
    await render();
    await advance(2500);
    vi.mocked(setupApi.getReviewRun).mockResolvedValue({ ...run, status: "failed" });
    await advance(2000);
    expect(signals[0].aborted).toBe(true);
    expect(container.textContent).toBe("closed");
    await advance(12000);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(2);
  });

  it("clears reconnect timers when no process is selected", async () => {
    vi.mocked(setupApi.streamReviewProgress).mockRejectedValueOnce(new Error("HTTP 429"));
    await render();
    await render(null);
    await advance(30000);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(1);
    expect(container.textContent).toBe("idle");
  });

  it("reopens monitoring when polling discovers a run after a terminal result", async () => {
    client.setQueryData(["reviews", 1, "run"], { ...run, status: "preview" });
    await render();
    expect(setupApi.streamReviewProgress).not.toHaveBeenCalled();
    vi.mocked(setupApi.getReviewRun).mockResolvedValue({ ...run, id: 11 });
    await advance(5100);
    expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(1);
  });
});

describe("review progress stream lifecycle", () => {
  it("waits for the initial run query before opening SSE", async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    let resolveRun!: (value: ReviewRun | null) => void;
    vi.mocked(setupApi.getReviewRun).mockReturnValue(
      new Promise((resolve) => {
        resolveRun = resolve;
      }),
    );
    const signals: AbortSignal[] = [];
    vi.mocked(setupApi.streamReviewProgress).mockImplementation((_prId, signal) => {
      signals.push(signal);
      return new Promise<void>((resolve) => signal.addEventListener("abort", () => resolve(), { once: true }));
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () =>
        root.render(
          <QueryClientProvider client={client}>
            <Monitor />
          </QueryClientProvider>,
        ),
      );
      expect(setupApi.streamReviewProgress).not.toHaveBeenCalled();
      await act(async () => {
        resolveRun(run);
        await new Promise((resolve) => setTimeout(resolve, 0));
        await new Promise((resolve) => setTimeout(resolve, 0));
      });
      expect(setupApi.streamReviewProgress).toHaveBeenCalledTimes(1);
    } finally {
      await act(async () => root.unmount());
      expect(signals[0]?.aborted).toBe(true);
      client.clear();
      vi.unstubAllGlobals();
    }
  });
});
