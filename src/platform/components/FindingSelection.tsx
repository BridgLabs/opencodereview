import { Lock } from "lucide-react";
import { type ReactNode, useId, useMemo, useState } from "react";
import { BrandLoading } from "../../components/BrandLoading";
import { DSButton, DSEmptyState, DSError } from "../../components/DesignSystem";
import { Markdown } from "../../components/Markdown";
import type { FindingAuditView } from "../../types";
import {
  candidateKind,
  findingAuditSummary,
  findingDisplayBody,
  findingTitle,
  groupFindingCandidates,
} from "../lib/findings";

export function FindingSelection({
  view,
  selectedIds,
  loading,
  busy,
  dirty,
  conflict,
  error,
  onToggle,
  onSave,
  onReset,
  children,
}: {
  view: FindingAuditView | undefined;
  selectedIds: string[];
  loading: boolean;
  busy: boolean;
  dirty: boolean;
  conflict: boolean;
  error: string | null;
  children?: ReactNode;
  onToggle: (id: string) => void;
  onSave: () => void;
  onReset: () => void;
}) {
  const selected = useMemo(() => new Set(selectedIds), [selectedIds]);
  const candidates = view?.audit?.candidates ?? [];
  const groups = groupFindingCandidates(candidates);
  const [tabId, setTabId] = useState("eligible");
  const panelId = useId();
  const group = groups.find((group) => group.id === tabId) ?? groups[0];
  const selectedOf = (kind: "finding" | "question") =>
    candidates.filter((candidate) => selected.has(candidate.id) && candidateKind(candidate) === kind).length;
  const editable = Boolean(view?.editable && !busy && !conflict && !error);
  return (
    <section className="finding-selection grid min-w-0 gap-3" aria-label="Review findings">
      <div className="finding-selection-intro grid gap-1.5 text-small text-fg-muted">
        <div className="finding-review-heading flex flex-wrap items-center justify-between gap-2 text-caption text-fg-subtle">
          <span>{error ? "Candidate audit could not be loaded." : findingAuditSummary(view)}</span>
          {view?.audit &&
            (view.editable ? (
              <span className="tabular-nums">
                {selectedOf("finding")} finding{selectedOf("finding") === 1 ? "" : "s"} · {selectedOf("question")}{" "}
                question{selectedOf("question") === 1 ? "" : "s"} selected
              </span>
            ) : (
              <span
                className="inline-flex items-center gap-1"
                title={!view.audit.complete ? "Audit incomplete. Selection unlocks after validation." : undefined}
              >
                <Lock className="size-3" aria-hidden="true" />
                {!view.audit.complete ? "Audit incomplete" : "Read-only"}
              </span>
            ))}
        </div>
        {candidates.some((candidate) => candidateKind(candidate) === "question") && (
          <p>
            Questions need confirmation, not a presumed fix. They are optional and cannot alone justify Request changes.
          </p>
        )}
        {view?.editable && candidates.length > 0 && (
          <p>Choose findings below, confirm them, then choose how to publish the review.</p>
        )}
      </div>
      {error && <DSError>{error}</DSError>}
      {conflict && (
        <DSError>
          The saved selection changed elsewhere. Reload it before saving; your draft has not been submitted.
        </DSError>
      )}
      <div className="finding-selection-list grid min-w-0 gap-3">
        {view?.audit ? (
          <>
            {candidates.length === 0 && (
              <DSEmptyState>
                {view.editable
                  ? "No finding candidates were generated. Choose a review decision below."
                  : "No finding candidates recorded for this run."}
              </DSEmptyState>
            )}
            {group && (
              <>
                {groups.length > 1 && (
                  <fieldset
                    className="m-0 min-w-0 border-0 p-0 finding-groups flex flex-wrap gap-1.5"
                    aria-label="Finding groups"
                  >
                    {groups.map((item) => (
                      <button
                        key={item.id}
                        type="button"
                        className={`finding-group-chip inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-full border px-2.5 text-caption font-medium transition-colors ${item.id === group.id ? "border-line-strong bg-surface-muted text-fg" : "border-line text-fg-muted hover:text-fg"}`}
                        aria-pressed={item.id === group.id}
                        aria-controls={panelId}
                        onClick={() => setTabId(item.id)}
                      >
                        {item.title}
                        <span className="text-fg-subtle tabular-nums">{item.candidates.length}</span>
                      </button>
                    ))}
                  </fieldset>
                )}
                <div id={panelId} className="finding-tab-panel grid min-w-0 gap-3" data-finding-group={group.id}>
                  <div className="finding-group-list grid min-w-0 gap-3">
                    {group.candidates.map((candidate) => {
                      const override = selected.has(candidate.id) !== candidate.defaultSelected;
                      const aiTag =
                        candidate.aiDecision === "kept"
                          ? null
                          : candidate.aiDecision === "unmapped"
                            ? "AI: decision unavailable"
                            : `AI: ${candidate.aiDecision}`;
                      return (
                        <article
                          key={candidate.id}
                          className={`finding-candidate grid min-w-0 gap-3 rounded-xl border border-line bg-surface p-4 pb-1 shadow-card transition-colors ${selected.has(candidate.id) ? "finding-candidate-selected border-brand/50 ring-1 ring-brand/20" : ""}`}
                        >
                          <label className="finding-candidate-choice flex cursor-pointer items-start gap-2.5 [&_input]:mt-0.5 [&_input]:size-4 [&_input]:flex-none [&_input]:accent-brand">
                            <input
                              type="checkbox"
                              aria-label={`Include ${candidateKind(candidate) === "question" ? "question" : "finding"} ${candidate.id}`}
                              checked={selected.has(candidate.id)}
                              disabled={!editable || Boolean(candidate.blockedReason)}
                              onChange={() => onToggle(candidate.id)}
                            />
                            <span className="min-w-0">
                              <span className="finding-candidate-title block text-small font-semibold text-fg [overflow-wrap:anywhere]">
                                {findingTitle(candidate)}
                              </span>
                              <span className="finding-candidate-location mt-1.5 inline-block max-w-full rounded-md bg-surface-muted px-1.5 py-0.5 font-mono text-caption text-fg-muted [overflow-wrap:anywhere]">
                                {candidate.comment
                                  ? `${candidate.comment.path ?? "Unknown path"}:${candidate.comment.line ?? "?"}`
                                  : candidateKind(candidate) === "question"
                                    ? "General review question"
                                    : "General findings in the review body"}
                                {candidate.chunkId !== null && ` · Chunk ${candidate.chunkId}`}
                              </span>
                            </span>
                          </label>
                          {(aiTag || override) && (
                            <div className="finding-candidate-tags flex flex-wrap gap-1.5 pl-6.5 [&_span]:rounded-full [&_span]:bg-surface-muted [&_span]:px-2 [&_span]:py-0.5 [&_span]:text-caption [&_span]:font-medium [&_span]:text-fg-muted">
                              {aiTag && <span>{aiTag}</span>}
                              {override && <span>Manual override</span>}
                            </div>
                          )}
                          {group.id !== "eligible" && (
                            <p className="finding-candidate-reason pl-6.5 text-small text-fg-muted [overflow-wrap:anywhere]">
                              {candidate.aiReason ??
                                (candidate.origin === "inherited"
                                  ? "No new AI pass ran; this came from prior review context."
                                  : candidate.aiDecision === "pending"
                                    ? "Awaiting the final integration decision."
                                    : candidate.aiDecision === "unmapped"
                                      ? "No reliable mapping was recorded. This candidate may have been rewritten or omitted; no discard reason is inferred."
                                      : "AI reason not recorded.")}
                            </p>
                          )}
                          {candidate.blockedReason && (
                            <p className="finding-candidate-blocked pl-6.5 text-small text-danger">
                              Cannot include: {candidate.blockedReason}
                            </p>
                          )}
                          <div className="finding-candidate-body min-w-0 pl-6.5 [overflow-wrap:anywhere]">
                            <Markdown>{findingDisplayBody(candidate.body)}</Markdown>
                            {group.id === "eligible" && (
                              <details className="finding-candidate-text disclosure text-small">
                                <summary>Why the AI kept it</summary>
                                <p className="finding-candidate-reason text-small text-fg-muted [overflow-wrap:anywhere]">
                                  {candidate.aiReason ?? "AI reason not recorded."}
                                </p>
                              </details>
                            )}
                            {candidate.originalBody && (
                              <details className="finding-candidate-text disclosure text-small">
                                <summary>Original finding from chunk</summary>
                                <Markdown>{candidate.originalBody}</Markdown>
                              </details>
                            )}
                          </div>
                        </article>
                      );
                    })}
                  </div>
                </div>
              </>
            )}
            {view.audit.originalBody && (
              <details className="finding-original-review disclosure text-small">
                <summary>Original AI summary</summary>
                <Markdown>{view.audit.originalBody}</Markdown>
              </details>
            )}
          </>
        ) : (
          <>
            {children}
            {loading && !error && <BrandLoading size="compact" label="Loading candidate audit…" />}
            {view && !loading && (
              <details className="finding-group min-w-0" data-finding-group="unavailable">
                <summary>
                  <span>Discarded findings</span>
                  <span>History unavailable</span>
                </summary>
                <p className="finding-candidate-reason text-small text-fg-muted [overflow-wrap:anywhere]">
                  No candidate audit was recorded for this run. Historical discard reasons cannot be reconstructed
                  reliably; this does not mean zero candidates were discarded.
                </p>
              </details>
            )}
          </>
        )}
      </div>
      {view?.editable && candidates.length > 0 && (
        <footer className="finding-selection-actions sticky -bottom-5 z-1 flex flex-wrap items-center justify-end gap-2 border-t border-line bg-surface py-3 [&>span]:min-w-30 [&>span]:flex-1 [&>span]:text-caption [&>span]:text-fg-subtle">
          <span>
            {dirty
              ? "Unsaved finding choices"
              : view.selectionSaved
                ? "Findings confirmed"
                : "Confirm findings before choosing an action"}
          </span>
          <DSButton disabled={busy} onClick={onReset}>
            {conflict ? "Reload selection" : "Reset to AI selection"}
          </DSButton>
          <DSButton tone="primary" disabled={!editable || (view.selectionSaved && !dirty)} onClick={onSave}>
            {busy ? "Confirming…" : "Confirm findings"}
          </DSButton>
        </footer>
      )}
    </section>
  );
}
