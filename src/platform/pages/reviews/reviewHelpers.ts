import type { Tone } from "../../../components/DesignSystem";
import type { ReviewCard, ReviewRisk, ReviewRun } from "../../../types";
import { formatError } from "../../lib/utils";

export type ReviewDecision = "approve_post" | "request_changes" | "comment" | "mark_reviewed" | "skip";

export function itemTime(value: number, now: number) {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "";
  const sameDay = date.toDateString() === new Date(now).toDateString();
  return sameDay
    ? date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "short", day: "numeric" });
}

/** Most recently updated first; cards without a usable time sink to the bottom. */
export function byRecent(items: ReviewCard[]) {
  const timestamp = (card: ReviewCard) => {
    const value = Number(card.updatedAt);
    return Number.isFinite(value) ? value : Number.NEGATIVE_INFINITY;
  };
  return [...items].sort((a, b) => timestamp(b) - timestamp(a) || b.id - a.id);
}

export function streamStatusLabel(state: string) {
  if (state === "live") return "Live via SSE";
  if (state === "connecting" || state === "reconnecting") return "Connecting live monitor";
  if (state === "fallback") return "Fallback polling";
  if (state === "closed") return "Process completed";
  return "Monitor idle";
}

interface InlineComment {
  path: string;
  line: number | string;
  body: string;
  status?: string;
  duplicateUrl?: string;
  duplicateAuthor?: string;
}

function cleanCommentBody(body: string) {
  return body
    .replace(/^<!-- ocr:fingerprint=.*?-->\n?/m, "")
    .replace(/\n{4,}/g, "\n\n\n")
    .trim();
}

export function parseComments(json: string | null): InlineComment[] {
  if (!json) return [];
  try {
    const parsed = JSON.parse(json);
    if (!Array.isArray(parsed?.comments)) return [];
    return parsed.comments.map((comment: Record<string, unknown>) => ({
      path: String(comment.path ?? ""),
      line: (comment.line as number | string) ?? "",
      body: cleanCommentBody(String(comment.body ?? "")),
      status: typeof comment.status === "string" ? comment.status : undefined,
      duplicateUrl: typeof comment.duplicateUrl === "string" ? comment.duplicateUrl : undefined,
      duplicateAuthor: typeof comment.duplicateAuthor === "string" ? comment.duplicateAuthor : undefined,
    }));
  } catch {
    return [];
  }
}

export const fmtTokens = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 100000 ? 0 : 1)}k` : `${n}`);
export const shortSha = (sha: string | null | undefined) => (sha ? sha.slice(0, 8) : "?");

export function durationLabel(run: ReviewRun) {
  if (!run.startedAt) return null;
  const seconds = Math.round(((run.finishedAt ?? Date.now()) - run.startedAt) / 1000);
  if (seconds < 0) return null;
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

export function parseRisk(run: ReviewRun): ReviewRisk | null {
  if (run.riskJson) {
    try {
      return JSON.parse(run.riskJson) as ReviewRisk;
    } catch {
      return null;
    }
  }
  if (run.riskScore === null || !run.riskLevel) return null;
  return {
    score: run.riskScore,
    level: run.riskLevel,
    summary: `${run.riskLevel} landing risk`,
    autoMergeEligible: false,
    drivers: [],
  };
}

export function reviewError(cause: unknown) {
  const message = formatError(cause);
  return message.includes("route not found")
    ? "Review API not available in the running server. Rebuild/restart the Docker backend so it picks up the new review routes."
    : message;
}

const LANE_LABELS: Record<string, string> = {
  manual_required: "Needs you",
  needs_setup: "Needs setup",
  ready: "Ready to post",
  reviewing: "Reviewing",
  queued: "Queued",
  open: "Open",
  draft: "Draft",
  auto_skipped: "Skipped",
  skipped: "Skipped",
  size_skipped: "Too large",
  auto_posted: "Findings posted",
  posted: "Findings posted",
  reviewed_cleanly: "No findings",
  approved: "Approved",
  merged: "Merged",
  closed: "Closed",
  failed: "Failed",
};

/** A short status for a review lane. */
export function labelLane(lane: string) {
  return LANE_LABELS[lane] ?? lane.replaceAll("_", " ");
}

/** Each outcome gets its own colour: posted findings are green, a clean review stays grey. */
export function laneTone(lane: string): Tone {
  if (["manual_required", "needs_setup", "ready"].includes(lane)) return "warning";
  if (["reviewing", "queued"].includes(lane)) return "brand";
  if (["auto_posted", "posted", "approved"].includes(lane)) return "success";
  if (lane === "merged") return "teal";
  if (lane === "failed") return "danger";
  return "neutral";
}
