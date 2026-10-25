import { describe, expect, it, vi } from "vitest";
import type { FindingAuditView, FindingCandidate } from "../../types";
import {
  canDecideFindings,
  type FindingSelectionDraft,
  findingAuditSummary,
  findingSelectionDirty,
  groupFindingCandidates,
  prepareEmptyFindingDecision,
  reviewBodyHasPostableFinding,
  reviewPointKind,
  selectedFindingIds,
} from "./findings";

const candidate = (id: string, selected: boolean, blockedReason: string | null = null): FindingCandidate => ({
  id,
  selected,
  defaultSelected: selected,
  blockedReason,
  origin: "chunk",
  chunkId: 1,
  aiDecision: selected ? "kept" : "discarded",
  aiReason: null,
  body: "[Finding] Example",
  originalBody: null,
  comment: { path: "src/example.ts", line: 1 },
});
const view: FindingAuditView = {
  runId: 10,
  prId: 1,
  revision: 2,
  requiresConfirmation: true,
  selectionSaved: false,
  editable: true,
  audit: {
    headSha: "abc",
    complete: true,
    originalBody: "Summary",
    candidates: [candidate("a", true), candidate("b", false), candidate("c", false, "Invalid anchor")],
  },
};
const draft: FindingSelectionDraft = { runId: 10, prId: 1, revision: 2, selectedIds: ["b"] };

describe("review point classification", () => {
  it.each([
    ["**[Question] Confirm**\n\n**[Finding] Fix**", "mixed"],
    ["**[Finding] Fix**\n\n**[Question] Confirm**", "mixed"],
    ["[Question] Confirm\n```text\n[Finding] Example\n```", "question"],
    ["[Finding] Fix\n~~~text\n[Question] Example\n~~~", "finding"],
    ["No findings.", "unknown"],
  ])("classifies all unfenced points: %s", (body, expected) => {
    expect(reviewPointKind(body)).toBe(expected);
  });
});

describe("finding groups", () => {
  it("separates eligible, discarded and blocked candidates without duplicating them", () => {
    const groups = groupFindingCandidates(view.audit!.candidates);
    expect(groups.map((group) => [group.id, group.candidates.map((item) => item.id)])).toEqual([
      ["eligible", ["a"]],
      ["discarded", ["b"]],
      ["blocked", ["c"]],
    ]);
    expect(groups.filter((group) => group.open).map((group) => group.id)).toEqual(["eligible"]);
  });
  it("does not label unknown or pending decisions as discards", () => {
    const groups = groupFindingCandidates([
      { ...candidate("a", false), aiDecision: "unmapped" },
      { ...candidate("b", false), aiDecision: "pending" },
    ]);
    expect(groups.map((group) => group.id)).toEqual(["unmapped", "pending"]);
  });
  it("keeps grouping stable when humans override AI selection", () => {
    const groups = groupFindingCandidates([
      { ...candidate("a", true), selected: false },
      { ...candidate("b", false), selected: true },
    ]);
    expect(groups.map((group) => [group.id, group.candidates[0].id])).toEqual([
      ["eligible", "a"],
      ["discarded", "b"],
    ]);
  });
});

describe("finding audit summary", () => {
  it("distinguishes unavailable history from zero discarded candidates", () => {
    expect(findingAuditSummary(undefined)).toBe("Loading candidate audit…");
    expect(findingAuditSummary({ ...view, audit: null })).toBe("Discard history unavailable for this run.");
    expect(findingAuditSummary({ ...view, audit: { ...view.audit!, candidates: [] } })).toBe(
      "0 candidates · 0 discarded by AI",
    );
  });
  it("counts explicit AI discards independently of human selection and blockers", () => {
    expect(findingAuditSummary(view)).toBe("3 candidates · 2 discarded by AI · 1 blocked");
    expect(
      findingAuditSummary({
        ...view,
        audit: { ...view.audit!, candidates: [{ ...candidate("a", true), aiDecision: "discarded" }] },
      }),
    ).toBe("1 candidate · 1 discarded by AI");
  });
  it("keeps missing decisions and incomplete audits explicit", () => {
    const candidates: FindingCandidate[] = [
      { ...candidate("a", false), aiDecision: "unmapped" },
      { ...candidate("b", false), aiDecision: "pending" },
    ];
    expect(findingAuditSummary({ ...view, audit: { ...view.audit!, complete: false, candidates } })).toBe(
      "2 candidates · 0 discarded by AI · 1 decision unavailable · 1 pending · incomplete audit",
    );
  });
});

describe("empty finding decision preparation", () => {
  const empty: FindingAuditView = { ...view, audit: { ...view.audit!, candidates: [] } };

  it("saves only the current empty audit and returns its updated revision", async () => {
    const saved = { ...empty, revision: 3, selectionSaved: true };
    const load = vi.fn().mockResolvedValue(empty);
    const save = vi.fn().mockResolvedValue(saved);
    expect(await prepareEmptyFindingDecision(1, 10, "abc", load, save)).toEqual(saved);
    expect(load).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledExactlyOnceWith(2);
  });

  it("reuses an already saved empty selection without writing again", async () => {
    const saved = { ...empty, selectionSaved: true };
    const save = vi.fn();
    expect(await prepareEmptyFindingDecision(1, 10, "abc", async () => saved, save)).toEqual(saved);
    expect(save).not.toHaveBeenCalled();
  });

  it.each<[string, FindingAuditView]>([
    ["missing audit", { ...empty, audit: null }],
    ["incomplete audit", { ...empty, audit: { ...empty.audit!, complete: false } }],
    ["read-only run", { ...empty, editable: false }],
    ["other PR", { ...empty, prId: 2 }],
    ["other run", { ...empty, runId: 11 }],
    ["new head", { ...empty, audit: { ...empty.audit!, headSha: "new" } }],
    [
      "existing discarded candidate",
      { ...empty, audit: { ...empty.audit!, candidates: [candidate("discarded", false)] } },
    ],
    [
      "existing blocked candidate",
      { ...empty, audit: { ...empty.audit!, candidates: [candidate("blocked", false, "Invalid anchor")] } },
    ],
  ])("never saves a selection for %s", async (_label, changed) => {
    const save = vi.fn();
    await expect(prepareEmptyFindingDecision(1, 10, "abc", async () => changed, save)).rejects.toThrow(
      "Review changed",
    );
    expect(save).not.toHaveBeenCalled();
  });

  it("rejects a changed or unconfirmed save response before any decision", async () => {
    for (const saved of [
      { ...empty, selectionSaved: false },
      { ...empty, selectionSaved: true, runId: 11 },
      { ...empty, selectionSaved: true, audit: { ...empty.audit!, candidates: [candidate("new", true)] } },
    ]) {
      const save = vi.fn().mockResolvedValue(saved);
      await expect(prepareEmptyFindingDecision(1, 10, "abc", async () => empty, save)).rejects.toThrow();
      expect(save).toHaveBeenCalledTimes(1);
    }
  });

  it("does not retry a conflicting save", async () => {
    const save = vi.fn().mockRejectedValue(new Error("Finding selection changed"));
    await expect(prepareEmptyFindingDecision(1, 10, "abc", async () => empty, save)).rejects.toThrow(
      "Finding selection changed",
    );
    expect(save).toHaveBeenCalledTimes(1);
  });
});

describe("finding selection confirmation", () => {
  it("requires confirmation for candidates but not for a complete empty audit", () => {
    expect(canDecideFindings(undefined, null)).toBe(false);
    expect(canDecideFindings(view, null)).toBe(false);
    expect(canDecideFindings({ ...view, selectionSaved: true }, null)).toBe(true);
    expect(canDecideFindings({ ...view, audit: { ...view.audit!, candidates: [] } }, null)).toBe(true);
    expect(canDecideFindings({ ...view, audit: { ...view.audit!, complete: false, candidates: [] } }, null)).toBe(
      false,
    );
    expect(canDecideFindings({ ...view, editable: false, audit: { ...view.audit!, candidates: [] } }, null)).toBe(
      false,
    );
  });
  it("blocks publication for unsaved overrides and stale revisions", () => {
    const saved = { ...view, selectionSaved: true };
    expect(canDecideFindings(saved, draft)).toBe(false);
    expect(canDecideFindings(saved, { ...draft, revision: 1, selectedIds: ["a"] })).toBe(false);
    expect(canDecideFindings(saved, { ...draft, selectedIds: ["a"] })).toBe(true);
  });
  it("keeps published and incomplete audits read-only", () => {
    expect(canDecideFindings({ ...view, selectionSaved: true, editable: false }, null)).toBe(false);
    expect(canDecideFindings({ ...view, selectionSaved: true, audit: { ...view.audit!, complete: false } }, null)).toBe(
      false,
    );
    expect(selectedFindingIds(view)).toEqual(["a"]);
  });
  it("does not mix drafts from other runs or mutate AI choices", () => {
    expect(findingSelectionDirty(view, { ...draft, runId: 11 })).toBe(false);
    expect(findingSelectionDirty(view, draft)).toBe(true);
    expect(view.audit!.candidates[0].aiDecision).toBe("kept");
    expect(view.audit!.candidates[1].aiDecision).toBe("discarded");
  });
});

describe("reviewBodyHasPostableFinding", () => {
  it("detects body-only findings and questions with markdown wrappers", () => {
    expect(reviewBodyHasPostableFinding("**[Finding] Issue**")).toBe(true);
    expect(reviewBodyHasPostableFinding("> **[Question] Intent?**")).toBe(true);
    expect(reviewBodyHasPostableFinding("No issues found.")).toBe(false);
  });
});
