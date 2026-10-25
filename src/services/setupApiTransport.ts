import { parseSseBuffer } from "../platform/lib/reviewEvents";
import type { Repo, RepoIndexStatus, ReviewProgressSnapshot } from "../types";

const httpBase = import.meta.env.VITE_SETUP_API_BASE ?? "http://localhost:8787";
const apiToken: string | undefined = import.meta.env.VITE_OCR_API_TOKEN;

export async function http<T>(path: string, options?: RequestInit): Promise<T> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (apiToken) {
    headers.authorization = `Bearer ${apiToken}`;
  }
  const response = await fetch(`${httpBase}${path}`, {
    cache: "no-store",
    ...options,
    headers: { ...headers, ...(options?.headers as Record<string, string> | undefined) },
  });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || response.statusText);
  return data as T;
}

export async function streamReviewProgress(
  prId: number,
  signal: AbortSignal,
  onSnapshot: (snapshot: ReviewProgressSnapshot | null) => void,
) {
  const headers: Record<string, string> = { accept: "text/event-stream" };
  if (apiToken) headers.authorization = `Bearer ${apiToken}`;
  const response = await fetch(`${httpBase}/reviews/${prId}/run/events`, { cache: "no-store", headers, signal });
  if (!response.ok) {
    let message = response.statusText || `Review progress stream failed (${response.status})`;
    try {
      const data = (await response.json()) as { error?: string };
      message = data.error || message;
    } catch {}
    throw new Error(message);
  }
  if (!response.body) throw new Error("Review progress stream is unavailable in this browser.");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const consume = (chunk: string) => {
    buffer += chunk;
    const parsed = parseSseBuffer(buffer);
    buffer = parsed.remainder;
    for (const message of parsed.messages) {
      if (message.event !== "review-progress") continue;
      onSnapshot(JSON.parse(message.data) as ReviewProgressSnapshot | null);
    }
  };
  try {
    while (true) {
      const next = await reader.read();
      if (next.done) break;
      consume(decoder.decode(next.value, { stream: true }));
    }
    const trailing = decoder.decode();
    if (trailing) consume(trailing);
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

export interface IndexJob {
  repo: Repo;
  status: RepoIndexStatus;
}

export interface IndexJobsResponse {
  active: number;
  jobs: IndexJob[];
}
