/** Indexed package dependency graph (internal packages) for the interactive view. */
export interface IndexGraphNode {
  id: string;
  name: string;
  path: string;
  kind: string;
}
export interface IndexGraphEdge {
  source: string;
  target: string;
  dev: boolean;
}
export interface IndexGraph {
  nodes: IndexGraphNode[];
  edges: IndexGraphEdge[];
}

export type RepoIndexState =
  | "queued"
  | "not_indexed"
  | "indexing"
  | "ready"
  | "stale"
  | "failed"
  | "cancelled"
  | "empty";

export interface RepoIndexStatus {
  repoId: number;
  status: RepoIndexState;
  stage: string | null;
  baseRef: string | null;
  baseSha: string | null;
  remoteBaseSha: string | null;
  specVersion: number;
  fileCount: number;
  packageCount: number;
  edgeCount: number;
  symbolCount: number;
  skippedCount: number;
  totalCount: number;
  startedAt: number | null;
  indexedAt: number | null;
  durationMs: number | null;
  errorMessage: string | null;
  updatedAt: number;
}

export interface ContextPackage {
  packagePath: string;
  name: string;
  kind: string;
}

export interface ContextFile {
  path: string;
  packagePath: string | null;
  reason: string;
  source: string;
}

export interface ReviewContextPlan {
  baseRef?: string | null;
  reviewedBaseSha?: string | null;
  mergeBaseSha?: string | null;
  scopeReason?: string | null;
  scopeSelection?: {
    mode: "full" | "incremental" | "base_sync" | "content_reuse" | "reprocess";
    requestedFull: boolean;
    sourceRunId: number | null;
    fullPrFiles: string[];
    reusedFiles: string[];
    excludedFiles?: Array<{ path: string; reason: string }>;
  } | null;
  repoId: number;
  changedFiles: string[];
  packages: ContextPackage[];
  dependentPackages: string[];
  relatedFiles: ContextFile[];
  testFiles: string[];
  conventionFiles: string[];
  omittedCount: number;
  incremental: boolean;
  range: string;
  headSha: string | null;
  baseSha: string | null;
  priorSha: string | null;
}
