import type {
  FindingAuditView,
  PendingAction,
  PollStatus,
  ReviewBoardGroup,
  ReviewContextPlan,
  ReviewRun,
  ReviewStageBreakdown,
  UsageStats,
} from "../types";
import { http, streamReviewProgress } from "./setupApiTransport";
export const setupApiReviews = {
  pollReviews: () => http<PollStatus>("/reviews/poll", { method: "POST" }),
  pollStatus: () => http<PollStatus>("/reviews/poll/status"),
  listPendingActions: () => http<PendingAction[]>("/pending"),
  markPrReviewSeen: (prId: number) =>
    http<{ ok: boolean }>(`/reviews/${prId}/seen`, { method: "POST" }).then(() => undefined),
  listReviewBoard: () => http<ReviewBoardGroup[]>("/reviews/board"),
  startReview: (prId: number, forceFull = false) =>
    http<{ ok: boolean }>(`/reviews/${prId}/start`, { method: "POST", body: JSON.stringify({ forceFull }) }).then(
      () => undefined,
    ),
  cancelReview: (prId: number) =>
    http<{ ok: boolean }>(`/reviews/${prId}/cancel`, { method: "POST" }).then(() => undefined),
  decideReview: (
    prId: number,
    decision: "approve_post" | "request_changes" | "comment" | "mark_reviewed" | "skip",
    runId?: number,
    selectionRevision?: number,
  ) =>
    http<{ ok: boolean }>(`/reviews/${prId}/decide`, {
      method: "POST",
      body: JSON.stringify({ decision, runId, selectionRevision }),
    }).then(() => undefined),
  getReviewFindingAudit: (prId: number, runId: number) =>
    http<FindingAuditView>(`/reviews/${prId}/runs/${runId}/findings`),
  saveReviewFindingSelection: (prId: number, runId: number, revision: number, selectedIds: string[]) =>
    http<FindingAuditView>(`/reviews/${prId}/runs/${runId}/findings/selection`, {
      method: "POST",
      body: JSON.stringify({ revision, selectedIds }),
    }),
  retryReviewResult: (prId: number) =>
    http<{ ok: boolean }>(`/reviews/${prId}/result/retry`, { method: "POST", body: JSON.stringify({}) }).then(
      () => undefined,
    ),
  getReviewRun: (prId: number) => http<ReviewRun | null>(`/reviews/${prId}/run`),
  streamReviewProgress,
  getReviewStageBreakdown: (prId: number, runId: number) =>
    http<ReviewStageBreakdown>(`/reviews/${prId}/runs/${runId}/stages`),
  getReviewRunContext: (prId: number, runId: number) =>
    http<ReviewContextPlan | null>(`/reviews/${prId}/runs/${runId}/context`),
  getPrUsage: (prId: number) => http<UsageStats>(`/reviews/${prId}/usage`),
};
