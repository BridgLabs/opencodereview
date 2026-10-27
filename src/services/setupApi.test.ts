import { afterEach, describe, expect, it, vi } from "vitest";
import { setupApi } from "./setupApi";

afterEach(() => vi.unstubAllGlobals());

describe("progress stream transport cleanup", () => {
  it("cancels the review response body on malformed events", async () => {
    const cancel = vi.fn();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("event: review-progress\ndata: invalid-json\n\n"));
      },
      cancel,
    });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(body)));
    await expect(setupApi.streamReviewProgress(1, new AbortController().signal, vi.fn())).rejects.toThrow();
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(body.locked).toBe(false);
  });

  it("releases the response body when a snapshot consumer throws", async () => {
    const cancel = vi.fn();
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode("event: review-progress\ndata: null\n\n"));
      },
      cancel,
    });
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(body)));
    await expect(
      setupApi.streamReviewProgress(1, new AbortController().signal, () => {
        throw new Error("consumer failed");
      }),
    ).rejects.toThrow("consumer failed");
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(body.locked).toBe(false);
  });

  it("keeps JSON requests and API error payloads stable", async () => {
    const fetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ theme: "system" }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetch);

    await setupApi.getSettings();

    expect(fetch).toHaveBeenCalledWith("http://localhost:8787/settings", {
      cache: "no-store",
      headers: { "content-type": "application/json" },
    });

    vi.mocked(fetch).mockResolvedValue(
      new Response(JSON.stringify({ error: "invalid settings" }), {
        status: 400,
        statusText: "Bad Request",
        headers: { "content-type": "application/json" },
      }),
    );
    await expect(setupApi.getSettings()).rejects.toThrow("invalid settings");
  });
});

describe("usage queries", () => {
  it("sends the range and timezone, and the repository only when filtered", async () => {
    const fetch = vi.fn().mockImplementation(async () => new Response("[]", { status: 200 }));
    vi.stubGlobal("fetch", fetch);
    const urls = () => fetch.mock.calls.map(([url]) => String(url).replace("http://localhost:8787", ""));

    await setupApi.usageReport({ days: 30, repoId: null, tz: -180 });
    await setupApi.usagePrs({ days: 7, repoId: 4, tz: 60 }, 25, 25);
    await setupApi.usagePrRuns(412, { days: 90, tz: 0 });

    expect(urls()).toEqual([
      "/usage?days=30&tz=-180",
      "/usage/prs?days=7&tz=60&repo=4&offset=25&limit=25",
      "/usage/prs/412/runs?days=90&tz=0",
    ]);
  });
});
