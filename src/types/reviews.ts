export interface ReviewCard {
  id: number;
  repoId: number;
  repoFullName: string;
  number: number;
  title: string;
  author: string;
  authorAvatarUrl: string | null;
  state: string;
  lane: string;
  statusReason: string | null;
  /** Who produced the posted decision: "you" (manual) or "auto" (AI auto-post). */
  decidedBy: string | null;
  baseRef: string | null;
  headRef: string | null;
  baseSha: string | null;
  headSha: string;
  latestRunHeadSha: string | null;
  isStacked: boolean;
  url: string | null;
  updatedAt: number;
  resultState: "clean" | "awaiting_action" | "findings_local" | "findings_posted" | "skipped" | null;
  resultObservation: "clean" | "findings" | "no_new_changes" | null;
  resultObservedHeadSha: string | null;
  resultReviewedHeadSha: string | null;
  resultPriorHeadSha: string | null;
  resultPublicationStatus: "disabled" | "pending" | "posting" | "posted" | "failed" | "superseded" | null;
  resultError: string | null;
  resultInherited: boolean;
}

export interface ReviewBoardGroup {
  lane: string;
  title: string;
  items: ReviewCard[];
}

export interface FindingCandidate {
  id: string;
  kind?: "finding" | "question" | "mixed" | "unknown";
  origin: string;
  chunkId: number | null;
  aiDecision: "pending" | "kept" | "discarded" | "unmapped";
  aiReason: string | null;
  defaultSelected: boolean;
  selected: boolean;
  blockedReason: string | null;
  comment: { path?: string | null; line?: number | null; side?: string | null; body?: string | null } | null;
  body: string;
  originalBody: string | null;
}

export interface FindingAuditView {
  runId: number;
  prId: number;
  revision: number;
  requiresConfirmation: boolean;
  selectionSaved: boolean;
  editable: boolean;
  audit: {
    headSha: string;
    complete: boolean;
    originalBody: string;
    candidates: FindingCandidate[];
    evidenceWarnings?: { reason: string; path: string | null; reference: string | null }[] | null;
  } | null;
}

export interface ReviewChunkProgress {
  id: number;
  status: "queued" | "running" | "validating" | "done" | "cached" | "failed" | "cancelled";
  queuedAt: number;
  startedAt: number | null;
  executionFinishedAt: number | null;
  finishedAt: number | null;
  fileCount: number;
  cached: boolean;
  attempts: number;
  log: string;
  error: string | null;
}

export interface ReviewPhase {
  inferenceSkipped?: boolean;
  chunks?: ReviewChunkProgress[];
  queuedAt?: number;
  id: string;
  label: string;
  status: string;
  detail?: string;
  log?: string;
  startedAt?: number;
  finishedAt?: number;
  events?: Array<{ at: number; message: string }>;
}

export interface ReviewRiskDriver {
  label: string;
  detail: string;
  points: number;
}

export interface ReviewRisk {
  score: number;
  level: "low" | "medium" | "high";
  summary: string;
  autoMergeEligible: boolean;
  drivers: ReviewRiskDriver[];
}

export interface ReviewRun {
  id: number;
  prId: number;
  status: string;
  headSha: string;
  reviewBody: string | null;
  commentsJson: string | null;
  riskScore: number | null;
  riskLevel: "low" | "medium" | "high" | null;
  riskJson: string | null;
  errorMessage: string | null;
  phaseJson: string | null;
  tail: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  createdAt: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  usageModel: string | null;
  model: string | null;
  contextIndexSha: string | null;
  reviewedHeadSha: string | null;
}

export interface ReviewProgressSnapshot {
  runId: number;
  prId: number;
  status: string;
  headSha: string;
  errorMessage: string | null;
  phaseJson: string | null;
  tail: string | null;
  startedAt: number | null;
  finishedAt: number | null;
  updatedAt: number;
}

export interface UsageStats {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  runCount: number;
}

export interface ReviewRunStage {
  id: number;
  runId: number;
  prId: number;
  stage: "single" | "chunk" | "final" | string;
  chunkId: number | null;
  model: string | null;
  status: string;
  cacheKey: string | null;
  cacheHit: boolean;
  potentialCacheHit: boolean;
  potentialSavingsTokens: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  promptBytes: number;
  diffBytes: number;
  contextBytes: number;
  fileCount: number;
  relatedFileCount: number;
  createdAt: number;
  finishedAt: number | null;
}

export interface ReviewStageBreakdown {
  runId: number;
  prId: number;
  stages: ReviewRunStage[];
  totalTokens: number;
  potentialCacheHits: number;
  potentialCacheSavingsTokens: number;
}

export interface PendingAction {
  id: number;
  kind: string;
  status: "unseen" | "seen" | "resolved";
  title: string;
  message: string;
  repoFullName: string | null;
  prId: number | null;
  prNumber: number | null;
  prTitle: string | null;
  author: string | null;
  authorAvatarUrl: string | null;
  createdAt: number;
  seenAt: number | null;
  resolvedAt: number | null;
}
