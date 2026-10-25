import { type ReactNode, useId, useMemo, useState } from "react";
import { BrandLoading } from "../../components/BrandLoading";
import { DSButton, DSEmptyState, DSError } from "../../components/DesignSystem";
import { Markdown } from "../../components/Markdown";
import type { FindingAuditView } from "../../types";
import { candidateKind, findingAuditSummary, findingTitle, groupFindingCandidates } from "../lib/findings";
import { type ArrowTab, ArrowTabs } from "./ArrowTabs";

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
  const tabs: ArrowTab[] = groups.map((group) => ({
    id: group.id,
    label: group.title,
    count: group.candidates.length,
  }));
  if (view?.audit?.originalBody) tabs.push({ id: "summary", label: "Original AI summary" });
  const activeTab = tabs.find((tab) => tab.id === tabId) ?? tabs[0];
  const group = groups.find((group) => group.id === activeTab?.id);
  const editable = Boolean(view?.editable && !busy && !conflict && !error);
  return (
    <section className="finding-selection grid min-w-0 gap-3" aria-label="Review findings">
      <div className="finding-selection-intro [&_h3]:text-heading [&_h3]:text-fg [&_p]:mt-1.5 [&_p]:text-small [&_p]:text-fg-muted">
        <div className="finding-review-heading flex flex-wrap items-center justify-between gap-2 [&>span]:text-caption [&>span]:text-fg-subtle">
          <h3>Findings & questions</h3>
          {view?.audit && (
            <span>
              {
                candidates.filter((candidate) => selected.has(candidate.id) && candidateKind(candidate) === "finding")
                  .length
              }{" "}
              findings ·{" "}
              {
                candidates.filter((candidate) => selected.has(candidate.id) && candidateKind(candidate) === "question")
                  .length
              }{" "}
              questions selected
            </span>
          )}
        </div>
        {candidates.some((candidate) => candidateKind(candidate) === "question") && (
          <p>
            Questions need confirmation, not a presumed fix. They are optional and cannot alone justify Request changes.
          </p>
        )}
        <p>{error ? "Candidate audit could not be loaded." : findingAuditSummary(view)}</p>
        {view?.editable && candidates.length > 0 && (
          <p>Choose findings below, confirm them, then choose how to publish the review.</p>
        )}
        {view?.audit && !view.editable && (
          <p>{!view.audit.complete ? "Audit incomplete. Selection unlocks after validation." : "Read-only review."}</p>
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
            {activeTab && (
              <>
                <ArrowTabs
                  tabs={tabs}
                  activeId={activeTab.id}
                  panelId={panelId}
                  label="Finding groups"
                  previousLabel="Previous finding group"
                  nextLabel="Next finding group"
                  onChange={setTabId}
                />
                <div
                  id={panelId}
                  role="tabpanel"
                  // biome-ignore lint/a11y/noNoninteractiveTabindex: WAI-ARIA tabs pattern: the panel is focusable so keyboard users can reach its content after the tab list
                  tabIndex={0}
                  aria-labelledby={`${panelId}-tab-${activeTab.id}`}
                  className="finding-tab-panel grid min-w-0 gap-3 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-brand"
                  data-finding-group={activeTab.id}
                >
                  {group ? (
                    <>
                      <p className="finding-tab-selection-count text-caption text-fg-subtle">
                        {group.candidates.filter((candidate) => selected.has(candidate.id)).length} selected in this
                        group
                      </p>
                      <div className="finding-group-list grid min-w-0 gap-3">
                        {group.candidates.map((candidate) => (
                          <article
                            key={candidate.id}
                            className={`finding-candidate min-w-0 rounded-xl border border-line bg-surface p-4 shadow-card transition-colors ${selected.has(candidate.id) ? "finding-candidate-selected border-brand/50 ring-1 ring-brand/20" : ""}`}
                          >
                            <label className="finding-candidate-choice flex cursor-pointer items-start gap-2.5 [&_input]:mt-0.5 [&_input]:size-4 [&_input]:flex-none [&_input]:accent-brand">
                              <input
                                type="checkbox"
                                aria-label={`Include ${candidateKind(candidate) === "question" ? "question" : "finding"} ${candidate.id}`}
                                checked={selected.has(candidate.id)}
                                disabled={!editable || Boolean(candidate.blockedReason)}
                                onChange={() => onToggle(candidate.id)}
                              />
                              <span>
                                <span className="finding-candidate-title block text-small font-semibold text-fg [overflow-wrap:anywhere]">
                                  {findingTitle(candidate)}
                                </span>
                                <span className="finding-candidate-location mt-1 block font-mono text-caption text-fg-subtle [overflow-wrap:anywhere]">
                                  {candidate.comment
                                    ? `${candidate.comment.path ?? "Unknown path"}:${candidate.comment.line ?? "?"} · ${candidate.comment.side ?? "?"}`
                                    : candidateKind(candidate) === "question"
                                      ? "General review question"
                                      : "General findings in the review body (group)"}
                                  {candidate.chunkId !== null
                                    ? ` · Chunk ${candidate.chunkId}`
                                    : ` · ${candidate.origin}`}
                                </span>
                              </span>
                            </label>
                            <div className="finding-candidate-tags mt-3 flex flex-wrap gap-1.5 [&_span]:rounded-full [&_span]:bg-slate-100 [&_span]:px-2 [&_span]:py-0.5 [&_span]:text-caption [&_span]:font-medium [&_span]:text-slate-600">
                              <span>
                                AI:{" "}
                                {candidate.aiDecision === "unmapped"
                                  ? "decision unavailable"
                                  : candidate.aiDecision === "kept"
                                    ? "selected"
                                    : candidate.aiDecision}
                              </span>
                              {selected.has(candidate.id) !== candidate.defaultSelected && <span>Manual override</span>}
                            </div>
                            {group.id !== "eligible" && (
                              <p className="finding-candidate-reason text-small text-fg-muted [overflow-wrap:anywhere]">
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
                              <p className="finding-candidate-blocked text-small text-danger">
                                Cannot include: {candidate.blockedReason}
                              </p>
                            )}
                            <div className="finding-candidate-body mt-3 [overflow-wrap:anywhere]">
                              <Markdown>{candidate.body}</Markdown>
                            </div>
                            {group.id === "eligible" && (
                              <details className="finding-candidate-text mt-2.5 [&_summary]:cursor-pointer [&_summary]:text-caption [&_summary]:font-semibold [&_summary]:text-fg-muted [&>div]:mt-2.5">
                                <summary>AI decision</summary>
                                <p className="finding-candidate-reason text-small text-fg-muted [overflow-wrap:anywhere]">
                                  {candidate.aiReason ?? "AI reason not recorded."}
                                </p>
                              </details>
                            )}
                            {candidate.originalBody && (
                              <details className="finding-candidate-text mt-2.5 [&_summary]:cursor-pointer [&_summary]:text-caption [&_summary]:font-semibold [&_summary]:text-fg-muted [&>div]:mt-2.5">
                                <summary>Original finding from chunk</summary>
                                <Markdown>{candidate.originalBody}</Markdown>
                              </details>
                            )}
                          </article>
                        ))}
                      </div>
                    </>
                  ) : (
                    <div className="finding-original-review [&_summary]:cursor-pointer [&_summary]:text-caption [&_summary]:font-semibold [&>div]:mt-2.5">
                      <Markdown>{view.audit.originalBody ?? ""}</Markdown>
                    </div>
                  )}
                </div>
              </>
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
